//! Post-routing nudging pass.
//!
//! Reads the routes produced by `router::route_all` and makes three targeted
//! post-processing adjustments:
//!
//! 1. Merge adjacent horizontal back-edge corridors (Bug 5).
//! 2. Evict routed path runs from the 1-cell halo around nodes when that node
//!    is not an endpoint of the edge (Bug 4).
//! 3. Evict endpoint-halo runs that sit directly against a node corner row/col
//!    and read as part of the box border.
//!
//! The pass operates on finished paths rather than A* costs. That keeps the
//! router's attach semantics intact and limits change to paths we can inspect
//! and re-stamp atomically.

use crate::layout::Grid;

/// Maximum row/col delta between two segments for them to be considered
/// "adjacent" and candidate for merging.
const MAX_NUDGE_DISTANCE: usize = 3;

/// Don't nudge very short segments. Stub segments are often endpoint-adjacent
/// and moving them detaches the edge from its source/target neighborhood.
const MIN_SEGMENT_LEN_FOR_NUDGE: usize = 4;

/// When evicting a halo run, try a few cells farther away before giving up.
const MAX_HALO_EVICTION_SHIFT: usize = 4;

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

#[derive(Debug, Clone, Copy)]
struct NodeRect {
    col: usize,
    row: usize,
    width: usize,
    height: usize,
}

#[derive(Debug, Clone, Copy)]
struct HaloRun {
    axis: Axis,
    start_idx: usize,
    end_idx: usize,
    fixed_coord: usize,
    shift_dir: isize,
}

/// Run the nudging pass. Mutates `paths` in place and re-stamps the grid.
///
/// Two scans run in sequence:
/// 1. **Parallel-corridor merge (Bug 5)** — pairs of horizontal back-edge
///    segments at adjacent rows with overlapping column ranges merge onto
///    the outer row. Back-edges only; forward edges have different attach-
///    point semantics that are out of scope.
/// 2. **Foreign-halo eviction (Bug 4)** — runs of cells in a non-endpoint
///    node's 1-cell halo shift outward, with bridges in the adjacent
///    segments. Skipped for edges with a label (the label needs adjacent
///    free space; shifting the route would push labels off the route).
/// 3. **Endpoint corner-row eviction** — runs in an endpoint node's halo that
///    touch the node's corner row/col shift outward so a stray `│` / `─`
///    doesn't appear welded to the box corner.
///
/// Arguments mirror per-edge state already collected in `render_inner`:
/// - `edge_is_back[i]` — true for back-edges.
/// - `edge_has_label[i]` — true when the edge carries a text label.
/// - `node_boxes` — every node bounding box, as `(col, row, w, h)`.
/// - `tip_for(i)` — arrow-tip glyph for edge `i`, used when re-stamping
///   the new path after a shift.
pub(crate) fn run(
    grid: &mut Grid,
    paths: &mut [Option<Vec<(usize, usize)>>],
    edge_is_back: &[bool],
    edge_has_label: &[bool],
    node_boxes: &[(usize, usize, usize, usize)],
    enable_endpoint_corner_nudge: bool,
    tip_for: impl Fn(usize) -> char,
) {
    let segments = collect_segments(paths);
    let shifts = plan_parallel_merges(&segments, paths, grid, edge_is_back);
    apply_shifts(grid, paths, shifts, &tip_for);
    evict_foreign_halo_runs(grid, paths, edge_has_label, node_boxes, &tip_for);
    if enable_endpoint_corner_nudge {
        evict_endpoint_corner_runs(grid, paths, edge_has_label, node_boxes, &tip_for);
        evict_destination_channel_runs(grid, paths, edge_has_label, node_boxes, &tip_for);
    }
}

/// Walk all routed paths and emit their maximal segments.
fn collect_segments(paths: &[Option<Vec<(usize, usize)>>]) -> Vec<Segment> {
    let mut out = Vec::new();
    for (edge_idx, path_opt) in paths.iter().enumerate() {
        let Some(path) = path_opt else { continue };
        if path.len() < 2 {
            continue;
        }
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

/// Determine the axis of the step `path[i] → path[i+1]`. Diagonal
/// steps (which shouldn't occur for orthogonal routes) fall through
/// to Horizontal as a safe default.
fn step_axis(path: &[(usize, usize)], i: usize) -> Axis {
    let (c, _r) = path[i];
    let (nc, _nr) = path[i + 1];
    if c == nc {
        Axis::Vertical
    } else {
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

/// Find pairs of back-edges with horizontal segments at adjacent rows and
/// overlapping col ranges; plan a shift that merges them.
fn plan_parallel_merges(
    segments: &[Segment],
    paths: &[Option<Vec<(usize, usize)>>],
    grid: &Grid,
    edge_is_back: &[bool],
) -> Vec<Shift> {
    let mut shifts = Vec::new();
    let horizontals: Vec<&Segment> = segments
        .iter()
        .filter(|s| edge_is_back.get(s.edge_idx).copied().unwrap_or(false))
        .filter(|s| s.axis == Axis::Horizontal)
        .filter(|s| s.range.1 - s.range.0 + 1 >= MIN_SEGMENT_LEN_FOR_NUDGE)
        .collect();

    let mut already_shifted: std::collections::HashSet<usize> = std::collections::HashSet::new();

    for i in 0..horizontals.len() {
        for j in (i + 1)..horizontals.len() {
            let (a, b) = (horizontals[i], horizontals[j]);
            if a.edge_idx == b.edge_idx {
                continue;
            }
            if already_shifted.contains(&a.edge_idx) || already_shifted.contains(&b.edge_idx) {
                continue;
            }
            let row_delta = a.fixed_coord.abs_diff(b.fixed_coord);
            if row_delta == 0 || row_delta > MAX_NUDGE_DISTANCE {
                continue;
            }
            let (a_lo, a_hi) = a.range;
            let (b_lo, b_hi) = b.range;
            if a_hi < b_lo || b_hi < a_lo {
                continue;
            }
            let target_row = a.fixed_coord.max(b.fixed_coord);
            let source_seg = if a.fixed_coord < b.fixed_coord { a } else { b };
            let Some(old_path) = paths[source_seg.edge_idx].as_ref() else {
                continue;
            };
            let new_path = build_shifted_segment_path(old_path, source_seg, target_row);
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

/// Evict at most ONE halo run per render call.
///
/// The pass intentionally applies a single shift even when multiple foreign
/// halo runs exist on different edges. Rationale:
/// - Each shift mutates the grid (re-stamps direction bits and obstacle
///   classification); subsequent shifts must be planned against the post-
///   shift state, which would require re-running `plan_next_halo_shift`
///   in a loop with a fixed-point check. The crossings comparison adds
///   non-trivial cost (each candidate clones the grid in
///   `crossings_after_shift`), so a multi-shift loop would be quadratic
///   in the worst case.
/// - Empirically (gallery + corpus) one shift per call clears the visible
///   diamond-join Bug 4 fixture and leaves all other diagrams unchanged.
///   Diagrams with multiple foreign halo runs are rare enough that the
///   conservative single-shift behaviour is acceptable; if a future
///   fixture needs more, lift this into a bounded loop with an iteration
///   cap and re-baseline the snapshot suite.
fn evict_foreign_halo_runs(
    grid: &mut Grid,
    paths: &mut [Option<Vec<(usize, usize)>>],
    edge_has_label: &[bool],
    node_boxes: &[(usize, usize, usize, usize)],
    tip_for: &impl Fn(usize) -> char,
) {
    let rects: Vec<NodeRect> = node_boxes
        .iter()
        .map(|&(col, row, width, height)| NodeRect {
            col,
            row,
            width,
            height,
        })
        .collect();

    if let Some(shift) = plan_next_halo_shift(paths, grid, edge_has_label, &rects) {
        apply_shifts(grid, paths, vec![shift], tip_for);
    }
}

fn evict_endpoint_corner_runs(
    grid: &mut Grid,
    paths: &mut [Option<Vec<(usize, usize)>>],
    edge_has_label: &[bool],
    node_boxes: &[(usize, usize, usize, usize)],
    tip_for: &impl Fn(usize) -> char,
) {
    let rects: Vec<NodeRect> = node_boxes
        .iter()
        .map(|&(col, row, width, height)| NodeRect {
            col,
            row,
            width,
            height,
        })
        .collect();

    if let Some(shift) = plan_endpoint_corner_shift(paths, grid, edge_has_label, &rects) {
        apply_shifts(grid, paths, vec![shift], tip_for);
    }
}

/// Shift a path's halo run away from any cell that is *another edge's*
/// arrow-tip cell. Without this, two edges sharing a destination column at
/// adjacent rows produce a path that visibly overshoots through the other
/// arrow's protected glyph — the user sees an "orphan" arrow on the row
/// past the overshoot. Same conservative gating + crossings budget as the
/// endpoint-corner nudge.
fn evict_destination_channel_runs(
    grid: &mut Grid,
    paths: &mut [Option<Vec<(usize, usize)>>],
    edge_has_label: &[bool],
    node_boxes: &[(usize, usize, usize, usize)],
    tip_for: &impl Fn(usize) -> char,
) {
    let rects: Vec<NodeRect> = node_boxes
        .iter()
        .map(|&(col, row, width, height)| NodeRect {
            col,
            row,
            width,
            height,
        })
        .collect();

    if let Some(shift) = plan_destination_channel_shift(paths, grid, edge_has_label, &rects) {
        apply_shifts(grid, paths, vec![shift], tip_for);
    }
}

fn plan_destination_channel_shift(
    paths: &[Option<Vec<(usize, usize)>>],
    grid: &Grid,
    edge_has_label: &[bool],
    _rects: &[NodeRect],
) -> Option<Shift> {
    let baseline_crossings = count_crossings_in_grid(grid);
    let tip_cells: Vec<Option<(usize, usize)>> = paths
        .iter()
        .map(|p_opt| p_opt.as_ref().and_then(|p| p.last().copied()))
        .collect();

    for (edge_idx, path_opt) in paths.iter().enumerate() {
        if edge_has_label.get(edge_idx).copied().unwrap_or(false) {
            continue;
        }
        let Some(path) = path_opt.as_ref() else {
            continue;
        };
        if path.len() < 4 {
            continue;
        }

        // Only fires when the cell directly preceding the tip is another
        // edge's tip — i.e., this path's final segment runs through the
        // destination's vertical port channel and overshoots past a
        // different incoming arrow. The fix is to bend earlier so the
        // final segment terminates at the tip without traversing other
        // tips.
        let tip = path[path.len() - 1];
        let pre_tip = path[path.len() - 2];
        let pre_tip_invasive = tip_cells
            .iter()
            .enumerate()
            .any(|(other_idx, other_tip)| other_idx != edge_idx && *other_tip == Some(pre_tip));
        if !pre_tip_invasive {
            continue;
        }

        // Final segment must be axis-aligned (orthogonal routing).
        let last_segment_vertical = pre_tip.0 == tip.0;
        let last_segment_horizontal = pre_tip.1 == tip.1;
        if !last_segment_vertical && !last_segment_horizontal {
            continue;
        }

        let baseline = count_invasive_tip_cells(path, &tip_cells, edge_idx);

        // Try increasing shift distances on each viable side.
        for shift in shift_candidates(pre_tip, tip, last_segment_vertical) {
            let Some(new_path) = build_tip_channel_evicted_path(path, shift) else {
                continue;
            };
            if !path_is_feasible(grid, &new_path) {
                continue;
            }
            let candidate = count_invasive_tip_cells(&new_path, &tip_cells, edge_idx);
            let candidate_crossings =
                crossings_after_shift(grid, path, &new_path).unwrap_or(usize::MAX);
            let crosses_ok = candidate_crossings <= baseline_crossings;
            if candidate < baseline && crosses_ok {
                return Some(Shift { edge_idx, new_path });
            }
        }
    }
    None
}

/// Generate candidate (axis, target_coord) shifts to try in order.
/// For a vertical last segment, we shift the bend column away from the
/// tip in BOTH directions (toward source and away). The first feasible,
/// crossings-safe shift wins.
fn shift_candidates(
    pre_tip: (usize, usize),
    tip: (usize, usize),
    last_segment_vertical: bool,
) -> Vec<(Axis, usize)> {
    let mut out = Vec::with_capacity(2 * MAX_HALO_EVICTION_SHIFT);
    if last_segment_vertical {
        // Bend column shifts. Try toward smaller cols first (toward LR
        // sources) then toward larger.
        for d in 1..=MAX_HALO_EVICTION_SHIFT {
            if let Some(c) = pre_tip.0.checked_sub(d) {
                out.push((Axis::Vertical, c));
            }
        }
        for d in 1..=MAX_HALO_EVICTION_SHIFT {
            out.push((Axis::Vertical, pre_tip.0 + d));
        }
    } else {
        // Bend row shifts. Symmetric for horizontal last segment (TD/BT).
        for d in 1..=MAX_HALO_EVICTION_SHIFT {
            if let Some(r) = pre_tip.1.checked_sub(d) {
                out.push((Axis::Horizontal, r));
            }
        }
        for d in 1..=MAX_HALO_EVICTION_SHIFT {
            out.push((Axis::Horizontal, pre_tip.1 + d));
        }
    }
    let _ = tip;
    out
}

/// Rebuild the path so its final segment bends at `(axis, target)` instead
/// of at the original `pre_tip` position, preserving the tip cell. Returns
/// `None` if the path doesn't have the expected shape (must end with at
/// least one cell at the same row as `pre_tip` for vertical-last-segment,
/// or at the same col for horizontal-last-segment).
fn build_tip_channel_evicted_path(
    old_path: &[(usize, usize)],
    shift: (Axis, usize),
) -> Option<Vec<(usize, usize)>> {
    if old_path.len() < 3 {
        return None;
    }
    let tip = *old_path.last()?;
    let pre_tip = *old_path.get(old_path.len() - 2)?;
    let (axis, target) = shift;

    match axis {
        Axis::Vertical => {
            // Last segment is vertical at col == pre_tip.0 == tip.0.
            // We bend at column `target` instead, so the new path goes:
            // ... cells with col strictly between source-side and target,
            // then horizontal to target at pre_tip.1, vertical at target
            // from pre_tip.1 to tip.1, horizontal to tip at tip.1.
            if target == pre_tip.0 {
                return None;
            }
            // Find the largest prefix index whose cell row matches
            // pre_tip.1 AND whose col is on the "source side" of target.
            // For LR shifts (target < pre_tip.0), we want cells with
            // col < target. For RL (target > pre_tip.0), col > target.
            let shift_left = target < pre_tip.0;
            let mut keep_until = None;
            for (i, &(c, r)) in old_path.iter().enumerate().take(old_path.len() - 1) {
                if r != pre_tip.1 {
                    continue;
                }
                if (shift_left && c < target) || (!shift_left && c > target) {
                    keep_until = Some(i);
                } else {
                    break;
                }
            }
            let keep_until = keep_until?;
            let last_kept = old_path[keep_until];

            let mut new_path = old_path[..=keep_until].to_vec();
            extend_horizontal(&mut new_path, pre_tip.1, last_kept.0, target);
            extend_vertical(&mut new_path, target, pre_tip.1, tip.1);
            extend_horizontal(&mut new_path, tip.1, target, tip.0);
            Some(new_path)
        }
        Axis::Horizontal => {
            if target == pre_tip.1 {
                return None;
            }
            let shift_up = target < pre_tip.1;
            let mut keep_until = None;
            for (i, &(c, r)) in old_path.iter().enumerate().take(old_path.len() - 1) {
                if c != pre_tip.0 {
                    continue;
                }
                if (shift_up && r < target) || (!shift_up && r > target) {
                    keep_until = Some(i);
                } else {
                    break;
                }
            }
            let keep_until = keep_until?;
            let last_kept = old_path[keep_until];

            let mut new_path = old_path[..=keep_until].to_vec();
            extend_vertical(&mut new_path, pre_tip.0, last_kept.1, target);
            extend_horizontal(&mut new_path, target, pre_tip.0, tip.0);
            extend_vertical(&mut new_path, tip.0, target, tip.1);
            Some(new_path)
        }
    }
}

fn count_invasive_tip_cells(
    path: &[(usize, usize)],
    tip_cells: &[Option<(usize, usize)>],
    own_edge_idx: usize,
) -> usize {
    path.iter()
        .enumerate()
        .filter(|(idx, _)| *idx > 0 && *idx + 1 < path.len())
        .filter(|&(_, &(c, r))| {
            tip_cells.iter().enumerate().any(|(other_idx, other_tip)| {
                other_idx != own_edge_idx && *other_tip == Some((c, r))
            })
        })
        .count()
}

fn plan_next_halo_shift(
    paths: &[Option<Vec<(usize, usize)>>],
    grid: &Grid,
    edge_has_label: &[bool],
    rects: &[NodeRect],
) -> Option<Shift> {
    let baseline_crossings = count_crossings_in_grid(grid);
    for (edge_idx, path_opt) in paths.iter().enumerate() {
        if edge_has_label.get(edge_idx).copied().unwrap_or(false) {
            continue;
        }
        let Some(path) = path_opt.as_ref() else {
            continue;
        };
        if path.len() < 4 {
            continue;
        }
        let endpoint_nodes = endpoint_node_indices(path, rects);
        let baseline = count_foreign_halo_cells(path, rects, &endpoint_nodes);
        if baseline == 0 {
            continue;
        }

        for (node_idx, rect) in rects.iter().enumerate() {
            if endpoint_nodes.contains(&node_idx) {
                continue;
            }
            for run in collect_halo_runs(path, *rect) {
                if run.start_idx <= 1 || run.end_idx + 1 >= path.len() - 1 {
                    continue;
                }
                if !halo_run_has_corner_or_junction(grid, path, &run) {
                    continue;
                }
                for distance in 1..=MAX_HALO_EVICTION_SHIFT {
                    let Some(target_fixed) =
                        apply_signed_offset(run.fixed_coord, run.shift_dir * distance as isize)
                    else {
                        break;
                    };
                    let new_path = build_evicted_run_path(path, &run, target_fixed);
                    if !path_is_feasible(grid, &new_path) {
                        continue;
                    }
                    let candidate = count_foreign_halo_cells(&new_path, rects, &endpoint_nodes);
                    let candidate_crossings =
                        crossings_after_shift(grid, path, &new_path).unwrap_or(usize::MAX);
                    let crosses_ok = candidate_crossings <= baseline_crossings
                        || (baseline_crossings == 0 && candidate_crossings == 1);
                    if candidate < baseline && crosses_ok {
                        return Some(Shift { edge_idx, new_path });
                    }
                }
            }
        }
    }
    None
}

fn plan_endpoint_corner_shift(
    paths: &[Option<Vec<(usize, usize)>>],
    grid: &Grid,
    edge_has_label: &[bool],
    rects: &[NodeRect],
) -> Option<Shift> {
    let baseline_crossings = count_crossings_in_grid(grid);
    for (edge_idx, path_opt) in paths.iter().enumerate() {
        if edge_has_label.get(edge_idx).copied().unwrap_or(false) {
            continue;
        }
        let Some(path) = path_opt.as_ref() else {
            continue;
        };
        if path.len() < 4 {
            continue;
        }
        let endpoint_nodes = endpoint_node_indices(path, rects);
        let baseline = count_endpoint_corner_adjacent_cells(path, rects, &endpoint_nodes);
        if baseline == 0 {
            continue;
        }
        for &node_idx in &endpoint_nodes {
            let rect = rects[node_idx];
            for run in collect_halo_runs(path, rect) {
                if run.end_idx + 1 >= path.len() {
                    continue;
                }
                if !run_touches_endpoint_corner_band(path, rect, &run) {
                    continue;
                }
                for distance in 1..=MAX_HALO_EVICTION_SHIFT {
                    let Some(target_fixed) =
                        apply_signed_offset(run.fixed_coord, run.shift_dir * distance as isize)
                    else {
                        break;
                    };
                    let new_path = build_evicted_run_path(path, &run, target_fixed);
                    if !path_is_feasible(grid, &new_path) {
                        continue;
                    }
                    let candidate =
                        count_endpoint_corner_adjacent_cells(&new_path, rects, &endpoint_nodes);
                    let candidate_crossings =
                        crossings_after_shift(grid, path, &new_path).unwrap_or(usize::MAX);
                    let crosses_ok = candidate_crossings <= baseline_crossings;
                    if candidate < baseline && crosses_ok {
                        return Some(Shift { edge_idx, new_path });
                    }
                }
            }
        }
    }
    None
}

fn halo_run_has_corner_or_junction(grid: &Grid, path: &[(usize, usize)], run: &HaloRun) -> bool {
    path.iter()
        .take(run.end_idx + 1)
        .skip(run.start_idx)
        .any(|&(col, row)| matches!(grid.get(col, row), '┬' | '┴' | '├' | '┤' | '┼'))
}

/// Speculatively apply `old_path → new_path` on a clone of the grid and
/// return the resulting crossing count.
///
/// Cost note: this clones the entire grid (O(width × height)) per call, and
/// `plan_next_halo_shift` may invoke it once per (edge × node × distance)
/// candidate. For typical gallery diagrams (≤ 50 cells × 30 cells × 12 edges
/// × 8 nodes × 4 distances) the wall-clock cost is well under the launch
/// budget's < 8 ms frame draw target, but for pathologically dense diagrams
/// this is the first thing to optimise (e.g. by tracking crossings
/// incrementally during `erase_path` / `draw_path`).
fn crossings_after_shift(
    grid: &Grid,
    old_path: &[(usize, usize)],
    new_path: &[(usize, usize)],
) -> Option<usize> {
    let mut scratch = grid.clone();
    scratch.erase_path(old_path);
    let &(tip_col, tip_row) = old_path.last()?;
    let tip = grid.get(tip_col, tip_row);
    scratch.draw_path(new_path.to_vec(), tip)?;
    Some(count_crossings_in_grid(&scratch))
}

fn count_crossings_in_grid(grid: &Grid) -> usize {
    let mut count = 0;
    for row in 0..grid.rows() {
        for col in 0..grid.cols() {
            let ch = grid.get(col, row);
            if ch == '┼' || ch == '╋' {
                count += 1;
            }
        }
    }
    count
}

fn endpoint_node_indices(path: &[(usize, usize)], rects: &[NodeRect]) -> Vec<usize> {
    let mut out = Vec::new();
    let endpoints = [path[0], *path.last().unwrap_or(&path[0])];
    for (idx, rect) in rects.iter().enumerate() {
        if endpoints
            .iter()
            .any(|&(c, r)| point_in_expanded_rect(*rect, c, r))
        {
            out.push(idx);
        }
    }
    out
}

fn count_foreign_halo_cells(
    path: &[(usize, usize)],
    rects: &[NodeRect],
    endpoint_nodes: &[usize],
) -> usize {
    path.iter()
        .enumerate()
        .filter(|(idx, _)| *idx > 0 && *idx + 1 < path.len())
        .filter(|&(_, &(c, r))| {
            rects.iter().enumerate().any(|(node_idx, rect)| {
                !endpoint_nodes.contains(&node_idx) && point_in_halo(*rect, c, r)
            })
        })
        .count()
}

fn count_endpoint_corner_adjacent_cells(
    path: &[(usize, usize)],
    rects: &[NodeRect],
    endpoint_nodes: &[usize],
) -> usize {
    path.iter()
        .enumerate()
        .filter(|(idx, _)| *idx > 0 && *idx + 1 < path.len())
        .filter(|&(_, &(c, r))| {
            endpoint_nodes.iter().any(|&node_idx| {
                rects
                    .get(node_idx)
                    .is_some_and(|&rect| point_is_corner_adjacent_halo(rect, c, r))
            })
        })
        .count()
}

fn collect_halo_runs(path: &[(usize, usize)], rect: NodeRect) -> Vec<HaloRun> {
    let mut runs = Vec::new();
    let mut i = 1usize;
    while i + 1 < path.len() {
        let (c, r) = path[i];
        if !point_in_halo(rect, c, r) {
            i += 1;
            continue;
        }
        let start = i;
        while i + 1 < path.len() {
            let (cc, rr) = path[i + 1];
            if !point_in_halo(rect, cc, rr) {
                break;
            }
            i += 1;
        }
        let end = i;
        if let Some(axis) = axis_for_run(path, start, end)
            && let Some(shift_dir) = shift_direction_for_run(rect, axis, path, start, end)
        {
            let fixed_coord = match axis {
                Axis::Horizontal => path[start].1,
                Axis::Vertical => path[start].0,
            };
            runs.push(HaloRun {
                axis,
                start_idx: start,
                end_idx: end,
                fixed_coord,
                shift_dir,
            });
        }
        i += 1;
    }
    runs
}

/// Return the axis of a halo run spanning `path[start_idx..=end_idx]`.
///
/// Multi-cell runs are classified by comparing the start and end cells
/// directly. Single-cell runs (`start_idx == end_idx`) have no run-internal
/// step to read, so we infer from the `path[start_idx-1] → path[start_idx]`
/// step. Callers always pass `start_idx ≥ 1`, so that lookup is in bounds.
fn axis_for_run(path: &[(usize, usize)], start_idx: usize, end_idx: usize) -> Option<Axis> {
    if start_idx < end_idx {
        if path[start_idx].0 == path[end_idx].0 {
            return Some(Axis::Vertical);
        }
        if path[start_idx].1 == path[end_idx].1 {
            return Some(Axis::Horizontal);
        }
        return None;
    }

    // Single-cell run: take the inbound step's axis. Optionally cross-check
    // against the outbound step's axis if the run isn't at the path tail —
    // a mismatch means the run cell IS a corner, which the run-shift logic
    // can't model, so we bail with `None`.
    let before = step_axis(path, start_idx - 1);
    if start_idx + 1 < path.len() {
        let after = step_axis(path, start_idx);
        if after != before {
            return None;
        }
    }
    Some(before)
}

fn shift_direction_for_run(
    rect: NodeRect,
    axis: Axis,
    path: &[(usize, usize)],
    start_idx: usize,
    end_idx: usize,
) -> Option<isize> {
    match axis {
        Axis::Vertical => {
            let col = path[start_idx].0;
            if col < rect.col {
                Some(-1)
            } else if col >= rect.col + rect.width {
                Some(1)
            } else if end_idx > start_idx {
                None
            } else {
                // One-cell corner overlap inside the top/bottom halo band.
                let prev = path[start_idx - 1];
                if prev.0 < col {
                    Some(1)
                } else if prev.0 > col {
                    Some(-1)
                } else {
                    None
                }
            }
        }
        Axis::Horizontal => {
            let row = path[start_idx].1;
            if row < rect.row {
                Some(-1)
            } else if row >= rect.row + rect.height {
                Some(1)
            } else if end_idx > start_idx {
                None
            } else {
                let prev = path[start_idx - 1];
                if prev.1 < row {
                    Some(1)
                } else if prev.1 > row {
                    Some(-1)
                } else {
                    None
                }
            }
        }
    }
}

fn run_touches_endpoint_corner_band(
    path: &[(usize, usize)],
    rect: NodeRect,
    run: &HaloRun,
) -> bool {
    path.iter()
        .take(run.end_idx + 1)
        .skip(run.start_idx)
        .any(|&(col, row)| point_is_corner_adjacent_halo(rect, col, row))
}

/// Build a new path where the horizontal segment at `seg.path_idx_range` is
/// moved from its current row to `target_row`.
fn build_shifted_segment_path(
    old_path: &[(usize, usize)],
    seg: &Segment,
    target_row: usize,
) -> Vec<(usize, usize)> {
    let (start_idx, end_idx) = seg.path_idx_range;
    let mut new_path = Vec::with_capacity(old_path.len() + 4);

    new_path.extend_from_slice(&old_path[..start_idx]);
    let start_c = old_path[start_idx].0;
    if let Some(&(prev_c, prev_r)) = new_path.last() {
        if prev_c == start_c {
            extend_vertical(&mut new_path, prev_c, prev_r, target_row);
        } else {
            extend_vertical(&mut new_path, start_c, prev_r, target_row);
        }
    } else {
        push_cell(&mut new_path, (start_c, target_row));
    }

    for &(c, _) in old_path.iter().take(end_idx + 1).skip(start_idx) {
        push_cell(&mut new_path, (c, target_row));
    }

    if end_idx + 1 < old_path.len() {
        let end_c = old_path[end_idx].0;
        let (next_c, next_r) = old_path[end_idx + 1];
        extend_vertical(&mut new_path, end_c, target_row, next_r);
        extend_horizontal(&mut new_path, next_r, end_c, next_c);
        for &cell in old_path.iter().skip(end_idx + 2) {
            push_cell(&mut new_path, cell);
        }
    }

    new_path
}

fn build_evicted_run_path(
    old_path: &[(usize, usize)],
    run: &HaloRun,
    target_fixed: usize,
) -> Vec<(usize, usize)> {
    let prev_idx = run.start_idx - 1;
    let next_idx = run.end_idx + 1;
    let prev = old_path[prev_idx];
    let next = old_path[next_idx];
    let mut new_path = Vec::with_capacity(old_path.len() + 8);

    new_path.extend_from_slice(&old_path[..=prev_idx]);

    match run.axis {
        Axis::Vertical => {
            extend_horizontal(&mut new_path, prev.1, prev.0, target_fixed);
            let start_row = old_path[run.start_idx].1;
            extend_vertical(&mut new_path, target_fixed, prev.1, start_row);
            for &(_, row) in old_path.iter().take(run.end_idx + 1).skip(run.start_idx) {
                push_cell(&mut new_path, (target_fixed, row));
            }
            let end_row = old_path[run.end_idx].1;
            extend_vertical(&mut new_path, target_fixed, end_row, next.1);
            extend_horizontal(&mut new_path, next.1, target_fixed, next.0);
        }
        Axis::Horizontal => {
            extend_vertical(&mut new_path, prev.0, prev.1, target_fixed);
            let start_col = old_path[run.start_idx].0;
            extend_horizontal(&mut new_path, target_fixed, prev.0, start_col);
            for &(col, _) in old_path.iter().take(run.end_idx + 1).skip(run.start_idx) {
                push_cell(&mut new_path, (col, target_fixed));
            }
            let end_col = old_path[run.end_idx].0;
            extend_horizontal(&mut new_path, target_fixed, end_col, next.0);
            extend_vertical(&mut new_path, next.0, target_fixed, next.1);
        }
    }

    for &cell in old_path.iter().skip(next_idx + 1) {
        push_cell(&mut new_path, cell);
    }

    new_path
}

fn extend_horizontal(path: &mut Vec<(usize, usize)>, row: usize, from_col: usize, to_col: usize) {
    if from_col == to_col {
        push_cell(path, (to_col, row));
        return;
    }
    if from_col < to_col {
        for col in (from_col + 1)..=to_col {
            push_cell(path, (col, row));
        }
    } else {
        for col in (to_col..from_col).rev() {
            push_cell(path, (col, row));
        }
    }
}

fn extend_vertical(path: &mut Vec<(usize, usize)>, col: usize, from_row: usize, to_row: usize) {
    if from_row == to_row {
        push_cell(path, (col, to_row));
        return;
    }
    if from_row < to_row {
        for row in (from_row + 1)..=to_row {
            push_cell(path, (col, row));
        }
    } else {
        for row in (to_row..from_row).rev() {
            push_cell(path, (col, row));
        }
    }
}

fn push_cell(path: &mut Vec<(usize, usize)>, cell: (usize, usize)) {
    if path.last().copied() != Some(cell) {
        path.push(cell);
    }
}

/// Check that `path` doesn't pass through any invisible protected cells or
/// hard node boxes, other than the final tip cell.
fn path_is_feasible(grid: &Grid, path: &[(usize, usize)]) -> bool {
    if path.is_empty() {
        return false;
    }
    let last = path.len() - 1;
    for (i, &(c, r)) in path.iter().enumerate() {
        if i == last {
            continue;
        }
        if !grid.can_draw_path_cell(c, r) {
            return false;
        }
    }
    true
}

fn point_in_box(rect: NodeRect, col: usize, row: usize) -> bool {
    col >= rect.col
        && col < rect.col + rect.width
        && row >= rect.row
        && row < rect.row + rect.height
}

fn point_in_expanded_rect(rect: NodeRect, col: usize, row: usize) -> bool {
    let min_col = rect.col.saturating_sub(1);
    let min_row = rect.row.saturating_sub(1);
    let max_col = rect.col + rect.width;
    let max_row = rect.row + rect.height;
    col >= min_col && col <= max_col && row >= min_row && row <= max_row
}

fn point_in_halo(rect: NodeRect, col: usize, row: usize) -> bool {
    point_in_expanded_rect(rect, col, row) && !point_in_box(rect, col, row)
}

fn point_is_corner_adjacent_halo(rect: NodeRect, col: usize, row: usize) -> bool {
    if !point_in_halo(rect, col, row) {
        return false;
    }

    let on_side_halo_col = col + 1 == rect.col || col == rect.col + rect.width;
    let on_side_halo_row = row + 1 == rect.row || row == rect.row + rect.height;
    let on_border_row = row == rect.row || row + 1 == rect.row + rect.height;
    let on_border_col = col == rect.col || col + 1 == rect.col + rect.width;

    (on_side_halo_col && on_border_row) || (on_side_halo_row && on_border_col)
}

fn apply_signed_offset(value: usize, delta: isize) -> Option<usize> {
    if delta >= 0 {
        value.checked_add(delta as usize)
    } else {
        value.checked_sub(delta.unsigned_abs())
    }
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
        let segs = collect_segments(&paths);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].axis, Axis::Vertical);
        assert_eq!(segs[1].axis, Axis::Horizontal);
        assert_eq!(segs[2].axis, Axis::Vertical);
    }

    #[test]
    fn collect_segments_keeps_forward_edges() {
        let path = make_path(&[(0, 0), (0, 1), (0, 2), (1, 2)]);
        let paths = vec![Some(path)];
        let segs = collect_segments(&paths);
        assert_eq!(segs.len(), 2);
    }

    #[test]
    fn extend_vertical_descends() {
        let mut p = vec![(5, 2)];
        extend_vertical(&mut p, 5, 2, 5);
        assert_eq!(p, vec![(5, 2), (5, 3), (5, 4), (5, 5)]);
    }

    #[test]
    fn extend_horizontal_ascends() {
        let mut p = vec![(5, 2)];
        extend_horizontal(&mut p, 2, 5, 2);
        assert_eq!(p, vec![(5, 2), (4, 2), (3, 2), (2, 2)]);
    }

    #[test]
    fn build_shifted_segment_path_moves_corridor_down_one_row() {
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
        let seg = Segment {
            edge_idx: 0,
            axis: Axis::Horizontal,
            fixed_coord: 3,
            range: (0, 4),
            path_idx_range: (3, 7),
        };
        let new_path = build_shifted_segment_path(&old_path, &seg, 4);
        assert!(new_path.contains(&(0, 4)));
        assert!(new_path.contains(&(4, 4)));
    }

    #[test]
    fn build_evicted_run_path_shifts_vertical_subrange_outward() {
        let old_path = vec![
            (0, 0),
            (1, 0),
            (2, 0),
            (2, 1),
            (2, 2),
            (2, 3),
            (2, 4),
            (1, 4),
            (0, 4),
        ];
        let run = HaloRun {
            axis: Axis::Vertical,
            start_idx: 3,
            end_idx: 5,
            fixed_coord: 2,
            shift_dir: 1,
        };
        let new_path = build_evicted_run_path(&old_path, &run, 3);
        assert!(new_path.contains(&(3, 1)));
        assert!(new_path.contains(&(3, 2)));
        assert!(new_path.contains(&(3, 3)));
        for w in new_path.windows(2) {
            let dc = w[0].0.abs_diff(w[1].0);
            let dr = w[0].1.abs_diff(w[1].1);
            assert_eq!(dc + dr, 1, "non-orthogonal step in {new_path:?}");
        }
    }

    #[test]
    fn count_foreign_halo_cells_exempts_endpoint_nodes() {
        let path = vec![(5, 0), (5, 1), (5, 2), (5, 3), (5, 4)];
        let rects = vec![
            NodeRect {
                col: 3,
                row: 0,
                width: 2,
                height: 1,
            },
            NodeRect {
                col: 3,
                row: 4,
                width: 2,
                height: 1,
            },
            NodeRect {
                col: 6,
                row: 2,
                width: 1,
                height: 1,
            },
        ];
        let endpoints = endpoint_node_indices(&path, &rects);
        assert_eq!(count_foreign_halo_cells(&path, &rects, &endpoints), 3);
    }

    #[test]
    fn endpoint_corner_halo_count_only_flags_corner_adjacent_side_cells() {
        let rect = NodeRect {
            col: 0,
            row: 7,
            width: 7,
            height: 3,
        };
        let path = vec![(7, 8), (7, 7), (7, 6), (8, 6), (9, 6)];

        assert_eq!(
            count_endpoint_corner_adjacent_cells(&path, &[rect], &[0]),
            1,
            "only the side-halo cell sharing the node's top border row should count"
        );
    }
}
