//! Unicode box-drawing renderer.
//!
//! Takes a [`Graph`] and a map of grid positions produced by the layout stage,
//! allocates a [`Grid`] large enough to fit all nodes and edges, draws
//! everything, and returns the final string.

use std::collections::HashMap;

use unicode_width::UnicodeWidthStr;

use crate::{
    layout::{Grid, grid::arrow, layered::GridPos},
    types::{Direction, Graph, Node, NodeShape},
};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Padding added to each side of a node label inside its box.
const LABEL_PADDING: usize = 2;

// ---------------------------------------------------------------------------
// Node geometry
// ---------------------------------------------------------------------------

/// The bounding box dimensions and interior text row for a node.
#[derive(Debug, Clone, Copy)]
struct NodeGeom {
    /// Total width of the box (including borders).
    pub width: usize,
    /// Total height of the box (including borders).
    pub height: usize,
    /// Row offset inside the box where text is centred.
    pub text_row: usize,
}

impl NodeGeom {
    fn for_node(node: &Node) -> Self {
        let label_w = UnicodeWidthStr::width(node.label.as_str());
        let inner_w = label_w + LABEL_PADDING * 2;

        match node.shape {
            // Diamond renders as a rectangle with ◇ markers at top/bottom
            // centre — same dimensions as a plain rectangle.
            NodeShape::Diamond => NodeGeom {
                width: inner_w,
                height: 3,
                text_row: 1,
            },
            NodeShape::Circle => NodeGeom {
                width: inner_w + 2,
                height: 3,
                text_row: 1,
            },
            _ => NodeGeom {
                width: inner_w,
                height: 3,
                text_row: 1,
            },
        }
    }

    /// Column of the horizontal centre of the box, relative to the box origin.
    fn cx(self) -> usize {
        self.width / 2
    }

    /// Row of the vertical centre of the box, relative to the box origin.
    fn cy(self) -> usize {
        self.height / 2
    }
}

// ---------------------------------------------------------------------------
// Attachment point computation
// ---------------------------------------------------------------------------

/// A pixel-precise attachment point on a node's border.
#[derive(Debug, Clone, Copy)]
struct Attach {
    pub col: usize,
    pub row: usize,
}

/// Compute the exit (source) attachment point for a given edge direction.
fn exit_point(pos: GridPos, geom: NodeGeom, dir: Direction) -> Attach {
    let (c, r) = pos;
    match dir {
        Direction::LeftToRight => Attach {
            col: c + geom.width, // one column past the right border
            row: r + geom.cy(),
        },
        Direction::RightToLeft => Attach {
            col: c.saturating_sub(1),
            row: r + geom.cy(),
        },
        Direction::TopToBottom => Attach {
            col: c + geom.cx(),
            row: r + geom.height, // one row below the bottom border
        },
        Direction::BottomToTop => Attach {
            col: c + geom.cx(),
            row: r.saturating_sub(1),
        },
    }
}

/// Compute the entry (destination) attachment point for a given edge direction.
fn entry_point(pos: GridPos, geom: NodeGeom, dir: Direction) -> Attach {
    let (c, r) = pos;
    match dir {
        Direction::LeftToRight => Attach {
            col: c.saturating_sub(1), // one column before the left border
            row: r + geom.cy(),
        },
        Direction::RightToLeft => Attach {
            col: c + geom.width,
            row: r + geom.cy(),
        },
        Direction::TopToBottom => Attach {
            col: c + geom.cx(),
            row: r.saturating_sub(1),
        },
        Direction::BottomToTop => Attach {
            col: c + geom.cx(),
            row: r + geom.height,
        },
    }
}

/// Select the correct arrow tip character for the given direction.
fn tip_char(dir: Direction) -> char {
    match dir {
        Direction::LeftToRight => arrow::RIGHT,
        Direction::RightToLeft => arrow::LEFT,
        Direction::TopToBottom => arrow::DOWN,
        Direction::BottomToTop => arrow::UP,
    }
}

// ---------------------------------------------------------------------------
// Grid sizing
// ---------------------------------------------------------------------------

/// Compute the minimum grid dimensions needed to hold all nodes and edges.
///
/// The grid must be wide/tall enough to hold node boxes plus any edge labels.
/// For LR/RL flows, labels are written horizontally between layers, so the
/// extra width needed is the longest edge label width. For TD/BT, the same
/// applies vertically (extra height). Both axes also receive a fixed +4 margin
/// for arrow heads and routing headroom.
fn grid_size(
    graph: &Graph,
    positions: &HashMap<String, GridPos>,
    geoms: &HashMap<String, NodeGeom>,
) -> (usize, usize) {
    let mut max_col = 0usize;
    let mut max_row = 0usize;

    for node in &graph.nodes {
        if let (Some(&(c, r)), Some(&g)) = (positions.get(&node.id), geoms.get(&node.id)) {
            max_col = max_col.max(c + g.width + 4);
            max_row = max_row.max(r + g.height + 4);
        }
    }

    // Extra room for edge labels: labels can extend past the last node.
    let max_label_w = graph
        .edges
        .iter()
        .filter_map(|e| e.label.as_deref())
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0);

    if max_label_w > 0 {
        // Reserve label width + 2 padding on both axes to cover worst-case
        // label positions (labels on back-edges can appear at the far edge).
        max_col += max_label_w + 2;
        max_row += max_label_w + 2;
    }

    (max_col.max(1), max_row.max(1))
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render `graph` with precomputed `positions` into a Unicode string.
///
/// # Arguments
///
/// * `graph`     — the parsed flowchart
/// * `positions` — map from node ID to `(col, row)` grid position (top-left
///   corner of the node's bounding box)
pub fn render(graph: &Graph, positions: &HashMap<String, GridPos>) -> String {
    // Pre-compute geometry for every node
    let geoms: HashMap<String, NodeGeom> = graph
        .nodes
        .iter()
        .map(|n| (n.id.clone(), NodeGeom::for_node(n)))
        .collect();

    let (width, height) = grid_size(graph, positions, &geoms);
    let mut grid = Grid::new(width, height);

    // Pass 0: Register all node bounding boxes as hard routing obstacles so
    // that A* edge routing will not route edges through node interiors.
    for node in &graph.nodes {
        let Some(&(col, row)) = positions.get(&node.id) else {
            continue;
        };
        let Some(&geom) = geoms.get(&node.id) else {
            continue;
        };
        grid.mark_node_box(col, row, geom.width, geom.height);
    }

    // Compute spread-adjusted attach points for all edges before drawing.
    // Both exit and entry points are spread so that multiple edges sharing
    // the same border cell each get their own distinct row/column.
    let attach_points = compute_spread_attaches(graph, positions, &geoms);

    // Pass 1: Route all edges using A* obstacle-aware routing.
    // Collect edge label placements for a deferred write — labels must be
    // written *after* all routing so that no subsequent A* path overwrites them.
    // Each entry is `(col, row, label_text)`.
    let mut pending_labels: Vec<(usize, usize, String)> = Vec::new();
    // Collision registry: `(col, row, display_width, height)` of committed labels.
    let mut placed_labels: Vec<(usize, usize, usize, usize)> = Vec::new();

    for (edge_idx, edge) in graph.edges.iter().enumerate() {
        let Some(Some((src, dst))) = attach_points.get(edge_idx) else {
            continue;
        };
        let (src, dst) = (*src, *dst);

        let tip = tip_char(graph.direction);
        let horizontal_first = graph.direction.is_horizontal();
        let path = grid.route_edge(src.col, src.row, dst.col, dst.row, horizontal_first, tip);

        // Compute edge label position using the actual routed path.
        if let (Some(lbl), Some(path)) = (&edge.label, &path)
            && let Some((lbl_col, lbl_row)) =
                label_position(path, lbl, graph.direction, &mut placed_labels)
        {
            pending_labels.push((lbl_col, lbl_row, lbl.clone()));
        }
    }

    // Pass 2: Draw node box outlines (overwrite any stray edge lines inside
    // the node boundary).
    for node in &graph.nodes {
        let Some(&pos) = positions.get(&node.id) else {
            continue;
        };
        let Some(&geom) = geoms.get(&node.id) else {
            continue;
        };
        draw_node_box(&mut grid, node, pos, geom);
    }

    // Pass 2b: Write all edge labels after node boxes so that node box
    // drawing (which uses `set()` unconditionally) cannot overwrite labels.
    // Labels are protected so that node labels in pass 3 cannot erase them.
    for (lbl_col, lbl_row, lbl) in &pending_labels {
        grid.write_text_protected(*lbl_col, *lbl_row, lbl);
    }

    // Pass 3: Draw node labels last so they are never overwritten.
    for node in &graph.nodes {
        let Some(&pos) = positions.get(&node.id) else {
            continue;
        };
        let Some(&geom) = geoms.get(&node.id) else {
            continue;
        };
        draw_label_centred(&mut grid, node, pos, geom);
    }

    grid.render()
}

// ---------------------------------------------------------------------------
// Endpoint spreading
// ---------------------------------------------------------------------------

/// Compute spread-adjusted `(src, dst)` attach pairs for every edge.
///
/// Edges that converge on the same destination cell (or diverge from the same
/// source cell) would all draw their arrow tips on the same pixel, producing
/// `┬┬` artefacts. This function redistributes those endpoints symmetrically
/// along the node border, one cell apart, so each edge gets its own row or
/// column.
///
/// Termaid spreads only destination endpoints (not source endpoints) to avoid
/// border artefacts from diverging jog segments. We follow the same approach.
///
/// Returns a `Vec` indexed identically to `graph.edges`; edges whose nodes
/// aren't present in `positions` are represented by `None`.
fn compute_spread_attaches(
    graph: &Graph,
    positions: &HashMap<String, GridPos>,
    geoms: &HashMap<String, NodeGeom>,
) -> Vec<Option<(Attach, Attach)>> {
    // --- Build the base (unspread) attach points ---
    let mut pairs: Vec<Option<(Attach, Attach)>> = graph
        .edges
        .iter()
        .map(|edge| {
            let from_pos = *positions.get(&edge.from)?;
            let to_pos = *positions.get(&edge.to)?;
            let from_geom = *geoms.get(&edge.from)?;
            let to_geom = *geoms.get(&edge.to)?;
            let src = exit_point(from_pos, from_geom, graph.direction);
            let dst = entry_point(to_pos, to_geom, graph.direction);
            Some((src, dst))
        })
        .collect();

    // --- Spread destination endpoints ---
    // Group edge indices by their base destination cell.
    let mut dst_groups: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (i, pair) in pairs.iter().enumerate() {
        if let Some((_, dst)) = pair {
            dst_groups.entry((dst.col, dst.row)).or_default().push(i);
        }
    }

    for indices in dst_groups.values() {
        if indices.len() <= 1 {
            continue;
        }
        // All edges in this group arrive at the same border cell on the same node.
        // Identify the target node and its geometry so we know the spread bounds.
        let first_edge = &graph.edges[indices[0]];
        let Some(&to_pos) = positions.get(&first_edge.to) else {
            continue;
        };
        let Some(&to_geom) = geoms.get(&first_edge.to) else {
            continue;
        };
        spread_destinations(&mut pairs, indices, to_pos, to_geom, graph.direction);
    }

    // --- Spread source endpoints ---
    // Group edge indices by their base source cell.
    let mut src_groups: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (i, pair) in pairs.iter().enumerate() {
        if let Some((src, _)) = pair {
            src_groups.entry((src.col, src.row)).or_default().push(i);
        }
    }

    for indices in src_groups.values() {
        if indices.len() <= 1 {
            continue;
        }
        let first_edge = &graph.edges[indices[0]];
        let Some(&from_pos) = positions.get(&first_edge.from) else {
            continue;
        };
        let Some(&from_geom) = geoms.get(&first_edge.from) else {
            continue;
        };
        spread_sources(&mut pairs, indices, from_pos, from_geom, graph.direction);
    }

    pairs
}

/// Spread destination attach points of `indices` symmetrically along the
/// target node's border, perpendicular to the flow direction.
///
/// For LR (horizontal flow): edges arrive from the left, so we spread
/// vertically (±row). For TD (vertical flow): we spread horizontally (±col).
fn spread_destinations(
    pairs: &mut [Option<(Attach, Attach)>],
    indices: &[usize],
    to_pos: GridPos,
    to_geom: NodeGeom,
    dir: Direction,
) {
    let n = indices.len();
    let (to_col, to_row) = to_pos;

    match dir {
        Direction::LeftToRight | Direction::RightToLeft => {
            // Destinations arrive one column before the left border; spread
            // vertically across the full node height.
            let min_row = to_row;
            let max_row = to_row + to_geom.height.saturating_sub(1);
            if max_row < min_row || max_row - min_row + 1 < n {
                return;
            }
            let centre = (to_row + to_geom.cy()) as isize;
            let spread_range = (max_row - min_row) as isize;
            let step = if n > 1 {
                (spread_range / (n as isize - 1)).clamp(1, 2)
            } else {
                1
            };
            for (i, &idx) in indices.iter().enumerate() {
                let offset = (i as isize - (n as isize - 1) / 2) * step;
                let new_row = (centre + offset)
                    .max(min_row as isize)
                    .min(max_row as isize) as usize;
                if let Some((_, dst)) = &mut pairs[idx] {
                    dst.row = new_row;
                }
            }
        }
        Direction::TopToBottom | Direction::BottomToTop => {
            // Destinations arrive one row above the top border; spread
            // horizontally across the full node width.
            let min_col = to_col;
            let max_col = to_col + to_geom.width.saturating_sub(1);
            if max_col < min_col || max_col - min_col + 1 < n {
                return;
            }
            let centre = (to_col + to_geom.cx()) as isize;
            let spread_range = (max_col - min_col) as isize;
            let step = if n > 1 {
                (spread_range / (n as isize - 1)).clamp(1, 2)
            } else {
                1
            };
            for (i, &idx) in indices.iter().enumerate() {
                let offset = (i as isize - (n as isize - 1) / 2) * step;
                let new_col = (centre + offset)
                    .max(min_col as isize)
                    .min(max_col as isize) as usize;
                if let Some((_, dst)) = &mut pairs[idx] {
                    dst.col = new_col;
                }
            }
        }
    }
}

/// Spread source attach points of `indices` symmetrically along the source
/// node's border, perpendicular to the flow direction.
fn spread_sources(
    pairs: &mut [Option<(Attach, Attach)>],
    indices: &[usize],
    from_pos: GridPos,
    from_geom: NodeGeom,
    dir: Direction,
) {
    let n = indices.len();
    let (from_col, from_row) = from_pos;

    match dir {
        Direction::LeftToRight | Direction::RightToLeft => {
            // Exit cells are one column past the right border. Spread rows
            // symmetrically across the full node height. When n > available
            // rows, some edges will share a row (clamping) — this still
            // reduces clustering vs. all sharing the centre row.
            let min_row = from_row;
            let max_row = from_row + from_geom.height.saturating_sub(1);
            if min_row > max_row {
                return;
            }
            let available = max_row - min_row + 1;
            if available < 2 {
                return; // single-row node, nothing to spread
            }
            let centre = (from_row + from_geom.cy()) as isize;
            let spread_range = (max_row - min_row) as isize;
            // Use at most half the range per step to keep paths adjacent.
            let step = if n > 1 {
                (spread_range / (n as isize - 1)).clamp(1, 2)
            } else {
                1
            };
            for (i, &idx) in indices.iter().enumerate() {
                let offset = (i as isize - (n as isize - 1) / 2) * step;
                let new_row = (centre + offset)
                    .max(min_row as isize)
                    .min(max_row as isize) as usize;
                if let Some((src, _)) = &mut pairs[idx] {
                    src.row = new_row;
                }
            }
        }
        Direction::TopToBottom | Direction::BottomToTop => {
            // Exit cells are one row past the bottom border. Spread columns
            // across the full node width.
            let min_col = from_col;
            let max_col = from_col + from_geom.width.saturating_sub(1);
            if min_col > max_col {
                return;
            }
            let available = max_col - min_col + 1;
            if available < 2 {
                return;
            }
            let centre = (from_col + from_geom.cx()) as isize;
            let spread_range = (max_col - min_col) as isize;
            let step = if n > 1 {
                (spread_range / (n as isize - 1)).clamp(1, 2)
            } else {
                1
            };
            for (i, &idx) in indices.iter().enumerate() {
                let offset = (i as isize - (n as isize - 1) / 2) * step;
                let new_col = (centre + offset)
                    .max(min_col as isize)
                    .min(max_col as isize) as usize;
                if let Some((src, _)) = &mut pairs[idx] {
                    src.col = new_col;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Node drawing
// ---------------------------------------------------------------------------

/// Draw the border/outline of a node box at `pos`, clearing the interior.
///
/// Interior cells are filled with spaces to erase any edge lines that the
/// layout may have routed through the node's bounding box (e.g. back-edges
/// in cyclic graphs). Labels are written in a separate pass after this.
fn draw_node_box(grid: &mut Grid, node: &Node, pos: GridPos, geom: NodeGeom) {
    let (col, row) = pos;

    // Clear the interior rows (all rows except top and bottom border).
    // For diamonds the interior is the space between the diagonal lines;
    // we clear every row to keep things simple.
    for y in (row + 1)..(row + geom.height.saturating_sub(1)) {
        for x in (col + 1)..(col + geom.width.saturating_sub(1)) {
            grid.set(x, y, ' ');
        }
    }

    match node.shape {
        NodeShape::Rectangle => {
            grid.draw_box(col, row, geom.width, geom.height);
        }
        NodeShape::Rounded => {
            grid.draw_rounded_box(col, row, geom.width, geom.height);
        }
        NodeShape::Diamond => {
            grid.draw_diamond(col, row, geom.width, geom.height);
        }
        NodeShape::Circle => {
            // Render circle as rounded box with extra parenthesis markers
            grid.draw_rounded_box(col, row, geom.width, geom.height);
            // Place '(' and ')' inside the leftmost/rightmost interior columns
            let mid = row + geom.cy();
            grid.set(col + 1, mid, '(');
            grid.set(col + geom.width - 2, mid, ')');
        }
    }
}

/// Write a node's label horizontally centred inside its bounding box.
fn draw_label_centred(grid: &mut Grid, node: &Node, pos: GridPos, geom: NodeGeom) {
    let (col, row) = pos;
    let label_w = UnicodeWidthStr::width(node.label.as_str());

    // Centre the label within the interior width (geom.width - 2 borders)
    let interior_w = geom.width.saturating_sub(2);
    let text_col = if label_w <= interior_w {
        col + 1 + (interior_w - label_w) / 2
    } else {
        col + 1
    };

    // Diamond now renders as a rectangle — no extra horizontal offset needed.
    // The standard centring computed above is correct for all shapes.

    grid.write_text(text_col, row + geom.text_row, &node.label);
}

// ---------------------------------------------------------------------------
// Edge label placement
// ---------------------------------------------------------------------------

/// Compute the `(col, row)` position where an edge label should be written.
///
/// Strategy (following termaid's `_find_last_turn` / `_try_place_on_segment`):
/// - For LR/RL flows: find the **last** horizontal segment in the path
///   (closest to the arrow tip — the part unique to this edge, not shared with
///   sibling edges from the same source). Place the label one row above the
///   segment, at the 1/3 point from the source end (to avoid crowding the
///   destination node).
/// - For TD/BT flows: find the **last** vertical segment and place the label
///   one column to the right of the segment midpoint.
///
/// `placed` is a collision registry of already-committed bounding boxes
/// `(col, row, display_width, height=1)`. On collision, up to 4 candidate
/// positions are tried before the label is silently dropped.
///
/// Returns `Some((col, row))` on success and updates `placed`. Returns `None`
/// if no collision-free position was found.
fn label_position(
    path: &[(usize, usize)],
    label: &str,
    dir: Direction,
    placed: &mut Vec<(usize, usize, usize, usize)>,
) -> Option<(usize, usize)> {
    if path.len() < 2 {
        return None;
    }
    let lbl_w = UnicodeWidthStr::width(label);
    if lbl_w == 0 {
        return None;
    }

    match dir {
        Direction::LeftToRight | Direction::RightToLeft => {
            // Find the last long horizontal segment (closest to the tip).
            let (seg_col, seg_row) = last_horizontal_segment(path)?;
            // Candidates: above the segment, then below.
            let candidates = [
                seg_row.saturating_sub(1),
                seg_row + 1,
                seg_row.saturating_sub(2),
                seg_row + 2,
            ];
            for lbl_row in candidates {
                if !collides(seg_col, lbl_row, lbl_w, placed) {
                    placed.push((seg_col, lbl_row, lbl_w, 1));
                    return Some((seg_col, lbl_row));
                }
            }
            None
        }
        Direction::TopToBottom | Direction::BottomToTop => {
            // Find the last long vertical segment (closest to the tip).
            let (seg_col, seg_row) = last_vertical_segment(path)?;
            // Try placing to the right of the segment, at the midpoint row
            // first, then adjacent rows as fallback.
            let col_candidates = [seg_col + 1, seg_col.saturating_sub(1), seg_col + 2];
            let row_offsets: [isize; 5] = [0, -1, 1, -2, 2];
            for lbl_col in col_candidates {
                for &dr in &row_offsets {
                    let lbl_row = (seg_row as isize + dr).max(0) as usize;
                    if !collides(lbl_col, lbl_row, lbl_w, placed) {
                        placed.push((lbl_col, lbl_row, lbl_w, 1));
                        return Some((lbl_col, lbl_row));
                    }
                }
            }
            None
        }
    }
}

/// Find the midpoint `(col, row)` of the **last** horizontal run in `path`
/// that is at least 2 cells long. "Last" = closest to the tip (end of path).
///
/// Returns `None` if no such segment exists.
fn last_horizontal_segment(path: &[(usize, usize)]) -> Option<(usize, usize)> {
    // Walk the path from the end, collecting runs of equal row.
    let n = path.len();
    let mut i = n.saturating_sub(2); // start one before the tip
    loop {
        let row = path[i].1;
        // Extend the run backward while on the same row.
        let mut start = i;
        while start > 0 && path[start - 1].1 == row {
            start -= 1;
        }
        let run_len = i - start + 1;
        if run_len >= 2 {
            // Midpoint column of this horizontal run.
            let mid_col = (path[start].0 + path[i].0) / 2;
            return Some((mid_col, row));
        }
        if i == 0 {
            break;
        }
        i = start.saturating_sub(1);
        if i == 0 && path[0].1 != row {
            break;
        }
    }
    None
}

/// Find the midpoint `(col, row)` of the **last** vertical run in `path`
/// that is at least 2 cells long. "Last" = closest to the tip.
///
/// Returns `None` if no such segment exists.
fn last_vertical_segment(path: &[(usize, usize)]) -> Option<(usize, usize)> {
    let n = path.len();
    let mut i = n.saturating_sub(2);
    loop {
        let col = path[i].0;
        let mut start = i;
        while start > 0 && path[start - 1].0 == col {
            start -= 1;
        }
        let run_len = i - start + 1;
        if run_len >= 2 {
            let mid_row = (path[start].1 + path[i].1) / 2;
            return Some((col, mid_row));
        }
        if i == 0 {
            break;
        }
        i = start.saturating_sub(1);
        if i == 0 && path[0].0 != col {
            break;
        }
    }
    None
}

/// Return `true` if a label of display width `w` placed at `(col, row)` would
/// overlap (or be directly adjacent to, with less than 1 cell gap) any
/// previously placed label bounding box in `placed`.
///
/// Each entry in `placed` is `(col, row, width, height)`. Labels are assumed
/// to be 1 row tall. A 1-cell margin is enforced on both sides to ensure
/// labels are visually separated.
fn collides(col: usize, row: usize, w: usize, placed: &[(usize, usize, usize, usize)]) -> bool {
    for &(pc, pr, pw, ph) in placed {
        // Row overlap
        let row_overlaps = (row >= pr && row < pr + ph) || (pr >= row && pr < row + 1);
        if row_overlaps {
            // Column overlap with 1-cell margin: treat the new label as
            // [col-1, col+w+1) and check against [pc, pc+pw).
            let padded_start = col.saturating_sub(1);
            let padded_end = col + w + 1;
            let no_col_overlap = padded_end <= pc || pc + pw <= padded_start;
            if !no_col_overlap {
                return true;
            }
        }
    }
    false
}


// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        layout::layered::{LayoutConfig, layout},
        parser,
    };

    fn render_diagram(src: &str) -> String {
        let graph = parser::parse(src).unwrap();
        let positions = layout(&graph, &LayoutConfig::default());
        render(&graph, &positions)
    }

    #[test]
    fn lr_output_contains_node_labels() {
        let out = render_diagram("graph LR\nA[Start] --> B[End]");
        assert!(out.contains("Start"), "missing 'Start' in:\n{out}");
        assert!(out.contains("End"), "missing 'End' in:\n{out}");
    }

    #[test]
    fn td_output_contains_node_labels() {
        let out = render_diagram("graph TD\nA[Top] --> B[Bottom]");
        assert!(out.contains("Top"), "missing 'Top' in:\n{out}");
        assert!(out.contains("Bottom"), "missing 'Bottom' in:\n{out}");
    }
}
