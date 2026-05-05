//! Post-routing nudging pass.
//!
//! Reads the routes produced by `router::route_all` and re-shifts
//! segments to merge co-directional parallel back-edge corridors
//! (Bug 5). Phase D will add corner-displacement for non-endpoint
//! halo cells (Bug 4).
//!
//! The pass operates on path data, not on A\* cost classes. Routing-
//! time cost tweaks ripple into specific cells with load-bearing
//! direction-bit conventions (e.g. the `┴` exit stub below a
//! state-diagram back-edge source); the post-routing approach
//! preserves those conventions because it only moves BEND cells —
//! exit stubs have only one axis of bits and are therefore not
//! candidates for any shift.
//!
//! Algorithm (Phase C — parallel-merge):
//! 1. Collect back-edge horizontal-segment occupancy per row.
//! 2. For two back-edges with horizontal segments on adjacent rows
//!    (≤ MAX_NUDGE_DISTANCE) AND overlapping col ranges, plan a
//!    shift that moves the shorter-path back-edge's corridor onto
//!    the other's row.
//! 3. Verify feasibility: every cell of the new path must be Free
//!    or already EdgeOccupied (cross is OK), never NodeBox.
//! 4. Apply: erase the old path's bits, regenerate the new path,
//!    draw the new path. Atomic per-shift to avoid race conditions
//!    between two shifts.

use crate::layout::Grid;

/// Maximum row/col delta between two segments for them to be
/// considered "adjacent" and candidate for merging. The Bug 5
/// fixture has back-edges 1 row apart; tolerating up to 3 covers
/// fixtures with thicker corridor zones.
const MAX_NUDGE_DISTANCE: usize = 3;

/// Don't nudge segments shorter than this. Stub segments (1-2
/// cells) are typically tied to an endpoint and shifting them
/// detaches.
const MIN_SEGMENT_LEN_FOR_NUDGE: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

/// A maximal collinear run of cells in one axis.
#[derive(Debug, Clone)]
struct Segment {
    edge_idx: usize,
    axis: Axis,
    /// row for Horizontal, col for Vertical.
    fixed_coord: usize,
    /// Inclusive start/end along the variable axis (col for H, row for V).
    range: (usize, usize),
    /// Index span in `paths[edge_idx]` that this segment covers
    /// (inclusive on both ends).
    path_idx_range: (usize, usize),
}

#[derive(Debug, Clone)]
struct Shift {
    edge_idx: usize,
    new_path: Vec<(usize, usize)>,
}

/// Run the nudging pass. Mutates `paths` in place and re-stamps
/// the grid.
///
/// `edge_is_back[i]` is true when edge `i` is a back-edge. Back-
/// edges are the only candidates for parallel-merge in Phase C.
/// `tip_for(i)` returns the arrow-tip glyph for edge `i`.
///
/// **Phase D (Bug 4) deferred:** the plan's corner-displacement
/// algorithm assumed bend cells (single-path with both H+V bits)
/// would be the targets, but the diamond_join fixture's `├` glyph
/// is a JUNCTION of two paths' separate bits — each path
/// individually transits through the cell without bending. A real
/// fix needs segment-level eviction: detect runs of cells in non-
/// endpoint halos and shift the entire run outward, with bridges
/// in the adjacent segments. That's a 2-3× expansion of this
/// module's scope and is deferred to a follow-up.
pub(crate) fn run(
    grid: &mut Grid,
    paths: &mut [Option<Vec<(usize, usize)>>],
    edge_is_back: &[bool],
    tip_for: impl Fn(usize) -> char,
) {
    let segments = collect_segments(paths, edge_is_back);
    let shifts = plan_parallel_merges(&segments, paths, grid);
    apply_shifts(grid, paths, shifts, &tip_for);
}

/// Walk all back-edge paths and emit their segments.
fn collect_segments(
    paths: &[Option<Vec<(usize, usize)>>],
    edge_is_back: &[bool],
) -> Vec<Segment> {
    let mut out = Vec::new();
    for (edge_idx, path_opt) in paths.iter().enumerate() {
        let Some(path) = path_opt else { continue };
        if !edge_is_back.get(edge_idx).copied().unwrap_or(false) {
            continue;
        }
        if path.len() < 2 {
            continue;
        }
        // Walk the path, emitting segments at axis transitions.
        let mut start_idx = 0usize;
        let mut current_axis = step_axis(path, 0);
        for i in 1..path.len() - 1 {
            let next_axis = step_axis(path, i);
            if next_axis != current_axis {
                emit_segment(path, edge_idx, current_axis, start_idx, i, &mut out);
                start_idx = i;
                current_axis = next_axis;
            }
        }
        // Emit the final segment up to (and including) the tip cell.
        emit_segment(
            path,
            edge_idx,
            current_axis,
            start_idx,
            path.len() - 1,
            &mut out,
        );
    }
    out
}

/// Determine the axis of the step `path[i] → path[i+1]`. Panics if
/// the two cells are equal (shouldn't happen for routed paths).
fn step_axis(path: &[(usize, usize)], i: usize) -> Axis {
    let (c, r) = path[i];
    let (nc, nr) = path[i + 1];
    if c == nc {
        Axis::Vertical
    } else if r == nr {
        Axis::Horizontal
    } else {
        // Diagonal — shouldn't happen for orthogonal routing. Fall
        // back to horizontal to avoid panicking on malformed input.
        Axis::Horizontal
    }
}

fn emit_segment(
    path: &[(usize, usize)],
    edge_idx: usize,
    axis: Axis,
    start_idx: usize,
    end_idx: usize,
    out: &mut Vec<Segment>,
) {
    if end_idx <= start_idx {
        return;
    }
    let (start_c, start_r) = path[start_idx];
    let (end_c, end_r) = path[end_idx];
    let (fixed_coord, range) = match axis {
        Axis::Horizontal => (start_r, (start_c.min(end_c), start_c.max(end_c))),
        Axis::Vertical => (start_c, (start_r.min(end_r), start_r.max(end_r))),
    };
    out.push(Segment {
        edge_idx,
        axis,
        fixed_coord,
        range,
        path_idx_range: (start_idx, end_idx),
    });
}

/// Find pairs of back-edges with horizontal segments at adjacent
/// rows and overlapping col ranges; plan a shift that merges them.
///
/// Strategy: the back-edge with the LATER segment (lower row /
/// further from the diagram body) wins — it's already on the
/// outer perimeter. The other back-edge's segment shifts to its
/// row.
fn plan_parallel_merges(
    segments: &[Segment],
    paths: &[Option<Vec<(usize, usize)>>],
    grid: &Grid,
) -> Vec<Shift> {
    let mut shifts = Vec::new();
    let horizontals: Vec<&Segment> = segments
        .iter()
        .filter(|s| s.axis == Axis::Horizontal)
        .filter(|s| s.range.1 - s.range.0 + 1 >= MIN_SEGMENT_LEN_FOR_NUDGE)
        .collect();

    let mut already_shifted: std::collections::HashSet<usize> =
        std::collections::HashSet::new();

    for i in 0..horizontals.len() {
        for j in (i + 1)..horizontals.len() {
            let (a, b) = (horizontals[i], horizontals[j]);
            if a.edge_idx == b.edge_idx {
                continue;
            }
            if already_shifted.contains(&a.edge_idx)
                || already_shifted.contains(&b.edge_idx)
            {
                continue;
            }
            let row_delta = a.fixed_coord.abs_diff(b.fixed_coord);
            if row_delta == 0 || row_delta > MAX_NUDGE_DISTANCE {
                continue;
            }
            // Range overlap check.
            let (a_lo, a_hi) = a.range;
            let (b_lo, b_hi) = b.range;
            if a_hi < b_lo || b_hi < a_lo {
                continue;
            }
            // Pick the "outer" target row — the larger fixed_coord
            // (further down the canvas), reflecting the perimeter.
            let target_row = a.fixed_coord.max(b.fixed_coord);
            let source_seg = if a.fixed_coord < b.fixed_coord { a } else { b };
            // Build the candidate new path.
            let Some(old_path) = paths[source_seg.edge_idx].as_ref() else {
                continue;
            };
            let new_path = build_shifted_path(old_path, source_seg, target_row);
            // Feasibility: no cell may be a NodeBox.
            if !path_is_feasible(grid, &new_path) {
                continue;
            }
            shifts.push(Shift {
                edge_idx: source_seg.edge_idx,
                new_path,
            });
            already_shifted.insert(source_seg.edge_idx);
        }
    }
    shifts
}

/// Build a new path where the horizontal segment at `seg.path_idx_range`
/// is moved from its current row to `target_row`. Cells in the bend
/// regions before/after the segment are extended/shortened to bridge
/// the row delta.
fn build_shifted_path(
    old_path: &[(usize, usize)],
    seg: &Segment,
    target_row: usize,
) -> Vec<(usize, usize)> {
    let (start_idx, end_idx) = seg.path_idx_range;
    let mut new_path = Vec::with_capacity(old_path.len() + 4);

    // Pre-segment cells: keep, but inject a vertical bridge from the
    // last pre-segment cell's row to target_row at start_c.
    new_path.extend_from_slice(&old_path[..start_idx]);
    let start_c = old_path[start_idx].0;
    if let Some(&(prev_c, prev_r)) = new_path.last() {
        // Bridge from (prev_c, prev_r) vertically to target_row at start_c.
        // First, walk vertically along prev_c if needed; then snap to start_c.
        if prev_c == start_c {
            extend_vertical(&mut new_path, prev_c, prev_r, target_row);
        } else {
            // The pre-segment cell at start_idx was a corner; the
            // last pre-cell shares a column with start_c (not common
            // — fall through to direct bridge).
            extend_vertical(&mut new_path, start_c, prev_r, target_row);
        }
    } else {
        new_path.push((start_c, target_row));
    }

    // Segment cells: same column range, target_row.
    for &(c, _) in old_path.iter().take(end_idx + 1).skip(start_idx) {
        let last = new_path.last().copied();
        if last != Some((c, target_row)) {
            new_path.push((c, target_row));
        }
    }

    // Post-segment cells: bridge from target_row back to old_path[end_idx+1].
    if end_idx + 1 < old_path.len() {
        let end_c = old_path[end_idx].0;
        let (next_c, next_r) = old_path[end_idx + 1];
        if next_c == end_c {
            extend_vertical(&mut new_path, end_c, target_row, next_r);
        } else {
            extend_vertical(&mut new_path, end_c, target_row, next_r);
            // Then horizontal-bridge to next_c at next_r.
            let mut c = end_c;
            while c != next_c {
                if c < next_c {
                    c += 1;
                } else {
                    c -= 1;
                }
                let cell = (c, next_r);
                if new_path.last().copied() != Some(cell) {
                    new_path.push(cell);
                }
            }
        }
        // Remaining post-segment cells (skip the one we just bridged to).
        for &cell in old_path.iter().skip(end_idx + 2) {
            if new_path.last().copied() != Some(cell) {
                new_path.push(cell);
            }
        }
    }

    new_path
}

/// Append cells (col, from_row+1), (col, from_row+2), ..., (col, to_row)
/// (or descending) to `path`. If `from_row == to_row`, this is a no-op.
/// The starting cell (col, from_row) is assumed to already be at the
/// end of `path` (or absent for path-start cases).
fn extend_vertical(
    path: &mut Vec<(usize, usize)>,
    col: usize,
    from_row: usize,
    to_row: usize,
) {
    if from_row == to_row {
        let cell = (col, to_row);
        if path.last().copied() != Some(cell) {
            path.push(cell);
        }
        return;
    }
    if from_row < to_row {
        for r in (from_row + 1)..=to_row {
            let cell = (col, r);
            if path.last().copied() != Some(cell) {
                path.push(cell);
            }
        }
    } else {
        for r in (to_row..from_row).rev() {
            let cell = (col, r);
            if path.last().copied() != Some(cell) {
                path.push(cell);
            }
        }
    }
}

/// Check that `path` doesn't pass through any NodeBox cells (other
/// than the destination tip, which is allowed to land on a node
/// border).
fn path_is_feasible(grid: &Grid, path: &[(usize, usize)]) -> bool {
    if path.is_empty() {
        return false;
    }
    let last = path.len() - 1;
    for (i, &(c, r)) in path.iter().enumerate() {
        if i == last {
            // Tip cell — allowed on a node border.
            continue;
        }
        if grid.is_node_box(c, r) {
            return false;
        }
    }
    true
}

/// Apply each shift atomically: erase old, draw new, update paths.
fn apply_shifts(
    grid: &mut Grid,
    paths: &mut [Option<Vec<(usize, usize)>>],
    shifts: Vec<Shift>,
    tip_for: &impl Fn(usize) -> char,
) {
    for shift in shifts {
        let Some(old_path) = paths[shift.edge_idx].clone() else {
            continue;
        };
        grid.erase_path(&old_path);
        let tip = tip_for(shift.edge_idx);
        if let Some(drawn) = grid.draw_path(shift.new_path.clone(), tip) {
            paths[shift.edge_idx] = Some(drawn);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_path(cells: &[(usize, usize)]) -> Vec<(usize, usize)> {
        cells.to_vec()
    }

    #[test]
    fn collect_segments_splits_at_corners() {
        let path = make_path(&[(0, 0), (0, 1), (0, 2), (1, 2), (2, 2), (2, 3)]);
        let paths = vec![Some(path)];
        let edge_is_back = vec![true];
        let segs = collect_segments(&paths, &edge_is_back);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].axis, Axis::Vertical);
        assert_eq!(segs[1].axis, Axis::Horizontal);
        assert_eq!(segs[2].axis, Axis::Vertical);
    }

    #[test]
    fn collect_segments_skips_forward_edges() {
        let path = make_path(&[(0, 0), (0, 1), (0, 2), (1, 2)]);
        let paths = vec![Some(path)];
        let edge_is_back = vec![false];
        let segs = collect_segments(&paths, &edge_is_back);
        assert!(segs.is_empty());
    }

    #[test]
    fn extend_vertical_descends() {
        let mut p = vec![(5, 2)];
        extend_vertical(&mut p, 5, 2, 5);
        assert_eq!(p, vec![(5, 2), (5, 3), (5, 4), (5, 5)]);
    }

    #[test]
    fn extend_vertical_ascends() {
        let mut p = vec![(5, 5)];
        extend_vertical(&mut p, 5, 5, 2);
        assert_eq!(p, vec![(5, 5), (5, 4), (5, 3), (5, 2)]);
    }

    #[test]
    fn build_shifted_path_moves_corridor_down_one_row() {
        // U-shape path: down 3, right 4, up 3.
        let old_path = vec![
            (0, 0),
            (0, 1),
            (0, 2),
            (0, 3),
            (1, 3),
            (2, 3),
            (3, 3),
            (4, 3),
            (4, 2),
            (4, 1),
            (4, 0),
        ];
        // The horizontal segment is path indices 3..=7 at row 3.
        let seg = Segment {
            edge_idx: 0,
            axis: Axis::Horizontal,
            fixed_coord: 3,
            range: (0, 4),
            path_idx_range: (3, 7),
        };
        let new_path = build_shifted_path(&old_path, &seg, 4);
        assert!(new_path.contains(&(0, 4)));
        assert!(new_path.contains(&(4, 4)));
        // Sanity: no diagonal jumps.
        for w in new_path.windows(2) {
            let (a, b) = (w[0], w[1]);
            let dc = a.0.abs_diff(b.0);
            let dr = a.1.abs_diff(b.1);
            assert!(
                dc + dr == 1,
                "non-orthogonal step from {a:?} to {b:?} in {new_path:?}"
            );
        }
    }
}
