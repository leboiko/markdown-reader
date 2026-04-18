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
//! - `SG_LABEL_HEIGHT = 2` — extra rows/cols reserved above the border for
//!   the label row.

use std::collections::HashMap;

use unicode_width::UnicodeWidthStr;

use crate::types::{Graph, Subgraph};

/// Cells of padding between nodes and the subgraph border.
pub const SG_BORDER_PAD: usize = 2;
/// Extra height (rows) above the content for the label row.
pub const SG_LABEL_HEIGHT: usize = 2;

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
/// Must match the values in `render::unicode` — kept in sync manually.
/// (We can't import from the render module here without a circular dep.)
fn node_draw_width(graph: &Graph, id: &str) -> usize {
    if let Some(node) = graph.node(id) {
        let label_w = UnicodeWidthStr::width(node.label.as_str());
        let inner = label_w + 4; // LABEL_PADDING * 2 = 4
        match node.shape {
            crate::types::NodeShape::Circle => inner + 2,
            _ => inner,
        }
    } else {
        6
    }
}

fn node_draw_height(_graph: &Graph, _id: &str) -> usize {
    3 // all shapes are 3 rows
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

    // Apply padding: expand the raw content rect by SG_BORDER_PAD on all
    // sides. The label is written into the top border line itself.
    let border_col = min_col.saturating_sub(SG_BORDER_PAD);
    let border_row = min_row.saturating_sub(SG_BORDER_PAD);

    let content_width = (max_col - min_col) + SG_BORDER_PAD * 2;
    // Ensure the border is wide enough to show the full label with 2-cell
    // padding on each side (the corners count as 1 cell each).
    let label_width = UnicodeWidthStr::width(sg.label.as_str()) + 4;
    let border_width = content_width.max(label_width);

    let border_height = (max_row - min_row) + SG_BORDER_PAD * 2;

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
