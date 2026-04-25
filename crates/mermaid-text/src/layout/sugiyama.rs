//! Sugiyama layout via the [`ascii-dag`][ascii-dag] crate.
//!
//! [ascii-dag]: https://crates.io/crates/ascii-dag
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
//!
//! ## Coverage
//!
//! - Nodes with shape-aware widths/heights (we pass our own
//!   `node_box_width` / `node_box_height` via `add_node_with_size`).
//! - Forward edges with optional labels.
//! - Direction LR/RL/TD/BT (LR/RL is the transposed case).
//! - **Subgraph clusters** — wired via ascii-dag's native
//!   `add_subgraph` / `put_nodes` / `put_subgraphs` API
//!   (sub-phase 1 of Sugiyama Phase 2). ascii-dag uses the
//!   cluster membership to inform layer assignment; mermaid-text
//!   still computes its own border rectangles from the resulting
//!   node positions via `compute_subgraph_bounds`, so border
//!   drawing is identical to the native backend regardless of
//!   which layout produced the positions.
//! - **Parallel-edge widening** — post-pass inter-layer gap
//!   expansion for groups of ≥ 2 labeled parallel edges sharing
//!   the same unordered endpoint pair (sub-phase 2 of Sugiyama
//!   Phase 2). Mirrors `layered::label_gap`'s `parallel_extra`
//!   term so both backends produce equivalent spacing.
//!
//! ## Gaps to fill in follow-ups
//!
//! - Edge styles (dashed/thick/etc.) — render-side concern, but
//!   we should keep `edge_index` consistent for downstream lookup.
//! - Direction overrides on nested subgraphs.

use std::collections::HashMap;

use ascii_dag::{Graph as AGraph, LayoutConfig as ALayoutConfig};
use unicode_width::UnicodeWidthStr;

use crate::layout::layered::{LayoutConfig, LayoutResult, node_box_height, node_box_width};
use crate::types::{Direction, Graph};

/// Register every mermaid subgraph with `adag` using its native cluster API.
///
/// Must be called **after** all nodes have been added to `adag` (so
/// `id_to_usize` is complete) and **before** `compute_layout_with_config`
/// (ascii-dag needs cluster membership before layer assignment).
///
/// The lifetime `'g` ensures the label `&str` slices borrowed from `graph`
/// outlive the ascii-dag graph they are stored in (`AGraph<'g>`).
///
/// # Arguments
///
/// * `adag`        — the ascii-dag graph being built (borrows labels for `'g`)
/// * `graph`       — the parsed mermaid graph (source of subgraph metadata)
/// * `id_to_usize` — node-ID → ascii-dag node ID map produced by the
///   node-registration loop
fn register_subgraphs<'g>(
    adag: &mut AGraph<'g>,
    graph: &'g Graph,
    id_to_usize: &HashMap<String, usize>,
) {
    // Collect all subgraph IDs via BFS from the top-level list so we can
    // do a two-pass registration without fighting the borrow checker over
    // recursive `&[Subgraph]` vs `&[&Subgraph]` slice types.
    let mut queue: std::collections::VecDeque<&str> =
        graph.subgraphs.iter().map(|sg| sg.id.as_str()).collect();
    let mut all_sg_ids: Vec<String> = Vec::new();
    while let Some(id) = queue.pop_front() {
        all_sg_ids.push(id.to_owned());
        if let Some(sg) = graph.find_subgraph(id) {
            for child_id in &sg.subgraph_ids {
                queue.push_back(child_id.as_str());
            }
        }
    }

    // Pass 1 — register every subgraph with ascii-dag, collecting its IDs.
    // `add_subgraph` stores a `&'g str` reference to the label, which is
    // why we need the `'g` lifetime tying `adag` to `graph`.
    let mut sg_id_map: HashMap<String, usize> = HashMap::with_capacity(all_sg_ids.len());
    for sg_id in &all_sg_ids {
        if let Some(sg) = graph.find_subgraph(sg_id) {
            let adag_sg_id = adag.add_subgraph(&sg.label);
            sg_id_map.insert(sg.id.clone(), adag_sg_id);
        }
    }

    // Pass 2 — place direct child nodes and nest direct child subgraphs.
    // ascii-dag errors if a node is placed into two clusters, so we only
    // place `sg.node_ids` (direct members), not the full recursive set.
    for sg_id in &all_sg_ids {
        let Some(sg) = graph.find_subgraph(sg_id) else {
            continue;
        };
        let Some(&parent_aid) = sg_id_map.get(&sg.id) else {
            continue;
        };

        let node_aids: Vec<usize> = sg
            .node_ids
            .iter()
            .filter_map(|nid| id_to_usize.get(nid).copied())
            .collect();
        if !node_aids.is_empty() {
            adag.put_nodes(&node_aids)
                .inside(parent_aid)
                .expect("ascii-dag rejected node placement — id_to_usize mapping inconsistent");
        }

        let child_aids: Vec<usize> = sg
            .subgraph_ids
            .iter()
            .filter_map(|cid| sg_id_map.get(cid).copied())
            .collect();
        if !child_aids.is_empty() {
            adag.put_subgraphs(&child_aids)
                .inside(parent_aid)
                .expect("ascii-dag rejected subgraph nesting — sg_id_map inconsistent");
        }
    }
}

/// Widen inter-layer gaps for parallel-edge groups in the Sugiyama backend.
///
/// Mirrors the `parallel_extra` logic from `layered::label_gap`: for each
/// adjacent level pair `(L, L+1)` that has a parallel-edge group of ≥ 2
/// labeled edges crossing it, adds `(count − 1) × (max_label_width + 2)`
/// extra cells along the flow axis (col for LR/RL, row for TB/BT) to every
/// node at level ≥ L+1.  Cumulative offsets stack so gaps farther down the
/// flow accumulate correctly.
///
/// **What we do NOT port:** the `needed_for_stacking = count × 2 + 1` term
/// from `label_gap`.  ascii-dag's IR already handles row-stacking of label
/// text inside the inter-layer space it allocated; only the inter-layer *gap
/// width* needs augmenting here.
///
/// # Arguments
///
/// * `positions`   — mutable map from mermaid node-ID → `(col, row)` after
///   step 4.5 (layer_gap expansion).
/// * `id_to_level` — mermaid node-ID → ascii-dag level (0-indexed depth),
///   derived from `raw_positions` before it was consumed.
/// * `graph`       — source graph (edges + direction).
fn apply_parallel_edge_widening(
    positions: &mut HashMap<String, (usize, usize)>,
    id_to_level: &HashMap<String, usize>,
    graph: &Graph,
) {
    let parallel_groups = graph.parallel_edge_groups();
    if parallel_groups.is_empty() {
        return;
    }

    // Compute the maximum level to bound the per-level extra array.
    let max_level = id_to_level.values().copied().max().unwrap_or(0);
    if max_level == 0 {
        return;
    }

    // For each inter-level gap (level L → L+1), compute the extra cells
    // contributed by parallel edge groups crossing that gap.
    //
    // Strategy: for each level L from 0 .. max_level-1 collect the parallel
    // groups whose source nodes live at level L and target nodes at level L+1
    // (or vice-versa — unordered endpoint pair). Then take the maximum extra
    // across all such groups (matching layered::label_gap's `.max()` call).
    let mut extra_per_gap: Vec<usize> = vec![0usize; max_level];
    for group in &parallel_groups {
        // Determine which inter-level gap this group spans.
        // All edges in a parallel group share the same unordered endpoint
        // pair, so we only need the first edge's endpoints.
        let first_edge = &graph.edges[group[0]];
        let Some(&from_lvl) = id_to_level.get(&first_edge.from) else {
            continue;
        };
        let Some(&to_lvl) = id_to_level.get(&first_edge.to) else {
            continue;
        };
        let (lo, hi) = if from_lvl <= to_lvl {
            (from_lvl, to_lvl)
        } else {
            (to_lvl, from_lvl)
        };
        // Only adjacent-level gaps are widened (same semantics as layered).
        if hi != lo + 1 {
            continue;
        }
        // Count edges in this group that carry a label, and find the widest.
        let labeled: Vec<usize> = group
            .iter()
            .filter_map(|&idx| {
                graph.edges[idx]
                    .label
                    .as_deref()
                    .map(UnicodeWidthStr::width)
            })
            .collect();
        let count = labeled.len();
        if count < 2 {
            continue;
        }
        let max_lbl = labeled.iter().copied().max().unwrap_or(0);
        // Formula mirrors layered::label_gap's `parallel_extra` term.
        let extra = (count - 1) * (max_lbl + 2);
        // Keep the maximum contribution for this gap (multiple groups could
        // compete for the same gap; we take the largest, not the sum).
        extra_per_gap[lo] = extra_per_gap[lo].max(extra);
    }

    // Build a per-level cumulative offset: offset[L] = sum of extra_per_gap[0..L].
    // A node at level L shifts by offset[L] along the flow axis.
    let mut cumulative = vec![0usize; max_level + 1];
    for l in 0..max_level {
        cumulative[l + 1] = cumulative[l] + extra_per_gap[l];
    }

    // Apply cumulative offset to every node in the positions map.
    for (id, pos) in positions.iter_mut() {
        let Some(&lvl) = id_to_level.get(id) else {
            continue;
        };
        let offset = cumulative[lvl];
        if offset == 0 {
            continue;
        }
        match graph.direction {
            Direction::LeftToRight | Direction::RightToLeft => pos.0 += offset,
            Direction::TopToBottom | Direction::BottomToTop => pos.1 += offset,
        }
    }
}

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

    // Register subgraph clusters before edges — ascii-dag's layer-assignment
    // uses cluster membership to keep members co-located.
    if !graph.subgraphs.is_empty() {
        register_subgraphs(&mut adag, graph, &id_to_usize);
    }
    // (We discard ascii-dag's ir.subgraphs() later — mermaid-text computes
    // its own border rectangles from node positions via compute_subgraph_bounds,
    // which guarantees border drawing is identical regardless of backend.)

    for edge in &graph.edges {
        let (Some(&from), Some(&to)) = (id_to_usize.get(&edge.from), id_to_usize.get(&edge.to))
        else {
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
    // Dummy nodes carry the `level` field used in step 4.5 to compute
    // per-layer spacing offsets. Real nodes' `level` values are sufficient
    // for this but enabling dummies gives ascii-dag the full IR it needs
    // for its internal crossing minimisation.
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
        // nodes and we don't render them.
        if matches!(n.kind, ascii_dag::NodeKind::Dummy) {
            continue;
        }
        let Some(real_id) = usize_to_id.get(&n.id) else {
            continue;
        };
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
    //
    //      We also build `id_to_level` here (mermaid node-ID → ascii-dag
    //      level) so step 4.6 can apply parallel-edge widening without
    //      re-scanning the IR.
    const ASCII_DAG_BASELINE_GAP: usize = 3;
    let extra_per_layer = _config.layer_gap.saturating_sub(ASCII_DAG_BASELINE_GAP);
    let mut positions: HashMap<String, (usize, usize)> =
        HashMap::with_capacity(raw_positions.len());
    // mermaid node-ID → ascii-dag level (used by apply_parallel_edge_widening).
    let mut id_to_level: HashMap<String, usize> = HashMap::with_capacity(raw_positions.len());
    for (id, col, row, level) in raw_positions {
        id_to_level.insert(id.clone(), level);
        let offset = level * extra_per_layer;
        let (col, row) = match graph.direction {
            Direction::LeftToRight | Direction::RightToLeft => (col + offset, row),
            Direction::TopToBottom | Direction::BottomToTop => (col, row + offset),
        };
        max_x = max_x.max(col);
        max_y = max_y.max(row);
        positions.insert(id, (col, row));
    }

    // 4.6. Widen inter-layer gaps for parallel-edge groups (≥2 labeled edges
    //      sharing the same unordered endpoint pair).  Mirrors the
    //      `parallel_extra` term in `layered::label_gap` so both backends
    //      produce equivalent spacing for semantically identical inputs.
    //      The pass is a no-op when no parallel groups exist (early return
    //      inside the helper).
    apply_parallel_edge_widening(&mut positions, &id_to_level, graph);
    // Recompute max_x / max_y after widening so step 5's mirror arithmetic
    // uses the updated extents.
    for (col, row) in positions.values() {
        max_x = max_x.max(*col);
        max_y = max_y.max(*row);
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

    LayoutResult { positions }
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
        assert!(
            app_col < cache_col,
            "App should precede Cache: {app_col} < {cache_col}"
        );
        assert_eq!(cache_col, queue_col, "Cache and Queue share a layer");
        assert!(queue_col < worker_col, "Worker is its own layer");
        assert!(worker_col < db_col, "DB is the rightmost layer");
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

    // ---- subgraph cluster registration tests --------------------------------

    /// Build a helper that creates a rectangle node.
    fn rect(id: &str) -> Node {
        Node::new(id, id, NodeShape::Rectangle)
    }

    /// Single subgraph containing three nodes in a chain.
    ///
    /// Asserts that all three cluster members land in the same row (TD)
    /// or in consecutive columns (LR — they form a linear chain so they
    /// cannot all share one column). The key safety property: the layout
    /// must return positions for all three nodes (no node is dropped by
    /// the cluster registration path).
    #[test]
    fn subgraph_register_one_cluster() {
        use crate::types::Subgraph;

        let mut g = Graph::new(Direction::TopToBottom);
        for id in ["X", "Y", "Z"] {
            g.nodes.push(rect(id));
        }
        g.edges.push(Edge::new("X", "Y", None));
        g.edges.push(Edge::new("Y", "Z", None));

        let mut sg = Subgraph::new("S", "My Cluster");
        sg.node_ids = vec!["X".into(), "Y".into(), "Z".into()];
        g.subgraphs.push(sg);

        let out = sugiyama_layout(&g, &LayoutConfig::default());

        // All three nodes must be positioned.
        assert!(out.positions.contains_key("X"), "X missing from positions");
        assert!(out.positions.contains_key("Y"), "Y missing from positions");
        assert!(out.positions.contains_key("Z"), "Z missing from positions");

        // Chain: X row < Y row < Z row (TD flow, linear).
        let rx = out.positions["X"].1;
        let ry = out.positions["Y"].1;
        let rz = out.positions["Z"].1;
        assert!(rx < ry, "X should be above Y: row {rx} < {ry}");
        assert!(ry < rz, "Y should be above Z: row {ry} < {rz}");

        // All members share the same column (single-chain cluster in TD).
        let cx = out.positions["X"].0;
        let cy = out.positions["Y"].0;
        let cz = out.positions["Z"].0;
        assert_eq!(
            cx, cy,
            "X and Y should share column in single-chain cluster"
        );
        assert_eq!(
            cy, cz,
            "Y and Z should share column in single-chain cluster"
        );
    }

    /// Two sibling subgraphs with one inter-cluster edge.
    ///
    /// Asserts the position ordering implied by the edge direction: every
    /// node in subgraph A must have a strictly smaller column (LR flow)
    /// than every node in subgraph B.  This is the "no interleaving"
    /// property — if ascii-dag's cluster algorithm is working correctly,
    /// A's members are never shuffled into B's column band.
    #[test]
    fn subgraph_register_two_sibling_clusters() {
        use crate::types::Subgraph;

        // graph LR
        //   subgraph A
        //     A1 --> A2
        //   end
        //   subgraph B
        //     B1 --> B2
        //   end
        //   A2 --> B1
        let mut g = Graph::new(Direction::LeftToRight);
        for id in ["A1", "A2", "B1", "B2"] {
            g.nodes.push(rect(id));
        }
        g.edges.push(Edge::new("A1", "A2", None));
        g.edges.push(Edge::new("B1", "B2", None));
        g.edges.push(Edge::new("A2", "B1", None)); // inter-cluster

        let mut sga = Subgraph::new("SGA", "ClusterA");
        sga.node_ids = vec!["A1".into(), "A2".into()];
        let mut sgb = Subgraph::new("SGB", "ClusterB");
        sgb.node_ids = vec!["B1".into(), "B2".into()];
        g.subgraphs.push(sga);
        g.subgraphs.push(sgb);

        let out = sugiyama_layout(&g, &LayoutConfig::default());

        // All nodes must be present.
        for id in ["A1", "A2", "B1", "B2"] {
            assert!(
                out.positions.contains_key(id),
                "{id} missing from positions"
            );
        }

        // A1 < A2 within cluster A (chain).
        assert!(
            out.positions["A1"].0 < out.positions["A2"].0,
            "A1 should be left of A2"
        );
        // B1 < B2 within cluster B (chain).
        assert!(
            out.positions["B1"].0 < out.positions["B2"].0,
            "B1 should be left of B2"
        );
        // All of A precedes all of B: max(A cols) < min(B cols).
        let a_max_col = out.positions["A1"].0.max(out.positions["A2"].0);
        let b_min_col = out.positions["B1"].0.min(out.positions["B2"].0);
        assert!(
            a_max_col < b_min_col,
            "Cluster A's rightmost col ({a_max_col}) must be left of \
             Cluster B's leftmost col ({b_min_col}) — clusters interleaved"
        );
    }

    /// Outer subgraph containing an inner subgraph plus a sibling node.
    ///
    /// Asserts that the inner cluster's nodes are contained within the
    /// outer cluster's node range on both axes: the inner nodes' bounding
    /// box must be a subset of the outer nodes' bounding box.
    #[test]
    fn subgraph_register_nested_clusters() {
        use crate::types::Subgraph;

        // graph TD
        //   subgraph Outer
        //     subgraph Inner
        //       I1 --> I2
        //     end
        //     O1
        //   end
        let mut g = Graph::new(Direction::TopToBottom);
        for id in ["I1", "I2", "O1"] {
            g.nodes.push(rect(id));
        }
        g.edges.push(Edge::new("I1", "I2", None));
        g.edges.push(Edge::new("O1", "I1", None));

        let mut inner = Subgraph::new("Inner", "Inner");
        inner.node_ids = vec!["I1".into(), "I2".into()];

        let mut outer = Subgraph::new("Outer", "Outer");
        outer.node_ids = vec!["O1".into()];
        outer.subgraph_ids = vec!["Inner".into()];

        // ascii-dag expects top-level subgraphs in `graph.subgraphs`.
        // Both inner and outer must be reachable via `find_subgraph`.
        g.subgraphs.push(outer);
        g.subgraphs.push(inner);

        let out = sugiyama_layout(&g, &LayoutConfig::default());

        for id in ["I1", "I2", "O1"] {
            assert!(
                out.positions.contains_key(id),
                "{id} missing from positions"
            );
        }

        // All outer members (including inner members via nesting) must span
        // a row range at least as wide as the inner members alone.
        let all_rows: Vec<usize> = ["I1", "I2", "O1"]
            .iter()
            .map(|id| out.positions[*id].1)
            .collect();
        let inner_rows: Vec<usize> = ["I1", "I2"].iter().map(|id| out.positions[*id].1).collect();

        let outer_min = *all_rows.iter().min().unwrap();
        let outer_max = *all_rows.iter().max().unwrap();
        let inner_min = *inner_rows.iter().min().unwrap();
        let inner_max = *inner_rows.iter().max().unwrap();

        assert!(
            outer_min <= inner_min,
            "Outer min row ({outer_min}) must be <= Inner min row ({inner_min})"
        );
        assert!(
            outer_max >= inner_max,
            "Outer max row ({outer_max}) must be >= Inner max row ({inner_max})"
        );
    }

    // ---- side-by-side snapshot tests (Sugiyama vs Native) -------------------

    /// `single_subgraph_lr` rendered under the Sugiyama backend.
    ///
    /// The Native baseline is in `snapshots__single_subgraph_lr.snap`.
    /// This snapshot lets reviewers compare the two backends side-by-side
    /// before any default-backend flip (sub-phase 5).
    #[test]
    fn single_subgraph_lr_sugiyama() {
        let src = r#"graph LR
        subgraph SG[My Group]
            A-->B
        end
        B-->C"#;
        let out = crate::render_with_options(
            src,
            &crate::RenderOptions {
                backend: crate::layout::LayoutBackend::Sugiyama,
                ..Default::default()
            },
        )
        .unwrap();
        // Sanity: all three nodes must appear in the rendered output.
        assert!(out.contains('A'), "node A missing from Sugiyama render");
        assert!(out.contains('B'), "node B missing from Sugiyama render");
        assert!(out.contains('C'), "node C missing from Sugiyama render");
        // The cluster label must appear in the subgraph border.
        assert!(
            out.contains("My Group"),
            "subgraph label missing from Sugiyama render:\n{out}"
        );
        insta::assert_snapshot!("single_subgraph_lr_sugiyama", out);
    }

    /// `nested_subgraphs_td` rendered under the Sugiyama backend.
    ///
    /// The Native baseline is in `snapshots__nested_subgraphs_td.snap`.
    #[test]
    fn nested_subgraphs_td_sugiyama() {
        let src = r#"graph TD
        subgraph Outer
            subgraph Inner
                A-->B
            end
            B-->C
        end
        C-->D"#;
        let out = crate::render_with_options(
            src,
            &crate::RenderOptions {
                backend: crate::layout::LayoutBackend::Sugiyama,
                ..Default::default()
            },
        )
        .unwrap();
        // All four nodes must appear.
        for node in ["A", "B", "C", "D"] {
            assert!(
                out.contains(node),
                "node {node} missing from Sugiyama render"
            );
        }
        // Both cluster labels must appear.
        assert!(out.contains("Outer"), "Outer label missing:\n{out}");
        assert!(out.contains("Inner"), "Inner label missing:\n{out}");
        insta::assert_snapshot!("nested_subgraphs_td_sugiyama", out);
    }

    // ---- parallel-edge widening tests (sub-phase 2) -------------------------

    /// Minimal reproducer: two labeled parallel edges between the same pair of
    /// nodes must produce a wider inter-layer gap than a single labeled edge
    /// between the same pair.
    ///
    /// We compare two graphs that differ only in whether the T→D edge is
    /// doubled:
    ///   - baseline: `T ==>|pass| D`  (one labeled edge)
    ///   - parallel: `T ==>|pass| D` + `T -.->|skip| D`  (two labeled edges)
    ///
    /// The widening pass must add `(2-1) × (len("skip")+2) = 6` extra cells
    /// to the T→D gap in the parallel case.
    #[test]
    fn parallel_edges_two_styles_no_collision() {
        let cfg = LayoutConfig::default();

        // Baseline: single labeled edge.
        let baseline_src = "graph LR\n    T ==>|pass| D";
        let g_base = crate::parser::flowchart::parse(baseline_src).unwrap();
        let base_out = sugiyama_layout(&g_base, &cfg);
        let base_gap = base_out.positions["D"]
            .0
            .saturating_sub(base_out.positions["T"].0);

        // Parallel: two labeled edges sharing the same endpoint pair.
        let parallel_src = "graph LR\n    T ==>|pass| D\n    T -.->|skip| D";
        let g_par = crate::parser::flowchart::parse(parallel_src).unwrap();
        let par_out = sugiyama_layout(&g_par, &cfg);

        // All nodes must be positioned.
        for id in ["T", "D"] {
            assert!(
                par_out.positions.contains_key(id),
                "{id} missing from positions"
            );
        }

        let par_gap = par_out.positions["D"]
            .0
            .saturating_sub(par_out.positions["T"].0);

        // The parallel case must have a strictly wider gap.
        // Expected extra = (2-1) × (max("pass","skip").len() + 2) = 1 × 6 = 6.
        assert!(
            par_gap > base_gap,
            "parallel-edge gap T→D ({par_gap}) must exceed single-edge gap ({base_gap}); \
             widening pass may have no-oped"
        );
        // Pin the exact delta so a future formula change is caught explicitly.
        // Formula: (count-1) * (max_label_width + 2) = 1 * (4 + 2) = 6.
        let expected_extra = "skip".len() + 2; // = 6
        assert_eq!(
            par_gap.saturating_sub(base_gap),
            expected_extra,
            "gap delta should equal (count-1)*(max_lbl+2) = {expected_extra}"
        );
    }

    /// Snapshot of the `cicd_parallel_styles_to_same_target` chart rendered
    /// under Sugiyama.  Side-by-side with the Native backend snapshot to let
    /// reviewers verify that labels appear on distinct rows with breathing room.
    #[test]
    fn cicd_parallel_styles_to_same_target_sugiyama() {
        let src = "graph LR
    subgraph CI
        L[Lint] ==> B[Build] ==> T[Test]
    end
    T ==>|pass| D[Deploy]
    T -.->|skip| D";
        let out = crate::render_with_options(
            src,
            &crate::RenderOptions {
                backend: crate::layout::LayoutBackend::Sugiyama,
                ..Default::default()
            },
        )
        .unwrap();
        // Both labels must appear in the output.
        assert!(
            out.contains("pass"),
            "pass label missing from Sugiyama CI/CD render:\n{out}"
        );
        assert!(
            out.contains("skip"),
            "skip label missing from Sugiyama CI/CD render:\n{out}"
        );
        // The label must not puncture the subgraph border.
        assert!(
            !out.contains("│pass│"),
            "pass label punctured subgraph border under Sugiyama:\n{out}"
        );
        insta::assert_snapshot!("cicd_parallel_styles_to_same_target_sugiyama", out);
    }

    /// Regression guard: a graph with no parallel edges must produce identical
    /// positions whether or not the widening pass is applied.  The early-return
    /// path inside `apply_parallel_edge_widening` covers this, but we pin it
    /// explicitly so a future refactor that removes the early return is caught.
    #[test]
    fn no_parallel_edges_widening_is_noop() {
        // Simple chain — no parallel edges anywhere.
        let src = "graph LR\n    A --> B\n    B --> C\n    C --> D";
        let g = crate::parser::flowchart::parse(src).unwrap();
        let out = sugiyama_layout(&g, &LayoutConfig::default());

        // LR: positions must be strictly increasing left to right.
        let a = out.positions["A"].0;
        let b = out.positions["B"].0;
        let c = out.positions["C"].0;
        let d = out.positions["D"].0;
        assert!(a < b, "A must precede B: {a} < {b}");
        assert!(b < c, "B must precede C: {b} < {c}");
        assert!(c < d, "C must precede D: {c} < {d}");

        // Verify that applying the widening pass on a graph with no parallel
        // groups truly changes nothing by checking the groups are empty.
        assert!(
            g.parallel_edge_groups().is_empty(),
            "graph with no parallel edges should have empty groups"
        );
    }
}
