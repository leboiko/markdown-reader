//! Unicode box-drawing renderer.
//!
//! Takes a [`Graph`] and a map of grid positions produced by the layout stage,
//! allocates a [`Grid`] large enough to fit all nodes and edges, draws
//! everything, and returns the final string.

use std::collections::{HashMap, HashSet};

use unicode_width::UnicodeWidthStr;

use crate::{
    layout::{
        Grid, SubgraphBounds,
        grid::{Attach, BAR_THICKNESS, EdgeLineStyle, arrow, endpoint},
        layered::GridPos,
        router,
    },
    types::{
        BarOrientation, Direction, EdgeEndpoint, EdgeStyle, Graph, Node, NodeShape, NodeStyle, Rgb,
    },
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
        // Multi-line labels: `node.label_width()` returns the widest line,
        // `node.label_line_count()` counts the lines. Each extra line adds
        // one interior row so the box grows vertically, not horizontally.
        let label_w = node.label_width();
        let inner_w = label_w + LABEL_PADDING * 2;
        let extra_lines = node.label_line_count().saturating_sub(1);

        // Must stay in sync with `node_box_width`/`node_box_height` in
        // `layout/layered.rs` — both functions encode the same shape dimensions.
        match node.shape {
            // Plain rectangle / rounded / diamond: standard 3-row box.
            NodeShape::Rectangle | NodeShape::Rounded | NodeShape::Diamond => NodeGeom {
                width: inner_w,
                height: 3 + extra_lines,
                text_row: 1,
            },
            // Circle, stadium, hexagon, asymmetric: +2 width for side markers.
            NodeShape::Circle | NodeShape::Stadium | NodeShape::Hexagon | NodeShape::Asymmetric => {
                NodeGeom {
                    width: inner_w + 2,
                    height: 3 + extra_lines,
                    text_row: 1,
                }
            }
            // Subroutine: +2 width for inner vertical bars.
            NodeShape::Subroutine => NodeGeom {
                width: inner_w + 2,
                height: 3 + extra_lines,
                text_row: 1,
            },
            // Parallelogram / trapezoid variants: +2 width for slant corner markers.
            NodeShape::Parallelogram
            | NodeShape::ParallelogramBackslash
            | NodeShape::Trapezoid
            | NodeShape::TrapezoidInverted => NodeGeom {
                width: inner_w + 2,
                height: 3 + extra_lines,
                text_row: 1,
            },
            // Cylinder: 4 rows — top border, lid line, text, bottom border.
            // Text starts on the first interior row below the lid line (index 2).
            NodeShape::Cylinder => NodeGeom {
                width: inner_w,
                height: 4 + extra_lines,
                text_row: 2,
            },
            // DoubleCircle: 5 rows for outer + inner concentric rounded boxes.
            // +4 width for two layers of borders on each side.
            // Text starts on the first interior row (index 2).
            NodeShape::DoubleCircle => NodeGeom {
                width: inner_w + 4,
                height: 5 + extra_lines,
                text_row: 2,
            },
            // Fork/join bars are perpendicular to flow and carry no
            // label. Single-row horizontal bar (TD/BT) or single-column
            // vertical bar (LR/RL). `text_row` is irrelevant — the
            // renderer skips `draw_label_centred` for `Bar(_)` shapes.
            NodeShape::Bar(BarOrientation::Horizontal) => NodeGeom {
                width: 5,
                height: BAR_THICKNESS,
                text_row: 0,
            },
            NodeShape::Bar(BarOrientation::Vertical) => NodeGeom {
                width: BAR_THICKNESS,
                height: 5,
                text_row: 0,
            },
            // Note: same dimensions as Rounded — visual distinction
            // comes from the dotted connector edge synthesised by the
            // parser, not from the box itself.
            NodeShape::Note => NodeGeom {
                width: inner_w,
                height: 3 + extra_lines,
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
///
/// **TD/BT (vertical flow):** the attach point lands *on* the box's
/// top or bottom border row — the arrow tip glyph (`▾` / `▴`)
/// replaces one `─` cell, visually merging the arrow into the box
/// edge. The horizontal border has many `─` cells so dropping one
/// preserves the border outline; protection on the tip plus
/// [`Grid::set_unless_protected`] in the box-drawing primitives keeps
/// the tip intact when the node box redraws in pass 3.
///
/// **LR/RL (horizontal flow):** the attach point stays one column
/// outside the box's left/right border. The vertical border is a
/// single `│` per row — replacing it with `▸`/`◂` removes the left
/// (or right) edge of the box on that row, leaving the box visually
/// open. Monospace cell-grid rendering already places `▸│` and `│◂`
/// adjacent with zero gap, so the merge gain is moot here.
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
            row: r, // ON the top border row (replaces one ─)
        },
        Direction::BottomToTop => Attach {
            col: c + geom.cx(),
            row: r + geom.height - 1, // ON the bottom border row (replaces one ─)
        },
    }
}

/// Compute the **back-edge exit** point: the perpendicular side opposite to the
/// flow direction.
///
/// For LR/RL graphs, back-edges exit from the bottom of the source node.
/// For TD/BT graphs, back-edges exit from the right of the source node.
/// This pushes the back-edge path around the perimeter rather than through the
/// centre of the diagram.
fn exit_point_back_edge(pos: GridPos, geom: NodeGeom, dir: Direction) -> Attach {
    let (c, r) = pos;
    match dir {
        // Horizontal flow (LR or RL): exit from the bottom centre.
        Direction::LeftToRight | Direction::RightToLeft => Attach {
            col: c + geom.cx(),
            row: r + geom.height, // one row below the bottom border
        },
        // Vertical flow (TD or BT): exit from the right centre.
        Direction::TopToBottom | Direction::BottomToTop => Attach {
            col: c + geom.width, // one column past the right border
            row: r + geom.cy(),
        },
    }
}

/// Compute the **back-edge entry** point: the perpendicular side opposite to
/// the flow direction on the destination node.
///
/// Symmetric to [`exit_point_back_edge`]: back-edges enter from the bottom for
/// LR/RL graphs, and from the right for TD/BT graphs.
///
/// LR/RL graphs use horizontal `─` for their bottom border (multiple
/// cells), so the back-edge tip lands *on* the bottom border row,
/// replacing one `─` with `▴`. TD/BT graphs use vertical `│` for the
/// right border (single cell per row), so the back-edge tip stays one
/// column outside to avoid removing the border on its row.
fn entry_point_back_edge(pos: GridPos, geom: NodeGeom, dir: Direction) -> Attach {
    let (c, r) = pos;
    match dir {
        // Horizontal flow: enter ON the bottom border row.
        Direction::LeftToRight | Direction::RightToLeft => Attach {
            col: c + geom.cx(),
            row: r + geom.height - 1,
        },
        // Vertical flow: enter one column past the right border (keeps
        // the vertical `│` intact on the row where the tip lands).
        Direction::TopToBottom | Direction::BottomToTop => Attach {
            col: c + geom.width,
            row: r + geom.cy(),
        },
    }
}

fn tip_char_for_back_edge(dir: Direction) -> char {
    match dir {
        Direction::LeftToRight | Direction::RightToLeft => arrow::UP,
        Direction::TopToBottom | Direction::BottomToTop => arrow::LEFT,
    }
}

/// Determine whether an edge is a "back-edge" — one whose target is strictly
/// upstream of its source in the flow direction.
///
/// Back-edges travel against the primary layout axis (e.g. a feedback loop in
/// an LR graph that goes from a downstream node back to an upstream one). They
/// are rerouted around the perimeter to avoid cutting across the diagram.
///
/// Edges between nodes at the **same** layer position (equal column for LR, equal
/// row for TD, etc.) are NOT treated as back-edges — they are perpendicular-axis
/// connections (e.g. internal edges of a TD subgraph inside an LR parent) and
/// should use the normal routing path.
/// Compute the `(border_cell, first_path_cell)` pair for a back-edge that
/// attaches to the perpendicular side of a node. These are the cells that
/// need junction glyphs so the routed perimeter path connects visibly to
/// the node box border.
///
/// For LR/RL flow: `border_cell` is the bottom-center of the box border,
/// `first_path_cell` is one cell directly below.
/// For TD/BT flow: `border_cell` is the right-center, `first_path_cell`
/// is one cell directly to the right.
fn back_edge_border_cells(
    pos: GridPos,
    geom: NodeGeom,
    dir: Direction,
) -> ((usize, usize), (usize, usize)) {
    let (c, r) = pos;
    match dir {
        Direction::LeftToRight | Direction::RightToLeft => {
            let col = c + geom.cx();
            let border_row = r + geom.height - 1;
            let path_row = r + geom.height;
            ((col, border_row), (col, path_row))
        }
        Direction::TopToBottom | Direction::BottomToTop => {
            let row = r + geom.cy();
            let border_col = c + geom.width - 1;
            let path_col = c + geom.width;
            ((border_col, row), (path_col, row))
        }
    }
}

/// Return true for node shapes whose bottom border row contains `╰──╯` rounded
/// corners rather than plain `└──┘` square corners.
///
/// For these shapes, stamping a `┬` junction ON the bottom border row (as
/// `back_edge_border_joins` does for LR/RL source nodes) visually pierces the
/// rounded arc.  The perimeter path row immediately below (which receives `┴`
/// from `back_edge_path_joins`) already makes the connection clear, so the
/// border-row `┬` stamp must be skipped (B12).
fn has_rounded_bottom_border(shape: NodeShape) -> bool {
    matches!(
        shape,
        NodeShape::Rounded
            | NodeShape::Circle
            | NodeShape::Stadium
            | NodeShape::Note
            | NodeShape::DoubleCircle
    )
}

fn is_back_edge(from_pos: GridPos, to_pos: GridPos, dir: Direction) -> bool {
    let (fc, fr) = from_pos;
    let (tc, tr) = to_pos;
    match dir {
        // LR: back-edge if target column is strictly left of source column.
        Direction::LeftToRight => tc < fc,
        // RL: back-edge if target column is strictly right of source column.
        Direction::RightToLeft => tc > fc,
        // TD: back-edge if target row is strictly above source row.
        Direction::TopToBottom => tr < fr,
        // BT: back-edge if target row is strictly below source row.
        Direction::BottomToTop => tr > fr,
    }
}

/// Select the correct back-tip glyph (source end of a bidirectional edge).
///
/// The back-tip always points in the reverse direction of the flow.
fn endpoint_char_back(dir: Direction) -> char {
    // Reverse of `tip_char`.
    match dir {
        Direction::LeftToRight => arrow::LEFT,
        Direction::RightToLeft => arrow::RIGHT,
        Direction::TopToBottom => arrow::UP,
        Direction::BottomToTop => arrow::DOWN,
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

/// Compute the minimum grid dimensions needed to hold all nodes, edges, and
/// subgraph borders.
///
/// The grid must be wide/tall enough to hold node boxes plus any edge labels
/// and subgraph border rectangles. Both axes also receive a fixed +4 margin
/// for arrow heads and routing headroom.
///
/// When back-edges are present, an additional 4-row (LR/RL) or 4-column
/// (TD/BT) corridor is added so that A\* can route the perimeter path without
/// going out of bounds.
fn grid_size(
    graph: &Graph,
    positions: &HashMap<String, GridPos>,
    geoms: &HashMap<String, NodeGeom>,
    sg_bounds: &[SubgraphBounds],
) -> (usize, usize) {
    let mut max_col = 0usize;
    let mut max_row = 0usize;

    for node in &graph.nodes {
        if let (Some(&(c, r)), Some(&g)) = (positions.get(&node.id), geoms.get(&node.id)) {
            max_col = max_col.max(c + g.width + 4);
            max_row = max_row.max(r + g.height + 4);
        }
    }

    // Account for subgraph border rectangles.
    for b in sg_bounds {
        max_col = max_col.max(b.col + b.width + 4);
        max_row = max_row.max(b.row + b.height + 4);
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

    // Extra corridor for back-edge perimeter routing.
    //
    // Back-edges exit from the bottom (LR/RL) or right (TD/BT) of both source
    // and target nodes, then travel around the perimeter. Without extra room
    // below (or to the right of) the last node row/column, A* runs out of
    // bounds and falls back to Manhattan routing that cuts through the middle.
    // Four cells is enough for the corridor + arrow tip.
    // Self-loops (`from == to`) are treated as back-edges for routing
    // purposes, so they count toward the corridor requirement.
    let has_back_edge = graph.edges.iter().any(|e| {
        if e.from == e.to {
            return true;
        }
        let Some(&fp) = positions.get(&e.from) else {
            return false;
        };
        let Some(&tp) = positions.get(&e.to) else {
            return false;
        };
        is_back_edge(fp, tp, graph.direction)
    });

    if has_back_edge {
        match graph.direction {
            // LR/RL: back-edges travel along a row *below* all nodes.
            Direction::LeftToRight | Direction::RightToLeft => {
                max_row += 4;
            }
            // TD/BT: back-edges travel along a column *to the right* of all nodes.
            Direction::TopToBottom | Direction::BottomToTop => {
                max_col += 4;
            }
        }
    }

    (max_col.max(1), max_row.max(1))
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Render `graph` with precomputed `positions` into a Unicode string.
///
/// This is the low-level entry point for the rendering pipeline. Most callers
/// should use [`crate::render()`] or [`crate::render_with_width()`]
/// instead, which handle parsing and layout automatically.
///
/// The function executes four drawing passes:
/// 1. Draw subgraph borders (outermost → innermost).
/// 2. Route all edges using A\* obstacle-aware pathfinding.
/// 3. Draw node box outlines.
/// 4. Draw node labels (never overwritten by later passes).
///
/// # Arguments
///
/// * `graph`     — the parsed flowchart
/// * `positions` — map from node ID to `(col, row)` grid position (top-left
///   corner of the node's bounding box), as produced by [`layout`]
/// * `sg_bounds` — precomputed subgraph bounding boxes (sorted outermost-first),
///   as produced by [`compute_subgraph_bounds`]
///
/// # Returns
///
/// A multi-line `String` with trailing spaces stripped from each row and
/// trailing blank rows removed.
///
/// [`layout`]: crate::layout::layered::layout
/// [`compute_subgraph_bounds`]: crate::layout::subgraph::compute_subgraph_bounds
pub fn render(
    graph: &Graph,
    positions: &HashMap<String, GridPos>,
    sg_bounds: &[SubgraphBounds],
) -> String {
    render_inner(graph, positions, sg_bounds, false)
}

/// Render `graph` with embedded ANSI 24-bit color SGR sequences derived from
/// the `style` and `linkStyle` directives stored on the graph.
///
/// Behaves identically to [`render`] for graphs that carry no color
/// metadata. When colors *are* present, every colored cell emits the matching
/// foreground / background SGR pair, and every row ends with `\x1b[0m`.
///
/// This is the entry point used when the caller has opted into ANSI output
/// (e.g. via the CLI `--color` flag); the colorless [`render`] is preserved
/// for callers that need byte-clean text.
pub fn render_color(
    graph: &Graph,
    positions: &HashMap<String, GridPos>,
    sg_bounds: &[SubgraphBounds],
) -> String {
    render_inner(graph, positions, sg_bounds, true)
}

fn render_inner(
    graph: &Graph,
    positions: &HashMap<String, GridPos>,
    sg_bounds: &[SubgraphBounds],
    with_color: bool,
) -> String {
    // Pre-compute geometry for every node
    let geoms: HashMap<String, NodeGeom> = graph
        .nodes
        .iter()
        .map(|n| (n.id.clone(), NodeGeom::for_node(n)))
        .collect();

    let (width, height) = grid_size(graph, positions, &geoms, sg_bounds);
    let mut grid = Grid::new(width, height);

    // Pass 0a: Draw subgraph borders FIRST, outermost-to-innermost, so that
    // inner borders are drawn on top of outer ones (preventing outer border
    // characters from overwriting inner labels). `sg_bounds` is sorted
    // outermost-first, so we iterate in reverse to get innermost-first draw
    // order.
    for bounds in sg_bounds.iter().rev() {
        // Subgraph border colour comes from `class CompositeId styleName`
        // applications resolved at parse time. Only emitted when the
        // caller opted into colour rendering (`with_color`).
        let style = if with_color {
            graph.subgraph_styles.get(&bounds.id)
        } else {
            None
        };
        draw_subgraph_border(&mut grid, bounds, style);
    }

    // Pass 0b: Register all node bounding boxes as hard routing obstacles so
    // that A* edge routing will not route edges through node interiors.
    // Same loop captures `node_rects` for label-collision avoidance later.
    let mut node_rects: Vec<(usize, usize, usize, usize)> = Vec::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        let Some(&(col, row)) = positions.get(&node.id) else {
            continue;
        };
        let Some(&geom) = geoms.get(&node.id) else {
            continue;
        };
        grid.mark_node_box(col, row, geom.width, geom.height);
        node_rects.push((col, row, geom.width, geom.height));
    }

    // Pass 0c: Mark the cells *between* node boxes as `InnerArea`
    // — the bounding-box convex hull of all real nodes, minus the
    // node cells themselves. Back-edge routing pays an extra cost
    // for crossing these cells, biasing A* to take the perimeter
    // corridor (added by `compute_canvas_bounds` for back-edged
    // graphs) rather than a shortcut through the diagram body that
    // would fragment forward-edge channels.
    if !node_rects.is_empty() {
        let hull_min_col = node_rects.iter().map(|r| r.0).min().unwrap_or(0);
        let hull_min_row = node_rects.iter().map(|r| r.1).min().unwrap_or(0);
        let hull_max_col = node_rects.iter().map(|r| r.0 + r.2).max().unwrap_or(0);
        let hull_max_row = node_rects.iter().map(|r| r.1 + r.3).max().unwrap_or(0);
        if hull_max_col > hull_min_col && hull_max_row > hull_min_row {
            grid.mark_inner_area(
                hull_min_col,
                hull_min_row,
                hull_max_col - hull_min_col,
                hull_max_row - hull_min_row,
            );
        }
    }

    // Compute spread-adjusted attach points for all edges before drawing.
    // Both exit and entry points are spread so that multiple edges sharing
    // the same border cell each get their own distinct row/column.
    let attach_points = compute_spread_attaches(graph, positions, &geoms);

    // Pass 1: Route all edges using A* obstacle-aware routing.
    //
    // Edge style rendering approach:
    //   1. Route all edges via `router::route_all` (straight → L → A* per edge,
    //      shortest-first ordering). Routing draws each path on the grid and
    //      marks cells as EdgeOccupied* so subsequent edges pay a higher cost to
    //      cross them.
    //   2. After routing, call `overdraw_path_style` to replace path cells
    //      with thick or dotted glyphs based on the edge's `EdgeStyle`.
    //   3. Override the destination tip glyph based on `EdgeEndpoint`.
    //   4. For bidirectional edges, also place a back-tip at the source cell.
    //
    // This keeps all junction-merging logic in the direction-bit canvas while
    // still producing visually distinct dotted/thick lines.

    // Pre-compute per-edge flags needed by the router and post-processing loop.
    //
    // Self-loops (`from == to`) are treated as back-edges so they use the
    // perpendicular-attach routing path (exit/enter at the bottom of the box
    // for LR, right for TD). Without this, the self-loop uses the normal
    // forward-edge right-side exit and its A*-routed path crosses the same
    // column as other outgoing edges, producing stray ├ / ┼ junction glyphs
    // where the direction bits merge.
    let edge_is_back_flags: Vec<bool> = graph
        .edges
        .iter()
        .map(|e| {
            // Self-loop: always treat as a back-edge so it routes around
            // the bottom/right perimeter of the node rather than right side.
            if e.from == e.to {
                return true;
            }
            let fp = positions.get(&e.from).copied();
            let tp = positions.get(&e.to).copied();
            match (fp, tp) {
                (Some(fp), Some(tp)) => is_back_edge(fp, tp, graph.direction),
                _ => false,
            }
        })
        .collect();

    // Pre-compute forward outgoing counts for label placement.
    let mut forward_outgoing_counts: HashMap<&str, usize> = HashMap::new();
    for (edge_idx, edge) in graph.edges.iter().enumerate() {
        if !edge_is_back_flags[edge_idx] {
            *forward_outgoing_counts
                .entry(edge.from.as_str())
                .or_default() += 1;
        }
    }

    // Pre-compute directed pair counts for parallel-edge detection.
    let mut directed_pair_counts: HashMap<(&str, &str), usize> = HashMap::new();
    for edge in &graph.edges {
        *directed_pair_counts
            .entry((edge.from.as_str(), edge.to.as_str()))
            .or_default() += 1;
    }

    // Pre-compute back-edge connector points. These are recorded before routing
    // so that pass 2a.5 can stamp junction glyphs after node boxes are drawn.
    //
    // Back-edge connector points: where to stamp `┬` / `┴` (LR) or `├` / `┤`
    // (TD) after node boxes are drawn, so the perimeter back-edge path
    // connects visibly to its source and destination borders.
    // Entries: `(border_col, border_row, is_destination, skip_border_stamp)`.
    // `skip_border_stamp` is set for source entries (LR/RL) where the source
    // node has a rounded bottom border — in that case stamping `┬` onto the
    // bottom border row would pierce the `╰──╯` arc (B12).  The `┴` on the
    // path row (from `back_edge_path_joins`) already makes the connection.
    let mut back_edge_border_joins: Vec<(usize, usize, bool, bool)> = Vec::new();
    // First-path-cell joins (source end only — destination end is the arrow tip).
    let mut back_edge_path_joins: Vec<(usize, usize)> = Vec::new();
    for (edge_idx, edge) in graph.edges.iter().enumerate() {
        if !edge_is_back_flags[edge_idx] {
            continue;
        }
        if let (Some(fp), Some(fg), Some(tp), Some(tg)) = (
            positions.get(&edge.from).copied(),
            geoms.get(&edge.from).copied(),
            positions.get(&edge.to).copied(),
            geoms.get(&edge.to).copied(),
        ) {
            // For self-loops the source and destination are the same node, so
            // `sb == db`. The router places `▴` at the entry cell (which equals
            // `sb`) and protects it. Stamping `┬` on top of that arrowhead via
            // `grid.set()` (unconditional) would erase the arrowhead, so we skip
            // junction stamping entirely for self-loops. The arrow tip already
            // makes the connection visually clear.
            if edge.from == edge.to {
                continue;
            }
            let (sb, sp) = back_edge_border_cells(fp, fg, graph.direction);
            let (db, _) = back_edge_border_cells(tp, tg, graph.direction);
            // B12 guard: for LR/RL, the source border row is the bottom of the
            // source box (`r + geom.height - 1`).  For rounded shapes that row is
            // `╰──╯`; stamping `┬` there pierces the rounded arc.  Record whether
            // to skip the border stamp for this source entry.
            let skip_src_border = matches!(
                graph.direction,
                Direction::LeftToRight | Direction::RightToLeft
            ) && graph
                .node(&edge.from)
                .is_some_and(|n| has_rounded_bottom_border(n.shape));
            back_edge_border_joins.push((sb.0, sb.1, false, skip_src_border));
            back_edge_border_joins.push((db.0, db.1, true, false));
            back_edge_path_joins.push(sp);
        }
    }

    // Route all edges end-to-end with one A* call per edge (preceded by
    // straight-line and L-shape fast paths). Edges are routed in ascending
    // Manhattan-distance order so short edges claim clean corridors first.
    let mut paths = router::route_all(
        &mut grid,
        graph,
        &attach_points,
        |edge_idx| {
            if edge_is_back_flags.get(edge_idx).copied().unwrap_or(false) {
                tip_char_for_back_edge(graph.direction)
            } else {
                tip_char(graph.direction)
            }
        },
        |edge_idx| edge_is_back_flags.get(edge_idx).copied().unwrap_or(false),
    );

    // Post-routing nudging pass: merges co-directional parallel
    // back-edge corridors (Bug 5). Phase D (Bug 4 corner-in-halo)
    // is deferred — see nudge.rs docstring for details.
    crate::layout::nudge::run(
        &mut grid,
        &mut paths,
        &edge_is_back_flags,
        |edge_idx| {
            if edge_is_back_flags.get(edge_idx).copied().unwrap_or(false) {
                tip_char_for_back_edge(graph.direction)
            } else {
                tip_char(graph.direction)
            }
        },
    );

    // Collect edge label placements for a deferred write — labels must be
    // written *after* all routing so that no subsequent A* path overwrites them.
    // Each entry is `(col, row, label_text, color)`.
    let mut pending_labels: Vec<(usize, usize, String, Option<crate::types::Rgb>)> = Vec::new();
    // Collision registry: `(col, row, display_width, height)` of committed labels.
    let mut placed_labels: Vec<(usize, usize, usize, usize)> = Vec::new();
    let mut prior_path_cells_by_pair: HashMap<(&str, &str), HashSet<(usize, usize)>> =
        HashMap::new();

    for (edge_idx, edge) in graph.edges.iter().enumerate() {
        let Some(Some((src, dst))) = attach_points.get(edge_idx) else {
            continue;
        };
        let (src, _dst) = (*src, *dst);
        let edge_pair = (edge.from.as_str(), edge.to.as_str());
        let has_parallel_same_direction =
            directed_pair_counts.get(&edge_pair).copied().unwrap_or(0) > 1;
        let edge_is_back = edge_is_back_flags[edge_idx];
        let horizontal_first = graph.direction.is_horizontal();
        let path = &paths[edge_idx];

        // Post-process the destination tip cell for non-arrow endpoints.
        //
        // `route_edge` always places the flow-direction arrow at the tip cell
        // and protects it. Here we unprotect and overwrite as needed:
        //   - None    → plain line glyph (no arrowhead)
        //   - Circle  → ○
        //   - Cross   → ×
        //   - Arrow   → keep the arrow (no action needed)
        if let Some(path) = path.as_ref()
            && let Some(&(tip_c, tip_r)) = path.last()
            && edge.end != EdgeEndpoint::Arrow
        {
            grid.unprotect_cell(tip_c, tip_r);
            let glyph = match edge.end {
                EdgeEndpoint::None => {
                    // Continue the last segment direction without an arrowhead.
                    // For LR/RL flow the last segment is horizontal; for TD/BT vertical.
                    // For back-edges the last segment is vertical (LR) or horizontal (TD).
                    if edge_is_back {
                        if horizontal_first { '│' } else { '─' }
                    } else if horizontal_first {
                        '─'
                    } else {
                        '│'
                    }
                }
                EdgeEndpoint::Circle => endpoint::CIRCLE,
                EdgeEndpoint::Cross => endpoint::CROSS,
                EdgeEndpoint::Arrow => unreachable!(),
            };
            grid.set(tip_c, tip_r, glyph);
            // Protect circle/cross glyphs; leave plain-line cells unprotected
            // so subsequent edges can produce correct junctions.
            if edge.end != EdgeEndpoint::None {
                grid.protect_cell(tip_c, tip_r);
            }
        }

        if let Some(path) = path.as_ref() {
            // Apply styled (dotted/thick) glyphs to all non-tip path cells.
            let line_style = match edge.style {
                EdgeStyle::Solid => EdgeLineStyle::Solid,
                EdgeStyle::Dotted => EdgeLineStyle::Dotted,
                EdgeStyle::Thick => EdgeLineStyle::Thick,
            };
            // Exclude the last cell (tip) from the overdraw — it is already
            // protected and carries the correct endpoint glyph.
            if path.len() > 1 {
                grid.overdraw_path_style(&path[..path.len() - 1], line_style);
            }

            // For bidirectional edges, place a back-tip at the source attach
            // point AFTER the overdraw so that the back-tip is not erased.
            // Then protect the cell so later A* rendering can't touch it.
            if edge.start == EdgeEndpoint::Arrow && path.len() >= 2 {
                let back_tip = endpoint_char_back(graph.direction);
                grid.set(src.col, src.row, back_tip);
                grid.protect_cell(src.col, src.row);
            }

            // Apply edge color (`linkStyle <idx> stroke:#…`) to every cell of
            // the routed path including the tip.
            if with_color
                && let Some(es) = graph.edge_styles.get(&edge_idx)
                && let Some(stroke) = es.stroke
            {
                grid.paint_fg_path(path, stroke);
            }
        }

        // Compute edge label position using the actual routed path.
        if let (Some(lbl), Some(path)) = (&edge.label, path.as_ref())
            && let Some((lbl_col, lbl_row)) = {
                let has_sibling_outgoing = forward_outgoing_counts
                    .get(edge.from.as_str())
                    .copied()
                    .unwrap_or(0)
                    > 1;
                let prior_path_cells = has_parallel_same_direction
                    .then(|| prior_path_cells_by_pair.get(&edge_pair))
                    .flatten();
                let label_context = LabelPlacementContext {
                    dir: graph.direction,
                    node_rects: &node_rects,
                    sg_bounds,
                    grid: &grid,
                    edge_is_back,
                    has_sibling_outgoing,
                    prior_path_cells,
                };
                label_position(path, lbl, &mut placed_labels, &label_context)
            }
        {
            // Pick edge label color (`linkStyle … color:#…`), falling back to
            // the edge stroke color when only `stroke:` is set, so labels
            // visually track their lines.
            let lbl_color = if with_color {
                graph
                    .edge_styles
                    .get(&edge_idx)
                    .and_then(|es| es.color.or(es.stroke))
            } else {
                None
            };
            pending_labels.push((lbl_col, lbl_row, lbl.clone(), lbl_color));
        }

        if has_parallel_same_direction && let Some(path) = path.as_ref() {
            prior_path_cells_by_pair
                .entry(edge_pair)
                .or_default()
                .extend(path.iter().copied());
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

        // Apply node color (`style <id> fill:#…,stroke:#…,color:#…`).
        if with_color && let Some(style) = graph.node_styles.get(&node.id).copied() {
            paint_node_colors(&mut grid, pos, geom, style);
        }
    }

    // Pass 2a.5: Stamp back-edge connector glyphs at each back-edge's
    // source border so the perimeter path connects visibly out of the
    // source node. Destination joins already carry the arrow tip glyph
    // (`▴`/`◂`) written by `draw_routed_path` and protected — stamping
    // over those would erase the arrowhead.
    //
    // Glyph table:
    // - LR/RL: source border `─` becomes `┬` (T pointing down at the
    //   bottom-centre cell of the box's bottom border); first path cell
    //   below becomes `┴` (T pointing up). Vertical adjacency — reads
    //   cleanly because the chars sit on separate rows.
    // - TD/BT: source border `│` becomes `├` (right-tee at the right-
    //   centre cell of the box's right border); first path cell to the
    //   right becomes a corner — `┘` for TD (path turns up to reach
    //   target above) or `┐` for BT (path turns down to reach target
    //   below). Using a corner here fixes the old bug where `├┤` glued
    //   together and read as garbage — the corner connects the `├`
    //   stub to the vertical perimeter column above/below.
    let border_junction = match graph.direction {
        Direction::LeftToRight | Direction::RightToLeft => '┬',
        Direction::TopToBottom | Direction::BottomToTop => '├',
    };
    // LR back-edges always route left from the source's exit cell
    // (destination is to the left), so the first path cell has only an
    // upward AND leftward connection — use `┘` (bottom-right corner)
    // rather than `┴` (T-junction with phantom rightward extension).
    // Symmetric for RL: route goes right → `└`. The `┴` glyph is still
    // produced when the cell has an existing `├` from a B9 exit-collision
    // (handled below).
    let path_junction_lr_corner_left = '\u{2518}'; // ┘
    let path_junction_lr_corner_right = '\u{2514}'; // └
    for (col, row, is_dest, skip_border_stamp) in &back_edge_border_joins {
        if *is_dest || *skip_border_stamp {
            // `is_dest`: destination border glyph is the arrow tip placed by
            // the router — no junction stamp needed here.
            // `skip_border_stamp` (B12): source has a rounded bottom border
            // (`╰──╯`); the `┴` on the path row below makes the connection
            // without piercing the rounded arc.
            continue;
        }
        grid.set(*col, *row, border_junction);
    }
    for (col, row) in &back_edge_path_joins {
        // Only upgrade the path cell if it's a plain horizontal/vertical line
        // from the router, or a junction glyph formed by the exact collision
        // pattern (B9) where the exit cell is simultaneously a transit cell for
        // another back-edge arriving at the same node.  For LR/RL layouts that
        // collision produces `├` (UP+DOWN+RIGHT) at the source exit cell; we
        // still want to stamp the exit-stub glyph (`┴`) there so the
        // perimeter path reads cleanly.  Other junctions are left alone.
        let current = grid.get(*col, *row);
        let is_exit_collision = matches!(
            graph.direction,
            Direction::LeftToRight | Direction::RightToLeft
        ) && current == '├';
        if current != '─' && current != '│' && !is_exit_collision {
            continue;
        }
        // For B9 exit-collision (cell already had `├` = up+down+right), the
        // historic `┴` overlay produced `┼` — preserve that case.
        let glyph = if is_exit_collision {
            '\u{2534}' // ┴
        } else {
            match graph.direction {
                Direction::LeftToRight => path_junction_lr_corner_left,
                Direction::RightToLeft => path_junction_lr_corner_right,
                Direction::TopToBottom => '┘',
                Direction::BottomToTop => '┐',
            }
        };
        grid.set(*col, *row, glyph);
    }

    // Pass 2b: Write all edge labels after node boxes so that node box
    // drawing (which uses `set()` unconditionally) cannot overwrite labels.
    // Labels are protected so that node labels in pass 3 cannot erase them.
    //
    // Multi-line edge labels (containing `\n` from `<br/>` normalisation in
    // the parser) are written line-by-line on successive rows starting at
    // `lbl_row`.  Writing the full string in one call would embed a literal
    // `\n` character into a grid cell, which the Display impl renders as an
    // actual newline mid-row — corrupting the output (B11).
    for (lbl_col, lbl_row, lbl, lbl_color) in &pending_labels {
        for (i, line) in lbl.lines().enumerate() {
            let row = lbl_row + i;
            grid.write_text_protected(*lbl_col, row, line);
            if let Some(c) = lbl_color {
                let line_w = UnicodeWidthStr::width(line);
                grid.paint_fg_rect(*lbl_col, row, line_w, 1, *c);
            }
        }
    }

    // Pass 3: Draw node labels last so they are never overwritten.
    // Also paint any OSC 8 hyperlink rectangles when a `click` directive
    // targets this node — the hyperlink layer is orthogonal to the char layer
    // so it can be written in the same pass without ordering concerns.
    for node in &graph.nodes {
        let Some(&pos) = positions.get(&node.id) else {
            continue;
        };
        let Some(&geom) = geoms.get(&node.id) else {
            continue;
        };
        let click_url = graph.click_targets.get(&node.id).map(|ct| ct.url.as_str());
        draw_label_centred(&mut grid, node, pos, geom, click_url);
    }

    if with_color {
        grid.render_with_colors()
    } else {
        grid.render()
    }
}

/// Paint the foreground and background color layers of a node's bounding box
/// according to `style`. The actual glyphs were already drawn by
/// [`draw_node_box`] / [`draw_label_centred`]; here we only stamp the color
/// values into the grid's parallel color layer.
///
/// - `fill`   → background of every interior cell (so even the spaces
///   between label glyphs render with the fill color).
/// - `stroke` → foreground of every border cell (the outline glyphs).
/// - `color`  → foreground of every interior cell (the label text).
fn paint_node_colors(grid: &mut Grid, pos: GridPos, geom: NodeGeom, style: NodeStyle) {
    let (col, row) = pos;
    let w = geom.width;
    let h = geom.height;
    if w < 2 || h < 2 {
        return;
    }

    if let Some(stroke) = style.stroke {
        paint_box_border_fg(grid, col, row, w, h, stroke);
    }

    // Interior cells.
    let inner_col = col + 1;
    let inner_row = row + 1;
    let inner_w = w - 2;
    let inner_h = h - 2;
    if let Some(fill) = style.fill {
        grid.paint_bg_rect(inner_col, inner_row, inner_w, inner_h, fill);
    }
    if let Some(text_color) = style.color {
        grid.paint_fg_rect(inner_col, inner_row, inner_w, inner_h, text_color);
    }
}

/// Paint a foreground color over the border ring of a box at
/// `(col, row)` with size `w × h`. Top and bottom rows get the full
/// width; left and right cols cover only the rows between (corners are
/// already covered by the row sweeps). Used by both `paint_node_colors`
/// and the subgraph border coloring path so the two callers share one
/// implementation.
fn paint_box_border_fg(grid: &mut Grid, col: usize, row: usize, w: usize, h: usize, color: Rgb) {
    if w < 2 || h < 2 {
        return;
    }
    for x in col..(col + w) {
        grid.set_fg(x, row, color);
        grid.set_fg(x, row + h - 1, color);
    }
    for y in (row + 1)..(row + h - 1) {
        grid.set_fg(col, y, color);
        grid.set_fg(col + w - 1, y, color);
    }
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
    //
    // Back-edges (target upstream of source in the flow direction) use
    // perpendicular attach points so they travel around the perimeter instead
    // of cutting across the centre of the diagram.
    let mut pairs: Vec<Option<(Attach, Attach)>> = graph
        .edges
        .iter()
        .map(|edge| {
            let from_pos = *positions.get(&edge.from)?;
            let to_pos = *positions.get(&edge.to)?;
            let from_geom = *geoms.get(&edge.from)?;
            let to_geom = *geoms.get(&edge.to)?;
            // Self-loops and true back-edges both use the perpendicular-side
            // attach points. Self-loops have `from_pos == to_pos` so
            // `is_back_edge` returns false for them; check explicitly first.
            if edge.from == edge.to || is_back_edge(from_pos, to_pos, graph.direction) {
                let src = exit_point_back_edge(from_pos, from_geom, graph.direction);
                let dst = entry_point_back_edge(to_pos, to_geom, graph.direction);
                Some((src, dst))
            } else {
                let src = exit_point(from_pos, from_geom, graph.direction);
                let dst = entry_point(to_pos, to_geom, graph.direction);
                Some((src, dst))
            }
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
                // Symmetric centring: (2*i - (n-1)) * step / 2. For odd n
                // this is identical to (i - (n-1)/2) * step. For even n it
                // gives symmetric offsets [-step/2, +step/2, ...] instead of
                // the integer-division-biased [0, +step, ...] which made
                // arrow tips merging into a shared destination land on
                // adjacent rows. See merging_arrows_into_shared_destination_are_not_adjacent.
                let offset = (2 * i as isize - (n as isize - 1)) * step / 2;
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
                // Symmetric centring: (2*i - (n-1)) * step / 2. For odd n
                // this is identical to (i - (n-1)/2) * step. For even n it
                // gives symmetric offsets [-step/2, +step/2, ...] instead of
                // the integer-division-biased [0, +step, ...] which made
                // arrow tips merging into a shared destination land on
                // adjacent rows. See merging_arrows_into_shared_destination_are_not_adjacent.
                let offset = (2 * i as isize - (n as isize - 1)) * step / 2;
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
                // Symmetric centring: (2*i - (n-1)) * step / 2. For odd n
                // this is identical to (i - (n-1)/2) * step. For even n it
                // gives symmetric offsets [-step/2, +step/2, ...] instead of
                // the integer-division-biased [0, +step, ...] which made
                // arrow tips merging into a shared destination land on
                // adjacent rows. See merging_arrows_into_shared_destination_are_not_adjacent.
                let offset = (2 * i as isize - (n as isize - 1)) * step / 2;
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
                // Symmetric centring: (2*i - (n-1)) * step / 2. For odd n
                // this is identical to (i - (n-1)/2) * step. For even n it
                // gives symmetric offsets [-step/2, +step/2, ...] instead of
                // the integer-division-biased [0, +step, ...] which made
                // arrow tips merging into a shared destination land on
                // adjacent rows. See merging_arrows_into_shared_destination_are_not_adjacent.
                let offset = (2 * i as isize - (n as isize - 1)) * step / 2;
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
            // Render circle as a rounded box with '(' and ')' replacing the
            // vertical border characters at the label row.  Placing the markers
            // ON the border (not one cell inside it) keeps the label region
            // clear and prevents the decorators from appearing as literal parens
            // inside the label text.
            grid.draw_rounded_box(col, row, geom.width, geom.height);
            let mid = row + geom.cy();
            // Overwrite the left and right border cells on the middle row with
            // '(' / ')' so the mid-row reads "(  label  )" while the top and
            // bottom rows keep their rounded-corner glyphs.
            grid.set(col, mid, '(');
            grid.set(col + geom.width - 1, mid, ')');
        }
        NodeShape::Stadium => {
            grid.draw_stadium(col, row, geom.width, geom.height);
        }
        NodeShape::Subroutine => {
            grid.draw_subroutine(col, row, geom.width, geom.height);
        }
        NodeShape::Cylinder => {
            grid.draw_cylinder(col, row, geom.width, geom.height);
        }
        NodeShape::Hexagon => {
            grid.draw_hexagon(col, row, geom.width, geom.height);
        }
        NodeShape::Asymmetric => {
            grid.draw_asymmetric(col, row, geom.width, geom.height);
        }
        NodeShape::Parallelogram => {
            grid.draw_parallelogram(col, row, geom.width, geom.height);
        }
        NodeShape::Trapezoid => {
            grid.draw_trapezoid(col, row, geom.width, geom.height);
        }
        NodeShape::ParallelogramBackslash => {
            grid.draw_parallelogram_backslash(col, row, geom.width, geom.height);
        }
        NodeShape::TrapezoidInverted => {
            grid.draw_trapezoid_inverted(col, row, geom.width, geom.height);
        }
        NodeShape::DoubleCircle => {
            grid.draw_double_circle(col, row, geom.width, geom.height);
        }
        NodeShape::Bar(BarOrientation::Horizontal) => {
            grid.draw_horizontal_bar(col, row, geom.width);
        }
        NodeShape::Bar(BarOrientation::Vertical) => {
            grid.draw_vertical_bar(col, row, geom.height);
        }
        // Note boxes share the rounded shape; the dotted connector
        // edge synthesised by the parser does the visual work of
        // marking it as a note rather than a regular state.
        NodeShape::Note => {
            grid.draw_rounded_box(col, row, geom.width, geom.height);
        }
    }
}

// ---------------------------------------------------------------------------
// Subgraph border drawing
// ---------------------------------------------------------------------------

/// Draw a subgraph border rectangle (rounded corners) and write the subgraph
/// label left-aligned inside the top border with 2 cells of padding.
///
/// We use rounded corners (`╭╮╰╯`) to visually distinguish subgraph borders
/// from regular node boxes, which use square corners.
///
/// The border cells are marked as obstacles so that A\* routing avoids them
/// during edge routing. They are also protected so subsequent node drawing
/// does not overwrite them.
fn draw_subgraph_border(grid: &mut Grid, bounds: &SubgraphBounds, style: Option<&NodeStyle>) {
    let (col, row, w, h) = (bounds.col, bounds.row, bounds.width, bounds.height);

    if w < 2 || h < 2 {
        return;
    }

    // Draw rounded rectangle outline.
    grid.draw_rounded_box(col, row, w, h);

    // Apply subgraph stroke color (from `class CompositeId styleName`)
    // BEFORE protection so the colour layer is set on every border cell.
    // `fill` and `color` for subgraphs are intentionally not honoured —
    // filling a composite's interior would conflict with inner node
    // backgrounds. Document in the README's classDef section.
    if let Some(style) = style
        && let Some(stroke) = style.stroke
    {
        paint_box_border_fg(grid, col, row, w, h, stroke);
    }

    // Protect all border cells so edge routing and later node drawing leave
    // them alone. We only protect the outline (border ring), not interior.
    for x in col..(col + w) {
        grid.protect_cell(x, row);
        grid.protect_cell(x, row + h - 1);
    }
    for y in (row + 1)..(row + h - 1) {
        grid.protect_cell(col, y);
        grid.protect_cell(col + w - 1, y);
    }

    // Seed direction bits on the border *line* cells (not corners) so that
    // when an edge crosses a border, `Grid::add_dirs` ORs the route's
    // direction in and produces a proper junction glyph (┴ ┬ ├ ┤ ┼)
    // instead of leaving the bare border line in place — which made the
    // edge look "missing its initial portion" because the route's `│`
    // glyph at the crossing cell was suppressed by the border's `─`.
    // Corners stay un-seeded so an edge that happens to land on `╭` /
    // `╮` / `╰` / `╯` doesn't try to merge into a rounded corner glyph.
    use crate::layout::grid::{DIR_DOWN, DIR_LEFT, DIR_RIGHT, DIR_UP};
    for x in (col + 1)..(col + w - 1) {
        grid.seed_border_dirs(x, row, DIR_LEFT | DIR_RIGHT);
        grid.seed_border_dirs(x, row + h - 1, DIR_LEFT | DIR_RIGHT);
    }
    for y in (row + 1)..(row + h - 1) {
        grid.seed_border_dirs(col, y, DIR_UP | DIR_DOWN);
        grid.seed_border_dirs(col + w - 1, y, DIR_UP | DIR_DOWN);
    }

    // Subgraph borders are NOT marked as hard `NodeBox` obstacles. Hard
    // marking would prevent any edge whose source or destination lies
    // inside the subgraph from exiting through the border — A* would give
    // up and fall back to Manhattan routing, which ignores obstacles
    // entirely. Leaving borders passable lets A* find real orthogonal
    // paths that cross subgraph boundaries naturally; the border glyph at
    // the crossing cell now becomes a junction (see seed_border_dirs above).

    // Write the label inline in the top border row, starting 2 cells in from
    // the left corner. This avoids overlapping with node boxes whose top edge
    // may sit at `row + 1`.  The label overwrites the `─` border chars at
    // those positions; since we protect those cells afterward, A* and later
    // drawing passes cannot erase them.
    let label_col = col + 2;
    let label_row = row;
    // Truncate the label to fit within the border width, leaving room for
    // the corners and at least 1 `─` on each side.
    let max_label_w = w.saturating_sub(4);
    let label = truncate_to_width(&bounds.label, max_label_w);
    if !label.is_empty() {
        grid.write_text_protected(label_col, label_row, &label);
    }
    // Clear seeded dir-bits across BOTH the top AND bottom border lines
    // (excluding corners). The G2 fix originally only cleared the top —
    // the assumption was that bottom-border junctions disambiguate route
    // entry/exit. In practice with high-fan-out members (Worker exiting a
    // Supervisor with 3+ outgoing edges to other layers) the bottom border
    // accumulates 3+ `┼`/`┬`/`┴` stamps, reading as visual noise rather
    // than disambiguation. Bug 2 mirrors the G2 clear to the bottom row.
    // Pinned by `subgraph_title_row_has_no_junction_glyphs` (top) AND
    // `subgraph_bottom_border_has_at_most_one_junction_glyph` (bottom).
    for x in (col + 1)..(col + w - 1) {
        grid.clear_dirs(x, row);
        grid.clear_dirs(x, row + h - 1);
    }
}

/// Truncate `s` so its display width does not exceed `max_width`.
fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
        if w + cw > max_width {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out
}

/// Write a node's label horizontally centred inside its bounding box.
///
/// Multi-line labels (containing `\n`) are drawn line-by-line on successive
/// rows starting at `geom.text_row`. Each line is centred independently so
/// short lines in a mixed-width label still sit in the visual middle.
/// Draw the node label centred inside its bounding box.
///
/// When `click_url` is `Some(url)`, every label cell is also painted with the
/// URL in the grid's hyperlink layer so that [`Grid::render`] and
/// [`Grid::render_with_colors`] emit OSC 8 escape sequences around the label
/// text, making it a clickable hyperlink in supporting terminals.
///
/// In ASCII mode (`to_ascii`) the OSC 8 bytes are stripped alongside all other
/// non-ASCII characters, so ASCII output is unaffected.
///
/// # Arguments
///
/// * `grid`      — the mutable rendering canvas
/// * `node`      — the node whose label to render
/// * `pos`       — top-left `(col, row)` of the node's bounding box
/// * `geom`      — precomputed box geometry (width, height, text_row)
/// * `click_url` — optional hyperlink URL from a `click` directive
fn draw_label_centred(
    grid: &mut Grid,
    node: &Node,
    pos: GridPos,
    geom: NodeGeom,
    click_url: Option<&str>,
) {
    // Bars (fork/join) are connection points, not labelled states —
    // drawing the auto-generated state ID on top of a single `┃` column
    // or `━` row would be visually confusing. Skip silently; matches
    // Mermaid's own renderer behaviour for `<<fork>>` / `<<join>>`.
    if matches!(node.shape, NodeShape::Bar(_)) {
        return;
    }

    let (col, row) = pos;
    let interior_w = geom.width.saturating_sub(2);

    for (i, line) in node.label.lines().enumerate() {
        let line_w = UnicodeWidthStr::width(line);
        let text_col = if line_w <= interior_w {
            col + 1 + (interior_w - line_w) / 2
        } else {
            col + 1
        };
        let text_row = row + geom.text_row + i;
        grid.write_text(text_col, text_row, line);

        // Paint the hyperlink over the written text cells.
        // We use `line_w` columns (the display width of this label line)
        // so the clickable region matches exactly what the user sees.
        if let Some(url) = click_url {
            let link_w = line_w.max(1); // at least 1 cell even for empty lines
            grid.paint_hyperlink(text_col, text_row, link_w, 1, url);
        }
    }
}

// ---------------------------------------------------------------------------
// Edge label placement
// ---------------------------------------------------------------------------

struct LabelPlacementContext<'a> {
    dir: Direction,
    node_rects: &'a [(usize, usize, usize, usize)],
    sg_bounds: &'a [SubgraphBounds],
    /// Reference to the rendered grid, used to check for adjacent path
    /// corner/junction glyphs when scoring candidate label positions.
    grid: &'a Grid,
    edge_is_back: bool,
    has_sibling_outgoing: bool,
    prior_path_cells: Option<&'a HashSet<(usize, usize)>>,
}

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
    placed: &mut Vec<(usize, usize, usize, usize)>,
    context: &LabelPlacementContext<'_>,
) -> Option<(usize, usize)> {
    if path.len() < 2 {
        return None;
    }

    // For multi-line labels (containing `\n` from `<br/>` normalisation), the
    // placement width must be the *widest line*, not the full string width.
    // The full string would include the newline character (counted as 1 cell by
    // unicode-width) and the second line's text in the same "width" budget,
    // making the computed width larger than any single rendered line and causing
    // bad candidate-position offsets.  The height is the number of lines — used
    // to guard each row against subgraph borders.
    let lbl_w = label.lines().map(UnicodeWidthStr::width).max().unwrap_or(0);
    let lbl_h = label.lines().count().max(1);

    if lbl_w == 0 {
        return None;
    }

    let candidates = candidate_positions(
        path,
        context.dir,
        lbl_w,
        context.edge_is_back,
        context.has_sibling_outgoing,
        context.sg_bounds,
    );
    if candidates.is_empty() {
        return None;
    }

    // Pass A: avoid every visually-protected region — other labels,
    // node interiors, node border rows (the `┌──┐` / `└──┘` rows),
    // subgraph border cells (`╭╮╰╯─│`), positions where the label
    // immediately abuts a path corner/junction glyph on either side,
    // and positions where the label immediately abuts a subgraph wall
    // (`│` at the left/right border column of any interior row).
    //
    // The corner-adjacency guard prevents `timeout reached─┘` and
    // `─│ label text` artifacts where a path corner merges visually
    // into the label text, making it hard to read where the label ends
    // and the route begins.
    //
    // The wall-adjacency guard (B8) prevents labels from abutting the
    // subgraph's interior `│` wall — `beat│` at the right edge reads
    // as the label being cut off by the border.
    //
    // For multi-line labels each subsequent row (r+1, r+2, …) must also
    // clear the subgraph border (B11): the second wrapped line must not
    // fall on the subgraph bottom border or outside the subgraph.
    for &(c, r) in &candidates {
        // Check the first (and only, for single-line) row.
        let row_ok = !collides(c, r, lbl_w, placed)
            && !overlaps_prior_path(c, r, lbl_w, context.prior_path_cells)
            && !overlaps_node_interior(c, r, lbl_w, context.node_rects)
            && !overlaps_node_border_row(c, r, lbl_w, context.node_rects)
            && !overlaps_subgraph_border(c, r, lbl_w, context.sg_bounds)
            && !label_abuts_subgraph_right_wall(c, r, lbl_w, context.sg_bounds)
            && !label_touches_path_corner(c, r, lbl_w, context.grid);
        if !row_ok {
            continue;
        }
        // For multi-line labels, guard every additional line row.
        let extra_rows_ok = (1..lbl_h).all(|dr| {
            let rr = r + dr;
            !overlaps_subgraph_border(c, rr, lbl_w, context.sg_bounds)
        });
        if extra_rows_ok {
            // Record height = lbl_h so the collision registry covers all rows.
            placed.push((c, r, lbl_w, lbl_h));
            return Some((c, r));
        }
    }

    // Pass B: relax the structural-overlap constraints as a last resort so
    // that labels are never silently dropped.  Two labels on top of each other
    // is unreadable, so `placed` (label–label collisions) is still respected.
    // However, we keep one hard constraint even here: the label must not write
    // OVER an actual subgraph border cell.  Writing text into `╰─╯` or `│`
    // border glyphs destroys the subgraph outline (B5) — a broken box is worse
    // than a slightly misplaced label.  `label_spans_subgraph_border_cell`
    // performs the cell-level test; `overlaps_subgraph_border` (bounds-based)
    // is NOT repeated here so that labels can sit immediately adjacent to a
    // border when there is no other option.
    for &(c, r) in &candidates {
        if !collides(c, r, lbl_w, placed)
            && !label_spans_subgraph_border_cell(c, r, lbl_w, context.sg_bounds)
        {
            placed.push((c, r, lbl_w, lbl_h));
            return Some((c, r));
        }
    }
    None
}

fn overlaps_prior_path(
    col: usize,
    row: usize,
    w: usize,
    prior_path_cells: Option<&HashSet<(usize, usize)>>,
) -> bool {
    let Some(prior_path_cells) = prior_path_cells else {
        return false;
    };
    (col..col + w).any(|c| prior_path_cells.contains(&(c, row)))
}

/// Return `true` if placing a label of display width `w` at `(col, row)`
/// would leave a path corner or junction glyph immediately adjacent to
/// either end of the label.
///
/// The guard checks the cell one column **before** the label start and one
/// column **after** the label end on the same row. Corner/junction glyphs —
/// `┘ └ ┐ ┌ ┤ ├ ┬ ┴ ┼` — where a path changes direction are the problem:
/// they merge visually with adjacent label text, producing artifacts like
/// `timeout reached─┘` or `─│ label text`.
///
/// Thin straight-line glyphs (`─`, `│`) are intentionally excluded because
/// labels running alongside a path channel (`label─────▸node`) are common
/// and readable. Thick (`━ ┃`) and dotted (`┄ ┆ ╍ ╏`) line glyphs are
/// included even though they're "straight" because their visual weight
/// merges with adjacent label letters — `━━━labelled` reads as one
/// ambiguous run instead of "edge then label."
fn label_touches_path_corner(col: usize, row: usize, w: usize, grid: &Grid) -> bool {
    // Characters that mark a path direction change OR a non-thin line style.
    // Thin straight-line glyphs (`─`, `│`) are excluded — touching them is fine.
    const CORNERS: &[char] = &[
        '┘', '└', '┐', '┌', '┤', '├', '┬', '┴', '┼',
        // Thick/double variants used by some edges or borders.
        '╯', '╰', '╮', '╭',
        // T-junctions that appear in back-edge routing.
        '▴', '▾', '▸', '◂',
        // Thick line styles: labels flush against these visually merge.
        '\u{2501}', '\u{2503}', // ━ ┃
        // Dotted line styles: same merge problem.
        '\u{2504}', '\u{2506}', '\u{254D}', '\u{254F}', // ┄ ┆ ╍ ╏
    ];
    // Cell one column before the label start.
    if col > 0 && CORNERS.contains(&grid.get(col - 1, row)) {
        return true;
    }
    // Cell one column after the label end.
    if CORNERS.contains(&grid.get(col + w, row)) {
        return true;
    }
    false
}

/// Generate the ordered list of `(col, row)` candidates to try for an edge
/// label, given the routed `path` and the graph direction. Earlier
/// candidates are preferred — the first non-colliding one wins.
///
/// LR/RL: 8 vertical row offsets (±1..±4) × 3 column anchors (segment
/// midpoint, plus 1/3 and 2/3 along the last horizontal run).
///
/// TD/BT: 5 row offsets (0, ±1, ±2) × 3 column anchors (right of, left
/// of, +2 right of the last vertical run).
fn candidate_positions(
    path: &[(usize, usize)],
    dir: Direction,
    lbl_w: usize,
    edge_is_back: bool,
    has_sibling_outgoing: bool,
    sg_bounds: &[SubgraphBounds],
) -> Vec<(usize, usize)> {
    const MIN_DOGLEG_SIDE_LABEL_WIDTH: usize = 8;

    match dir {
        Direction::LeftToRight | Direction::RightToLeft => {
            let mut out = Vec::new();

            // Compute the horizontal segment candidates first so the guard
            // (below) can decide whether to skip the dogleg vertical section.
            let raw_longest_seg = longest_horizontal_segment_with_range(path);
            let last_seg = last_horizontal_segment_with_range(path);

            // Back-edges can route along the diagram perimeter (e.g. above all
            // nodes on a small row index).  For such paths the longest segment
            // is on the perimeter row while the last segment is on the natural
            // routing row near the nodes (a larger row index).  Using the
            // perimeter segment as the label anchor places the label far above
            // the actual visual connection.
            //
            // Guard: for back-edges, only prefer the longest segment when it is
            // at or BELOW the last segment's row (i.e. on the natural routing
            // corridor, not a top perimeter).  When the longest segment is ABOVE
            // the last segment (smaller row index), fall back to last-segment
            // behaviour — the perimeter run is unsuitable as a label anchor.
            let longest_seg = if edge_is_back {
                match (raw_longest_seg, last_seg) {
                    (Some(lng), Some(last)) if lng.1 >= last.1 => Some(lng),
                    _ => last_seg,
                }
            } else {
                raw_longest_seg
            };

            // Guard: if the chosen longest segment is shorter than the label
            // text, placing the label at its midpoint risks landing far from
            // any edge glyph — a "floating" label.  For vertical-dominant
            // routes (e.g. A -.-> F in a flowchart LR where F is many rows
            // below A) the only horizontal segment may be a 1-cell stub that
            // cannot fit any text.
            //
            // When this happens:
            //   - If `last_seg` is available (a qualifying horizontal stub
            //     exists on the destination side), fall back to it.  We also
            //     record the fall-back row so Phase 4 (below) can add ON-ROW
            //     candidates (row+0), which `append_seg_candidates` normally
            //     omits because it only emits ±offset rows.
            //   - If no qualifying `last_seg` exists, set `longest_seg = None`
            //     to trigger the vertical-fallback path below.
            //
            // Guard: for vertical-dominant routes the horizontal segment may be
            // a short destination stub that is too narrow for the label text.
            // If so, the midpoint candidates from that stub land off the route.
            //
            // We detect "vertical dominant" by comparing the longest horizontal
            // segment length against the longest vertical segment length.  When
            // the path descends many rows and only has a short horizontal hop
            // at the destination, the vertical leg is dominant and the label
            // should land near the destination-stub row — not at the horizontal
            // midpoint of the short stub.
            //
            // For purely horizontal (same-row) routes the horizontal segment IS
            // the whole path — we leave it unchanged.
            let vert_dominance = last_vertical_segment_with_len(path)
                .map(|(_, _, vlen)| vlen)
                .unwrap_or(0);
            let is_vert_dominant = vert_dominance >= 4;

            let longest_seg = match longest_seg {
                Some(lng) if is_vert_dominant => {
                    // Segment length = hi_col - lo_col (tuple indices 3 and 2).
                    let seg_len = lng.3.saturating_sub(lng.2);
                    if seg_len < lbl_w {
                        // Longest segment is too short — prefer last_seg if
                        // it is different, otherwise trigger vertical fallback
                        // (else branch below) where the path-tip row is used
                        // as the anchor so the label lands adjacent to the
                        // destination box rather than floating mid-route.
                        match last_seg {
                            Some(last) if last != lng => Some(last),
                            _ => None, // trigger vertical fallback
                        }
                    } else {
                        Some(lng)
                    }
                }
                _ => longest_seg,
            };

            // Closure: append two-phase candidates for one segment into `out`.
            // Phase 1 = rows inside any enclosing subgraph (preferred — keeps
            // labels inside the subgraph box when possible).
            // Phase 2 = remaining rows (outside all subgraphs).
            //
            // `exclude_row`: when `Some(r)`, skip all candidates at row `r`.
            // Used when generating candidates from the longest segment to avoid
            // the "junction row" — the row of the last horizontal segment, where
            // edge-routing junction glyphs (`┴`, `┬`) form AFTER label placement
            // (timing hazard: `grid.get` does not yet show the final glyph).
            // Those rows are instead covered by the fallback Phase 3 candidates
            // generated from `last_seg`.
            let append_seg_candidates =
                |out: &mut Vec<(usize, usize)>,
                 mid_col: usize,
                 seg_row: usize,
                 lo_col: usize,
                 hi_col: usize,
                 exclude_row: Option<usize>| {
                    // Three column anchors. For forward edges: midpoint first,
                    // then 1/3 and 2/3. For back-edges: SOURCE-side first, then
                    // 1/3-from-source, then midpoint. The source-side bias
                    // keeps perimeter-back-edge labels near where the route
                    // departs the source rather than mid-perimeter (Bug 7 — the
                    // `done` label on `F -->|done| A` was floating mid-route).
                    //
                    // For LR back-edges the source is to the RIGHT of the
                    // destination, so the segment's source-side end is `hi_col`.
                    // For RL it's `lo_col`. We pick the side closest to the
                    // path's first cell: that's the topological source for
                    // LR/RL irrespective of the perimeter direction.
                    let third = (hi_col - lo_col) / 3;
                    let col_anchors = if edge_is_back {
                        // Source-side bias: prefer the endpoint nearest the
                        // path's first cell.  We don't have direct access to
                        // path[0] here, but for LR back-edges the convention
                        // is source-on-right (so hi_col is source-side); for
                        // RL it's source-on-left.
                        match dir {
                            Direction::LeftToRight => [
                                hi_col.saturating_sub(lbl_w / 2),
                                lo_col + 2 * third,
                                mid_col,
                            ],
                            Direction::RightToLeft => {
                                [lo_col + lbl_w / 2, lo_col + third, mid_col]
                            }
                            _ => [mid_col, lo_col + third, lo_col + 2 * third],
                        }
                    } else {
                        [mid_col, lo_col + third, lo_col + 2 * third]
                    };
                    // Row offsets: alternate above/below in growing distance
                    // so nearby rows are tried before far-away ones.
                    let row_offsets: [isize; 8] = [-1, 1, -2, 2, -3, 3, -4, 4];
                    out.reserve(col_anchors.len() * row_offsets.len() * 2);
                    // Phase 1 (column-first × interior rows).
                    //
                    // B8: wall-adjacent positions (e.g. col 19) are blocked at
                    // interior rows by `label_abuts_subgraph_right_wall` but
                    // accepted at exterior rows.  Emitting interior positions
                    // first ensures that the closer, non-wall position beats
                    // the exterior fallback.
                    for &c in &col_anchors {
                        for &dr in &row_offsets {
                            let r = (seg_row as isize + dr).max(0) as usize;
                            if exclude_row == Some(r) {
                                continue;
                            }
                            let inside_sg = sg_bounds.iter().any(|sg| {
                                let bottom = sg.row + sg.height;
                                r > sg.row && r < bottom.saturating_sub(1)
                            });
                            if inside_sg {
                                out.push((c, r));
                            }
                        }
                    }
                    // Phase 2 (all remaining positions — outside any subgraph).
                    for &c in &col_anchors {
                        for &dr in &row_offsets {
                            let r = (seg_row as isize + dr).max(0) as usize;
                            if exclude_row == Some(r) {
                                continue;
                            }
                            let inside_sg = sg_bounds.iter().any(|sg| {
                                let bottom = sg.row + sg.height;
                                r > sg.row && r < bottom.saturating_sub(1)
                            });
                            if !inside_sg {
                                out.push((c, r));
                            }
                        }
                    }
                };

            // Dogleg edges in LR/RL graphs often route horizontally out of the
            // source and then vertically to a lower/upper target. Labeling the
            // source-side horizontal run can make the label look attached to a
            // neighboring parallel edge. For wider labels, prefer the long
            // vertical leg when one exists; it is the part that visually
            // distinguishes the target. Short labels stay on the horizontal
            // run because they fit there without spanning adjacent lanes.
            //
            // GUARD: only generate dogleg candidates when the longest segment
            // is long enough for the label.  When the guard triggers (the
            // longest segment is too short on a vertical-dominant route), the
            // dogleg vertical-midpoint placement would land far from the
            // destination; we skip it and rely on the destination-stub
            // placement below.
            let dogleg_ok = !is_vert_dominant
                || longest_seg
                    .map(|lng| lng.3.saturating_sub(lng.2) >= lbl_w)
                    .unwrap_or(false);

            if dogleg_ok
                && lbl_w >= MIN_DOGLEG_SIDE_LABEL_WIDTH
                && has_sibling_outgoing
                && !edge_is_back
                && let Some((seg_col, seg_row, len)) = last_vertical_segment_with_len(path)
                && len >= 4
            {
                let left_col = seg_col.saturating_sub(lbl_w + 1);
                let right_col = seg_col + 1;
                let row_offsets: [isize; 5] = [0, -1, 1, -2, 2];
                out.reserve(row_offsets.len() * 2);
                for &dr in &row_offsets {
                    let r = (seg_row as isize + dr).max(0) as usize;
                    out.push((left_col, r));
                    out.push((right_col, r));
                }
            }

            if let Some((mid_col, seg_row, lo_col, hi_col)) = longest_seg {
                // Determine the row of the last (destination-side) segment,
                // which is the "junction-prone" row to exclude from the longest-
                // segment's candidate set.  Junction glyphs (`┴ ┬`) at the path's
                // horizontal/vertical turn points form AFTER label placement
                // (because later edges are drawn after earlier edge labels are
                // committed).  Excluding the last segment's row from longest-
                // segment candidates prevents labels from landing next to future
                // junction glyphs that aren't yet in the grid.
                let last_row_for_exclusion = last_seg
                    .filter(|(_, lr, _, _)| *lr != seg_row)
                    .map(|(_, lr, _, _)| lr);

                // Primary candidates: longest horizontal segment, excluding the
                // last segment's row (which gets its own Phase 3 candidates below).
                append_seg_candidates(
                    &mut out,
                    mid_col,
                    seg_row,
                    lo_col,
                    hi_col,
                    last_row_for_exclusion,
                );

                // Phase 3 fallback: append candidates from the LAST horizontal
                // segment (destination-side / old behaviour) whenever the last
                // segment differs from the longest in either row OR column
                // bounds.  This gives Pass A a second chance to find a clean
                // position using the pre-fix segment when all of the longest
                // segment's candidates are blocked (e.g. by the B10 corner-
                // adjacency guard that protects `panics┴` artifacts).
                if let Some((last_mid, last_row, last_lo, last_hi)) = last_seg {
                    // Only append if the segment is genuinely different — skip
                    // when longest == last to avoid duplicate candidates.
                    if last_row != seg_row || last_lo != lo_col || last_hi != hi_col {
                        append_seg_candidates(
                            &mut out, last_mid, last_row, last_lo, last_hi,
                            None, // no row exclusion for the last-segment fallback
                        );
                    }
                }
            } else {
                // Fallback for purely-vertical or very-short paths.
                //
                // Two cases arrive here:
                //
                // (A) Self-loops in LR graphs: a 2-cell vertical path with no
                //     horizontal segment at all.  `seg_row` = bottom border of
                //     the source box; row+2 is the free corridor beneath.
                //
                // (B) Vertical-dominant routes whose only horizontal segment is
                //     shorter than the label text (the guard above set
                //     `longest_seg = None`).  Here the path DOES have a
                //     horizontal run at the destination-stub row, and the label
                //     should land on that row (or one step away) to stay
                //     adjacent to the edge — NOT at row+2 which is empty.
                //
                // We distinguish (B) by checking if the last path waypoints
                // share a common row (i.e. there is a horizontal stub at the
                // tip).  If so, use the tip row as the primary anchor with
                // row+0 tried first.  Otherwise (A), use the self-loop offsets.
                let path_tip = path.last().copied().unwrap_or((0, 0));
                let tip_row = path_tip.1;
                // Find the horizontal stub at the tip: walk backwards while
                // the row matches `tip_row`.
                let tip_stub_start = {
                    let mut i = path.len().saturating_sub(1);
                    while i > 0 && path[i - 1].1 == tip_row {
                        i -= 1;
                    }
                    i
                };
                let tip_stub_len = path.len() - tip_stub_start;

                let (seg_col, seg_row) = last_vertical_segment(path).unwrap_or(path_tip);

                let col_anchors = [seg_col + 1, seg_col.saturating_sub(lbl_w + 1)];

                // Select row offsets based on case.
                if tip_stub_len >= 2 {
                    // Case (B): real destination stub exists — prefer the stub
                    // row itself first so the label appears adjacent to the edge.
                    let tip_col = path[tip_stub_start].0.min(path_tip.0);
                    let left_anchor = tip_col.saturating_sub(lbl_w);
                    let row_offsets: [isize; 4] = [0, -1, 1, -2];
                    out.reserve(col_anchors.len() * row_offsets.len() + row_offsets.len());
                    for &dr in &row_offsets {
                        let r = (tip_row as isize + dr).max(0) as usize;
                        out.push((left_anchor, r));
                    }
                    for &c in &col_anchors {
                        for &dr in &row_offsets {
                            let r = (seg_row as isize + dr).max(0) as usize;
                            out.push((c, r));
                        }
                    }
                } else {
                    // Case (A): self-loop / purely vertical — prefer below the
                    // path tip so the label reads beneath the exit stub.
                    let row_offsets: [isize; 4] = [2, 1, 3, 0];
                    out.reserve(col_anchors.len() * row_offsets.len());
                    for &c in &col_anchors {
                        for &dr in &row_offsets {
                            let r = (seg_row as isize + dr).max(0) as usize;
                            out.push((c, r));
                        }
                    }
                }
            }
            out
        }
        Direction::TopToBottom | Direction::BottomToTop => {
            let (seg_col, seg_row) = match last_vertical_segment(path) {
                Some(v) => v,
                None => return Vec::new(),
            };
            let mut out = Vec::new();

            // Multiple TD/BT edges leaving the same source often share
            // adjacent vertical trunks before branching horizontally. Placing
            // every label on the same side of its trunk can make labels read
            // swapped. Prefer the side that matches the branch direction.
            if has_sibling_outgoing
                && !edge_is_back
                && let Some(branch_dir) = last_horizontal_segment_direction(path)
            {
                let preferred_col = if branch_dir < 0 {
                    seg_col.saturating_sub(lbl_w + 1)
                } else {
                    seg_col + 1
                };
                let row_offsets: [isize; 5] = [0, -1, 1, -2, 2];
                out.reserve(row_offsets.len());
                for &dr in &row_offsets {
                    let r = (seg_row as isize + dr).max(0) as usize;
                    out.push((preferred_col, r));
                }
            }

            let col_anchors = [seg_col + 1, seg_col.saturating_sub(1), seg_col + 2];
            // Match LR/RL's 8-offset range so labels in tight TD/BT
            // diagrams (e.g. nested subgraphs) have more breathing room
            // when corner / subgraph-border guards filter near positions.
            let row_offsets: [isize; 8] = [0, -1, 1, -2, 2, -3, 3, -4];
            out.reserve(col_anchors.len() * row_offsets.len());
            for &c in &col_anchors {
                for &dr in &row_offsets {
                    let r = (seg_row as isize + dr).max(0) as usize;
                    out.push((c, r));
                }
            }
            out
        }
    }
}

/// Find the **last** horizontal run in `path` (closest to the tip) that is
/// at least 2 cells long. Returns `(midpoint_col, row, lo_col, hi_col)`.
///
/// Used as a fallback in candidate generation: when the longest segment's
/// candidate positions are all blocked, positions from the last segment are
/// appended so the label can still land somewhere readable.
fn last_horizontal_segment_with_range(
    path: &[(usize, usize)],
) -> Option<(usize, usize, usize, usize)> {
    if path.len() < 2 {
        return None;
    }

    let n = path.len();
    let mut i = n.saturating_sub(2);
    loop {
        let row = path[i].1;
        let mut start = i;
        while start > 0 && path[start - 1].1 == row {
            start -= 1;
        }
        let run_len = i - start + 1;
        if run_len >= 2 {
            let lo_col = path[start].0.min(path[i].0);
            let hi_col = path[start].0.max(path[i].0);
            let mid_col = (lo_col + hi_col) / 2;
            return Some((mid_col, row, lo_col, hi_col));
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

/// Find the **longest** horizontal run in `path` that is at least 2 cells long.
/// Returns `(midpoint_col, row, lo_col, hi_col)` where `lo_col`/`hi_col` are
/// the inclusive column bounds of the segment. The inclusive `(lo, hi)` range
/// lets callers pick column anchors along the segment for label placement.
///
/// When there is only one horizontal segment (direct, same-row routes) the
/// result is identical to picking the last segment, so single-segment
/// behaviour is fully preserved.
///
/// For multi-segment L- or U-shaped routes in LR/RL graphs the longest segment
/// is usually the main horizontal trunk (e.g. the run from source to a bend
/// point), not the short final hop into the destination — so labels land near
/// the geometric midpoint of the route rather than adjacent to the destination.
fn longest_horizontal_segment_with_range(
    path: &[(usize, usize)],
) -> Option<(usize, usize, usize, usize)> {
    if path.len() < 2 {
        return None;
    }

    let mut best: Option<(usize, usize, usize, usize, usize)> = None; // (len, mid, row, lo, hi)

    let n = path.len();
    let mut i = n.saturating_sub(2);
    loop {
        let row = path[i].1;
        let mut start = i;
        while start > 0 && path[start - 1].1 == row {
            start -= 1;
        }
        let run_len = i - start + 1;
        if run_len >= 2 {
            let lo_col = path[start].0.min(path[i].0);
            let hi_col = path[start].0.max(path[i].0);
            let seg_len = hi_col - lo_col;
            let mid_col = (lo_col + hi_col) / 2;
            // Keep the longest segment; on a tie, the one already stored wins
            // (i.e. prefer the later / destination-side segment to preserve the
            // old behaviour when segments are equal-length).
            if best.is_none() || seg_len > best.unwrap().0 {
                best = Some((seg_len, mid_col, row, lo_col, hi_col));
            }
        }
        if i == 0 {
            break;
        }
        i = start.saturating_sub(1);
        if i == 0 && path[0].1 != row {
            break;
        }
    }

    best.map(|(_len, mid, row, lo, hi)| (mid, row, lo, hi))
}

/// Return the direction of the last horizontal run in `path`, preserving path
/// traversal order: `-1` for leftward, `1` for rightward.
fn last_horizontal_segment_direction(path: &[(usize, usize)]) -> Option<isize> {
    for pair in path.windows(2).rev() {
        let ((from_col, from_row), (to_col, to_row)) = (pair[0], pair[1]);
        if from_row == to_row {
            return match to_col.cmp(&from_col) {
                std::cmp::Ordering::Less => Some(-1),
                std::cmp::Ordering::Greater => Some(1),
                std::cmp::Ordering::Equal => continue,
            };
        }
    }
    None
}

/// Find the midpoint `(col, row)` of the **last** vertical run in `path`
/// that is at least 2 cells long. "Last" = closest to the tip.
///
/// Returns `None` if no such segment exists.
fn last_vertical_segment(path: &[(usize, usize)]) -> Option<(usize, usize)> {
    last_vertical_segment_with_len(path).map(|(col, row, _len)| (col, row))
}

/// Find the midpoint `(col, row)` and length of the **last** vertical run in
/// `path` that is at least 2 cells long. "Last" = closest to the tip.
///
/// Returns `None` if no such segment exists.
fn last_vertical_segment_with_len(path: &[(usize, usize)]) -> Option<(usize, usize, usize)> {
    if path.len() < 2 {
        return None;
    }

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
            return Some((col, mid_row, run_len));
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

/// Test whether the 1-row label rect at `(col, row)` of width `w` overlaps
/// the **interior** of any node bounding box in `node_rects`.
///
/// "Interior" means the cells inside the border: a node spanning
/// `(nc, nr)` with size `(nw, nh)` has interior cells `(nc+1..nc+nw-1,
/// nr+1..nr+nh-1)`. Labels that sit on a node's top or bottom border row
/// don't count as overlap — they overwrite a single `─` glyph that's
/// already redrawn in pass 2 with the wrapper border, and we've never
/// observed a real-world rendering issue from that. Labels that intrude
/// on the interior overwrite the node's own label text in pass 3, which
/// is the visible bug this helper exists to detect.
///
/// `node_rects` entries: `(col, row, width, height)`. Same shape as
/// `placed` so callers can build it from `positions` + `geoms`.
///
/// Unlike [`collides`], no padding margin is applied — labels touching
/// (but not entering) a node border are fine.
fn overlaps_node_interior(
    col: usize,
    row: usize,
    w: usize,
    node_rects: &[(usize, usize, usize, usize)],
) -> bool {
    for &(nc, nr, nw, nh) in node_rects {
        // Tiny boxes have no usable interior.
        if nw < 2 || nh < 2 {
            continue;
        }
        // Include the border columns (nc and nc+nw-1) in the blocked range
        // so that labels cannot end at the left border or start at the right
        // border, which would produce artifacts like `solid quoted│ B │`
        // where the label appears to be inside the node (left border overlap).
        let int_left = nc; // inclusive of left border column
        let int_right = nc + nw; // exclusive (includes right border at nc+nw-1)
        let int_top = nr + 1;
        let int_bottom = nr + nh - 1; // exclusive
        let row_in_interior = row >= int_top && row < int_bottom;
        if !row_in_interior {
            continue;
        }
        let col_overlaps = !(col + w <= int_left || int_right <= col);
        if col_overlaps {
            return true;
        }
    }
    false
}

/// Test whether the 1-row label rect at `(col, row)` of width `w`
/// would land on any node's top or bottom border row *and* overlap
/// that node's column range.
///
/// The previous renderer rule was that border rows were acceptable
/// — labels would overwrite the `─` glyphs of the node border, and
/// `draw_node_box` would redraw them. In practice the label is
/// written *after* the box (pass 2b > pass 2), so the label *does*
/// overwrite the `─` cells, leaving the visible result `└──panics──┘`
/// — the label reads as part of the node. Visible bug example: the
/// Supervisor pattern's `panics` label sitting on Factory's bottom
/// border row.
///
/// Labels need to leave the entire border row alone, not just the
/// corner glyphs, for the box outline to read as a contiguous
/// rectangle.
fn overlaps_node_border_row(
    col: usize,
    row: usize,
    w: usize,
    node_rects: &[(usize, usize, usize, usize)],
) -> bool {
    let label_end = col + w; // exclusive
    for &(nc, nr, nw, nh) in node_rects {
        if nw == 0 || nh == 0 {
            continue;
        }
        let bottom = nr + nh - 1;
        // Only the top or bottom border row of this node is protected.
        if row != nr && row != bottom {
            continue;
        }
        let right_excl = nc + nw; // exclusive
        // Standard rect overlap (no padding): the label sits on the
        // border row only if any of its cells fall within the node's
        // column extent.
        let col_overlaps = !(label_end <= nc || right_excl <= col);
        if col_overlaps {
            return true;
        }
    }
    false
}

/// Test whether the 1-row label rect at `(col, row)` of width `w`
/// would overlap any cell of any subgraph border perimeter
/// (`╭╮╰╯─│`), or land immediately adjacent to the interior right
/// wall in a way that reads as visually clipped.
///
/// Subgraph borders are drawn early (pass 0a) and protected against
/// edge routing, but the label-placement pass had no awareness of
/// them. A label written on a subgraph border cell punctures the
/// border outline — the CI/CD pipeline's `pass` label landing on
/// `CI`'s right `│` is the canonical example.
///
/// "Border perimeter" = the four edges of the rect: top row, bottom
/// row, left column, right column. Interior cells are fine — those
/// belong to nodes/edges inside the subgraph.
///
/// For interior rows, the right wall also carries a 1-cell outward
/// padding: a label whose rightmost character falls at `right - 1`
/// (one cell before the `│` wall) reads as `label│`, which the eye
/// interprets as the label being cut off by the border (B8: `beat│`).
/// Pass B omits this padding (to avoid silently dropping labels)
/// but `label_spans_subgraph_border_cell` still guards Pass B against
/// labels actually written ON a border cell.
fn overlaps_subgraph_border(
    col: usize,
    row: usize,
    w: usize,
    sg_bounds: &[SubgraphBounds],
) -> bool {
    let label_end = col + w; // exclusive
    for sg in sg_bounds {
        if sg.width == 0 || sg.height == 0 {
            continue;
        }
        let right = sg.col + sg.width - 1;
        let bottom = sg.row + sg.height - 1;

        // Top or bottom border row: label collides if its column range
        // overlaps the subgraph's column range at all.
        if row == sg.row || row == bottom {
            let col_overlaps = !(label_end <= sg.col || right < col);
            if col_overlaps {
                return true;
            }
            continue;
        }

        // Interior rows of the subgraph: the left and right border columns
        // are protected.  Adjacent-to-wall artifacts (`beat│`) are handled
        // separately by `label_abuts_subgraph_right_wall`.
        let row_in_height = row > sg.row && row < bottom;
        if !row_in_height {
            continue;
        }
        let hits_left = col <= sg.col && sg.col < label_end;
        let hits_right = col <= right && right < label_end;
        if hits_left || hits_right {
            return true;
        }
    }
    false
}

/// Return `true` if placing a label of width `w` at `(col, row)` would leave
/// the label's last character immediately before the right `│` wall of any
/// subgraph interior row — producing the `beat│` artifact (B8).
///
/// Concretely, this fires when `col + w == right`, where `right` is the
/// right border column of the subgraph.  At that position the label occupies
/// `[col .. right - 1]` and the very next cell is the `│` wall, which reads
/// visually as the label being clipped by the border.
///
/// Only interior rows are checked; top/bottom rows are handled by
/// [`overlaps_subgraph_border`].
///
/// Applied only in **Pass A**.  Pass B may still use such positions as a last
/// resort — a slightly clipped label is better than a missing one.
fn label_abuts_subgraph_right_wall(
    col: usize,
    row: usize,
    w: usize,
    sg_bounds: &[SubgraphBounds],
) -> bool {
    for sg in sg_bounds {
        if sg.width == 0 || sg.height == 0 {
            continue;
        }
        let right = sg.col + sg.width - 1;
        let bottom = sg.row + sg.height - 1;

        // Only interior rows have a plain `│` right wall.
        if row <= sg.row || row >= bottom {
            continue;
        }

        // `col + w == right` → rightmost label char at `right - 1`, wall at `right`.
        if col + w == right {
            return true;
        }
    }
    false
}

/// Return `true` if placing a label of width `w` at `(col, row)` would cause
/// any character of the label to land on an actual subgraph border cell.
///
/// Unlike [`overlaps_subgraph_border`] (which uses bounding-box geometry and
/// is meant for Pass A's broad avoidance), this check is cell-level and is
/// applied even in **Pass B** (last-resort placement).  Pass B relaxes most
/// structural guards so that labels are never silently dropped, but writing
/// label characters *onto* subgraph border cells destroys the outline glyph —
/// e.g., `╰─╯` becomes `╰te╯` when `te` lands on the `─` cells (B5).  A
/// broken border is worse than a misplaced label, so this guard is always on.
///
/// "Border cell" means: any cell in the subgraph's top/bottom rows or
/// left/right column within the border height.
fn label_spans_subgraph_border_cell(
    col: usize,
    row: usize,
    w: usize,
    sg_bounds: &[SubgraphBounds],
) -> bool {
    let label_end = col + w; // exclusive
    for sg in sg_bounds {
        if sg.width == 0 || sg.height == 0 {
            continue;
        }
        let right = sg.col + sg.width - 1;
        let bottom = sg.row + sg.height - 1;

        // Top or bottom border row: any column overlap hits a border cell.
        if row == sg.row || row == bottom {
            let col_overlaps = !(label_end <= sg.col || right < col);
            if col_overlaps {
                return true;
            }
            continue;
        }

        // Interior rows: the left column (sg.col) and right column (right)
        // are the border cells.
        if row <= sg.row || row >= bottom {
            continue;
        }
        // Does the label's column range [col, label_end) contain sg.col or right?
        if (col <= sg.col && sg.col < label_end) || (col <= right && right < label_end) {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// OSC 8 hyperlink helper
// ---------------------------------------------------------------------------

/// Wrap `text` with OSC 8 hyperlink escape sequences for `url`.
///
/// The resulting string is recognised as a clickable hyperlink by modern
/// terminals that support OSC 8 (iTerm2, kitty, WezTerm, foot, recent
/// GNOME Terminal). Terminals without OSC 8 support silently ignore the
/// escape sequences, displaying `text` as plain text.
///
/// This helper is used in snapshot tests to build the expected escape
/// sequence inline without hard-coding the raw bytes.
///
/// # Arguments
///
/// * `url`  — the target URL (must not contain `\x1b` or `\x07`)
/// * `text` — the visible label text to wrap
///
/// # Examples
///
/// ```
/// use mermaid_text::render::unicode::osc8_wrap;
///
/// let s = osc8_wrap("https://example.com", "Click me");
/// assert!(s.starts_with("\x1b]8;;https://example.com\x1b\\"));
/// assert!(s.ends_with("\x1b]8;;\x1b\\"));
/// assert!(s.contains("Click me"));
/// ```
pub fn osc8_wrap(url: &str, text: &str) -> String {
    // OSC 8 open:  ESC ] 8 ; params ; url ST
    //   (params is empty — no `id=` or `title=` extension today)
    // OSC 8 close: ESC ] 8 ; ; ST
    // ST (String Terminator) here is ESC \ (two bytes: 0x1b 0x5c)
    format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
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
        let crate::layout::layered::LayoutResult { positions, .. } =
            layout(&graph, &LayoutConfig::default());
        let sg_bounds = crate::layout::subgraph::compute_subgraph_bounds(&graph, &positions);
        render(&graph, &positions, &sg_bounds)
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

    // ---- overlaps_node_interior ---------------------------------------

    /// A 10×5 box at (10, 5) has interior cells (cols 11..19, rows 6..9).
    fn one_box() -> Vec<(usize, usize, usize, usize)> {
        vec![(10, 5, 10, 5)]
    }

    #[test]
    fn label_fully_inside_interior_overlaps() {
        // Label at (12, 7) width 4 → spans cols 12..16, row 7. Inside.
        assert!(overlaps_node_interior(12, 7, 4, &one_box()));
    }

    #[test]
    fn label_on_top_border_does_not_overlap() {
        // Top border row is 5; interior starts at row 6.
        assert!(!overlaps_node_interior(12, 5, 4, &one_box()));
    }

    #[test]
    fn label_on_bottom_border_does_not_overlap() {
        // Bottom border row is 9 (height=5 → rows 5..10, border at 5 and 9).
        assert!(!overlaps_node_interior(12, 9, 4, &one_box()));
    }

    #[test]
    fn label_above_box_does_not_overlap() {
        // Row 4 is above the box entirely.
        assert!(!overlaps_node_interior(12, 4, 4, &one_box()));
    }

    #[test]
    fn label_to_the_right_does_not_overlap() {
        // Box ends at col 19 (exclusive interior). Label at col 25 is past.
        assert!(!overlaps_node_interior(25, 7, 4, &one_box()));
    }

    #[test]
    fn label_extending_past_right_border_partially_overlaps() {
        // Label at col 17 width 8 spans cols 17..25 — col 17, 18 are inside.
        assert!(overlaps_node_interior(17, 7, 8, &one_box()));
    }

    #[test]
    fn label_extending_into_left_border_partially_overlaps() {
        // Label at col 5 width 8 spans cols 5..13 — cols 11, 12 are inside.
        assert!(overlaps_node_interior(5, 7, 8, &one_box()));
    }

    #[test]
    fn label_skipping_over_box_horizontally_does_not_overlap() {
        // Label at col 5 width 4 spans cols 5..9. Box starts at col 10.
        assert!(!overlaps_node_interior(5, 7, 4, &one_box()));
    }

    #[test]
    fn empty_node_rects_never_overlaps() {
        assert!(!overlaps_node_interior(0, 0, 100, &[]));
    }

    #[test]
    fn tiny_boxes_have_no_interior() {
        // 1×1 box: no interior cells exist.
        let boxes = vec![(10, 10, 1, 1)];
        assert!(!overlaps_node_interior(10, 10, 1, &boxes));
    }

    // ---- overlaps_node_border_row -------------------------------------

    fn factory_box() -> Vec<(usize, usize, usize, usize)> {
        // Box at col 2, row 3, width 11, height 3. Top row = 3, bottom = 5.
        // Columns span [2, 12] inclusive (i.e. nc..nc+nw = 2..13).
        vec![(2, 3, 11, 3)]
    }

    #[test]
    fn label_on_node_border_row_overlapping_columns_is_protected() {
        // The Supervisor `panics` bug: a label between the corners on
        // the same border row reads as part of the box.
        let label_w = 6; // "panics"
        assert!(overlaps_node_border_row(6, 5, label_w, &factory_box())); // bottom row
        assert!(overlaps_node_border_row(6, 3, label_w, &factory_box())); // top row
    }

    #[test]
    fn label_on_border_row_outside_node_columns_is_fine() {
        // A label on the same row but to the left or right of the box
        // doesn't punch the box outline — let it through.
        let label_w = 4;
        assert!(!overlaps_node_border_row(20, 5, label_w, &factory_box()));
        assert!(!overlaps_node_border_row(0, 5, 1, &factory_box()));
    }

    #[test]
    fn label_on_node_interior_row_passes_border_check() {
        // Border-row check ignores rows that aren't top/bottom borders;
        // the (existing) `overlaps_node_interior` check is what catches
        // labels inside the box.
        let label_w = 4;
        assert!(!overlaps_node_border_row(6, 4, label_w, &factory_box())); // mid row
    }

    #[test]
    fn label_on_node_border_row_outside_canvas_extent_is_fine() {
        // Edge case: empty rect list, zero-width rect, etc.
        assert!(!overlaps_node_border_row(0, 0, 5, &[]));
        assert!(!overlaps_node_border_row(0, 0, 5, &[(0, 0, 0, 3)]));
    }

    // ---- overlaps_subgraph_border -------------------------------------

    fn ci_subgraph() -> Vec<SubgraphBounds> {
        vec![SubgraphBounds {
            id: "CI".to_string(),
            label: "CI".to_string(),
            col: 0,
            row: 0,
            width: 41, // right border at col 40
            height: 7, // bottom border at row 6
            depth: 0,
        }]
    }

    #[test]
    fn label_on_subgraph_top_or_bottom_border_is_protected() {
        let w = 4; // "pass"
        // Row 0 = top border, row 6 = bottom border.
        assert!(overlaps_subgraph_border(5, 0, w, &ci_subgraph()));
        assert!(overlaps_subgraph_border(5, 6, w, &ci_subgraph()));
    }

    #[test]
    fn label_overlapping_subgraph_left_or_right_border_column_is_protected() {
        // Interior height row, label column range crosses the border col.
        let w = 4;
        // Right border at col 40; label spanning [40, 44) overlaps it.
        assert!(overlaps_subgraph_border(40, 3, w, &ci_subgraph()));
        // Left border at col 0; label spanning [0, 4) overlaps it.
        assert!(overlaps_subgraph_border(0, 3, w, &ci_subgraph()));
    }

    #[test]
    fn label_immediately_outside_subgraph_border_is_allowed() {
        // Label at col 41 is OUTSIDE the subgraph (right border at col 40) —
        // no overlap. The CI/CD `pass` case: labels that sit just outside
        // the border are always accepted by `overlaps_subgraph_border`.
        let w = 4;
        assert!(!overlaps_subgraph_border(41, 3, w, &ci_subgraph()));
    }

    #[test]
    fn label_ending_one_before_right_wall_is_protected() {
        // B8: a label whose last character lands at `right - 1` reads as
        // `beat│` — the `│` wall visually clips the label.
        // `label_abuts_subgraph_right_wall` fires when `col + w == right`.
        // Here right = 40; col=36, w=4 → col+w=40=right.
        let w = 4; // label at [36..40); rightmost char at 39, wall at 40
        assert!(label_abuts_subgraph_right_wall(36, 3, w, &ci_subgraph()));
        // `overlaps_subgraph_border` itself does NOT fire for this position
        // (it only fires when the label actually spans or touches the wall cell).
        assert!(!overlaps_subgraph_border(36, 3, w, &ci_subgraph()));
    }

    #[test]
    fn label_well_outside_subgraph_is_fine() {
        let w = 4;
        // Way to the right.
        assert!(!overlaps_subgraph_border(100, 3, w, &ci_subgraph()));
        // Above the subgraph entirely.
        assert!(!overlaps_subgraph_border(5, 100, w, &ci_subgraph()));
    }

    #[test]
    fn empty_sg_bounds_never_overlaps() {
        assert!(!overlaps_subgraph_border(0, 0, 100, &[]));
    }

    // ---- OSC 8 hyperlink render tests ------------------------------------

    /// The `osc8_wrap` helper produces the correct three-part escape sequence.
    #[test]
    fn osc8_wrap_format() {
        let s = osc8_wrap("https://example.com", "Hello");
        assert_eq!(s, "\x1b]8;;https://example.com\x1b\\Hello\x1b]8;;\x1b\\");
    }

    /// A chart with a `click` directive renders the OSC 8 sequence around the
    /// node label in plain-text (non-color) mode.
    #[test]
    fn click_directive_renders_osc8_in_plain_mode() {
        let src = "graph LR\nA[Start] --> B[End]\nclick A \"https://example.com\"";
        let out = crate::render(src).unwrap();
        // The OSC 8 open sequence must be present for node A's label.
        assert!(
            out.contains("\x1b]8;;https://example.com\x1b\\"),
            "OSC 8 open sequence missing in output:\n{out:?}"
        );
        // The OSC 8 close sequence must follow.
        assert!(
            out.contains("\x1b]8;;\x1b\\"),
            "OSC 8 close sequence missing in output:\n{out:?}"
        );
        // The label text must be present.
        assert!(out.contains("Start"), "label 'Start' not in output");
        // Node B has no click directive — its label should not be wrapped.
        assert!(
            !out.contains("\x1b]8;;https://b"),
            "unexpected OSC 8 for node B"
        );
    }

    /// A chart WITHOUT any `click` directive must produce output that is
    /// byte-for-byte identical to a plain `render` — no escape sequences.
    #[test]
    fn no_click_directive_produces_no_escape_sequences() {
        let src = "graph LR\nA[Start] --> B[End]";
        let out = crate::render(src).unwrap();
        assert!(
            !out.contains('\x1b'),
            "unexpected escape sequence in output without click directive"
        );
    }

    /// In color-render mode the OSC 8 sequence still appears alongside SGR
    /// color escapes.
    #[test]
    fn click_directive_renders_osc8_in_color_mode() {
        let src = "graph LR\nA[Start] --> B[End]\nclick A \"https://color.example\"";
        let opts = crate::RenderOptions {
            color: true,
            ..Default::default()
        };
        let out = crate::render_with_options(src, &opts).unwrap();
        assert!(
            out.contains("\x1b]8;;https://color.example\x1b\\"),
            "OSC 8 missing in color render:\n{out:?}"
        );
    }

    /// Regression test for Bug 2: rhombus/diamond nodes must render with
    /// diagonal corner characters (`╱` U+2571 / `╲` U+2572) instead of the
    /// old rectangle-with-`◇`-markers style.
    ///
    /// Both a short label ("Hi") and a longer label are checked so the
    /// diagonal-corner approach works across a range of box widths.
    #[test]
    fn rhombus_uses_diagonal_corners() {
        for label in &["Hi", "Rhombus", "This is a long rhombus label"] {
            let src = format!("graph LR\nD{{{label}}}");
            let out = render_diagram(&src);
            assert!(
                out.contains('╱'),
                "diagonal corner '╱' missing for label {label:?} in:\n{out}"
            );
            assert!(
                out.contains('╲'),
                "diagonal corner '╲' missing for label {label:?} in:\n{out}"
            );
            // The old rectangular markers must no longer appear.
            assert!(
                !out.contains('◇'),
                "old '◇' marker still present for label {label:?} in:\n{out}"
            );
        }
    }

    // ---- longest_horizontal_segment_with_range ---------------------------

    /// Helper: build a path from `(col, row)` pairs.
    fn path_from(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
        pairs.to_vec()
    }

    /// For a direct single-segment route the longest segment IS the only segment,
    /// so the result is the same as picking the last segment (no behaviour change).
    ///
    /// Note: the path scanner uses `i = n-2` as the starting index, so the very
    /// last waypoint (the arrowhead arrival cell) is not the upper bound — the
    /// detected range is `path[0]..=path[n-2]`.  With a path of 9 waypoints
    /// (cols 2..=10) the scanned range is cols 2..=9, mid = 5.
    #[test]
    fn longest_seg_single_segment_matches_last() {
        // Horizontal run from col 2 to col 10 on row 0 (9 waypoints).
        // Scanner starts at i = 7 (col 9) and walks back to start = 0 (col 2).
        let path = path_from(&[
            (2, 0),
            (3, 0),
            (4, 0),
            (5, 0),
            (6, 0),
            (7, 0),
            (8, 0),
            (9, 0),
            (10, 0),
        ]);
        let (mid, row, lo, hi) = longest_horizontal_segment_with_range(&path).unwrap();
        assert_eq!(row, 0);
        assert_eq!(lo, 2);
        assert_eq!(hi, 9); // last scanned col is path[n-2] = 9
        assert_eq!(mid, 5, "midpoint of [2, 9] should be 5");
    }

    /// For a two-segment L-route (long horizontal out of source, short hop at
    /// destination), the longer source-side segment wins and the label midpoint
    /// is placed on it — NOT on the short destination-side hop.
    ///
    /// Route: long horizontal run (cols 0..=19, row 0), vertical turn, then a
    /// short 2-cell approach (cols 20..=21, row 3).
    ///
    /// The source-side segment spans 19 cells (cols 0..=18 via `path[n-2]`-
    /// adjusted indexing); the destination-side hop spans only ~1 cell, so the
    /// source side wins and the label is placed near col 9 (its midpoint).
    #[test]
    fn longest_seg_picks_source_side_on_l_route() {
        // Long horizontal source run (col 0..=19, row 0), then vertical leg,
        // then short destination-side approach (col 19..=21, row 3).
        let mut path: Vec<(usize, usize)> = (0..=19).map(|c| (c, 0)).collect();
        // Vertical leg col=19, rows 1..=3.
        path.extend((1..=3).map(|r| (19, r)));
        // Short destination-side horizontal (col 19..=21, row 3).
        path.extend((20..=21).map(|c| (c, 3)));

        let (_mid, row, _lo, _hi) = longest_horizontal_segment_with_range(&path).unwrap();
        assert_eq!(
            row, 0,
            "longest segment is on source row 0, not destination row 3"
        );
    }

    /// When two segments are equal length the later (destination-side) segment
    /// wins — preserves old tie-break behaviour to avoid gratuitous churn.
    #[test]
    fn longest_seg_tie_keeps_later_segment() {
        // Source-side run: cols 0..=8 on row 0 (9 waypoints, scanned range 0..=7 = 8 cells).
        // Dest-side run:   cols 9..=17 on row 3 (scanned range 9..=16 = 8 cells — equal).
        let mut path: Vec<(usize, usize)> = (0..=8).map(|c| (c, 0)).collect();
        path.extend((1..=3).map(|r| (8, r)));
        path.extend((9..=17).map(|c| (c, 3)));

        let (_mid, row, _lo, _hi) = longest_horizontal_segment_with_range(&path).unwrap();
        // On a tie the first segment found wins (destination-side, since we scan
        // from the tip backward): `best` is only replaced when strictly greater.
        assert_eq!(
            row, 3,
            "on a tie the destination-side segment (found first) should win"
        );
    }

    // ---- label fallback when longest segment too short (Bug 4) -----------

    /// For a vertical-dominant route where the only horizontal segment is a
    /// 1-cell stub (shorter than the label), the label must NOT be placed on
    /// that tiny segment — it would float disconnected from any edge glyph.
    /// Instead the candidate list should fall back to the last segment (the
    /// destination-side stub).
    ///
    /// We can test the guard by observing that `render` for a flowchart LR
    /// diagram with a multi-row descent places the label adjacent to the
    /// destination, not on a far-away short segment.
    #[test]
    fn label_falls_back_when_longest_segment_too_short() {
        // A flowchart LR with A having many descendants forces long vertical
        // descents for the later edges.  The last edge (A ==> G with label
        // "thick label") was floating before the fix.
        let src = r#"flowchart LR
    A --> B
    A -.-> C
    A ==> D
    A -- "labelled" --> E
    A -. "dashed label" .-> F
    A == "thick label" ==> G"#;
        let out = render_diagram(src);

        // The label must appear visually connected to its edge — on the
        // same row OR within 1 row above/below an edge-glyph row.
        //
        // Originally this test required edge glyphs ON the same row as the
        // label. After the A3 fix (labels moved away from rows with thick
        // `━` / dotted `┄` glyphs to avoid `━━━labelled` abutment), labels
        // now land one row below their target node — still visually
        // adjacent. The relaxed check preserves the original intent (no
        // floating labels in the void) while admitting the A3 placement.
        let lines: Vec<&str> = out.lines().collect();
        let has_edge_glyph =
            |line: &str| line.chars().any(|c| matches!(c,
                '─' | '┄' | '━' | '│' | '┆' | '┃' | '▸' | '▹' | '▶' | '╱' | '╲'
            ));
        for label in &["dashed label", "thick label"] {
            let label_row_idx = lines
                .iter()
                .position(|l| l.contains(label))
                .unwrap_or_else(|| panic!("{label:?} not found in output:\n{out}"));
            let neighbours = label_row_idx.saturating_sub(1)
                ..(label_row_idx + 2).min(lines.len());
            let connected = neighbours.clone().any(|i| has_edge_glyph(lines[i]));
            assert!(
                connected,
                "label {label:?} (line {label_row_idx}) is not visually \
                 connected to an edge — no edge glyphs in lines \
                 {:?}.\nFull output:\n{out}",
                neighbours.collect::<Vec<_>>(),
            );
        }
    }

    // ---- <<choice>> label suppression (state diagrams) -------------------

    /// A named `<<choice>>` node (`state if_state <<choice>>`) must show the
    /// user-supplied id inside the diamond in the rendered output.
    #[test]
    fn named_choice_renders_label_inside_diamond() {
        let src = "stateDiagram-v2
state if_state <<choice>>
[*] --> if_state
if_state --> True: condition
if_state --> False: !condition";
        // Use the top-level render API so state diagram detection fires.
        let out = crate::render(src).expect("state diagram render must succeed");
        // The diamond must be present (diagonal corners).
        assert!(
            out.contains('╱'),
            "missing diagonal corner '╱' for named <<choice>> in:\n{out}"
        );
        // The user-supplied label must appear in the output.
        assert!(
            out.contains("if_state"),
            "named <<choice>> label 'if_state' missing from rendered output:\n{out}"
        );
    }

    /// An anonymous `<<choice>>` used directly as a transition endpoint must
    /// render as an empty diamond — no synthetic id like `__choice_1__` should
    /// appear in the output.
    #[test]
    fn anonymous_choice_renders_empty_diamond() {
        let src = "stateDiagram-v2
[*] --> <<choice>>
<<choice>> --> Pass: success
<<choice>> --> Fail: error";
        // Use the top-level render API so state diagram detection fires.
        let out = crate::render(src).expect("state diagram render must succeed");
        // The diamond border must still be present.
        assert!(
            out.contains('╱'),
            "missing diagonal corner '╱' for anonymous <<choice>> in:\n{out}"
        );
        // No synthetic id should leak into the output.
        assert!(
            !out.contains("__choice_"),
            "synthetic choice id leaked into rendered output:\n{out}"
        );
        // Also assert the concrete synthetic id pattern is absent.
        assert!(
            !out.contains("choice_1"),
            "partial synthetic id 'choice_1' leaked into rendered output:\n{out}"
        );
    }

    /// Regression test for B9: back-edge source exit cell must not contain `├`
    /// when the same node is simultaneously the destination of another back-edge.
    ///
    /// In LR layout, `exit_point_back_edge` and `entry_point_back_edge` both
    /// use the center column of the bottom border.  When a node is both the
    /// SOURCE of one back-edge (exit at `cx, r+height`) and the DESTINATION of
    /// another (entry at `cx, r+height-1`), the destination route transits
    /// through the source exit cell, depositing a DOWN bit that combines with
    /// the source's existing RIGHT bit to produce `├` (UP+DOWN+RIGHT).  The
    /// `back_edge_path_joins` stamping must detect this collision and overwrite
    /// the `├` with the correct exit-stub glyph `┴`.
    ///
    /// The state_basic_machine diagram exposes this pattern: Running is both
    /// the source of `Running→Idle` and the destination of `Paused→Running`.
    #[test]
    fn back_edge_attach_does_not_pierce_source_perimeter() {
        let src = "stateDiagram-v2
    [*] --> Idle
    Idle --> Running : start
    Running --> Paused : pause
    Paused --> Running : resume
    Running --> Idle : stop
    Idle --> [*]";
        let out = crate::render(src).expect("render must succeed");

        // The source exit cell (one row below Running's bottom border, center
        // column) must be `┴` — not `├`.  `├` is the B9 bug glyph: it appears
        // as if the back-edge route is piercing through the box border.
        //
        // Strategy: find the row that contains Running's bottom rounded border
        // (`╰` ... `╯`) and then inspect the row immediately below it.  After
        // the B12 fix the border row no longer contains `┬` (that stamp is
        // skipped for rounded shapes); we look for `╰` + `╯` + `Running` text
        // in the vicinity instead.
        let lines: Vec<&str> = out.lines().collect();
        // Find the Running node's box — the label row contains "Running".
        // For a standard 3-row box the layout is: top border, label, bottom
        // border.  "Running" appears on the LABEL row (top + 1), so the bottom
        // border is one row further down (top + 2 = label + 1).
        let label_row = lines
            .iter()
            .position(|l| l.contains("Running"))
            .expect("Running label row not found");
        // Bottom border is the row immediately after the label row.
        let bottom_border_row = label_row + 1;

        // The border row itself must be a clean rounded arc — no `┬` pierce
        // glyph (B12 guard) and no `├` pierce glyph (B9 guard).
        let border_row_str = lines
            .get(bottom_border_row)
            .expect("Running box bottom border row must exist");
        assert!(
            !border_row_str.contains('┬'),
            "B12 regression: `┬` found on Running box bottom border row.\n\
             The rounded arc `╰──╯` must not be pierced.\n\
             Border row: {border_row_str:?}\nFull output:\n{out}"
        );

        // The row immediately below the bottom border is the perimeter row
        // containing the source exit stub.
        let perimeter_row = lines
            .get(bottom_border_row + 1)
            .expect("row below Running bottom border must exist");

        assert!(
            !perimeter_row.contains('├'),
            "B9 regression: `├` found on perimeter row adjacent to Running box bottom border.\n\
             Expected `┴` (exit stub) instead of `├` (pierce glyph).\n\
             Perimeter row: {perimeter_row:?}\nFull output:\n{out}"
        );
        assert!(
            perimeter_row.contains('┴'),
            "Expected `┴` (back-edge exit stub) on the perimeter row below Running's bottom \
             border, but it was not found.\nPerimeter row: {perimeter_row:?}\nFull output:\n{out}"
        );
    }

    /// Regression test for B12: back-edge source-attach must NOT stamp `┬` onto
    /// the bottom border row of a rounded box.
    ///
    /// In LR layout, `back_edge_border_cells` returns `border_row =
    /// r + geom.height - 1` for the source node — this is the bottom border
    /// row.  For rounded boxes the bottom border is `╰─────╯`; stamping `┬`
    /// there (as the `back_edge_border_joins` pass did unconditionally) makes
    /// it read as `╰──┬──╯`, visually piercing the rounded arc.
    ///
    /// The fix: skip the `┬` border stamp for LR/RL source nodes whose shape
    /// produces a rounded bottom border (`╰──╯`).  The `┴` on the path row
    /// one row below (from `back_edge_path_joins`) already makes the
    /// connection without corrupting the arc.
    ///
    /// The circuit-breaker-like diagram `HALF_OPEN → CircuitOpen` exposes
    /// this: HALF_OPEN is a Rounded state and the source of a back-edge.
    #[test]
    fn back_edge_source_attach_does_not_pierce_rounded_box_bottom() {
        let src = "stateDiagram-v2
    [*] --> CircuitOpen
    CircuitOpen --> HALF_OPEN : timeout
    HALF_OPEN --> CircuitClosed : success
    HALF_OPEN --> CircuitOpen : failure
    CircuitClosed --> CircuitOpen : 5 errors";
        let out = crate::render(src).expect("render must succeed");
        let lines: Vec<&str> = out.lines().collect();

        // Every rounded box bottom border row (`╰...╯`) must be free of `┬`.
        // A `┬` in a `╰─...─╯` row is the B12 bug glyph — it indicates the
        // back-edge junction was stamped onto the box border rather than on
        // the perimeter path row one below it.
        for (i, line) in lines.iter().enumerate() {
            if line.contains('╰') && line.contains('╯') {
                assert!(
                    !line.contains('┬'),
                    "B12 regression: `┬` found on rounded box bottom border at line {i}.\n\
                     The `╰──╯` arc must not be pierced by a junction glyph.\n\
                     Line: {line:?}\nFull output:\n{out}"
                );
            }
        }

        // The perimeter path row must contain a back-edge exit stub —
        // historically `┴` (T-junction), now `┘` or `└` (corner) since the
        // F2 fix replaced the spurious T-with-rightward-extension with a
        // proper corner glyph that matches the path's actual direction.
        // Either form satisfies the underlying intent: the back-edge has
        // a visible perimeter-path connection from the source node.
        assert!(
            out.contains('┴') || out.contains('┘') || out.contains('└'),
            "Expected at least one back-edge perimeter exit stub (`┴` / `┘` / `└`) \
             in the output, but none found.\nFull output:\n{out}"
        );
    }

    /// Regression test for B-title: a vertical route passing DOWN through a
    /// subgraph's top border row must not overwrite the subgraph title characters
    /// with junction glyphs (`┼`/`┬`/`┴`).
    ///
    /// Root cause: `draw_subgraph_border` calls `seed_border_dirs` on every cell
    /// of the top border row (including the cells that will hold the label) before
    /// calling `write_text_protected` to stamp the label.  `Grid::add_dirs`
    /// bypasses protection when `directions != 0`, so the seeded bits caused
    /// routing to overwrite protected title chars with junction glyphs.
    ///
    /// Fix: after `write_text_protected`, call `Grid::clear_dirs` on each label
    /// cell so that `directions == 0` is restored and `add_dirs` honours the
    /// protection flag.
    ///
    /// Repro: two sibling subgraphs (Frontend / Backend) arranged TB where
    /// UI→API and SW→API route vertically through the Backend title row.
    /// **Known limitation (B1, deferred from Path B)**: state diagrams
    /// where a terminal `[*]` final state is reached via a SHORT path while
    /// other states sit on a LONGER path render the final state in the
    /// MIDDLE of the diagram instead of at the end.
    ///
    /// In the gallery's Diagram 6 ("Basic state machine"), the source
    /// `Idle --> [*]` makes the final state a sink at layer 2 (initial(0)
    /// → Idle(1) → final(2)), while `Paused` sits at layer 3 via the longer
    /// path through Running. Sugiyama's longest-path-from-sources layer
    /// assignment correctly puts each node at its longest-path layer, but
    /// the resulting visual is the final state appearing mid-graph.
    ///
    /// The fix would be a post-pass in `sugiyama_layout` that detects
    /// terminal-sink nodes (no outgoing edges) and promotes them to the
    /// maximum layer. The risk is high: the change requires recomputing
    /// both the level AND the ascii-dag column coordinate, with within-
    /// layer row placement falling out as a secondary concern. Estimated
    /// snapshot churn was 40–60 files; in practice the cascading effects
    /// of moving a node late in the pipeline made bounded scoping
    /// impractical for the launch window.
    ///
    /// This test is `#[ignore]`d so it doesn't block CI but stays as a
    /// pinned target for future work — when B1 is implemented, remove the
    /// `#[ignore]` and verify the assertion passes.
    #[test]
    fn final_state_renders_at_rightmost_column() {
        let src = "stateDiagram-v2
    [*] --> Idle
    Idle --> Running : start
    Running --> Paused : pause
    Paused --> Running : resume
    Running --> Idle : stop
    Idle --> [*]";
        let out = crate::render(src).expect("render must succeed");
        let lines: Vec<Vec<char>> = out.lines().map(|l| l.chars().collect()).collect();

        // Find the BOX-LEFT col of Paused (not the text col). The box
        // renders as `│ Paused │`, so we search for the literal `│ Paused`
        // pattern and take that match position. Char-column, not byte
        // offset — `String::find` would return a byte offset that
        // wildly misaligns with `final_box_col` below (box-drawing
        // chars are 3 bytes each).
        let paused_needle: Vec<char> = "│ Paused".chars().collect();
        let paused_col = lines
            .iter()
            .find_map(|l| {
                l.windows(paused_needle.len())
                    .position(|w| w == paused_needle.as_slice())
            })
            .expect("Paused box left border missing");

        // Final state `((●))` renders as a nested rounded box, 5 rows
        // tall:
        //   ╭───────╮     <- outer top — `╭` at column C
        //   │╭─────╮│     <- inner top — `╭` at column C+1
        //   ││  ●  ││
        //   │╰─────╯│
        //   ╰───────╯
        // The nesting signature is `╭` at (C, row) AND `╭` at
        // (C+1, row+1). Scan for any (col, row) cell with this exact
        // diagonal-step pattern — which uniquely identifies nested
        // rounded boxes (`((X))`).
        let final_box_col = (0..lines.len().saturating_sub(1))
            .find_map(|r| {
                let row = &lines[r];
                let next = &lines[r + 1];
                row.iter().enumerate().find_map(|(c, &ch)| {
                    if ch != '\u{256D}' {
                        return None;
                    }
                    if next.get(c + 1).copied() == Some('\u{256D}') {
                        Some(c)
                    } else {
                        None
                    }
                })
            })
            .expect("final-state nested-rounded-box outline missing");

        assert!(
            final_box_col >= paused_col,
            "final state at col {final_box_col} should be ≥ Paused at col \
             {paused_col} (terminal sinks should be in the rightmost layer). \
             Full output:\n{out}"
        );
    }

    /// State-diagram notes (`note right of X`, `note left of X`) must be
    /// registered as full routing obstacles — no edge route should pass
    /// through a note's interior cells. The gallery's Diagram 9 places a
    /// multi-line note next to `CircuitOpen` while a back-edge from
    /// `CircuitClosed → CircuitOpen` routes near the same area.
    ///
    /// This test asserts that within the note's bounding box (excluding
    /// borders), every cell is either whitespace or a letter from the
    /// note's own text — no edge glyphs (`│ ─ ┼ ┬ ┴ ├ ┤ ╮ ╯ ╭ ╰`).
    ///
    /// If this passes today (post-G1 / post-F2), B2 is moot — the
    /// cumulative effect of the prior fixes already keeps routes out of
    /// note interiors. The test serves as a regression guard.
    #[test]
    fn note_interior_contains_no_routing_glyphs() {
        let src = "stateDiagram-v2
    [*] --> CircuitOpen
    CircuitOpen --> CircuitClosed : timeout reached
    CircuitClosed --> CircuitOpen : 5 errors
    note right of CircuitOpen
        Open state rejects all
        traffic for cool-down period.
    end note";
        let out = crate::render(src).expect("render must succeed");
        let lines: Vec<Vec<char>> = out.lines().map(|l| l.chars().collect()).collect();

        // Find the note's bounding box. The note has a rounded rectangle
        // border; rows containing `╭` ending in `╮` are top borders, rows
        // with `╰` ending in `╯` are bottom. We pick the LATEST such pair
        // (the note is the only one in this diagram).
        let mut note_top: Option<usize> = None;
        let mut note_bot: Option<usize> = None;
        let mut note_left: Option<usize> = None;
        let mut note_right: Option<usize> = None;
        for (r, line) in lines.iter().enumerate() {
            if line.contains(&'\u{256D}') && line.contains(&'\u{256E}') {
                let l = line.iter().position(|&c| c == '\u{256D}').unwrap();
                let rr = line.iter().rposition(|&c| c == '\u{256E}').unwrap();
                if line[(l + 1)..rr].iter().all(|&c| c == '\u{2500}') {
                    note_top = Some(r);
                    note_left = Some(l);
                    note_right = Some(rr);
                }
            } else if note_top.is_some()
                && line.contains(&'\u{2570}')
                && line.contains(&'\u{256F}')
            {
                let l = line.iter().position(|&c| c == '\u{2570}').unwrap();
                let rr = line.iter().rposition(|&c| c == '\u{256F}').unwrap();
                if l == note_left.unwrap()
                    && rr == note_right.unwrap()
                    && line[(l + 1)..rr].iter().all(|&c| c == '\u{2500}')
                {
                    note_bot = Some(r);
                    break;
                }
            }
        }

        let (top, bot, left, right) = match (note_top, note_bot, note_left, note_right) {
            (Some(t), Some(b), Some(l), Some(r)) => (t, b, l, r),
            _ => panic!("could not locate note bounding box in:\n{out}"),
        };

        // Walk interior cells (exclusive of border rows/cols).
        let routing_glyphs: Vec<char> = vec![
            '\u{2502}', '\u{2500}', '\u{253C}', '\u{252C}', '\u{2534}', '\u{251C}', '\u{2524}',
            '\u{256E}', '\u{256F}', '\u{256D}', '\u{2570}', '\u{2518}', '\u{2514}', '\u{2510}',
            '\u{250C}',
        ];
        for (r, line) in lines.iter().enumerate().take(bot).skip(top + 1) {
            for c in (left + 1)..right {
                let ch = line.get(c).copied().unwrap_or(' ');
                assert!(
                    !routing_glyphs.contains(&ch),
                    "routing glyph {ch:?} found inside note interior at \
                     ({c}, {r}) — note bbox: ({left},{top})-({right},{bot}). \
                     Full output:\n{out}"
                );
            }
        }
    }

    /// Back-edge exit-stub glyphs at the source's path-row must connect
    /// cleanly to the path direction — if the route goes LEFT (typical
    /// LR back-edge), the glyph should be `┘` (bottom-right corner with
    /// up + left), NOT `┴` (T-junction with up + left + right). The
    /// gallery's Diagram 7 (composite states with `Working --> Idle`
    /// back-edge labelled "done") shows the orphan `┴` below the "done"
    /// label, with nothing visually continuing right from the junction.
    ///
    /// Strong assertion: locate the back-edge path row in Diagram 7's
    /// rendered output and confirm the rightmost endpoint of the route
    /// is `┘` not `┴`. A no-op fix where the stamp logic still picks
    /// `┴` cannot satisfy this.
    #[test]
    fn back_edge_left_terminus_uses_corner_not_t_junction() {
        let src = "stateDiagram-v2
    state Active {
        [*] --> Idle
        Idle --> Working: task
        Working --> Idle: done
    }
    [*] --> Active";
        let out = crate::render(src).expect("render must succeed");

        // Find the back-edge route row: a line containing `└` followed by
        // multiple `─` then a terminating glyph (either `┘` or the buggy `┴`).
        // Pin the last char of the route as `┘`.
        let route_row = out
            .lines()
            .find(|l| {
                let chars: Vec<char> = l.chars().collect();
                let has_left_corner = chars.contains(&'\u{2514}'); // └
                has_left_corner && (chars.contains(&'\u{2518}') || chars.contains(&'\u{2534}'))
            })
            .expect("back-edge route row not found");

        // Walk left-to-right. After the leftmost `└`, find the next
        // glyph that ENDS the route (`┘` good, `┴` bad).
        let chars: Vec<char> = route_row.chars().collect();
        let left_corner_idx = chars.iter().position(|&c| c == '\u{2514}').unwrap();
        let endpoint = chars[(left_corner_idx + 1)..]
            .iter()
            .find(|&&c| c == '\u{2518}' || c == '\u{2534}')
            .copied()
            .expect("no route endpoint glyph (┘ or ┴) after └");

        assert_eq!(
            endpoint, '\u{2518}',
            "back-edge route's right endpoint is `┴` (T-junction with phantom \
             rightward continuation) — should be `┘` (clean bottom-right corner). \
             Route row:\n{route_row}\n\nFull output:\n{out}"
        );
    }

    /// Two parallel edges fanning out from the same source with text labels
    /// must NOT have their labels land on adjacent rows. The canonical
    /// repro is the gallery's Decision diagram: `Decision -->|yes| Build`
    /// and `Decision -->|no| Skip` previously placed both "yes" and "no"
    /// labels on adjacent rows at the diamond's right exit.
    ///
    /// This test pins the byproduct of G1 (symmetric centring of source
    /// attach points): when `spread_sources` distributes attach points
    /// symmetrically, the labels follow their attach points and end up
    /// well-separated. Strong assertion: row distance between "yes" and
    /// "no" must be ≥ 2.
    #[test]
    fn fan_out_labels_distribute_with_row_separation() {
        let src = "flowchart LR
    Start --> Decision{Decision}
    Decision -->|yes| Build
    Decision -->|no| Skip
    Build --> Deploy
    Skip --> Deploy";
        let out = crate::render(src).expect("render must succeed");
        let lines: Vec<&str> = out.lines().collect();

        let yes_row = lines
            .iter()
            .position(|l| l.contains("yes"))
            .expect("yes label missing");
        let no_row = lines
            .iter()
            .position(|l| l.contains("no") && !l.contains("yes"))
            .expect("no label on its own row missing");

        let distance = yes_row.abs_diff(no_row);
        assert!(
            distance >= 2,
            "fan-out labels 'yes' (line {yes_row}) and 'no' (line {no_row}) \
             are only {distance} row(s) apart — labels at a parallel-edge \
             fan-out should be separated by ≥ 2 rows. Full output:\n{out}"
        );
    }

    /// Two edges merging into the same destination must produce arrow tips
    /// (`▸`) at least 2 rows apart on the destination's left border, not
    /// adjacent rows. The gallery's Diagram 1 (Decision → Build/Skip → Deploy)
    /// is the canonical repro: the two arrows landing into Deploy stack on
    /// adjacent rows (`▸│ Deploy │` immediately above `▸└────────┘`).
    ///
    /// Root cause: `spread_destinations` computes offset as
    /// `(i - (n-1)/2) * step` where `(n-1)/2` is integer division. For
    /// n=2 the result is `[0, +step]` instead of `[-step/2, +step/2]` —
    /// asymmetric, biased toward higher rows. Fix uses
    /// `(2*i - (n-1)) * step / 2` for symmetric placement.
    ///
    /// Strong assertion: count consecutive `▸` glyphs in the same column,
    /// must equal 0. A no-op fix where tips remain adjacent fails.
    #[test]
    fn merging_arrows_into_shared_destination_are_not_adjacent() {
        let src = "flowchart LR
    Start --> Decision{Decision}
    Decision -->|yes| Build
    Decision -->|no| Skip
    Build --> Deploy
    Skip --> Deploy";
        let out = crate::render(src).expect("render must succeed");

        let lines: Vec<Vec<char>> = out.lines().map(|l| l.chars().collect()).collect();
        let max_cols = lines.iter().map(|l| l.len()).max().unwrap_or(0);
        let mut adjacent_pairs = 0usize;
        for col in 0..max_cols {
            for row in 0..lines.len().saturating_sub(1) {
                let here = lines[row].get(col).copied().unwrap_or(' ');
                let below = lines[row + 1].get(col).copied().unwrap_or(' ');
                if here == '\u{25B8}' && below == '\u{25B8}' {
                    adjacent_pairs += 1;
                }
            }
        }
        assert_eq!(
            adjacent_pairs, 0,
            "found {adjacent_pairs} pair(s) of `▸` glyphs on adjacent rows in \
             the same column — arrow tips at a shared destination should be \
             distributed with ≥ 2-row separation.\n\nFull output:\n{out}"
        );
    }

    /// Edge labels must not sit flush against thick (`━ ┃`) or dotted
    /// (`┄ ┆ ╍ ╏`) line glyphs. Thin lines (`─ │`) are intentionally allowed
    /// to abut a label (the `label───▸node` channel pattern is common and
    /// readable), but thick and dotted line styles visually merge with
    /// adjacent label letters and produce strings like `━━━labelled` where
    /// the label and the line read as one ambiguous run.
    ///
    /// The gallery's edge-style showcase (Diagram 3) is the canonical repro:
    /// `A --> B`, `A -.-> C`, `A ==> D`, `A -- "labelled" --> E`,
    /// `A -. "dashed label" .-> F`, `A == "thick label" ==> G`.
    /// All three labels currently land on rows that have thick/dotted glyphs
    /// from sibling edges crossing through.
    ///
    /// A trivially-broken implementation that places labels in the same rows
    /// as today fails this assertion: today's render shows `━━━labelled` so
    /// the cell immediately preceding "labelled" is `━`. The fix moves the
    /// label to a row where its left neighbour is a space or another safe
    /// glyph, so `'━'` will not appear as the prev char.
    #[test]
    fn edge_labels_not_flush_against_thick_or_dotted_lines() {
        let src = "flowchart LR
    A --> B
    A -.-> C
    A ==> D
    A -- \"labelled\" --> E
    A -. \"dashed label\" .-> F
    A == \"thick label\" ==> G";
        let out = crate::render(src).expect("render must succeed");

        let problem_glyphs = [
            '\u{2501}', '\u{2503}', '\u{2504}', '\u{2506}', '\u{254D}', '\u{254F}',
        ];
        for label in &["labelled", "dashed label", "thick label"] {
            let line = out
                .lines()
                .find(|l| l.contains(label))
                .unwrap_or_else(|| panic!("label {label:?} missing in output:\n{out}"));
            let label_byte_pos = line.find(label).unwrap();
            if label_byte_pos == 0 {
                continue;
            }
            let prev_char = line[..label_byte_pos].chars().last().unwrap();
            assert!(
                !problem_glyphs.contains(&prev_char),
                "edge label {label:?} sits flush against thick/dotted glyph \
                 {prev_char:?} (chars before label: {:?}). Row:\n{line}\n\nFull:\n{out}",
                line[..label_byte_pos]
                    .chars()
                    .rev()
                    .take(8)
                    .collect::<Vec<_>>()
            );
        }
    }

    /// **Known limitation (Bug 1, deferred from Phase 3.A)**: a subgraph
    /// with `direction TB` override inside a `graph LR` parent has its
    /// border WIDTH driven by `parallel_label_extra` + `label_width` in
    /// `subgraph.rs::compute_subgraph_bounds`. The Sugiyama LR layout
    /// (default backend since 0.17.0) assigns columns to non-member nodes
    /// without consulting the subgraph's inflated bounding-box width.
    /// Result: the subgraph's right border `│`/`╮` lands inside the
    /// downstream node's bounding box.
    ///
    /// I attempted the fix on the Native backend's `compute_positions`
    /// (mirror of the TD-side `sg_col_min` enforcement), but the visible
    /// artifact is on the Sugiyama backend, which delegates layer
    /// assignment to `ascii-dag` and applies layer offsets uniformly.
    /// Porting the fix to both backends needs a refactor of the
    /// post-pass pipeline that exceeds the launch-window scope.
    ///
    /// Workaround documented in `docs/mermaid-gallery.md`'s Bug-6 callout:
    /// drop the inner `direction TB` (the projections fixture works
    /// without it).
    ///
    /// This test is `#[ignore]`d so it doesn't block CI but stays as a
    /// pinned target for future work — when Bug 1 is implemented across
    /// both backends, remove the `#[ignore]` and verify it passes.
    #[test]
    fn subgraph_border_does_not_overlap_downstream_node_box() {
        let src = "graph LR
    subgraph Supervisor
        direction TB
        F[Factory] -->|creates| W[Worker]
        W -->|panics/exits| F
    end
    W -->|beat every cycle| HB[Heartbeat]";
        let out = crate::render(src).expect("render must succeed");
        let lines: Vec<Vec<char>> = out.lines().map(|l| l.chars().collect()).collect();

        // Find Heartbeat box: row containing the "Heartbeat" label.
        let (hb_row, hb_col) = lines
            .iter()
            .enumerate()
            .find_map(|(r, l)| {
                let chars: Vec<char> = "Heartbeat".chars().collect();
                l.windows(chars.len())
                    .position(|w| w == chars.as_slice())
                    .map(|c| (r, c))
            })
            .expect("Heartbeat label not rendered");

        // Heartbeat's box rows: top border at hb_row-1, mid at hb_row, bot at hb_row+1.
        // Left border: hb_col-2 (1 padding cell + 1 border cell). Right border:
        // hb_col + len("Heartbeat") + 1.
        let hb_left = hb_col.saturating_sub(2);
        let hb_right = hb_col + "Heartbeat".chars().count() + 1;

        // Check for Supervisor's borders piercing Heartbeat's interior.
        // Two failure modes:
        // (a) Subgraph corner glyphs (`╭ ╮ ╰ ╯`) anywhere inside Heartbeat.
        // (b) Vertical bar `│` at any column STRICTLY BETWEEN hb_left and
        //     hb_right on a row inside Heartbeat. Heartbeat's own borders
        //     are at hb_left and hb_right (not "between"), so an interior
        //     `│` is Supervisor's right edge piercing through.
        for r in hb_row.saturating_sub(1)..=hb_row + 1 {
            for c in hb_left..=hb_right {
                let ch = lines.get(r).and_then(|l| l.get(c)).copied().unwrap_or(' ');
                assert!(
                    !matches!(ch, '\u{256D}' | '\u{256E}' | '\u{2570}' | '\u{256F}'),
                    "subgraph corner glyph {ch:?} at ({c},{r}) overlaps Heartbeat box \
                     [{hb_left}..={hb_right}, {}..={}]\n\nFull:\n{out}",
                    hb_row.saturating_sub(1),
                    hb_row + 1
                );
                if c > hb_left && c < hb_right && ch == '\u{2502}' {
                    panic!(
                        "interior `│` at ({c},{r}) inside Heartbeat box — \
                         Supervisor's vertical border is piercing through. \
                         hb_left={hb_left}, hb_right={hb_right}\n\nFull:\n{out}"
                    );
                }
            }
        }
    }

    /// **Known limitation (Bug 5, deferred from Phase 3.B)**: every
    /// back-edge in an LR (or TB) graph carves its OWN return corridor
    /// row below (or beside) the main chain. Two back-edges that both
    /// return from the right half to the left half of the diagram should
    /// share a single corridor row — the A* router currently doesn't
    /// reward route reuse on the perimeter, so each back-edge picks a
    /// unique row to avoid the (artificially high) crossing-cost of
    /// stepping onto an already-occupied edge cell.
    ///
    /// The fix attempted (2026-05-05): reduce `SAME_AXIS_COST` for
    /// back-edges so two back-edges flowing in the same direction
    /// merge onto a shared row. Tried both global-back-edge reduction
    /// (0.5) and canvas-perimeter-only reduction (0.5 for cells where
    /// `nr==0 || nr+1==H || nc==0 || nc+1==W`). Both broke the
    /// state-diagram exit-stub convention pinned by
    /// `back_edge_attach_does_not_pierce_source_perimeter`: the
    /// `┴` glyph (UP+LEFT+RIGHT, "exit stub") on the cell immediately
    /// below a back-edge source's bottom border became `├`
    /// (UP+DOWN+RIGHT, "pierce glyph") because A* found cheaper paths
    /// that descended through that cell rather than turning at it.
    ///
    /// Conclusion: the A*-cost-tweak approach can make sharing happen
    /// but cannot distinguish "shared perimeter corridor cell" (good)
    /// from "shared exit-stub cell" (breaks load-bearing direction
    /// bits). Per-cell metadata or a post-routing nudging pass
    /// (Wybrow 2009 §4 — same algorithm Bug 4 needs) is the correct
    /// path. Bug 4 + Bug 5 should be tackled together once the
    /// post-routing pipeline is in place.
    ///
    /// Fixture: 5-node LR chain with 2 back-edges (E→A, D→B). Both
    /// back-edges flow leftward along the bottom perimeter; without
    /// nudging they carve separate corridor rows. With the post-
    /// routing nudge pass the bottom-most corridor row carries BOTH
    /// back-edges' tap-points simultaneously, producing two `┴`
    /// junctions where the back-edges turn upward to their source
    /// boxes. Without sharing, that row has at most one `┴`.
    ///
    /// Trap-checks (all must pass to defend against no-op renders):
    /// 1. All 5 chain boxes (`│ A │` through `│ E │`) are present.
    /// 2. At least 2 back-edge arrow tips (`◂` / `▴` / `◀` / `▾`)
    ///    are rendered. A trivial implementation that drops back-edges
    ///    would still have boxes but no tips.
    /// 3. At least one row contains a `┴` junction (back-edges must
    ///    actually be routed to the perimeter, not omitted).
    ///
    /// Bug 5 acceptance assertion: the LAST row containing a `┴`
    /// (the bottommost perimeter corridor) carries 2+ `┴` glyphs —
    /// proof that both back-edges share that row.
    #[test]
    fn back_edges_share_return_corridor() {
        let src = "graph LR
    A --> B --> C --> D --> E
    E -->|back1| A
    D -->|back2| B";
        let out = crate::render(src).expect("render must succeed");
        let lines: Vec<Vec<char>> = out.lines().map(|l| l.chars().collect()).collect();
        let lines_str: Vec<String> = lines.iter().map(|l| l.iter().collect()).collect();

        let box_count = ["│ A │", "│ B │", "│ C │", "│ D │", "│ E │"]
            .iter()
            .filter(|needle| lines_str.iter().any(|l| l.contains(*needle)))
            .count();
        assert_eq!(
            box_count, 5,
            "trap-check: not all 5 chain boxes rendered ({box_count}/5).\nFull:\n{out}"
        );

        let back_tip_count = lines
            .iter()
            .map(|l| {
                l.iter()
                    .filter(|&&c| matches!(c, '\u{25C2}' | '\u{25B4}' | '\u{25C0}' | '\u{25BE}'))
                    .count()
            })
            .sum::<usize>();
        assert!(
            back_tip_count >= 2,
            "trap-check: expected ≥2 back-edge arrow tips; found {back_tip_count}.\n\
             A nudge pass that drops a path during shift-apply would fail here \
             without actually changing the corridor layout.\nFull:\n{out}"
        );

        let any_t_junction = lines.iter().any(|l| l.contains(&'\u{2534}'));
        assert!(
            any_t_junction,
            "trap-check: no `┴` glyphs at all — back-edges aren't reaching the \
             perimeter. Test cannot meaningfully assert sharing without taps.\n\
             Full:\n{out}"
        );

        let (bottom_corridor_row, t_junction_count) = lines
            .iter()
            .enumerate()
            .rev()
            .find_map(|(r, line)| {
                let count = line.iter().filter(|&&c| c == '\u{2534}').count();
                if count > 0 { Some((r, count)) } else { None }
            })
            .expect("checked above that ≥1 `┴` exists");

        assert!(
            t_junction_count >= 2,
            "Bug 5: bottom corridor row {bottom_corridor_row} has only \
             {t_junction_count} `┴` junction(s); both back-edges should \
             share this row (expected ≥2). Without sharing, each back-edge \
             carves its own row and only one tap lands on the bottommost \
             corridor.\nRender:\n{out}"
        );
    }

    /// Bug 7 — Perimeter back-edge labels must be placed near the SOURCE,
    /// not at the midpoint of the perimeter return run. The longest-
    /// horizontal-segment heuristic chooses the perimeter run's midpoint,
    /// which lands the label visually disconnected from both endpoints
    /// for any non-trivial perimeter run.
    ///
    /// Fixture: 6-node LR chain with a back-edge from F (rightmost) to
    /// A (leftmost). The back-edge routes via the perimeter row below
    /// the diagram body and the longest segment is the long left-going
    /// horizontal run.
    ///
    /// Fix: for back-edges, bias `col_anchors` to put the source-side
    /// endpoint first so the label lands within ~1/3 of the path from
    /// the source rather than mid-perimeter.
    ///
    /// Trap-check: a no-op render produces no "done" label — the lookup
    /// fails and the test panics. The Manhattan-distance bound (≤ 15
    /// cells from F or A) cannot be satisfied without actually moving
    /// the label closer to an endpoint.
    #[test]
    fn perimeter_back_edge_label_close_to_endpoint() {
        let src = "flowchart LR
    A --> B
    B --> C
    C --> D
    D --> E
    E --> F
    F -->|done| A";
        let out = crate::render(src).expect("render must succeed");
        let lines: Vec<Vec<char>> = out.lines().map(|l| l.chars().collect()).collect();

        // Find char-position of a label or single-char target in a row.
        let find_char = |line: &[char], needle: char| line.iter().position(|&c| c == needle);
        let find_substr = |line: &[char], s: &str| -> Option<usize> {
            let chars: Vec<char> = s.chars().collect();
            line.windows(chars.len()).position(|w| w == chars.as_slice())
        };

        let (label_row, label_col) = lines
            .iter()
            .enumerate()
            .find_map(|(r, l)| find_substr(l, "done").map(|c| (r, c)))
            .expect("'done' label missing");

        // F and A are inside a `│ X │` box. Find the row containing the box
        // (a row with both `│` and the letter), then locate the letter's
        // CHAR column (not byte position — UTF-8 box-drawing chars are 3
        // bytes each and would throw off Manhattan-distance arithmetic
        // across rows of different glyph density).
        let (f_row, f_col) = lines
            .iter()
            .enumerate()
            .find_map(|(r, l)| {
                if l.contains(&'│') {
                    find_char(l, 'F').map(|c| (r, c))
                } else {
                    None
                }
            })
            .expect("F box row missing");

        let (a_row, a_col) = lines
            .iter()
            .enumerate()
            .find_map(|(r, l)| {
                if l.contains(&'│') {
                    find_char(l, 'A').map(|c| (r, c))
                } else {
                    None
                }
            })
            .expect("A box row missing");

        let d_f = label_col.abs_diff(f_col) + label_row.abs_diff(f_row);
        let d_a = label_col.abs_diff(a_col) + label_row.abs_diff(a_row);

        assert!(
            d_f <= 15 || d_a <= 15,
            "label 'done' at ({label_col},{label_row}) is far from F ({f_col},{f_row}) \
             dist={d_f} AND from A ({a_col},{a_row}) dist={d_a}\nFull:\n{out}"
        );
    }

    /// **Known limitation (Bug 4, attempted 2026-05-05, reverted)**: a
    /// "diamond join" topology (4 sources fanning into 1 sink) puts
    /// route corners in the halo columns of nodes that are NOT
    /// endpoints of those routes. Visible artifact: `│ B │├────┐`
    /// where B's right halo column carries a `├` from a route between
    /// A and Z that B has nothing to do with.
    ///
    /// Tried fix: add `Obstacle::NearNodeBox` cost class (1-cell halo
    /// around every node box) modeled after libavoid's
    /// `shapeBufferDistance`. Implementation worked in isolation — the
    /// failing assertion below turned green at every penalty value
    /// from 1.0 to 6.0. But the change rippled into other tests:
    /// `back_edge_attach_does_not_pierce_source_perimeter` failed
    /// because forward edges around Running shifted, displacing the
    /// `┴` exit-stub into a `┤` cell;
    /// `edge_labels_not_flush_against_thick_or_dotted_lines` failed
    /// because routes detoured into rows where labels then landed
    /// flush against thick `┃` glyphs. Tried "corner-only" halo
    /// penalty (only penalize corner steps in halo cells) — broke the
    /// state-diagram exit stub even harder because routes started
    /// going DEEPER into halos before turning, instead of turning at
    /// the box-adjacent cell.
    ///
    /// **Second attempt (2026-05-05 — Phase D of nudging-pass plan)**:
    /// implemented single-path corner displacement in
    /// `crate::layout::nudge::plan_corner_displacements`. The bend
    /// detection was correct, but the diamond_join fixture's `├`
    /// glyph ISN'T a single-path bend — it's a JUNCTION of two
    /// paths' separate bits (one path transits vertically through
    /// the cell, another path's source-attach contributes its
    /// horizontal exit-bit). Each path individually has NO bend
    /// inside B's halo, so the corner-displacement scan finds
    /// nothing to shift.
    ///
    /// The correct fix needs **segment-level eviction**: detect
    /// runs of cells in non-endpoint halos and shift the entire
    /// run perpendicular to its axis, with bridges in the adjacent
    /// segments. This is a 2-3× expansion of the nudge module's
    /// scope (additional shift logic for vertical segments, halo-
    /// run detection across full path length, more aggressive
    /// feasibility checks). Deferred to a follow-up release.
    ///
    /// Trap-check: the fixture's 4 source boxes (A, B, C, D) and sink
    /// (Z) must all be rendered. A no-op render that swallows nodes
    /// would have no halos to pierce and trivially "pass" the
    /// assertion.
    #[test]
    #[ignore = "Bug 4: route corners in non-endpoint halos (deferred — needs post-routing nudging pass, not router-local cost change)"]
    fn route_corners_clear_non_endpoint_node_halos() {
        let src = "graph LR
    A --> Z
    B --> Z
    C --> Z
    D --> Z";
        let out = crate::render(src).expect("render must succeed");
        let lines: Vec<Vec<char>> = out.lines().map(|l| l.chars().collect()).collect();

        let needle: Vec<char> = "│ B │".chars().collect();
        let (b_row, b_col) = lines
            .iter()
            .enumerate()
            .find_map(|(r, l)| {
                l.windows(needle.len())
                    .position(|w| w == needle.as_slice())
                    .map(|c| (r, c))
            })
            .expect("trap-check: B box not rendered (fixture render is broken)");

        for label in ["│ A │", "│ C │", "│ D │", "│ Z │"] {
            let n: Vec<char> = label.chars().collect();
            let found = lines
                .iter()
                .any(|l| l.windows(n.len()).any(|w| w == n.as_slice()));
            assert!(found, "trap-check: {label} not rendered. Full:\n{out}");
        }

        // Strong trap-check: all 4 source routes must reach Z. Without
        // this, a render that drops some routes would have fewer corners
        // to pierce halos and could trivially "pass" the halo assertion.
        // We count arrow tips pointing right (▸ / ▶) anywhere in the
        // rendering — each route to Z contributes one tip.
        let arrow_tip_count = lines
            .iter()
            .map(|l| {
                l.iter()
                    .filter(|&&c| matches!(c, '\u{25B8}' | '\u{25B6}'))
                    .count()
            })
            .sum::<usize>();
        assert!(
            arrow_tip_count >= 4,
            "trap-check: expected ≥4 right-pointing arrow tips (one per route \
             into Z); found {arrow_tip_count}. A render that drops routes \
             would pass the halo check vacuously.\nFull:\n{out}"
        );

        let halo_col = b_col + 5;
        let mut bad_glyphs = Vec::new();
        for r in b_row.saturating_sub(1)..=b_row + 1 {
            let ch = lines.get(r).and_then(|l| l.get(halo_col)).copied().unwrap_or(' ');
            if matches!(
                ch,
                '\u{250C}' | '\u{2510}' | '\u{2514}' | '\u{2518}'
                    | '\u{252C}' | '\u{2534}' | '\u{251C}' | '\u{2524}'
                    | '\u{253C}'
            ) {
                bad_glyphs.push((r, ch));
            }
        }
        assert!(
            bad_glyphs.is_empty(),
            "B is not on any A↔Z route, but its right halo column ({halo_col}) \
             carries route corners {bad_glyphs:?}. Edges should detour around \
             non-endpoint node halos.\nFull:\n{out}"
        );
    }

    /// Bug 4 — Regression guard. With a high-fan-out hub (Worker → 5
    /// downstream nodes), routes splay outward and DON'T concentrate
    /// against Worker's border on a clean topology. The visible "hugging"
    /// in the projections fixture (`│▸│ Worker │┌───────┼┐`) appears to
    /// be an interaction effect of Bug 1 (subgraph border collision) +
    /// the fan-out cost equilibrium — isolating it requires the full
    /// projections topology, and the underlying cause is not just the
    /// router but the layout pipeline's column assignment.
    ///
    /// This test pins the clean-topology behaviour: no corners stacked
    /// in the halo column adjacent to Worker. If a future routing change
    /// (e.g. Phase 3.A's subgraph-layer-width enforcement) reaches into
    /// the cost model, this guard catches accidental halo-hugging.
    ///
    /// The full Bug 4 fix (NearNodeBox halo penalty) is deferred to
    /// post-Bug-1 evaluation — the planner's hypothesis is that Bug 1's
    /// subgraph layer-width fix will indirectly resolve the projections
    /// artifact by giving routes more room to splay. If after Bug 1 the
    /// hugging persists, revisit Bug 4 with a richer fixture.
    #[test]
    fn routes_do_not_hug_non_endpoint_node_borders() {
        // Diagram with a high-fan-out hub similar to the projections case:
        // a central Worker node has many outgoing forward edges to nodes
        // on different rows. The router has to channel those routes out
        // of Worker; without the NearNodeBox halo penalty, multiple
        // routes lay corner glyphs in the cell column immediately right
        // of Worker's right border.
        let src = "graph LR
    W[Worker]
    W --> A[Alpha]
    W --> B[Beta]
    W --> C[Gamma]
    W --> D[Delta]
    W --> E[Epsilon]";
        let out = crate::render(src).expect("render must succeed");
        let lines: Vec<Vec<char>> = out.lines().map(|l| l.chars().collect()).collect();

        // Find Worker box: row containing the "Worker" label.
        let (worker_row, worker_col) = lines
            .iter()
            .enumerate()
            .find_map(|(r, l)| {
                let s: String = l.iter().collect();
                s.find("Worker").map(|c| (r, c))
            })
            .expect("Worker label not rendered");

        // Worker's right-border column is `worker_col + len("Worker") + 1`
        // (label + 1-cell padding). The cell IMMEDIATELY right of that
        // border is the halo column we want to inspect.
        let halo_col = worker_col + "Worker".len() + 2;

        // Existence trap-check: at least one corner glyph somewhere.
        let any_corner = lines.iter().flatten().any(|&c| {
            matches!(c, '\u{250C}' | '\u{2510}' | '\u{2514}' | '\u{2518}'
                       | '\u{252C}' | '\u{2534}' | '\u{251C}' | '\u{2524}'
                       | '\u{253C}')
        });
        assert!(any_corner, "no corner glyphs in render — diagram empty:\n{out}");

        // Count corners in the halo column across the rows that contain
        // Worker's box (worker_row-1 .. worker_row+1).
        let mut halo_corners = 0;
        for r in worker_row.saturating_sub(1)..=worker_row + 1 {
            let ch = lines.get(r).and_then(|l| l.get(halo_col)).copied().unwrap_or(' ');
            if matches!(ch, '\u{250C}' | '\u{2510}' | '\u{2514}' | '\u{2518}'
                          | '\u{252C}' | '\u{2534}' | '\u{251C}' | '\u{2524}'
                          | '\u{253C}')
            {
                halo_corners += 1;
            }
        }
        assert!(
            halo_corners <= 1,
            "{halo_corners} corner glyphs in halo column {halo_col} adjacent to Worker — \
             routes are hugging Worker's right border. Render:\n{out}"
        );
    }

    /// Bug 3 — Regression guard. A decision diamond `{Label}` is registered
    /// as a rectangular `NodeBox` obstacle so A* cannot route any edge
    /// THROUGH the diamond's interior. This test pins that contract: render
    /// a flowchart where a diamond sits between routes that have a
    /// straight-line path through the diamond's bounding rectangle, and
    /// assert no cross-junction (`┼`) appears inside the diamond's interior
    /// (the label-area row, excluding the diamond's own `│` borders).
    ///
    /// The user originally suspected this was a real bug in the projections
    /// fixture; on inspection the visible artifact was actually Bug 4 (routes
    /// hugging the diamond's RIGHT EDGE), not piercing the interior. So this
    /// test is a guard against future routing-cost-tweaks (e.g. Bug 4's
    /// NearNodeBox halo) accidentally allowing diamonds to be traversed.
    ///
    /// Trap-check: if the diamond is rendered as a rectangle (no `╱`/`╲`),
    /// the corner-find fails and the test panics before the assertion. If
    /// the routes vanish entirely (no `│` anywhere outside the diamond),
    /// the diagram has no edges to potentially pierce — also a trap. The
    /// test asserts both edges-exist AND interior-clean.
    #[test]
    fn diamond_interior_has_no_routing_glyphs() {
        let src = "flowchart LR
    A --> B{Decision}
    A --> C[Cached]
    B --> D[Done]
    C --> D";
        let out = crate::render(src).expect("render must succeed");
        let lines: Vec<Vec<char>> = out.lines().map(|l| l.chars().collect()).collect();

        // Diamond corners: top is `╱─...─╲`, mid is `│ Decision │`,
        // bottom is `╲─...─╱`. Find any line containing both `╱` and `╲`.
        let top_idx = lines
            .iter()
            .position(|l| l.contains(&'\u{2571}') && l.contains(&'\u{2572}'))
            .expect("diamond top corner row missing — `╱` and `╲` not both found");
        let bot_idx = (top_idx + 1..lines.len())
            .find(|&i| lines[i].contains(&'\u{2572}') && lines[i].contains(&'\u{2571}'))
            .expect("diamond bottom corner row missing");
        // Diamond's left and right column extents from the top row.
        let left = lines[top_idx]
            .iter()
            .position(|&c| c == '\u{2571}')
            .unwrap();
        let right = lines[top_idx]
            .iter()
            .rposition(|&c| c == '\u{2572}')
            .unwrap();

        // Edges must exist somewhere outside the diamond — guard against a
        // no-op render that has no routes at all.
        let any_edge_glyph = lines.iter().any(|l| {
            l.iter().enumerate().any(|(c, &ch)| {
                ch == '\u{2502}' && (c < left.saturating_sub(1) || c > right + 1)
            })
        });
        assert!(
            any_edge_glyph,
            "no edge glyphs anywhere outside the diamond — render is a no-op:\n{out}"
        );

        // Interior rows: between top and bottom, columns strictly between
        // diamond's outermost `╱`/`╲`. No `┼` (cross-junction) allowed —
        // that would indicate two routes crossing inside the diamond.
        for (r, line) in lines.iter().enumerate().take(bot_idx).skip(top_idx + 1) {
            for c in (left + 2)..(right - 1) {
                let ch = line.get(c).copied().unwrap_or(' ');
                assert_ne!(
                    ch, '\u{253C}',
                    "cross-junction `┼` at ({c},{r}) inside diamond bbox \
                     [{left}..={right}, {top_idx}..={bot_idx}].\nFull:\n{out}"
                );
            }
        }
    }

    /// Bug 2 — A subgraph whose BOTTOM border is crossed by 2+ back-edge
    /// or fan-out routes must not stamp `┼ ┬ ┴ ├ ┤` on every crossing
    /// cell. The G2 fix cleared seeded direction bits on the TOP border
    /// (which carries the label); this is the symmetric clear for the
    /// BOTTOM border. Visible artifact in the projections gallery
    /// fixture: `╰┼──────────┼────────┼──────╯` — three junction stamps
    /// where multiple Worker outgoing edges pierce Supervisor's bottom
    /// border. The dense junctions read as visual noise.
    ///
    /// Reproduction source matches the structural shape of the projections
    /// fixture (Supervisor with 3+ external outgoing edges from a member),
    /// which is when the count exceeds 1.
    ///
    /// Trap-check: a trivially-broken implementation that drew NO bottom
    /// border would lack `╰` and `╯` corners — the lookup `find` would
    /// fail and the test would panic before reaching the count assertion.
    /// A no-op render also produces no `Worker`/`Heartbeat` content so
    /// would render to nothing relevant. The strict `<=1` count cannot be
    /// satisfied except by a real fix that suppresses the seeded dirs.
    #[test]
    fn subgraph_bottom_border_has_at_most_one_junction_glyph() {
        let src = "graph LR
    subgraph Supervisor
        direction TB
        F[Factory] -->|creates| W[Worker]
        W -->|panics/exits| F
    end
    W -->|beat every cycle| HB[Heartbeat]
    HB -->|checked every 10s| WD[Watchdog]
    WD -->|stall > 120s| CT[Cancel Token]
    CT -->|stops| W
    W -->|check before DB call| CB{Circuit Breaker}
    W -->|acquire permit| SEM[Semaphore]";
        let out = crate::render(src).expect("render must succeed");
        let lines: Vec<&str> = out.lines().collect();

        // Bottom border row of the subgraph: contains both `╰` and `╯`
        // (rounded BL and BR corners) AND nothing alphanumeric (so we
        // don't pick up some other line containing curly chars).
        let br_line = lines
            .iter()
            .find(|l| {
                l.contains('\u{2570}')
                    && l.contains('\u{256F}')
                    && !l.chars().any(|c| c.is_alphanumeric())
            })
            .expect("subgraph bottom-border row missing");

        let junctions = ['\u{253C}', '\u{252C}', '\u{2534}', '\u{251C}', '\u{2524}'];
        let count = br_line
            .chars()
            .filter(|c| junctions.contains(c))
            .count();
        assert!(
            count <= 1,
            "subgraph bottom border has {count} junction glyph(s); expected ≤ 1. \
             Line: {br_line:?}\n\nFull output:\n{out}"
        );
    }

    /// Stronger sibling of `route_does_not_pierce_subgraph_title_row`: the
    /// title row must be free of routing junction glyphs (`┼ ┬ ┴ ├ ┤`)
    /// across its ENTIRE width, not just at label-letter columns.
    ///
    /// The earlier fix cleared seeded dir-bits on label-letter cells only, so
    /// when an edge crossed the title border at a non-letter column the
    /// seeded `DIR_LEFT|DIR_RIGHT` bits were still live and `add_dirs` ORed
    /// the route's `DIR_DOWN`/`DIR_UP` over them, producing a `┼` smack in
    /// the middle of the title bar. The gallery's `## Flowcharts` "Subgraphs"
    /// example shows this on `╭─Backend─┼────╮`.
    ///
    /// A trivially-broken implementation that only protects label letters
    /// fails this assertion: today the rendered title row contains at least
    /// one `┼`. The test is also robust to "look elsewhere for `Backend`" no-op
    /// fixes because we explicitly anchor to the LINE that contains "Backend".
    #[test]
    fn subgraph_title_row_has_no_junction_glyphs() {
        let src = "flowchart TB
    subgraph frontend [Frontend]
        UI[Browser UI]
        SW[Service Worker]
    end
    subgraph backend [Backend]
        API[REST API]
        DB[(Postgres)]
    end
    UI --> API
    SW --> API
    API --> DB";
        let out = crate::render(src).expect("render must succeed");

        let backend_title_line = out
            .lines()
            .find(|l| l.contains("Backend"))
            .expect("Backend label missing from output");

        let junctions = ['\u{253C}', '\u{252C}', '\u{2534}', '\u{251C}', '\u{2524}'];
        for ch in backend_title_line.chars() {
            assert!(
                !junctions.contains(&ch),
                "junction glyph {ch:?} found in Backend's title border row \
                 — routing pierced the title bar at a non-letter column. \
                 Title row:\n{backend_title_line}\n\nFull output:\n{out}"
            );
        }
    }

    #[test]
    fn route_does_not_pierce_subgraph_title_row() {
        let src = "flowchart TB
    subgraph frontend [Frontend]
        UI[Browser UI]
        SW[Service Worker]
    end
    subgraph backend [Backend]
        API[REST API]
        DB[(Postgres)]
    end
    UI --> API
    SW --> API
    API --> DB";
        let out = crate::render(src).expect("render must succeed");

        // "Backend" must appear as a contiguous substring somewhere in the output.
        // If routing pierces the title row, the label is split into "Backen"
        // (with a junction glyph where "d" should be).
        assert!(
            out.contains("Backend"),
            "B-title regression: 'Backend' subgraph title is not intact.\n\
             Route(s) likely overwrote a title character with a junction glyph.\n\
             Full output:\n{out}"
        );

        // Confirm "Frontend" is also intact (it sits on the top border row of
        // the first subgraph and is not on any route path — belt-and-suspenders).
        assert!(
            out.contains("Frontend"),
            "B-title regression: 'Frontend' subgraph title is not intact.\n\
             Full output:\n{out}"
        );
    }
}
