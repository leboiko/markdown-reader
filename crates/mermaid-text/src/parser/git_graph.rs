//! Parser for Mermaid `gitGraph` diagrams.
//!
//! Accepted syntax (Phase 1):
//!
//! ```text
//! gitGraph
//!     commit
//!     commit id: "second"
//!     branch develop
//!     checkout develop
//!     commit
//!     commit id: "feature-x"
//!     checkout main
//!     merge develop
//!     commit
//!     commit tag: "v1.0"
//!     cherry-pick id: "feature-x"
//! ```
//!
//! **Parsing strategy.** The parser walks the source line by line, maintaining
//! mutable state for the current branch and an auto-increment commit counter
//! used to generate short ids (`c0`, `c1`, …) when none is supplied. Each
//! token is dispatched to a dedicated handler that appends to `GitGraph`.
//!
//! **Silently ignored.** The direction modifier on `gitGraph LR` and extended
//! commit attributes (`type: REVERSE`, `type: HIGHLIGHT`, `message:`,
//! `accTitle`, `accDescr`) are silently ignored for forward compatibility.
//!
//! # Examples
//!
//! ```
//! use mermaid_text::parser::git_graph::parse;
//!
//! let src = "gitGraph\n  commit\n  commit id: \"hello\"";
//! let g = parse(src).unwrap();
//! assert_eq!(g.commits.len(), 2);
//! assert_eq!(g.commits[1].id, "hello");
//! ```

use crate::Error;
use crate::git_graph::{Branch, Commit, CommitKind, Event, GitGraph};
use crate::parser::common::strip_inline_comment;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Parse a `gitGraph` source string into a [`GitGraph`].
///
/// # Errors
///
/// - [`Error::ParseError`] — missing `gitGraph` header, `checkout` of an
///   unknown branch, `cherry-pick` of an unknown commit id, or a `merge` of
///   an unknown branch.
pub fn parse(src: &str) -> Result<GitGraph, Error> {
    let mut graph = GitGraph::default();
    let mut header_seen = false;
    // Auto-increment counter for generating commit ids `c0`, `c1`, …
    let mut commit_counter: usize = 0;
    // The name of the current branch (the branch commits land on).
    let mut current_branch = "main".to_string();

    // main always exists from the start.
    graph.branches.push(Branch {
        name: "main".to_string(),
        created_after_commit: None,
    });

    for raw_line in src.lines() {
        let line = strip_inline_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        if !header_seen {
            // First non-blank line must begin with "gitGraph" (case-sensitive
            // per Mermaid spec — camelCase matters here).
            if !line.starts_with("gitGraph") {
                return Err(Error::ParseError(format!(
                    "expected `gitGraph` header, got {line:?}"
                )));
            }
            header_seen = true;
            continue;
        }

        // Dispatch on the first token of the line.
        let first = line.split_whitespace().next().unwrap_or("");
        match first {
            "commit" => {
                handle_commit(line, &mut graph, &current_branch, &mut commit_counter)?;
            }
            "branch" => {
                // Clone current_branch so we can pass it immutably while also
                // passing &mut current_branch. The clone is cheap (short branch
                // names); this is the Rust-idiomatic way to break the aliasing.
                let cb = current_branch.clone();
                handle_branch(line, &mut graph, &cb, &mut current_branch)?;
            }
            "checkout" => {
                handle_checkout(line, &mut graph, &mut current_branch)?;
            }
            "merge" => {
                handle_merge(line, &mut graph, &current_branch, &mut commit_counter)?;
            }
            "cherry-pick" => {
                handle_cherry_pick(line, &mut graph, &current_branch, &mut commit_counter)?;
            }
            // Silently ignore accessibility metadata and other unknown directives
            // so real-world diagrams with extra annotations don't fail.
            _ => {}
        }
    }

    if !header_seen {
        return Err(Error::ParseError(
            "missing `gitGraph` header line".to_string(),
        ));
    }

    Ok(graph)
}

// ---------------------------------------------------------------------------
// Token handlers
// ---------------------------------------------------------------------------

/// Handle a `commit` line.
///
/// Supported optional attributes (parsed from `key: "value"` pairs on the line):
/// - `id: "..."` — explicit commit id
/// - `tag: "..."` — annotation tag
/// - `type: ...` — silently ignored (REVERSE, HIGHLIGHT, etc.)
/// - `message:` — silently ignored
fn handle_commit(
    line: &str,
    graph: &mut GitGraph,
    current_branch: &str,
    counter: &mut usize,
) -> Result<(), Error> {
    // Extract optional id and tag from the line.
    let id = extract_quoted_attr(line, "id").unwrap_or_else(|| {
        // Auto-generate a short id when none is provided.
        let auto = format!("c{counter}");
        auto
    });
    let tag = extract_quoted_attr(line, "tag");

    // Increment counter regardless of whether we used it — this keeps the
    // auto-id sequence monotonically increasing even when explicit ids appear.
    *counter += 1;

    // Parent is the last commit on this branch. If no commits exist on this
    // branch yet (first commit after a `branch X` + `checkout X`), inherit
    // the fork point: the commit that was HEAD when the branch was created.
    // This matches git's model where the first commit on a new branch has the
    // fork-point commit as its parent.
    let parent = graph.head_of(current_branch).or_else(|| {
        graph
            .branches
            .iter()
            .find(|b| b.name == current_branch)
            .and_then(|b| b.created_after_commit)
    });
    let commit_idx = graph.commits.len();

    graph.commits.push(Commit {
        id,
        branch: current_branch.to_string(),
        tag,
        kind: CommitKind::Normal,
        parent,
        merge_parent: None,
    });
    graph.events.push(Event::Commit(commit_idx));
    Ok(())
}

/// Handle a `branch <name>` line.
///
/// Creates the branch from the current branch's HEAD and switches to it.
/// The new branch becomes the current branch after creation, matching Mermaid's
/// semantics (a `branch X` implicitly checks out X).
fn handle_branch(
    line: &str,
    graph: &mut GitGraph,
    current_branch: &str,
    new_current: &mut String,
) -> Result<(), Error> {
    let name = rest_after_keyword(line, "branch").trim().to_string();
    if name.is_empty() {
        return Err(Error::ParseError("branch: missing branch name".to_string()));
    }

    // If the branch already exists, treat as a no-op (idempotent).
    if graph.lane_of(&name).is_some() {
        *new_current = name;
        return Ok(());
    }

    let created_after = graph.head_of(current_branch);
    let branch_idx = graph.branches.len();
    graph.branches.push(Branch {
        name: name.clone(),
        created_after_commit: created_after,
    });
    graph.events.push(Event::BranchCreated(branch_idx));
    // Mermaid's `branch X` implicitly checks out the new branch.
    graph.events.push(Event::Checkout(name.clone()));
    *new_current = name;
    Ok(())
}

/// Handle a `checkout <name>` line.
///
/// Switches the current branch; errors if the branch does not exist.
fn handle_checkout(
    line: &str,
    graph: &mut GitGraph,
    current_branch: &mut String,
) -> Result<(), Error> {
    let name = rest_after_keyword(line, "checkout").trim().to_string();
    if name.is_empty() {
        return Err(Error::ParseError(
            "checkout: missing branch name".to_string(),
        ));
    }
    if graph.lane_of(&name).is_none() {
        return Err(Error::ParseError(format!(
            "checkout: branch {name:?} does not exist"
        )));
    }
    graph.events.push(Event::Checkout(name.clone()));
    *current_branch = name;
    Ok(())
}

/// Handle a `merge <branch>` line.
///
/// Creates a merge commit on the current branch with `merge_parent` pointing
/// to the HEAD of the merged branch. Errors if the branch does not exist or
/// has no commits.
fn handle_merge(
    line: &str,
    graph: &mut GitGraph,
    current_branch: &str,
    counter: &mut usize,
) -> Result<(), Error> {
    let source_name = rest_after_keyword(line, "merge").trim().to_string();
    if source_name.is_empty() {
        return Err(Error::ParseError("merge: missing branch name".to_string()));
    }
    if graph.lane_of(&source_name).is_none() {
        return Err(Error::ParseError(format!(
            "merge: branch {source_name:?} does not exist"
        )));
    }

    // The merge-source HEAD: the last commit on the source branch.
    let merge_parent = graph.head_of(&source_name).ok_or_else(|| {
        Error::ParseError(format!(
            "merge: branch {source_name:?} has no commits to merge"
        ))
    })?;

    let id = extract_quoted_attr(line, "id").unwrap_or_else(|| {
        let auto = format!("c{counter}");
        auto
    });
    let tag = extract_quoted_attr(line, "tag");
    *counter += 1;

    let parent = graph.head_of(current_branch).or_else(|| {
        graph
            .branches
            .iter()
            .find(|b| b.name == current_branch)
            .and_then(|b| b.created_after_commit)
    });
    let commit_idx = graph.commits.len();

    graph.commits.push(Commit {
        id,
        branch: current_branch.to_string(),
        tag,
        kind: CommitKind::Merge,
        parent,
        merge_parent: Some(merge_parent),
    });
    graph.events.push(Event::Merge(commit_idx));
    Ok(())
}

/// Handle a `cherry-pick id: "..."` line.
///
/// Copies a commit by id to the current branch. Errors if the id is not found
/// in `commits`.
fn handle_cherry_pick(
    line: &str,
    graph: &mut GitGraph,
    current_branch: &str,
    counter: &mut usize,
) -> Result<(), Error> {
    let source_id = extract_quoted_attr(line, "id")
        .ok_or_else(|| Error::ParseError("cherry-pick: missing id attribute".to_string()))?;

    // Verify the commit id exists.
    if !graph.commits.iter().any(|c| c.id == source_id) {
        return Err(Error::ParseError(format!(
            "cherry-pick: commit id {source_id:?} not found"
        )));
    }

    let id = format!("c{counter}");
    *counter += 1;

    let parent = graph.head_of(current_branch).or_else(|| {
        graph
            .branches
            .iter()
            .find(|b| b.name == current_branch)
            .and_then(|b| b.created_after_commit)
    });
    let commit_idx = graph.commits.len();

    graph.commits.push(Commit {
        id,
        branch: current_branch.to_string(),
        tag: None,
        kind: CommitKind::CherryPick,
        parent,
        merge_parent: None,
    });
    graph.events.push(Event::CherryPick(commit_idx));
    Ok(())
}

// ---------------------------------------------------------------------------
// Attribute parsing helpers
// ---------------------------------------------------------------------------

/// Extract a quoted attribute value from a line: `key: "value"`.
///
/// Returns the content between the quotes, or `None` if the attribute is
/// not present or has no quoted value. The search is case-sensitive.
///
/// Example: `extract_quoted_attr("commit id: \"hello\"", "id")` → `Some("hello")`.
fn extract_quoted_attr(line: &str, key: &str) -> Option<String> {
    // Look for `key:` (with optional space before the quote).
    let needle = format!("{key}:");
    let start = line.find(needle.as_str())?;
    let after_colon = &line[start + needle.len()..];
    // Find the opening quote.
    let open = after_colon.find('"')?;
    let rest = &after_colon[open + 1..];
    // Find the closing quote.
    let close = rest.find('"')?;
    Some(rest[..close].to_string())
}

/// Return the remainder of `line` after `keyword` and at least one space.
///
/// If the keyword is not matched (e.g. the line is exactly the keyword with
/// no trailing content), returns an empty string.
fn rest_after_keyword<'a>(line: &'a str, keyword: &str) -> &'a str {
    let klen = keyword.len();
    if line.len() > klen && line.as_bytes()[klen].is_ascii_whitespace() {
        &line[klen + 1..]
    } else {
        ""
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_graph::CommitKind;

    // ---- (1) minimal: just gitGraph + 2 commits → 2 commits on main -------

    #[test]
    fn minimal_two_commits_on_main() {
        let src = "gitGraph\n  commit\n  commit";
        let g = parse(src).unwrap();
        assert_eq!(g.commits.len(), 2);
        assert_eq!(g.commits[0].branch, "main");
        assert_eq!(g.commits[1].branch, "main");
        assert_eq!(g.commits[0].parent, None);
        assert_eq!(g.commits[1].parent, Some(0));
    }

    // ---- (2) branch + checkout + commit on new branch ---------------------

    #[test]
    fn branch_checkout_commit_on_new_branch() {
        let src = "gitGraph\n  commit\n  branch dev\n  checkout dev\n  commit";
        let g = parse(src).unwrap();
        // The second commit should be on dev.
        assert_eq!(g.commits[1].branch, "dev");
        // Its parent is the first commit on main (index 0).
        assert_eq!(g.commits[1].parent, Some(0));
        assert_eq!(g.branches.len(), 2);
        assert_eq!(g.branches[1].name, "dev");
    }

    // ---- (3) merge creates a merge commit with merge_parent set ----------

    #[test]
    fn merge_creates_merge_commit_with_merge_parent() {
        let src = "gitGraph\n  commit\n  branch dev\n  checkout dev\n  commit id: \"feat\"\n  checkout main\n  merge dev";
        let g = parse(src).unwrap();
        let merge = g.commits.last().unwrap();
        assert_eq!(merge.kind, CommitKind::Merge);
        assert_eq!(merge.branch, "main");
        // merge_parent must point to the HEAD of dev (commit with id "feat").
        let feat_idx = g.commits.iter().position(|c| c.id == "feat").unwrap();
        assert_eq!(merge.merge_parent, Some(feat_idx));
    }

    // ---- (4) explicit commit id honours the id ---------------------------

    #[test]
    fn explicit_commit_id_is_used() {
        let src = "gitGraph\n  commit id: \"my-commit\"";
        let g = parse(src).unwrap();
        assert_eq!(g.commits[0].id, "my-commit");
    }

    // ---- (5) tag populates the commit's tag field -------------------------

    #[test]
    fn tag_populates_commit_tag() {
        let src = "gitGraph\n  commit tag: \"v1.0\"";
        let g = parse(src).unwrap();
        assert_eq!(g.commits[0].tag.as_deref(), Some("v1.0"));
    }

    // ---- (6) cherry-pick creates a CherryPick commit ---------------------

    #[test]
    fn cherry_pick_creates_cherry_pick_commit() {
        let src = "gitGraph\n  commit id: \"feat\"\n  branch dev\n  checkout dev\n  cherry-pick id: \"feat\"";
        let g = parse(src).unwrap();
        let cp = g.commits.last().unwrap();
        assert_eq!(cp.kind, CommitKind::CherryPick);
        assert_eq!(cp.branch, "dev");
    }

    // ---- (7) branching off a non-main branch -----------------------------

    #[test]
    fn branch_off_non_main_branch() {
        let src = "gitGraph\n  commit\n  branch dev\n  checkout dev\n  commit id: \"d1\"\n  branch feature\n  checkout feature\n  commit id: \"f1\"";
        let g = parse(src).unwrap();
        // feature branch was created when dev HEAD was "d1" (index 1).
        let feature = g.branches.iter().find(|b| b.name == "feature").unwrap();
        let d1_idx = g.commits.iter().position(|c| c.id == "d1").unwrap();
        assert_eq!(feature.created_after_commit, Some(d1_idx));
        // f1 commit's parent is d1 (since feature was checked out from dev).
        let f1 = g.commits.iter().find(|c| c.id == "f1").unwrap();
        assert_eq!(f1.parent, Some(d1_idx));
    }

    // ---- (8) %% comments stripped -----------------------------------------

    #[test]
    fn comments_stripped() {
        let src = "%% header comment\ngitGraph\n  %% inline comment line\n  commit %% trailing";
        let g = parse(src).unwrap();
        assert_eq!(g.commits.len(), 1);
    }

    // ---- (9) multiple branches with interleaved commits ------------------

    #[test]
    fn multiple_branches_interleaved_commits() {
        let src = "gitGraph\n  commit id: \"m1\"\n  branch dev\n  checkout dev\n  commit id: \"d1\"\n  checkout main\n  commit id: \"m2\"\n  checkout dev\n  commit id: \"d2\"";
        let g = parse(src).unwrap();
        // m1, d1, m2, d2 in order.
        let ids: Vec<&str> = g.commits.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["m1", "d1", "m2", "d2"]);
        assert_eq!(g.commits[3].parent, Some(1)); // d2's parent is d1
    }

    // ---- (10) merge of a branch that has its own branch ------------------

    #[test]
    fn merge_picks_correct_head_when_branch_has_sub_branches() {
        // develop branches off main, feature branches off develop,
        // feature gets a commit, then main merges develop (not feature).
        let src = "gitGraph\n  commit id: \"m1\"\n  branch develop\n  checkout develop\n  commit id: \"dev1\"\n  branch feature\n  checkout feature\n  commit id: \"feat1\"\n  checkout main\n  merge develop";
        let g = parse(src).unwrap();
        let merge = g.commits.last().unwrap();
        assert_eq!(merge.kind, CommitKind::Merge);
        // merge_parent must be the HEAD of develop, which is dev1 (not feat1).
        let dev1_idx = g.commits.iter().position(|c| c.id == "dev1").unwrap();
        assert_eq!(merge.merge_parent, Some(dev1_idx));
    }

    // ---- (11) auto-generated ids are sequential --------------------------

    #[test]
    fn auto_generated_ids_are_sequential() {
        let src = "gitGraph\n  commit\n  commit\n  commit id: \"explicit\"\n  commit";
        let g = parse(src).unwrap();
        // First two and last get auto-ids c0, c1, c3 (counter increments even
        // for explicit-id commits, keeping the sequence monotone).
        assert_eq!(g.commits[0].id, "c0");
        assert_eq!(g.commits[1].id, "c1");
        assert_eq!(g.commits[2].id, "explicit");
        assert_eq!(g.commits[3].id, "c3");
    }

    // ---- (12) missing header returns parse error -------------------------

    #[test]
    fn missing_header_returns_error() {
        let err = parse("commit").unwrap_err();
        assert!(err.to_string().contains("gitGraph"));
    }

    // ---- (13) checkout of unknown branch returns parse error ------------

    #[test]
    fn checkout_unknown_branch_returns_error() {
        let src = "gitGraph\n  checkout ghost";
        let err = parse(src).unwrap_err();
        assert!(err.to_string().contains("ghost"), "unexpected error: {err}");
    }
}
