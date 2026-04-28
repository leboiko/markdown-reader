//! Renderer for [`GitGraph`]. Produces a Unicode lane-diagram string.
//!
//! **Layout.**
//!
//! Each branch occupies a vertical "lane" (a column). Lanes are assigned in
//! branch-creation order: `main` is lane 0, the first `branch X` is lane 1,
//! etc. Time flows top-to-bottom; each commit occupies one row.
//!
//! **Glyph alphabet** (geometric line-drawing characters — not emoji):
//!
//! | Glyph | Meaning                              |
//! |-------|--------------------------------------|
//! | `*`   | Normal commit                        |
//! | `M`   | Merge commit                         |
//! | `C`   | Cherry-pick commit                   |
//! | `│`   | Vertical lane continuation           |
//! | `╭`   | Branch fork connector (parent lane)  |
//! | `╮`   | Branch fork connector (child lane)   |
//! | `╯`   | Merge incoming connector (src lane)  |
//! | `╰`   | Merge incoming connector (dst lane)  |
//! | `─`   | Horizontal connector segment         |
//!
//! **Row anatomy.** Each output row consists of lane columns separated by a
//! single space. The lane column width is 1 character (`*`, `M`, `C`, `│`, or
//! space for an inactive lane). After all lane columns a label field appears:
//! `id [tag]` where `[tag]` is omitted when the commit has no tag. A
//! connector row (branch fork or merge) emits glyphs across the two lanes
//! involved and a horizontal `─` fill between them.
//!
//! **Bottom labels.** After all commit rows, a label row prints each branch
//! name centred under its lane column.
//!
//! **`max_width` handling.** When `max_width` is `Some(n)`, commit ids that
//! would push the label column past the budget are truncated with `…`.

use crate::git_graph::{CommitKind, Event, GitGraph};

/// Glyph for a normal commit.
const GLYPH_NORMAL: char = '*';
/// Glyph for a merge commit.
const GLYPH_MERGE: char = 'M';
/// Glyph for a cherry-pick commit.
const GLYPH_CHERRY: char = 'C';
/// Vertical lane continuation character.
const LANE_VERT: char = '│';
/// Horizontal connector fill.
const CONN_HORIZ: char = '─';
/// Top-left fork: the lane that forks INTO the new branch.
const CONN_FORK_LEFT: char = '╭';
/// Top-right fork: position on the new (right) branch.
const CONN_FORK_RIGHT: char = '╮';
/// Bottom-right merge source: the lane being merged away FROM.
const CONN_MERGE_SRC: char = '╯';
/// Bottom-left merge destination: the lane receiving the merge.
const CONN_MERGE_DST: char = '╰';

/// Render a [`GitGraph`] to a Unicode string.
///
/// # Arguments
///
/// * `diag`      — the parsed git graph
/// * `max_width` — optional column budget; when `Some(N)` commit ids are
///   truncated with `…` so the label column stays within the budget.
///
/// # Returns
///
/// A multi-line string ready for printing. Branch names appear at the bottom,
/// one per lane column.
pub fn render(diag: &GitGraph, max_width: Option<usize>) -> String {
    if diag.branches.is_empty() {
        return String::new();
    }

    let lane_count = diag.branches.len();
    // Each lane column is 1 char wide; columns are separated by a single space.
    // Total lane section width: lane_count + (lane_count - 1) spaces.
    let lane_section_width = lane_count + (lane_count.saturating_sub(1));

    // Label column starts after the lane section + 2-space gap.
    let label_offset = lane_section_width + 2;

    // The label budget: chars available for the "id [tag]" field.
    let label_budget = max_width.map(|w| w.saturating_sub(label_offset));

    let mut out = String::new();

    // Track which lanes are "alive" (have been created). A lane becomes alive
    // when its branch is created and stays alive until end-of-output.
    // Lane 0 (main) is always alive from the start.
    let mut alive: Vec<bool> = vec![false; lane_count];
    alive[0] = true;

    for event in &diag.events {
        match event {
            Event::Commit(idx) | Event::Merge(idx) | Event::CherryPick(idx) => {
                let commit = &diag.commits[*idx];
                let lane = diag.lane_of(&commit.branch).unwrap_or(0);

                // If this is a merge commit, emit a connector row first showing
                // where the merge source lane joins this lane.
                if commit.kind == CommitKind::Merge
                    && let Some(mp_idx) = commit.merge_parent
                {
                    let src_lane = diag.lane_of(&diag.commits[mp_idx].branch).unwrap_or(0);
                    if src_lane != lane {
                        // Emit a merge-arc row connecting src_lane → lane.
                        // The merge destination is lane (left), source is src_lane (right),
                        // assuming source lanes come from branches to the right.
                        out.push_str(&render_arc_row(
                            lane_count,
                            &alive,
                            lane,
                            src_lane,
                            CONN_MERGE_DST,
                            CONN_MERGE_SRC,
                        ));
                        out.push('\n');
                    }
                }

                // Emit the commit row.
                let glyph = commit_glyph(commit.kind);
                let row = render_commit_row(
                    lane_count,
                    &alive,
                    lane,
                    glyph,
                    &commit.id,
                    commit.tag.as_deref(),
                    label_budget,
                );
                out.push_str(&row);
                out.push('\n');
            }

            Event::BranchCreated(branch_idx) => {
                let branch = &diag.branches[*branch_idx];
                let new_lane = *branch_idx; // branches are in creation order

                // The parent lane is the lane of the branch from which this
                // branch was forked. We find it by looking at created_after_commit.
                // WHY: a branch always forks from wherever the current branch HEAD
                // was at the time `branch X` was issued; that commit's branch
                // gives us the parent lane.
                let parent_lane = branch
                    .created_after_commit
                    .and_then(|ci| diag.lane_of(&diag.commits[ci].branch))
                    .unwrap_or(0);

                // Mark this lane as alive before emitting the fork row.
                if new_lane < alive.len() {
                    alive[new_lane] = true;
                }

                // Emit the fork arc row: parent lane forks into the new lane.
                out.push_str(&render_arc_row(
                    lane_count,
                    &alive,
                    parent_lane,
                    new_lane,
                    CONN_FORK_LEFT,
                    CONN_FORK_RIGHT,
                ));
                out.push('\n');
            }

            Event::Checkout(_) => {
                // Checkout events carry no visible row — they only change state.
                // (State is tracked implicitly via the current_branch in the parser;
                // the renderer uses commit.branch directly.)
            }
        }
    }

    // Branch label row — one label per lane, space-separated to align under lanes.
    out.push_str(&render_label_row(diag));

    // Trim trailing newline if present (the label row does not add one).
    out
}

// ---------------------------------------------------------------------------
// Row builders
// ---------------------------------------------------------------------------

/// Build a commit row for a single commit at `lane`.
///
/// Layout per row: one char per lane, separated by spaces, then 2 spaces,
/// then the label (`id [tag]`).
///
/// Lanes that are alive but not the commit's lane show `│`; the commit's
/// lane shows `glyph`; dead lanes show ` `.
fn render_commit_row(
    lane_count: usize,
    alive: &[bool],
    commit_lane: usize,
    glyph: char,
    id: &str,
    tag: Option<&str>,
    label_budget: Option<usize>,
) -> String {
    let lane_part = build_lane_part(lane_count, alive, |lane| {
        if lane == commit_lane {
            glyph
        } else if alive[lane] {
            LANE_VERT
        } else {
            ' '
        }
    });

    let label = build_label(id, tag, label_budget);
    format!("{lane_part}  {label}")
}

/// Build an arc connector row (fork or merge) between two lanes.
///
/// `left_lane` receives `left_glyph`; `right_lane` receives `right_glyph`.
/// Lanes between them show `─`; all other alive lanes show `│`.
///
/// WHY: the arc row visually connects two adjacent (or distant) lanes with a
/// horizontal bridge. For fork rows the parent lane goes on the left and the
/// new branch on the right (since branches are added to the right). For merge
/// rows the destination is on the left and the source on the right.
///
/// NOTE: this function builds the row character-by-character including the
/// inter-lane separator positions. Within the horizontal span `[lo, hi]` the
/// separators between lane columns are `─` (continuing the bridge); outside
/// the span they remain spaces, matching the rest of the diagram.
fn render_arc_row(
    lane_count: usize,
    alive: &[bool],
    left_lane: usize,
    right_lane: usize,
    left_glyph: char,
    right_glyph: char,
) -> String {
    let lo = left_lane.min(right_lane);
    let hi = left_lane.max(right_lane);

    // Build the row manually so we can emit `─` for the inter-lane separator
    // cells that fall inside the horizontal arc span [lo, hi].  The standard
    // `build_lane_part` always uses a space separator, which leaves a visible
    // gap between the corner glyphs even for immediately adjacent lanes.
    let mut s = String::with_capacity(lane_count * 2);
    for lane in 0..lane_count {
        if lane > 0 {
            // Separator cell: `─` inside the arc span, space outside.
            if lane > lo && lane <= hi {
                s.push(CONN_HORIZ);
            } else {
                s.push(' ');
            }
        }
        let ch = if lane < alive.len() {
            if lane == lo {
                if lo == left_lane {
                    left_glyph
                } else {
                    right_glyph
                }
            } else if lane == hi {
                if hi == right_lane {
                    right_glyph
                } else {
                    left_glyph
                }
            } else if lane > lo && lane < hi {
                // Interior lane: horizontal fill.
                CONN_HORIZ
            } else if alive[lane] {
                LANE_VERT
            } else {
                ' '
            }
        } else {
            ' '
        };
        s.push(ch);
    }
    s
}

/// Build the bottom label row listing branch names under their lanes.
///
/// Each lane column is 1 char wide with a single space between lanes. Branch
/// names are placed directly at each lane's starting position. When names
/// overlap (because adjacent branch names are wider than the inter-lane gap)
/// they are separated by a single space instead, keeping the output readable.
///
/// WHY: a simple "place at lane position" approach causes shorter names to be
/// overwritten by wider neighbors. Instead we build the label row incrementally
/// left-to-right, tracking the write cursor and advancing past the previous
/// label before appending the next.
fn render_label_row(diag: &GitGraph) -> String {
    if diag.branches.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    // `cursor` tracks the number of display characters written so far.
    let mut cursor: usize = 0;

    for (i, branch) in diag.branches.iter().enumerate() {
        // The column position for this lane in the lane-section layout is i * 2
        // (1 char per lane + 1 space separator).
        let lane_pos = i * 2;

        if cursor < lane_pos {
            // Pad with spaces to reach the lane position.
            let pad = lane_pos - cursor;
            for _ in 0..pad {
                out.push(' ');
            }
            cursor = lane_pos;
        } else if cursor > lane_pos && i > 0 {
            // Previous label spilled past this lane's position; add a single
            // space separator to visually distinguish the names.
            out.push(' ');
            cursor += 1;
        }

        out.push_str(&branch.name);
        cursor += branch.name.len();
    }

    out
}

// ---------------------------------------------------------------------------
// Lane rendering helpers
// ---------------------------------------------------------------------------

/// Build the lane section of a row using a per-lane mapping function.
///
/// Returns a string of `lane_count` lane characters separated by single
/// spaces. The mapping function receives the lane index and returns the
/// character to place at that position.
fn build_lane_part(lane_count: usize, alive: &[bool], mut f: impl FnMut(usize) -> char) -> String {
    let mut s = String::with_capacity(lane_count * 2);
    for i in 0..lane_count {
        if i > 0 {
            // Separator: if both the current lane and the previous are in the
            // horizontal span of an arc, the separator itself becomes `─`.
            // In this function we simply use space; the arc builder handles
            // the `─` between endpoints differently (it applies to lane slots,
            // not separators). This keeps the separator logic simple and the
            // ASCII/Unicode rendering consistent.
            s.push(' ');
        }
        let ch = if i < alive.len() { f(i) } else { ' ' };
        s.push(ch);
    }
    s
}

/// Build the label string for a commit row: `"id"` or `"id [tag]"`.
///
/// If `label_budget` is `Some(n)` and the label would exceed `n` chars, the
/// id is truncated with `…` to make it fit.
fn build_label(id: &str, tag: Option<&str>, budget: Option<usize>) -> String {
    let full = match tag {
        Some(t) => format!("{id} [{t}]"),
        None => id.to_string(),
    };
    match budget {
        None => full,
        Some(b) => {
            if full.len() <= b {
                full
            } else if b == 0 {
                String::new()
            } else {
                // Truncate: take at most b-1 chars from the id plus `…`.
                let chars: Vec<char> = full.chars().collect();
                let take = b.saturating_sub(1);
                let truncated: String = chars.into_iter().take(take).collect();
                format!("{truncated}\u{2026}") // …
            }
        }
    }
}

/// Map a [`CommitKind`] to its display glyph.
fn commit_glyph(kind: CommitKind) -> char {
    match kind {
        CommitKind::Normal => GLYPH_NORMAL,
        CommitKind::Merge => GLYPH_MERGE,
        CommitKind::CherryPick => GLYPH_CHERRY,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::git_graph::parse;

    // ---- (1) single-branch linear history renders as a vertical chain -----

    #[test]
    fn single_branch_linear_history() {
        let src = "gitGraph\n  commit id: \"a\"\n  commit id: \"b\"\n  commit id: \"c\"";
        let g = parse(src).unwrap();
        let out = render(&g, None);

        // Every commit glyph must appear.
        let commit_count = out
            .lines()
            .filter(|l| l.trim_start().starts_with('*'))
            .count();
        assert_eq!(commit_count, 3, "expected 3 commit rows:\n{out}");

        // All ids must appear.
        assert!(out.contains("a"), "id 'a' missing:\n{out}");
        assert!(out.contains("b"), "id 'b' missing:\n{out}");
        assert!(out.contains("c"), "id 'c' missing:\n{out}");
    }

    // ---- (2) two-branch fork shows lane separation -----------------------

    #[test]
    fn two_branch_fork_shows_lanes() {
        let src = "gitGraph\n  commit\n  branch dev\n  checkout dev\n  commit id: \"d1\"";
        let g = parse(src).unwrap();
        let out = render(&g, None);

        // There must be a fork connector row (contains CONN_FORK_LEFT or similar).
        let has_fork = out.contains(CONN_FORK_LEFT) || out.contains(CONN_FORK_RIGHT);
        assert!(has_fork, "no fork connector found:\n{out}");

        // Both main and dev should appear in the label row.
        assert!(out.contains("main"), "main label missing:\n{out}");
        assert!(out.contains("dev"), "dev label missing:\n{out}");

        // The dev commit must appear somewhere.
        assert!(out.contains("d1"), "dev commit id missing:\n{out}");
    }

    // ---- (3) merge renders both incoming arrows --------------------------

    #[test]
    fn merge_renders_merge_glyph_and_arc() {
        let src = "gitGraph\n  commit\n  branch dev\n  checkout dev\n  commit id: \"feat\"\n  checkout main\n  merge dev";
        let g = parse(src).unwrap();
        let out = render(&g, None);

        // The merge glyph must appear.
        assert!(
            out.lines().any(|l| l.contains(GLYPH_MERGE)),
            "no merge glyph 'M' found:\n{out}"
        );
        // The merge arc must contain the merge-incoming connectors.
        let has_arc = out.contains(CONN_MERGE_SRC) || out.contains(CONN_MERGE_DST);
        assert!(has_arc, "no merge arc connector found:\n{out}");
    }

    // ---- (4) tag appears in output ---------------------------------------

    #[test]
    fn tag_appears_in_output() {
        let src = "gitGraph\n  commit tag: \"v1.0\"";
        let g = parse(src).unwrap();
        let out = render(&g, None);
        assert!(out.contains("v1.0"), "tag 'v1.0' not found:\n{out}");
        assert!(out.contains('['), "tag bracket missing:\n{out}");
    }

    // ---- (5) empty diagram returns empty string -------------------------

    #[test]
    fn empty_graph_returns_empty_string() {
        let g = GitGraph::default();
        let out = render(&g, None);
        assert!(out.is_empty());
    }

    // ---- (6) max_width truncates long ids --------------------------------

    #[test]
    fn max_width_truncates_long_id() {
        let src = "gitGraph\n  commit id: \"very-long-commit-identifier-here\"";
        let g = parse(src).unwrap();
        // Use a very narrow budget so truncation must happen.
        let out = render(&g, Some(12));
        // The output must not contain the full id.
        assert!(
            !out.contains("very-long-commit-identifier-here"),
            "full id not truncated:\n{out}"
        );
        // But it must contain the truncation ellipsis.
        assert!(out.contains('\u{2026}'), "ellipsis not found:\n{out}");
    }

    // ---- (7) fork arc has horizontal connector between corners -----------

    #[test]
    fn fork_arc_has_horizontal_connector() {
        // A fork arc between two immediately adjacent lanes (main=0, dev=1)
        // must have a `─` between the corner glyphs — no gap allowed.
        let src = "gitGraph
  commit id: \"first\"
  branch dev
  checkout dev
  commit id: \"d1\"
  checkout main
  merge dev";
        let g = parse(src).unwrap();
        let out = render(&g, None);

        // Find the fork row: contains CONN_FORK_LEFT.
        let fork_row = out
            .lines()
            .find(|l| l.contains(CONN_FORK_LEFT))
            .expect("no fork-arc row found in output");

        // The fork row must contain a `─` connector (not be a bare `╰ ╮`).
        assert!(
            fork_row.contains(CONN_HORIZ),
            "fork-arc row has no horizontal connector `─`; got: {fork_row:?}"
        );

        // Also verify the merge arc has a connector.
        let merge_row = out
            .lines()
            .find(|l| l.contains(CONN_MERGE_DST))
            .expect("no merge-arc row found in output");
        assert!(
            merge_row.contains(CONN_HORIZ),
            "merge-arc row has no horizontal connector `─`; got: {merge_row:?}"
        );
    }
}
