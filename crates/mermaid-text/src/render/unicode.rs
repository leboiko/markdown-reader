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
            NodeShape::Diamond => NodeGeom {
                width: inner_w + 4, // extra room for diagonal sides
                height: 5,
                text_row: 2,
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

    // Pass 1: Draw edges first so node boxes and labels render on top.
    for edge in &graph.edges {
        let (Some(&from_pos), Some(&to_pos)) = (positions.get(&edge.from), positions.get(&edge.to))
        else {
            continue;
        };
        let (Some(&from_geom), Some(&to_geom)) = (geoms.get(&edge.from), geoms.get(&edge.to))
        else {
            continue;
        };

        let src = exit_point(from_pos, from_geom, graph.direction);
        let dst = entry_point(to_pos, to_geom, graph.direction);
        let tip = tip_char(graph.direction);

        let horizontal_first = graph.direction.is_horizontal();
        grid.draw_manhattan(src.col, src.row, dst.col, dst.row, horizontal_first, tip);

        // Draw edge label if present
        if let Some(ref lbl) = edge.label {
            place_edge_label(&mut grid, src, dst, lbl, graph.direction);
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

    // For diamond shape, the interior width at the text row is narrower;
    // add extra horizontal offset
    let text_col = if node.shape == NodeShape::Diamond {
        let indent = geom.height / 2; // diagonal encroachment at centre row
        col + indent
            + (geom
                .width
                .saturating_sub(indent * 2)
                .saturating_sub(label_w))
                / 2
    } else {
        text_col
    };

    grid.write_text(text_col, row + geom.text_row, &node.label);
}

// ---------------------------------------------------------------------------
// Edge label placement
// ---------------------------------------------------------------------------

/// Place an edge label on the routing path between `src` and `dst`.
///
/// For LR/RL flows the label is placed on the horizontal segment, vertically
/// offset toward the destination. This ensures that two edges diverging from
/// the same source to different rows each get a distinct label row.
///
/// For TD/BT flows the label is placed on the vertical segment, one column
/// to the right.
fn place_edge_label(grid: &mut Grid, src: Attach, dst: Attach, label: &str, dir: Direction) {
    let (lbl_col, lbl_row) = match dir {
        Direction::LeftToRight | Direction::RightToLeft => {
            // For horizontal-primary routing: horizontal segment runs at
            // src.row; label is placed one row in the direction of dst.row
            // (up if dst is higher, down if lower, above if same row).
            // This ensures edges from the same source to different rows get
            // non-overlapping label positions.
            let mid_col = (src.col + dst.col) / 2;
            let row = if dst.row < src.row {
                // edge bends upward — place label above the horizontal seg
                src.row.saturating_sub(1)
            } else if dst.row > src.row {
                // edge bends downward — place label below the horizontal seg
                src.row + 1
            } else {
                // straight horizontal — label above
                src.row.saturating_sub(1)
            };
            (mid_col, row)
        }
        Direction::TopToBottom | Direction::BottomToTop => {
            // Vertical-primary: label to the right, midpoint vertically.
            let mid_row = (src.row + dst.row) / 2;
            let col = if dst.col > src.col {
                src.col.saturating_sub(1)
            } else {
                src.col + 1
            };
            (col, mid_row)
        }
    };

    grid.write_text(lbl_col, lbl_row, label);
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
