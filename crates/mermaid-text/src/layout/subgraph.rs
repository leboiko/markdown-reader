//! Subgraph bounding-box computation.
//!
//! After the layered layout has placed every node at a `(col, row)` grid
//! position, this module walks the subgraph tree depth-first (innermost
//! first) and computes the screen-space rectangle that encloses each
//! subgraph, including padding for the border and the label.
//!
//! Constants ported from termaid's `grid.py`:
//! - `SG_BORDER_PAD = 2`  — cells of empty space between the enclosed nodes
//!   and the border line.
//!
//! The label is written inline in the top border row rather than consuming
//! extra rows above it.

use std::collections::HashMap;

use unicode_width::UnicodeWidthStr;

use crate::types::{Direction, Graph, Subgraph};

/// Cells of padding between nodes and the subgraph border.
pub const SG_BORDER_PAD: usize = 2;

/// Compute extra horizontal/vertical room a subgraph needs because of
/// parallel-edge labels between its direct members.
///
/// Returns `(extra_w, extra_h)` cells.
///
/// **Only fires when the subgraph overrides the parent graph's flow
/// direction** (e.g., a `direction TB` subgraph inside an `LR` graph).
/// When the subgraph inherits the parent direction, the existing
/// `label_gap` mechanism in `compute_positions` already widens the
/// inter-layer crossing to fit parallel labels — adding extra here
/// would double-count and inflate empty subgraph rows/cols.
///
/// Axis: perpendicular to the subgraph's own flow direction.
///   - TB/BT subgraph: labels stack horizontally between rows →
///     expand WIDTH (so the LR/RL parent layer that contains the
///     subgraph's column gets wider).
///   - LR/RL subgraph: labels stack vertically between columns →
///     expand HEIGHT (so the TB/BT parent layer that contains the
///     subgraph's row gets taller).
///
/// Both [`compute_subgraph_bounds`] and the layered layout consume
/// this so the border wraps cleanly around the labels AND external
/// nodes get pushed out by the same amount, avoiding collisions.
pub fn parallel_label_extra(graph: &Graph, sg: &Subgraph) -> (usize, usize) {
    // Only kicks in when the subgraph overrides the parent direction.
    let Some(sg_dir) = sg.direction else {
        return (0, 0);
    };
    if direction_axis(sg_dir) == direction_axis(graph.direction) {
        return (0, 0);
    }

    let parallel_groups = graph.parallel_edge_groups();
    if parallel_groups.is_empty() {
        return (0, 0);
    }
    let members: std::collections::HashSet<&str> =
        sg.node_ids.iter().map(|s| s.as_str()).collect();

    let mut max_label_width: usize = 0;
    for group in &parallel_groups {
        let Some(&first_idx) = group.first() else { continue };
        let Some(first_edge) = graph.edges.get(first_idx) else { continue };
        if !members.contains(first_edge.from.as_str())
            || !members.contains(first_edge.to.as_str())
        {
            continue;
        }
        for &edge_idx in group {
            if let Some(edge) = graph.edges.get(edge_idx)
                && let Some(label) = &edge.label
            {
                max_label_width = max_label_width.max(UnicodeWidthStr::width(label.as_str()));
            }
        }
    }

    if max_label_width == 0 {
        return (0, 0);
    }

    // 2 cells of breathing room beyond the widest label so neither end
    // touches the border or the adjacent box edge.
    let extra = max_label_width + 2;
    match sg_dir {
        Direction::TopToBottom | Direction::BottomToTop => (extra, 0),
        Direction::LeftToRight | Direction::RightToLeft => (0, extra),
    }
}

/// Returns `'h'` for horizontal flows (LR/RL) and `'v'` for vertical
/// (TB/BT). Used to decide whether two directions share an axis.
fn direction_axis(d: Direction) -> char {
    match d {
        Direction::LeftToRight | Direction::RightToLeft => 'h',
        Direction::TopToBottom | Direction::BottomToTop => 'v',
    }
}

/// Axis-aligned bounding box for a rendered subgraph border.
///
/// Coordinates are in character-grid cells, origin top-left.
#[derive(Debug, Clone)]
pub struct SubgraphBounds {
    /// Subgraph ID.
    pub id: String,
    /// Subgraph label (displayed at the top-left of the border).
    pub label: String,
    /// Left column of the outer border (inclusive).
    pub col: usize,
    /// Top row of the outer border (inclusive).
    pub row: usize,
    /// Width of the border rectangle (including border cells).
    pub width: usize,
    /// Height of the border rectangle (including border cells).
    pub height: usize,
    /// Nesting depth (0 = top-level subgraph). Used for draw ordering.
    pub depth: usize,
}

/// Node box dimensions used when computing bounding boxes.
///
/// Must match the values in `render::unicode::NodeGeom` — kept in sync
/// manually. (We can't import from the render module here without a
/// circular dep.)
fn node_draw_width(graph: &Graph, id: &str) -> usize {
    if let Some(node) = graph.node(id) {
        let label_w = node.label_width();
        let inner = label_w + 4; // LABEL_PADDING * 2 = 4
        match node.shape {
            crate::types::NodeShape::Circle
            | crate::types::NodeShape::Stadium
            | crate::types::NodeShape::Hexagon
            | crate::types::NodeShape::Asymmetric
            | crate::types::NodeShape::Subroutine
            | crate::types::NodeShape::Parallelogram
            | crate::types::NodeShape::Trapezoid => inner + 2,
            crate::types::NodeShape::DoubleCircle => inner + 4,
            _ => inner,
        }
    } else {
        6
    }
}

fn node_draw_height(graph: &Graph, id: &str) -> usize {
    if let Some(node) = graph.node(id) {
        let extra = node.label_line_count().saturating_sub(1);
        match node.shape {
            crate::types::NodeShape::Cylinder => 4 + extra,
            crate::types::NodeShape::DoubleCircle => 5 + extra,
            _ => 3 + extra,
        }
    } else {
        3
    }
}

/// Compute bounding boxes for every subgraph in `graph`.
///
/// # Arguments
///
/// * `graph`     — the parsed graph (contains subgraph membership)
/// * `positions` — map from node ID to `(col, row)` grid position (top-left
///   of the node box), as returned by the layout stage
///
/// # Returns
///
/// A list of [`SubgraphBounds`] ordered **outermost first** (suitable for
/// drawing in reverse order so that inner borders are drawn on top of outer
/// ones, preventing outer borders from overwriting inner labels).
///
/// Top-level subgraphs whose nodes are all absent from `positions` are
/// silently omitted.
pub fn compute_subgraph_bounds(
    graph: &Graph,
    positions: &HashMap<String, (usize, usize)>,
) -> Vec<SubgraphBounds> {
    let mut result: Vec<SubgraphBounds> = Vec::new();

    for sg in &graph.subgraphs {
        compute_bounds_recursive(graph, sg, positions, 0, &mut result);
    }

    // Sort: outermost first (ascending depth). Within the same depth,
    // preserve declaration order (stable sort). This ordering is used by
    // the renderer to draw outermost-first, then innermost on top.
    result.sort_by_key(|b| b.depth);

    result
}

/// Recursive helper: compute bounds for `sg` and all its descendants.
///
/// Returns the computed [`SubgraphBounds`] for `sg` (if any nodes were placed),
/// and appends child bounds to `out` first (innermost-first ordering within
/// each branch). The final sort in [`compute_subgraph_bounds`] reorders by depth.
///
/// The parent bounds are expanded to enclose child bounds so that nested
/// subgraph borders are fully contained within their parent's border.
fn compute_bounds_recursive(
    graph: &Graph,
    sg: &Subgraph,
    positions: &HashMap<String, (usize, usize)>,
    depth: usize,
    out: &mut Vec<SubgraphBounds>,
) -> Option<SubgraphBounds> {
    // Recurse into nested subgraphs first; collect their bounds.
    let mut child_bounds: Vec<SubgraphBounds> = Vec::new();
    for child_id in &sg.subgraph_ids {
        if let Some(child) = graph.find_subgraph(child_id)
            && let Some(cb) = compute_bounds_recursive(graph, child, positions, depth + 1, out)
        {
            child_bounds.push(cb);
        }
    }

    // Gather ONLY direct node positions (not descendants, since descendants
    // are covered by child bounds below).
    let mut min_col = usize::MAX;
    let mut min_row = usize::MAX;
    let mut max_col = 0usize;
    let mut max_row = 0usize;
    let mut any = false;

    // Direct node members.
    for nid in &sg.node_ids {
        if let Some(&(col, row)) = positions.get(nid) {
            let w = node_draw_width(graph, nid);
            let h = node_draw_height(graph, nid);
            min_col = min_col.min(col);
            min_row = min_row.min(row);
            max_col = max_col.max(col + w);
            max_row = max_row.max(row + h);
            any = true;
        }
    }

    // Expand to enclose child subgraph borders (including their padding).
    // This ensures the parent border wraps around nested borders, not just
    // around the raw node positions of descendants.
    for cb in &child_bounds {
        min_col = min_col.min(cb.col);
        min_row = min_row.min(cb.row);
        max_col = max_col.max(cb.col + cb.width);
        max_row = max_row.max(cb.row + cb.height);
        any = true;
    }

    if !any {
        return None; // subgraph has no placed nodes — skip
    }

    // Per-axis extra room needed for parallel-edge labels between
    // direct members. The layered layout has already widened the
    // matching layer/row by the same amount (see `compute_positions`),
    // so external nodes won't collide with the grown border.
    let (extra_w, extra_h) = parallel_label_extra(graph, sg);

    // Apply padding: expand the raw content rect by SG_BORDER_PAD on all
    // sides. The label is written into the top border line itself.
    let border_col = min_col.saturating_sub(SG_BORDER_PAD);
    let border_row = min_row.saturating_sub(SG_BORDER_PAD);

    let content_width = (max_col - min_col) + SG_BORDER_PAD * 2 + extra_w;
    // Ensure the border is wide enough to show the full label with 2-cell
    // padding on each side (the corners count as 1 cell each).
    let label_width = UnicodeWidthStr::width(sg.label.as_str()) + 4;
    let border_width = content_width.max(label_width);

    let border_height = (max_row - min_row) + SG_BORDER_PAD * 2 + extra_h;

    let bounds = SubgraphBounds {
        id: sg.id.clone(),
        label: sg.label.clone(),
        col: border_col,
        row: border_row,
        width: border_width,
        height: border_height,
        depth,
    };

    out.push(bounds.clone());
    Some(bounds)
}
