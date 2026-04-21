//! Sugiyama layout via the [`ascii-dag`] crate.
//!
//! Wraps `ascii_dag::Graph::compute_layout` so we can use its
//! mature crossing-minimisation + Brandes-Köpf coordinate
//! assignment + dummy-node insertion in place of the in-house
//! `layered::layout` for graphs that benefit from it.
//!
//! `ascii-dag` produces top-down coordinates (Y = level depth,
//! X = position within a level). For LR/RL graphs we transpose
//! the IR — swapping per-axis spans — so the rest of our
//! pipeline (renderer, subgraph bounds, edge routing) consumes
//! the same `LayoutResult` shape regardless of layout backend.
//!
//! Per-edge waypoints are extracted from `EdgePath::MultiSegment`
//! (the variant ascii-dag emits for edges spanning multiple
//! layers via dummy nodes) so our long-edge router threads
//! through them the same way it does for our native layered
//! layout.
//!
//! ## Coverage
//!
//! - Nodes with shape-aware widths/heights (we pass our own
//!   `node_box_width` / `node_box_height` via `add_node_with_size`).
//! - Forward edges with optional labels.
//! - Direction LR/RL/TD/BT (LR/RL is the transposed case).
//! - **Subgraphs are NOT yet wired** — first pass focuses on
//!   the no-subgraph case (e.g. README architecture diagram #4).
//!
//! ## Gaps to fill in follow-ups
//!
//! - Subgraph clusters (`add_subgraph` + `put_nodes`).
//! - Parallel-edge groups (ascii-dag collapses them; we'd need
//!   to dedupe + use our existing `parallel_edge_groups` widening).
//! - Edge styles (dashed/thick/etc.) — render-side concern, but
//!   we should keep `edge_index` consistent for downstream lookup.
//! - Direction overrides on nested subgraphs.

use std::collections::HashMap;

use ascii_dag::{EdgePath, Graph as AGraph, LayoutConfig as ALayoutConfig};

use crate::layout::layered::{
    EdgeWaypoints, LayoutConfig, LayoutResult, node_box_height, node_box_width,
};
use crate::types::{Direction, Graph};

/// Compute positions + edge waypoints for `graph` using `ascii-dag`.
///
/// Returns the same [`LayoutResult`] shape as
/// [`crate::layout::layered::layout`], so callers can swap in
/// either backend behind the same interface.
///
/// The grid is mapped from ascii-dag's IR by:
///   1. Building an `ascii_dag::Graph` with our shape-aware
///      `node_box_width` / `node_box_height` per node.
///   2. Calling `compute_layout()` to get the IR.
///   3. For LR/RL, transposing each node's `(x, y)` to `(y, x)`
///      and the same for edge waypoints.
///   4. For RL/BT, mirroring the transposed axis.
///
/// The `LayoutConfig`'s `node_gap` / `layer_gap` are passed
/// through ascii-dag's spacing controls so behaviour matches
/// our native pipeline.
pub fn sugiyama_layout(graph: &Graph, _config: &LayoutConfig) -> LayoutResult {
    if graph.nodes.is_empty() {
        return LayoutResult::default();
    }

    // 1. Map our node IDs (String) to ascii-dag IDs (usize).
    let mut id_to_usize: HashMap<String, usize> = HashMap::with_capacity(graph.nodes.len());
    let mut usize_to_id: HashMap<usize, String> = HashMap::with_capacity(graph.nodes.len());
    for (i, node) in graph.nodes.iter().enumerate() {
        let aid = i + 1; // ascii-dag uses non-zero IDs by convention
        id_to_usize.insert(node.id.clone(), aid);
        usize_to_id.insert(aid, node.id.clone());
    }

    // 2. Build the ascii-dag graph with our shape-aware sizes.
    //    For LR/RL we'll transpose the IR after layout, so we have to
    //    SWAP width/height when feeding ascii-dag — what we call a
    //    node's width (along the LR flow) becomes its height (along
    //    ascii-dag's TB flow), and vice versa. Without this swap the
    //    inter-level spacing comes out perpendicular to what we need.
    let transpose = matches!(
        graph.direction,
        Direction::LeftToRight | Direction::RightToLeft
    );
    let mut adag: AGraph = AGraph::new();
    for node in &graph.nodes {
        let aid = id_to_usize[&node.id];
        let our_w = node_box_width(graph, &node.id);
        let our_h = node_box_height(graph, &node.id);
        let (adag_w, adag_h) = if transpose {
            (our_h, our_w)
        } else {
            (our_w, our_h)
        };
        adag.add_node_with_size(aid, &node.id, adag_w, adag_h);
    }
    for edge in &graph.edges {
        let (Some(&from), Some(&to)) = (
            id_to_usize.get(&edge.from),
            id_to_usize.get(&edge.to),
        ) else {
            continue;
        };
        adag.add_edge(from, to, edge.label.as_deref());
    }

    // 3. Compute the layout. STANDARD preset — fast enough for
    //    interactive use and produces near-optimal crossings on
    //    the diagrams we care about.
    //
    //    Note: ascii-dag's `level_spacing` and `node_spacing` config
    //    fields are vestigial in 0.9.1 (line 157 of heap.rs hardcodes
    //    `+3` regardless). We pass our config values for
    //    forward-compat but apply our own spacing in step 4.5 below.
    let mut cfg = ALayoutConfig::standard();
    cfg.level_spacing = _config.layer_gap;
    cfg.node_spacing = _config.node_gap;
    // We need dummy nodes in the IR to look up the level of each
    // long-edge waypoint when applying per-layer offsets in step 6.
    cfg.include_dummy_nodes = true;
    let ir = adag.compute_layout_with_config(&cfg);

    // 4. Translate IR → our LayoutResult, transposing for LR/RL.
    //    We collect the level-axis coordinate of each node first so
    //    step 4.5 can apply per-layer offsets to widen the inter-
    //    layer gap from ascii-dag's hardcoded 3 cells to our
    //    `_config.layer_gap` (default 6).
    let mut raw_positions: Vec<(String, usize, usize, usize)> =
        Vec::with_capacity(ir.nodes().len()); // (id, col, row, level)
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    for n in ir.nodes() {
        // Skip dummy nodes — they don't correspond to real graph
        // nodes and we don't render them. The waypoint extraction
        // below pulls their coords from the edge path instead.
        if matches!(n.kind, ascii_dag::NodeKind::Dummy) {
            continue;
        }
        let Some(real_id) = usize_to_id.get(&n.id) else { continue };
        let (col, row) = if transpose { (n.y, n.x) } else { (n.x, n.y) };
        raw_positions.push((real_id.clone(), col, row, n.level));
        max_x = max_x.max(col);
        max_y = max_y.max(row);
    }

    // 4.5. Apply per-layer offset along the flow axis to expand
    //      ascii-dag's hardcoded 3-cell inter-layer spacing to our
    //      `_config.layer_gap`. For LR/RL the flow axis is `col`;
    //      for TB/BT it's `row`. Without this, edge-routing chrome
    //      from our renderer collides with the tight gaps and we
    //      see junction-glyph mush around node corners.
    const ASCII_DAG_BASELINE_GAP: usize = 3;
    let extra_per_layer = _config.layer_gap.saturating_sub(ASCII_DAG_BASELINE_GAP);
    let mut positions: HashMap<String, (usize, usize)> =
        HashMap::with_capacity(raw_positions.len());
    for (id, col, row, level) in raw_positions {
        let offset = level * extra_per_layer;
        let (col, row) = match graph.direction {
            Direction::LeftToRight | Direction::RightToLeft => (col + offset, row),
            Direction::TopToBottom | Direction::BottomToTop => (col, row + offset),
        };
        max_x = max_x.max(col);
        max_y = max_y.max(row);
        positions.insert(id, (col, row));
    }

    // 5. Mirror the per-axis range for RL / BT.
    if matches!(graph.direction, Direction::RightToLeft) {
        for (col, _) in positions.values_mut() {
            *col = max_x - *col;
        }
    }
    if matches!(graph.direction, Direction::BottomToTop) {
        for (_, row) in positions.values_mut() {
            *row = max_y - *row;
        }
    }

    // 6. Extract edge waypoints. ascii-dag emits `MultiSegment`
    //    for any edge that crossed dummy nodes on its way; the
    //    `waypoints` list is the chain of dummy positions. We
    //    transpose them the same way as node positions, and apply
    //    the same per-level offset (from step 4.5) so waypoints
    //    line up with the expanded node positions.
    //
    //    Build a (raw_x, raw_y) → level map by scanning every
    //    node in the IR (real + dummy, since `include_dummy_nodes`
    //    is on). Waypoints land at dummy node positions, so we can
    //    look up each waypoint's level by exact-coordinate match
    //    against this map.
    let mut coord_to_level: HashMap<(usize, usize), usize> =
        HashMap::with_capacity(ir.nodes().len());
    for n in ir.nodes() {
        coord_to_level.insert((n.x, n.y), n.level);
    }

    let mut edge_waypoints: Vec<EdgeWaypoints> = Vec::new();
    for (idx, edge) in graph.edges.iter().enumerate() {
        let (Some(&from), Some(&to)) = (
            id_to_usize.get(&edge.from),
            id_to_usize.get(&edge.to),
        ) else {
            continue;
        };
        // Find the corresponding ascii-dag edge.
        let Some(adag_edge) = ir
            .edges()
            .iter()
            .find(|e| e.from_id == from && e.to_id == to)
        else {
            continue;
        };
        if let EdgePath::MultiSegment { waypoints, .. } = &adag_edge.path {
            let mut points: Vec<(usize, usize)> = waypoints
                .iter()
                .map(|&(raw_x, raw_y)| {
                    // Apply the per-layer offset using the dummy node's
                    // level. Waypoints not landing on a known node
                    // coord (rare — interpolated path cells) get no
                    // offset, which keeps them in the right ballpark.
                    let level = coord_to_level.get(&(raw_x, raw_y)).copied().unwrap_or(0);
                    let level_offset = level * extra_per_layer;
                    let (col, row) = if transpose { (raw_y, raw_x) } else { (raw_x, raw_y) };
                    let (col, row) = match graph.direction {
                        Direction::LeftToRight | Direction::RightToLeft => {
                            (col + level_offset, row)
                        }
                        Direction::TopToBottom | Direction::BottomToTop => {
                            (col, row + level_offset)
                        }
                    };
                    (col, row)
                })
                .collect();
            // Mirror axis for RL/BT to match the position transform above.
            if matches!(graph.direction, Direction::RightToLeft) {
                for (col, _) in points.iter_mut() {
                    *col = max_x.saturating_sub(*col);
                }
            }
            if matches!(graph.direction, Direction::BottomToTop) {
                for (_, row) in points.iter_mut() {
                    *row = max_y.saturating_sub(*row);
                }
            }
            if !points.is_empty() {
                edge_waypoints.push(EdgeWaypoints {
                    edge_idx: idx,
                    waypoints: points,
                });
            }
        }
    }

    LayoutResult {
        positions,
        edge_waypoints,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Direction, Edge, Node, NodeShape};

    #[test]
    fn empty_graph_returns_empty() {
        let g = Graph::new(Direction::TopToBottom);
        let out = sugiyama_layout(&g, &LayoutConfig::default());
        assert!(out.positions.is_empty());
        assert!(out.edge_waypoints.is_empty());
    }

    #[test]
    fn simple_chain_lr() {
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
        g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
        g.nodes.push(Node::new("C", "C", NodeShape::Rectangle));
        g.edges.push(Edge::new("A", "B", None));
        g.edges.push(Edge::new("B", "C", None));

        let out = sugiyama_layout(&g, &LayoutConfig::default());
        // LR: A is left of B is left of C.
        assert!(out.positions["A"].0 < out.positions["B"].0);
        assert!(out.positions["B"].0 < out.positions["C"].0);
    }

    #[test]
    fn architecture_case_has_4_distinct_layers() {
        // Mirrors README #04 (the case sugiyama exists to fix):
        //     graph LR
        //     App --> DB[(PostgreSQL)]
        //     App --> Cache[(Redis)]
        //     App --> Queue[(RabbitMQ)]
        //     Queue --> Worker[Worker]
        //     Worker --> DB
        // Native layered layout collapses Worker into the same layer
        // as Cache/RabbitMQ (3 layers, ugly crossings); sugiyama
        // gives the topologically correct 4 layers with the long
        // App→DB edge routed through a dummy.
        let src = "graph LR\n    App --> DB[(PostgreSQL)]\n    App --> Cache[(Redis)]\n    App --> Queue[(RabbitMQ)]\n    Queue --> Worker[Worker]\n    Worker --> DB";
        let g = crate::parser::flowchart::parse(src).unwrap();
        let out = sugiyama_layout(&g, &LayoutConfig::default());

        // 4 distinct layer columns expected (App < Cache=Queue < Worker < DB).
        let app_col = out.positions["App"].0;
        let cache_col = out.positions["Cache"].0;
        let queue_col = out.positions["Queue"].0;
        let worker_col = out.positions["Worker"].0;
        let db_col = out.positions["DB"].0;
        assert!(app_col < cache_col, "App should precede Cache: {app_col} < {cache_col}");
        assert_eq!(cache_col, queue_col, "Cache and Queue share a layer");
        assert!(queue_col < worker_col, "Worker is its own layer");
        assert!(worker_col < db_col, "DB is the rightmost layer");

        // Long edge App→DB must produce waypoints (otherwise it'd
        // route as a single A* call and might cross intermediate
        // nodes, the very thing sugiyama is fixing).
        let app_db_idx = g
            .edges
            .iter()
            .position(|e| e.from == "App" && e.to == "DB")
            .expect("App→DB edge exists");
        let app_db_wp = out
            .edge_waypoints
            .iter()
            .find(|w| w.edge_idx == app_db_idx)
            .expect("App→DB has waypoints");
        assert!(
            app_db_wp.waypoints.len() >= 1,
            "App→DB long edge gets at least one dummy waypoint: {:?}",
            app_db_wp.waypoints,
        );
    }

    #[test]
    fn diamond_no_crossings() {
        // A → B, A → C, B → D, C → D
        let mut g = Graph::new(Direction::TopToBottom);
        for id in ["A", "B", "C", "D"] {
            g.nodes.push(Node::new(id, id, NodeShape::Rectangle));
        }
        g.edges.push(Edge::new("A", "B", None));
        g.edges.push(Edge::new("A", "C", None));
        g.edges.push(Edge::new("B", "D", None));
        g.edges.push(Edge::new("C", "D", None));

        let out = sugiyama_layout(&g, &LayoutConfig::default());
        // TD: A above D; B and C in the middle row.
        assert!(out.positions["A"].1 < out.positions["B"].1);
        assert!(out.positions["A"].1 < out.positions["C"].1);
        assert!(out.positions["B"].1 < out.positions["D"].1);
        assert!(out.positions["C"].1 < out.positions["D"].1);
        assert_eq!(out.positions["B"].1, out.positions["C"].1);
    }
}
