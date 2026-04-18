//! Simplified Sugiyama-inspired layered layout algorithm.
//!
//! # Algorithm overview
//!
//! 1. **Layer assignment** — topological sort; each node is placed at layer
//!    `max(layer(predecessor)) + 1` (longest-path from sources).
//!
//! 2. **Within-layer ordering** — two passes of the barycenter heuristic:
//!    forward (left/top-to-right/bottom) and backward.
//!
//! 3. **Position computation** — convert (layer, rank) pairs into character-
//!    grid `(col, row)` coordinates using configurable spacing constants.

use std::collections::HashMap;

use unicode_width::UnicodeWidthStr;

use crate::types::{Direction, Graph, NodeShape};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Layout spacing constants.
#[derive(Debug, Clone, Copy)]
pub struct LayoutConfig {
    /// Minimum gap (in characters) between layers (the axis perpendicular to
    /// the flow direction). Includes the width/height of the node box plus
    /// the inter-layer spacing itself.
    pub layer_gap: usize,
    /// Minimum gap (in characters) between sibling nodes in the same layer.
    pub node_gap: usize,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            layer_gap: 6,
            node_gap: 2,
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Character-grid position of a node's top-left corner.
pub type GridPos = (usize, usize); // (col, row)

/// Compute character-grid positions for every node in `graph`.
///
/// Returns a map from node ID to `(col, row)` top-left position on the
/// character grid.  The grid's origin is `(0, 0)`.
///
/// # Arguments
///
/// * `graph`  — the parsed graph
/// * `config` — spacing parameters
pub fn layout(graph: &Graph, config: &LayoutConfig) -> HashMap<String, GridPos> {
    if graph.nodes.is_empty() {
        return HashMap::new();
    }

    // 1. Assign layers
    let layers = assign_layers(graph);

    // 2. Group nodes into per-layer lists and sort them by barycenter
    let ordered = order_within_layers(graph, &layers);

    // 3. Convert to grid coordinates
    compute_positions(graph, &ordered, config)
}

// ---------------------------------------------------------------------------
// Step 1: Layer assignment (longest path from sources)
// ---------------------------------------------------------------------------

/// Returns a map from node ID to layer index (0 = leftmost/topmost).
fn assign_layers(graph: &Graph) -> HashMap<String, usize> {
    let mut layer: HashMap<String, usize> = HashMap::new();

    // Build adjacency: predecessors[id] = list of ids that point TO id
    let mut predecessors: HashMap<&str, Vec<&str>> = HashMap::new();
    for node in &graph.nodes {
        predecessors.entry(node.id.as_str()).or_default();
    }
    for edge in &graph.edges {
        predecessors
            .entry(edge.to.as_str())
            .or_default()
            .push(edge.from.as_str());
    }

    // Iterative longest-path. We keep running passes until nothing changes.
    // This handles cycles by capping at max_iter = node_count.
    let max_iter = graph.nodes.len() + 1;
    let mut changed = true;
    let mut iter = 0;

    // Initialise all nodes to layer 0
    for node in &graph.nodes {
        layer.insert(node.id.clone(), 0);
    }

    while changed && iter < max_iter {
        changed = false;
        iter += 1;
        for edge in &graph.edges {
            let from_layer = layer.get(edge.from.as_str()).copied().unwrap_or(0);
            let to_layer = layer.entry(edge.to.clone()).or_insert(0);
            if from_layer + 1 > *to_layer {
                *to_layer = from_layer + 1;
                changed = true;
            }
        }
    }

    // Ensure all nodes appear even if they have no edges
    for node in &graph.nodes {
        layer.entry(node.id.clone()).or_insert(0);
    }

    layer
}

// ---------------------------------------------------------------------------
// Step 2: Within-layer ordering (barycenter heuristic)
// ---------------------------------------------------------------------------

/// Returns per-layer ordered lists of node IDs.
/// Index 0 of the outer Vec is layer 0 (sources).
///
/// Uses an iterative barycenter heuristic with best-seen retention:
/// alternating forward/backward sweeps for up to `MAX_PASSES` iterations,
/// keeping the ordering with the fewest edge crossings ever observed, and
/// exiting early after `NO_IMPROVEMENT_CAP` consecutive non-improving passes.
fn order_within_layers(graph: &Graph, layers: &HashMap<String, usize>) -> Vec<Vec<String>> {
    // Find max layer
    let max_layer = layers.values().copied().max().unwrap_or(0);
    let num_layers = max_layer + 1;

    // Bucket nodes into layers (preserve declaration order as initial order)
    let mut buckets: Vec<Vec<String>> = vec![Vec::new(); num_layers];
    for node in &graph.nodes {
        let l = layers[&node.id];
        buckets[l].push(node.id.clone());
    }

    // Build successor/predecessor maps for barycenter computation.
    let mut successors: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut predecessors: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in &graph.edges {
        successors
            .entry(edge.from.as_str())
            .or_default()
            .push(edge.to.as_str());
        predecessors
            .entry(edge.to.as_str())
            .or_default()
            .push(edge.from.as_str());
    }

    // Per-node layer lookup for the crossing counter. Borrows from `layers`
    // rather than `buckets` so that it stays live across mutations of the
    // latter during sweep passes.
    let node_layer: HashMap<&str, usize> = layers.iter().map(|(id, &l)| (id.as_str(), l)).collect();

    // Iterative refinement. Termaid's implementation caps at 8 passes with
    // early termination after 4 non-improving passes; the same constants
    // work well here.
    const MAX_PASSES: usize = 8;
    const NO_IMPROVEMENT_CAP: usize = 4;

    let mut best = buckets.clone();
    let mut best_crossings = count_crossings(graph, &node_layer, &best);
    let mut no_improvement = 0usize;

    for _ in 0..MAX_PASSES {
        sort_by_barycenter(&mut buckets, &predecessors, SweepDirection::Forward);
        sort_by_barycenter(&mut buckets, &successors, SweepDirection::Backward);

        let c = count_crossings(graph, &node_layer, &buckets);
        if c < best_crossings {
            best = buckets.clone();
            best_crossings = c;
            no_improvement = 0;
        } else {
            no_improvement += 1;
            if no_improvement >= NO_IMPROVEMENT_CAP {
                break;
            }
        }

        if best_crossings == 0 {
            break;
        }
    }

    best
}

/// Direction of a barycenter sweep.
#[derive(Copy, Clone)]
enum SweepDirection {
    /// Sort each layer (except layer 0) by the average position of its
    /// predecessors in the previous layer.
    Forward,
    /// Sort each layer (except the last) by the average position of its
    /// successors in the next layer.
    Backward,
}

/// Sort each layer in `buckets` by the barycenter of its neighbors in the
/// adjacent layer, as selected by `dir`.
///
/// `neighbors` maps each node to its predecessors (for Forward) or successors
/// (for Backward). Nodes without neighbors keep their current position via a
/// stable sort — this prevents the heuristic from shuffling isolated nodes.
fn sort_by_barycenter(
    buckets: &mut [Vec<String>],
    neighbors: &HashMap<&str, Vec<&str>>,
    dir: SweepDirection,
) {
    let num_layers = buckets.len();
    if num_layers < 2 {
        return;
    }

    let layer_iter: Box<dyn Iterator<Item = usize>> = match dir {
        SweepDirection::Forward => Box::new(1..num_layers),
        SweepDirection::Backward => Box::new((0..num_layers - 1).rev()),
    };

    for l in layer_iter {
        let ref_layer = match dir {
            SweepDirection::Forward => l - 1,
            SweepDirection::Backward => l + 1,
        };

        let ref_positions: HashMap<&str, f64> = buckets[ref_layer]
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i as f64))
            .collect();

        // Pair each node with its current position, so nodes with no neighbors
        // can fall back to it (preserves stability and prevents isolated nodes
        // from drifting to 0).
        let mut keyed: Vec<(String, f64)> = buckets[l]
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let neigh = neighbors.get(id.as_str()).cloned().unwrap_or_default();
                let bc = if neigh.is_empty() {
                    i as f64
                } else {
                    let sum: f64 = neigh
                        .iter()
                        .map(|n| ref_positions.get(n).copied().unwrap_or(i as f64))
                        .sum();
                    sum / neigh.len() as f64
                };
                (id.clone(), bc)
            })
            .collect();

        keyed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        buckets[l] = keyed.into_iter().map(|(id, _)| id).collect();
    }
}

/// Count the number of edge crossings implied by the given layer ordering.
///
/// For each pair of edges `(u1, v1)` and `(u2, v2)` that both span the same
/// layer gap (`u.layer → v.layer`), they cross iff the relative positions
/// of `u1,u2` in their layer differ from the relative positions of `v1,v2`.
/// This is the classic inversion test; O(E²) per gap, which is fine for the
/// small graphs this crate targets.
fn count_crossings(
    graph: &Graph,
    node_layer: &HashMap<&str, usize>,
    buckets: &[Vec<String>],
) -> usize {
    // Per-layer rank lookup.
    let mut rank: HashMap<&str, usize> = HashMap::new();
    for layer_nodes in buckets {
        for (i, id) in layer_nodes.iter().enumerate() {
            rank.insert(id.as_str(), i);
        }
    }

    // Group edges by the ordered (from_layer, to_layer) gap they cross.
    // Edges that stay within a single layer or that skip layers still "count"
    // here because they still produce visual crossings at the rendered gap.
    let edges_with_gaps: Vec<(usize, usize, usize, usize)> = graph
        .edges
        .iter()
        .filter_map(|e| {
            let fl = *node_layer.get(e.from.as_str())?;
            let tl = *node_layer.get(e.to.as_str())?;
            let fr = *rank.get(e.from.as_str())?;
            let tr = *rank.get(e.to.as_str())?;
            Some((fl, tl, fr, tr))
        })
        .collect();

    let mut total = 0usize;
    for i in 0..edges_with_gaps.len() {
        let (fl1, tl1, fr1, tr1) = edges_with_gaps[i];
        for &(fl2, tl2, fr2, tr2) in &edges_with_gaps[i + 1..] {
            // Only edges spanning the same gap can cross.
            if (fl1, tl1) != (fl2, tl2) {
                continue;
            }
            // Inversion test: crosses iff one pair is strictly ordered and the
            // other pair is strictly ordered in the opposite direction.
            let from_order = fr1.cmp(&fr2);
            let to_order = tr1.cmp(&tr2);
            if from_order != std::cmp::Ordering::Equal
                && to_order != std::cmp::Ordering::Equal
                && from_order != to_order
            {
                total += 1;
            }
        }
    }

    total
}

// ---------------------------------------------------------------------------
// Step 3: Position computation
// ---------------------------------------------------------------------------

/// Compute the display width of a node (its box width in characters).
///
/// Must stay in sync with `NodeGeom::for_node` in `render/unicode.rs`.
fn node_box_width(graph: &Graph, id: &str) -> usize {
    if let Some(node) = graph.node(id) {
        let label_width = unicode_width::UnicodeWidthStr::width(node.label.as_str());
        let inner = label_width + 4; // 2-char padding each side
        match node.shape {
            // Diamond renders as a plain rectangle.
            NodeShape::Diamond => inner,
            // Circle/Stadium/Hexagon/Asymmetric add 1 extra char on each side
            // for their distinctive markers inside the border.
            NodeShape::Circle
            | NodeShape::Stadium
            | NodeShape::Hexagon
            | NodeShape::Asymmetric => inner + 2,
            // Subroutine adds 1 extra char on each side for inner vertical bars.
            NodeShape::Subroutine => inner + 2,
            // Cylinder: standard width — arcs are drawn at top/bottom centre.
            NodeShape::Cylinder => inner,
            // Parallelogram / Trapezoid: add 2 extra chars for slant markers.
            NodeShape::Parallelogram | NodeShape::Trapezoid => inner + 2,
            // DoubleCircle: needs 4 extra chars for the concentric inner border.
            NodeShape::DoubleCircle => inner + 4,
            // Plain shapes.
            NodeShape::Rectangle | NodeShape::Rounded => inner,
        }
    } else {
        6 // fallback
    }
}

/// Compute the display height of a node (its box height in characters).
///
/// Must stay in sync with `NodeGeom::for_node` in `render/unicode.rs`.
fn node_box_height(graph: &Graph, id: &str) -> usize {
    if let Some(node) = graph.node(id) {
        match node.shape {
            // Standard 3-row shapes: top border + text + bottom border.
            NodeShape::Diamond
            | NodeShape::Rectangle
            | NodeShape::Rounded
            | NodeShape::Circle
            | NodeShape::Stadium
            | NodeShape::Hexagon
            | NodeShape::Asymmetric
            | NodeShape::Parallelogram
            | NodeShape::Trapezoid
            | NodeShape::Subroutine => 3,
            // Cylinder needs 5 rows: arc-top, arc-inner, text, arc-inner, arc-bottom.
            NodeShape::Cylinder => 5,
            // DoubleCircle needs 5 rows for the concentric inner border.
            NodeShape::DoubleCircle => 5,
        }
    } else {
        3
    }
}

/// Build a map from node ID to its assigned layer index.
///
/// This is a copy of `assign_layers` output, returned here so that
/// `compute_positions` can look up which layer a given node lives in.
fn build_node_layer_map(ordered: &[Vec<String>]) -> HashMap<&str, usize> {
    let mut map = HashMap::new();
    for (layer_idx, layer_nodes) in ordered.iter().enumerate() {
        for id in layer_nodes {
            map.insert(id.as_str(), layer_idx);
        }
    }
    map
}

/// Compute the minimum inter-layer gap needed to fit all edge labels that
/// cross the gap between `layer_a` and `layer_b`.
///
/// An edge crosses a gap when its source is in layer `layer_a` and its
/// destination is in layer `layer_b` (or vice-versa for reversed directions).
/// The gap must be wide enough to display the longest such label plus 2
/// cells of padding on each side.
///
/// Multiple labeled edges from the same source node stacked in the same gap
/// each occupy 2 rows, so we also account for stacking height.
fn label_gap(
    graph: &Graph,
    node_layer: &HashMap<&str, usize>,
    layer_a: usize,
    layer_b: usize,
    default_gap: usize,
) -> usize {
    // Collect widths of all labels on edges that cross this gap.
    let mut label_widths: Vec<usize> = graph
        .edges
        .iter()
        .filter(|e| {
            let fl = node_layer.get(e.from.as_str()).copied().unwrap_or(0);
            let tl = node_layer.get(e.to.as_str()).copied().unwrap_or(0);
            // Edge crosses the gap in either direction.
            (fl == layer_a && tl == layer_b) || (fl == layer_b && tl == layer_a)
        })
        .filter_map(|e| e.label.as_deref())
        .map(UnicodeWidthStr::width)
        .collect();

    if label_widths.is_empty() {
        return default_gap;
    }

    // Widest single label + 2 padding cells.
    let max_lbl = label_widths.iter().copied().max().unwrap_or(0);
    let needed_for_width = max_lbl + 2;

    // If multiple labels compete for vertical space in the same gap, each
    // occupies 2 rows (one for the label text, one spacing row). We keep at
    // least that many rows available.
    label_widths.sort_unstable();
    let count = label_widths.len();
    let needed_for_stacking = count * 2 + 1;

    default_gap.max(needed_for_width).max(needed_for_stacking)
}

/// Convert the ordered layer buckets into character-grid `(col, row)` positions.
fn compute_positions(
    graph: &Graph,
    ordered: &[Vec<String>],
    config: &LayoutConfig,
) -> HashMap<String, GridPos> {
    let mut positions: HashMap<String, GridPos> = HashMap::new();

    // Build a node-to-layer map once; used by the label-gap calculation.
    let node_layer = build_node_layer_map(ordered);

    match graph.direction {
        Direction::LeftToRight | Direction::RightToLeft => {
            // Layers are columns; nodes within a layer are rows.
            let mut col = 0usize;

            for (layer_idx, layer_nodes) in ordered.iter().enumerate() {
                if layer_nodes.is_empty() {
                    continue;
                }

                // Column width = widest node in this layer
                let layer_width = layer_nodes
                    .iter()
                    .map(|id| node_box_width(graph, id))
                    .max()
                    .unwrap_or(6);

                let mut row = 0usize;
                for id in layer_nodes {
                    let h = node_box_height(graph, id);
                    positions.insert(id.clone(), (col, row));
                    row += h + config.node_gap;
                }

                // Inter-layer gap: at least default, but wide enough for edge
                // labels that cross into the next layer.
                let gap = if layer_idx + 1 < ordered.len() {
                    label_gap(
                        graph,
                        &node_layer,
                        layer_idx,
                        layer_idx + 1,
                        config.layer_gap,
                    )
                } else {
                    config.layer_gap
                };

                col += layer_width + gap;
            }

            // Reverse column positions for RL direction
            if graph.direction == Direction::RightToLeft {
                let max_col = positions.values().map(|(c, _)| *c).max().unwrap_or(0);
                for (col, _) in positions.values_mut() {
                    *col = max_col - *col;
                }
            }
        }

        Direction::TopToBottom | Direction::BottomToTop => {
            // Layers are rows; nodes within a layer are columns.
            let mut row = 0usize;

            for (layer_idx, layer_nodes) in ordered.iter().enumerate() {
                if layer_nodes.is_empty() {
                    continue;
                }

                // Row height = tallest node in this layer
                let layer_height = layer_nodes
                    .iter()
                    .map(|id| node_box_height(graph, id))
                    .max()
                    .unwrap_or(3);

                let mut col = 0usize;
                for id in layer_nodes {
                    let w = node_box_width(graph, id);
                    positions.insert(id.clone(), (col, row));
                    col += w + config.node_gap;
                }

                // Inter-layer gap: at least default, but tall enough for edge
                // labels that cross into the next layer.
                let gap = if layer_idx + 1 < ordered.len() {
                    label_gap(
                        graph,
                        &node_layer,
                        layer_idx,
                        layer_idx + 1,
                        config.layer_gap,
                    )
                } else {
                    config.layer_gap
                };

                row += layer_height + gap;
            }

            // Reverse row positions for BT direction
            if graph.direction == Direction::BottomToTop {
                let max_row = positions.values().map(|(_, r)| *r).max().unwrap_or(0);
                for (_, row) in positions.values_mut() {
                    *row = max_row - *row;
                }
            }
        }
    }

    positions
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Direction, Edge, Graph, Node, NodeShape};

    fn simple_lr_graph() -> Graph {
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
        g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
        g.nodes.push(Node::new("C", "C", NodeShape::Rectangle));
        g.edges.push(Edge::new("A", "B", None));
        g.edges.push(Edge::new("B", "C", None));
        g
    }

    #[test]
    fn lr_nodes_have_increasing_columns() {
        let g = simple_lr_graph();
        let cfg = LayoutConfig::default();
        let pos = layout(&g, &cfg);
        assert!(pos["A"].0 < pos["B"].0);
        assert!(pos["B"].0 < pos["C"].0);
    }

    #[test]
    fn td_nodes_have_increasing_rows() {
        let mut g = Graph::new(Direction::TopToBottom);
        g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
        g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
        g.edges.push(Edge::new("A", "B", None));

        let cfg = LayoutConfig::default();
        let pos = layout(&g, &cfg);
        assert!(pos["A"].1 < pos["B"].1);
    }

    #[test]
    fn cyclic_graph_terminates() {
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
        g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
        g.edges.push(Edge::new("A", "B", None));
        g.edges.push(Edge::new("B", "A", None));

        let cfg = LayoutConfig::default();
        let pos = layout(&g, &cfg);
        assert_eq!(pos.len(), 2);
    }

    #[test]
    fn single_node_layout() {
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "Alone", NodeShape::Rectangle));

        let cfg = LayoutConfig::default();
        let pos = layout(&g, &cfg);
        assert_eq!(pos["A"], (0, 0));
    }
}
