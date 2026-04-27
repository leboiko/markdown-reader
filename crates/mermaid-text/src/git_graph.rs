//! Data model for Mermaid `gitGraph` diagrams.
//!
//! A git graph diagram represents a commit history across one or more branches,
//! rendered as a timeline flowing top-to-bottom with branch lanes as columns.
//!
//! Example source:
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
//!     commit tag: "v1.0"
//! ```
//!
//! Constructed by [`crate::parser::git_graph::parse`] and consumed by
//! [`crate::render::git_graph::render`].

/// The kind of a commit in a git graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitKind {
    /// An ordinary commit (glyph: `*`).
    Normal,
    /// A merge commit with two parents (glyph: `M`).
    Merge,
    /// A cherry-picked commit copied from another branch (glyph: `C`).
    CherryPick,
}

/// A single commit on a branch in the git graph.
///
/// `id` is the display identifier (auto-generated as `c0`, `c1`, … when the
/// source omits `id: "..."`). `branch` is the name of the branch this commit
/// lives on. `tag` is an optional label rendered next to the commit in `[...]`.
/// `parent` is the index into `GitGraph::commits` of the preceding commit on
/// the same branch (or `None` for the initial commit of `main`). `merge_parent`
/// is only set for `Merge` commits and points to the HEAD of the branch being
/// merged in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// Short display id (e.g. `"c0"`, `"second"`, `"feature-x"`).
    pub id: String,
    /// Name of the branch this commit belongs to.
    pub branch: String,
    /// Optional release / annotation tag rendered as `[tag]`.
    pub tag: Option<String>,
    /// Classification of this commit.
    pub kind: CommitKind,
    /// Index into `GitGraph::commits` of the direct parent (same-branch
    /// predecessor). `None` only for the very first commit of `main`.
    pub parent: Option<usize>,
    /// Index into `GitGraph::commits` of the merge-source HEAD.
    /// Only set when `kind == CommitKind::Merge`.
    pub merge_parent: Option<usize>,
}

/// A branch in the git graph.
///
/// `name` is the branch name. `created_after_commit` is the index into
/// `GitGraph::commits` of the commit that was HEAD on the parent branch when
/// this branch was created via `branch <name>`. It is `None` only for `main`,
/// which always exists from the start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub name: String,
    /// The commit (by index) from which this branch was forked, or `None`
    /// for `main` (the initial branch, which has no parent commit).
    pub created_after_commit: Option<usize>,
}

/// A source-ordered event in the git timeline.
///
/// Replaying `events` in order re-creates the exact sequence of operations
/// the author wrote, which the layout pass needs to position rows correctly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A commit was added (index into `GitGraph::commits`).
    Commit(usize),
    /// A new branch was created (index into `GitGraph::branches`).
    BranchCreated(usize),
    /// The active branch changed. Value is the branch name.
    Checkout(String),
    /// A merge was performed; the merge commit index is stored.
    Merge(usize),
    /// A cherry-pick was performed; the cherry-pick commit index is stored.
    CherryPick(usize),
}

/// A parsed `gitGraph` diagram.
///
/// `branches` lists all branches in creation order (`main` always first).
/// `commits` lists all commits in timeline order (the order they were emitted
/// by the source). `events` is the source-ordered operation log used by the
/// renderer to determine row ordering and glyph connections.
///
/// Constructed by [`crate::parser::git_graph::parse`] and consumed by
/// [`crate::render::git_graph::render`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitGraph {
    /// All branches in branch-creation order; `main` is always at index 0.
    pub branches: Vec<Branch>,
    /// All commits in timeline order (the order they appear in the source).
    pub commits: Vec<Commit>,
    /// Source-ordered event log for the layout pass to replay.
    pub events: Vec<Event>,
}

impl GitGraph {
    /// Return the lane index (0-based column) assigned to `branch_name`.
    ///
    /// Lanes are assigned in branch-creation order so `main` is always lane 0.
    /// Returns `None` if the branch does not exist.
    pub fn lane_of(&self, branch_name: &str) -> Option<usize> {
        self.branches.iter().position(|b| b.name == branch_name)
    }

    /// Return the index of the most recent commit on `branch_name`, scanning
    /// backwards through `commits` to find the last one on that branch.
    ///
    /// Returns `None` if no commit exists on the branch yet.
    pub fn head_of(&self, branch_name: &str) -> Option<usize> {
        self.commits
            .iter()
            .enumerate()
            .rev()
            .find(|(_, c)| c.branch == branch_name)
            .map(|(i, _)| i)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_graph() -> GitGraph {
        let mut g = GitGraph {
            branches: vec![
                Branch {
                    name: "main".to_string(),
                    created_after_commit: None,
                },
                Branch {
                    name: "develop".to_string(),
                    created_after_commit: Some(1),
                },
            ],
            ..Default::default()
        };
        // Commit 0: c0 on main, no parent
        g.commits.push(Commit {
            id: "c0".to_string(),
            branch: "main".to_string(),
            tag: None,
            kind: CommitKind::Normal,
            parent: None,
            merge_parent: None,
        });
        // Commit 1: c1 on main
        g.commits.push(Commit {
            id: "c1".to_string(),
            branch: "main".to_string(),
            tag: None,
            kind: CommitKind::Normal,
            parent: Some(0),
            merge_parent: None,
        });
        // Commit 2: c2 on develop, forked from c1
        g.commits.push(Commit {
            id: "c2".to_string(),
            branch: "develop".to_string(),
            tag: None,
            kind: CommitKind::Normal,
            parent: Some(1),
            merge_parent: None,
        });
        // Commit 3: merge commit on main
        g.commits.push(Commit {
            id: "c3".to_string(),
            branch: "main".to_string(),
            tag: Some("v1.0".to_string()),
            kind: CommitKind::Merge,
            parent: Some(1),
            merge_parent: Some(2),
        });
        g
    }

    // ---- (1) lane_of returns correct column indices -----------------------

    #[test]
    fn lane_of_returns_correct_indices() {
        let g = make_graph();
        assert_eq!(g.lane_of("main"), Some(0));
        assert_eq!(g.lane_of("develop"), Some(1));
        assert_eq!(g.lane_of("nonexistent"), None);
    }

    // ---- (2) head_of returns the last commit on a branch -----------------

    #[test]
    fn head_of_returns_latest_commit_on_branch() {
        let g = make_graph();
        // After the merge, the last commit on main is c3 (index 3).
        assert_eq!(g.head_of("main"), Some(3));
        // The last commit on develop is c2 (index 2).
        assert_eq!(g.head_of("develop"), Some(2));
        // Unknown branch returns None.
        assert_eq!(g.head_of("feature"), None);
    }

    // ---- (3) merge commit has both parent indices set --------------------

    #[test]
    fn merge_commit_has_both_parents() {
        let g = make_graph();
        let merge = &g.commits[3];
        assert_eq!(merge.kind, CommitKind::Merge);
        assert_eq!(merge.parent, Some(1));
        assert_eq!(merge.merge_parent, Some(2));
        assert_eq!(merge.tag.as_deref(), Some("v1.0"));
    }

    // ---- (4) default graph is empty -------------------------------------

    #[test]
    fn default_graph_is_empty() {
        let g = GitGraph::default();
        assert!(g.branches.is_empty());
        assert!(g.commits.is_empty());
        assert!(g.events.is_empty());
        assert_eq!(g.lane_of("main"), None);
        assert_eq!(g.head_of("main"), None);
    }

    // ---- (5) commit kind variants are distinct --------------------------

    #[test]
    fn commit_kind_variants_are_distinct() {
        assert_ne!(CommitKind::Normal, CommitKind::Merge);
        assert_ne!(CommitKind::Merge, CommitKind::CherryPick);
        assert_ne!(CommitKind::Normal, CommitKind::CherryPick);
    }
}
