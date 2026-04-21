//! Unicode box-drawing renderer.
//!
//! Takes a [`Graph`] and a map of grid positions produced by the layout stage,
//! allocates a [`Grid`] large enough to fit all nodes and edges, draws
//! everything, and returns the final string.

use std::collections::HashMap;

use unicode_width::UnicodeWidthStr;

use crate::{
    layout::{
        Grid, SubgraphBounds,
        grid::{EdgeLineStyle, arrow, endpoint},
        layered::GridPos,
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
            // Parallelogram / trapezoid: +2 width for slant corner markers.
            NodeShape::Parallelogram | NodeShape::Trapezoid => NodeGeom {
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
                height: 1,
                text_row: 0,
            },
            NodeShape::Bar(BarOrientation::Vertical) => NodeGeom {
                width: 1,
                height: 5,
                text_row: 0,
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
fn entry_point_back_edge(pos: GridPos, geom: NodeGeom, dir: Direction) -> Attach {
    let (c, r) = pos;
    match dir {
        // Horizontal flow: enter from the bottom centre.
        Direction::LeftToRight | Direction::RightToLeft => Attach {
            col: c + geom.cx(),
            row: r + geom.height, // one row below the bottom border
        },
        // Vertical flow: enter from the right centre.
        Direction::TopToBottom | Direction::BottomToTop => Attach {
            col: c + geom.width, // one column past the right border
            row: r + geom.cy(),
        },
    }
}

/// Return the arrow-tip character appropriate when a back-edge *arrives* at its
/// destination via the perpendicular side.
///
/// For LR/RL: the edge enters from below the target node, so the tip points UP.
/// For TD/BT: the edge enters from the right of the target node, so the tip
/// points LEFT.
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
    let has_back_edge = graph.edges.iter().any(|e| {
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

    // Compute spread-adjusted attach points for all edges before drawing.
    // Both exit and entry points are spread so that multiple edges sharing
    // the same border cell each get their own distinct row/column.
    let attach_points = compute_spread_attaches(graph, positions, &geoms);

    // Pass 1: Route all edges using A* obstacle-aware routing.
    //
    // Edge style rendering approach:
    //   1. Route with a temporary solid arrow tip — this populates the
    //      direction-bit canvas (needed for junction resolution).
    //   2. After routing, call `overdraw_path_style` to replace path cells
    //      with thick or dotted glyphs based on the edge's `EdgeStyle`.
    //   3. Override the destination tip glyph based on `EdgeEndpoint`.
    //   4. For bidirectional edges, also place a back-tip at the source cell.
    //
    // This keeps all junction-merging logic in the direction-bit canvas while
    // still producing visually distinct dotted/thick lines.
    //
    // Collect edge label placements for a deferred write — labels must be
    // written *after* all routing so that no subsequent A* path overwrites them.
    // Each entry is `(col, row, label_text)`.
    let mut pending_labels: Vec<(usize, usize, String, Option<crate::types::Rgb>)> = Vec::new();
    // Collision registry: `(col, row, display_width, height)` of committed labels.
    let mut placed_labels: Vec<(usize, usize, usize, usize)> = Vec::new();
    // Back-edge connector points: where to stamp `┬` / `┴` (LR) or `├` / `┤`
    // (TD) after node boxes are drawn, so the perimeter back-edge path
    // connects visibly to its source and destination borders.
    // Entries: `(border_col, border_row, is_destination)`.
    let mut back_edge_border_joins: Vec<(usize, usize, bool)> = Vec::new();
    // First-path-cell joins (source end only — destination end is the arrow tip).
    let mut back_edge_path_joins: Vec<(usize, usize)> = Vec::new();

    for (edge_idx, edge) in graph.edges.iter().enumerate() {
        let Some(Some((src, dst))) = attach_points.get(edge_idx) else {
            continue;
        };
        let (src, dst) = (*src, *dst);

        // Determine whether this edge is a back-edge so we can select the
        // correct tip character. Back-edges enter their target from a
        // perpendicular side, so the tip must point in the perpendicular
        // direction rather than the primary flow direction.
        let edge_is_back = {
            let from_pos = positions.get(&edge.from).copied();
            let to_pos = positions.get(&edge.to).copied();
            match (from_pos, to_pos) {
                (Some(fp), Some(tp)) => is_back_edge(fp, tp, graph.direction),
                _ => false,
            }
        };

        // For back-edges, the perimeter path attaches one cell past the
        // source's perpendicular border (below for LR, right for TB). The
        // node border there is drawn solid by pass 2, which visually
        // disconnects the path. Record the border cell and the adjacent
        // path cell so a post-pass can stamp junction glyphs.
        if edge_is_back
            && let (Some(fp), Some(fg), Some(tp), Some(tg)) = (
                positions.get(&edge.from).copied(),
                geoms.get(&edge.from).copied(),
                positions.get(&edge.to).copied(),
                geoms.get(&edge.to).copied(),
            )
        {
            let (sb, sp) = back_edge_border_cells(fp, fg, graph.direction);
            let (db, _) = back_edge_border_cells(tp, tg, graph.direction);
            back_edge_border_joins.push((sb.0, sb.1, false));
            back_edge_border_joins.push((db.0, db.1, true));
            back_edge_path_joins.push(sp);
        }

        // Select the forward tip character (destination end).
        // For EdgeEndpoint::None (plain line), we route with the normal arrow
        // so the path is drawn correctly by route_edge, then immediately clear
        // the tip protection so the no-arrow cell merges into the path glyph.
        let fwd_tip = if edge_is_back {
            tip_char_for_back_edge(graph.direction)
        } else {
            tip_char(graph.direction)
        };
        let horizontal_first = graph.direction.is_horizontal();
        let path = grid.route_edge(
            src.col,
            src.row,
            dst.col,
            dst.row,
            horizontal_first,
            fwd_tip,
        );

        // Post-process the destination tip cell for non-arrow endpoints.
        //
        // `route_edge` always places the flow-direction arrow at the tip cell
        // and protects it. Here we unprotect and overwrite as needed:
        //   - None    → plain line glyph (no arrowhead)
        //   - Circle  → ○
        //   - Cross   → ×
        //   - Arrow   → keep the arrow (no action needed)
        if let Some(ref path) = path
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

        if let Some(ref path) = path {
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
        if let (Some(lbl), Some(path)) = (&edge.label, &path)
            && let Some((lbl_col, lbl_row)) =
                label_position(path, lbl, graph.direction, &mut placed_labels, &node_rects)
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
        if with_color
            && let Some(style) = graph.node_styles.get(&node.id).copied()
        {
            paint_node_colors(&mut grid, pos, geom, style);
        }
    }

    // Pass 2a.5: Stamp back-edge connector glyphs at each back-edge's
    // source/destination border so the perimeter path connects visibly.
    // Pass 2 just redrew the node box's bottom/right border as a plain line,
    // hiding the routed back-edge's exit/entry point.
    let (border_junction, path_junction) = match graph.direction {
        Direction::LeftToRight | Direction::RightToLeft => ('┬', '┴'),
        Direction::TopToBottom | Direction::BottomToTop => ('├', '┤'),
    };
    for (col, row, _is_dest) in &back_edge_border_joins {
        grid.set(*col, *row, border_junction);
    }
    for (col, row) in &back_edge_path_joins {
        // Only upgrade the path cell if it's a plain horizontal/vertical line
        // from the router — corners and other junctions are left alone so we
        // don't mangle complex routed paths.
        let current = grid.get(*col, *row);
        if current == '─' || current == '│' {
            grid.set(*col, *row, path_junction);
        }
    }

    // Pass 2b: Write all edge labels after node boxes so that node box
    // drawing (which uses `set()` unconditionally) cannot overwrite labels.
    // Labels are protected so that node labels in pass 3 cannot erase them.
    for (lbl_col, lbl_row, lbl, lbl_color) in &pending_labels {
        grid.write_text_protected(*lbl_col, *lbl_row, lbl);
        if let Some(c) = lbl_color {
            let lbl_w = UnicodeWidthStr::width(lbl.as_str());
            grid.paint_fg_rect(*lbl_col, *lbl_row, lbl_w, 1, *c);
        }
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
            if is_back_edge(from_pos, to_pos, graph.direction) {
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
        NodeShape::DoubleCircle => {
            grid.draw_double_circle(col, row, geom.width, geom.height);
        }
        NodeShape::Bar(BarOrientation::Horizontal) => {
            grid.draw_horizontal_bar(col, row, geom.width);
        }
        NodeShape::Bar(BarOrientation::Vertical) => {
            grid.draw_vertical_bar(col, row, geom.height);
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
    // them alone.  We only protect the outline (border ring), not interior.
    for x in col..(col + w) {
        grid.protect_cell(x, row);
        grid.protect_cell(x, row + h - 1);
    }
    for y in (row + 1)..(row + h - 1) {
        grid.protect_cell(col, y);
        grid.protect_cell(col + w - 1, y);
    }

    // Subgraph borders are *protected* (so their glyphs survive edge
    // routing) but NOT marked as hard `NodeBox` obstacles. Hard marking
    // would prevent any edge whose source or destination lies inside the
    // subgraph from exiting through the border — A* would give up and
    // fall back to Manhattan routing, which ignores obstacles entirely.
    // Leaving borders passable lets A* find real orthogonal paths that
    // cross subgraph boundaries naturally; the border glyph at the
    // crossing cell stays intact thanks to `protect_cell`.

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
fn draw_label_centred(grid: &mut Grid, node: &Node, pos: GridPos, geom: NodeGeom) {
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
        grid.write_text(text_col, row + geom.text_row + i, line);
    }
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
    node_rects: &[(usize, usize, usize, usize)],
) -> Option<(usize, usize)> {
    if path.len() < 2 {
        return None;
    }
    let lbl_w = UnicodeWidthStr::width(label);
    if lbl_w == 0 {
        return None;
    }

    let candidates = candidate_positions(path, dir);
    if candidates.is_empty() {
        return None;
    }

    // Pass A: prefer positions that avoid both other labels and node interiors.
    for &(c, r) in &candidates {
        if !collides(c, r, lbl_w, placed) && !overlaps_node_interior(c, r, lbl_w, node_rects) {
            placed.push((c, r, lbl_w, 1));
            return Some((c, r));
        }
    }
    // Pass B: relax the node-overlap constraint as a last resort. Two
    // labels on top of each other is unreadable, so we still respect
    // `placed`; a label sitting on a node's border row is awkward but
    // strictly better than the previous "overwrite the node interior
    // silently" behaviour.
    for &(c, r) in &candidates {
        if !collides(c, r, lbl_w, placed) {
            placed.push((c, r, lbl_w, 1));
            return Some((c, r));
        }
    }
    None
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
fn candidate_positions(path: &[(usize, usize)], dir: Direction) -> Vec<(usize, usize)> {
    match dir {
        Direction::LeftToRight | Direction::RightToLeft => {
            let Some((mid_col, seg_row, lo_col, hi_col)) = last_horizontal_segment_with_range(path)
            else {
                return Vec::new();
            };
            // Three column anchors along the segment.
            let third = (hi_col - lo_col) / 3;
            let col_anchors = [mid_col, lo_col + third, lo_col + 2 * third];
            // Row offsets: alternate above/below, growing in distance.
            let row_offsets: [isize; 8] = [-1, 1, -2, 2, -3, 3, -4, 4];
            let mut out = Vec::with_capacity(col_anchors.len() * row_offsets.len());
            for &c in &col_anchors {
                for &dr in &row_offsets {
                    let r = (seg_row as isize + dr).max(0) as usize;
                    out.push((c, r));
                }
            }
            out
        }
        Direction::TopToBottom | Direction::BottomToTop => {
            let (seg_col, seg_row) = match last_vertical_segment(path) {
                Some(v) => v,
                None => return Vec::new(),
            };
            let col_anchors = [seg_col + 1, seg_col.saturating_sub(1), seg_col + 2];
            let row_offsets: [isize; 5] = [0, -1, 1, -2, 2];
            let mut out = Vec::with_capacity(col_anchors.len() * row_offsets.len());
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
/// at least 2 cells long. Returns `(midpoint_col, row, lo_col, hi_col)`
/// — the inclusive `(lo, hi)` range lets callers pick column anchors
/// along the segment (not just its midpoint) for label placement.
fn last_horizontal_segment_with_range(
    path: &[(usize, usize)],
) -> Option<(usize, usize, usize, usize)> {
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
        let int_left = nc + 1;
        let int_right = nc + nw - 1; // exclusive
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
}
