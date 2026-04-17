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

    // Build successor map for barycenter computation
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

    // Forward pass: sort each layer by barycenter of predecessor positions
    for l in 1..num_layers {
        let prev_positions: HashMap<&str, f64> = buckets[l - 1]
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i as f64))
            .collect();

        let bary: HashMap<String, f64> = buckets[l]
            .iter()
            .map(|id| {
                let preds = predecessors.get(id.as_str()).cloned().unwrap_or_default();
                let bc = if preds.is_empty() {
                    0.0
                } else {
                    let sum: f64 = preds
                        .iter()
                        .map(|p| prev_positions.get(p).copied().unwrap_or(0.0))
                        .sum();
                    sum / preds.len() as f64
                };
                (id.clone(), bc)
            })
            .collect();

        buckets[l].sort_by(|a, b| {
            bary[a]
                .partial_cmp(&bary[b])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    // Backward pass: sort each layer by barycenter of successor positions
    if num_layers >= 2 {
        for l in (0..num_layers - 1).rev() {
            let next_positions: HashMap<&str, f64> = buckets[l + 1]
                .iter()
                .enumerate()
                .map(|(i, id)| (id.as_str(), i as f64))
                .collect();

            let bary: HashMap<String, f64> = buckets[l]
                .iter()
                .map(|id| {
                    let succs = successors.get(id.as_str()).cloned().unwrap_or_default();
                    let bc = if succs.is_empty() {
                        // Keep current position as tiebreaker
                        buckets[l].iter().position(|x| x == id).unwrap_or(0) as f64
                    } else {
                        let sum: f64 = succs
                            .iter()
                            .map(|s| next_positions.get(s).copied().unwrap_or(0.0))
                            .sum();
                        sum / succs.len() as f64
                    };
                    (id.clone(), bc)
                })
                .collect();

            buckets[l].sort_by(|a, b| {
                bary[a]
                    .partial_cmp(&bary[b])
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    }

    buckets
}

// ---------------------------------------------------------------------------
// Step 3: Position computation
// ---------------------------------------------------------------------------

/// Compute the display width of a node (its box width in characters).
fn node_box_width(graph: &Graph, id: &str) -> usize {
    if let Some(node) = graph.node(id) {
        let label_width = node.label.chars().count();
        let inner = label_width + 4; // 2-char padding each side
        match node.shape {
            // Diamond needs extra width for the diagonal sides
            NodeShape::Diamond => inner + 4,
            NodeShape::Circle => inner + 2,
            _ => inner,
        }
    } else {
        6 // fallback
    }
}

/// Compute the display height of a node (its box height in characters).
fn node_box_height(graph: &Graph, id: &str) -> usize {
    if let Some(node) = graph.node(id) {
        match node.shape {
            NodeShape::Diamond => 5,
            _ => 3,
        }
    } else {
        3
    }
}

/// Convert the ordered layer buckets into character-grid `(col, row)` positions.
fn compute_positions(
    graph: &Graph,
    ordered: &[Vec<String>],
    config: &LayoutConfig,
) -> HashMap<String, GridPos> {
    let mut positions: HashMap<String, GridPos> = HashMap::new();

    match graph.direction {
        Direction::LeftToRight | Direction::RightToLeft => {
            // Layers are columns; nodes within a layer are rows.
            let mut col = 0usize;

            for layer_nodes in ordered {
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

                col += layer_width + config.layer_gap;
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

            for layer_nodes in ordered {
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

                row += layer_height + config.layer_gap;
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
