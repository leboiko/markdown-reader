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

use crate::layout::subgraph::{SG_BORDER_PAD, parallel_label_extra};
use crate::types::{BarOrientation, Direction, Graph, NodeShape, Subgraph};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Extra cells reserved between two adjacent same-layer nodes per subgraph
/// boundary that separates them.
///
/// Each boundary takes `SG_BORDER_PAD` cells of padding on each side of the
/// subgraph's border line, plus one cell for the border itself. The gap
/// between two sibling nodes crossing one boundary therefore widens by
/// `SG_BORDER_PAD + 1`; two boundaries (siblings in different subgraphs of
/// the same parent) widens by `2 * (SG_BORDER_PAD + 1)`, and so on.
const SG_GAP_PER_BOUNDARY: usize = SG_BORDER_PAD + 1;

/// Layout spacing constants used by the Sugiyama-inspired layered layout.
///
/// These control the amount of whitespace placed between layers (columns in LR
/// flow, rows in TD flow) and between sibling nodes within the same layer.
/// Reducing them compacts the output; [`Default`] gives a comfortable reading
/// size suitable for most terminals.
///
/// # Examples
///
/// ```
/// use mermaid_text::layout::layered::LayoutConfig;
///
/// let default_cfg = LayoutConfig::default();
/// assert_eq!(default_cfg.layer_gap, 6);
/// assert_eq!(default_cfg.node_gap, 2);
///
/// let compact = LayoutConfig { layer_gap: 2, node_gap: 1 };
/// assert!(compact.layer_gap < default_cfg.layer_gap);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct LayoutConfig {
    /// Minimum gap (in characters) between layers (the axis perpendicular to
    /// the flow direction). The gap accommodates routing corridors and edge
    /// labels; the renderer may widen it automatically when long labels require
    /// more space.
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

/// Per-edge waypoint trail through the intermediate layers between
/// source and target. Used by the renderer to route long edges as a
/// chain of straight segments rather than a single A* pathfinding run
/// that has to detour around intervening real nodes.
///
/// Each waypoint sits at a chosen `(col, row)` cell — the layout pass
/// anchors them on each intermediate layer's spine, interpolated
/// between source and target along the perpendicular axis. The
/// renderer routes A* segments through these cells.
#[derive(Debug, Clone)]
pub struct EdgeWaypoints {
    /// Index into the original `Graph::edges`.
    pub edge_idx: usize,
    /// Ordered intermediate positions, one per dummy node, source-to-target.
    pub waypoints: Vec<GridPos>,
}

/// Output of [`layout`]. Holds real-node positions plus per-edge
/// waypoint trails for any edge that spans more than one layer.
#[derive(Debug, Clone, Default)]
pub struct LayoutResult {
    /// Top-left `(col, row)` of every real node. Excludes dummies.
    pub positions: HashMap<String, GridPos>,
    /// Waypoints for long edges, keyed by edge index. Empty for graphs
    /// where no edge spans more than one layer.
    pub edge_waypoints: Vec<EdgeWaypoints>,
}

/// Compute character-grid positions for every node in `graph`.
///
/// Implements a three-step Sugiyama-inspired layered layout:
/// 1. **Layer assignment** via longest-path from sources.
/// 2. **Within-layer ordering** via iterative barycenter heuristic with
///    best-seen retention.
/// 3. **Position computation** converting `(layer, rank)` pairs to
///    `(col, row)` character-grid coordinates.
///
/// # Arguments
///
/// * `graph`  — the parsed flowchart graph
/// * `config` — spacing parameters (layer gap and node gap)
///
/// # Returns
///
/// A map from node ID to `(col, row)` grid position of the node's top-left
/// corner. The grid origin is `(0, 0)`. Returns an empty map if `graph` has
/// no nodes.
///
/// # Examples
///
/// ```
/// use mermaid_text::{Graph, Node, Edge, Direction, NodeShape};
/// use mermaid_text::layout::layered::{layout, LayoutConfig};
///
/// let mut g = Graph::new(Direction::LeftToRight);
/// g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
/// g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
/// g.edges.push(Edge::new("A", "B", None));
///
/// let positions = layout(&g, &LayoutConfig::default()).positions;
/// // In LR layout, A is to the left of B.
/// assert!(positions["A"].0 < positions["B"].0);
/// ```
pub fn layout(graph: &Graph, config: &LayoutConfig) -> LayoutResult {
    if graph.nodes.is_empty() {
        return LayoutResult::default();
    }

    // 1. Assign layers.
    let layers = assign_layers(graph);

    // 2. Build edge-tuple list once for ordering / crossing-min.
    //    (No dummy augmentation — see Phase A.2 for the planned
    //    barycenter-with-dummies upgrade once Brandes-Köpf coordinate
    //    assignment is in place to keep the rendering compact.)
    let edges: Vec<(String, String)> = graph
        .edges
        .iter()
        .map(|e| (e.from.clone(), e.to.clone()))
        .collect();

    // 3. Group nodes into per-layer lists and sort by barycenter.
    let ordered = order_within_layers(graph, &layers, &edges);

    // 4. Convert to grid coordinates.
    let positions = compute_positions(graph, &ordered, config);

    // 5. Compute long-edge waypoints — one per intermediate layer for
    //    each edge spanning >1 layer. Each waypoint lands at the
    //    source node's row inside the intermediate layer's column,
    //    giving the router a near-straight channel hint without
    //    moving any real-node positions. The router segments through
    //    these waypoints (see `route_via_waypoints` in render/unicode.rs).
    let edge_waypoints = compute_edge_waypoints(graph, &layers, &positions);

    LayoutResult {
        positions,
        edge_waypoints,
    }
}

/// Compute waypoint trails for every edge whose endpoints span more
/// than one layer. Each waypoint sits in the *intermediate* layer's
/// column (LR/RL) or row (TD/BT) at a position picked to clear all
/// real nodes in that layer, biased toward the straight line from
/// source to target.
///
/// Skips back-edges (`layer(to) <= layer(from)`) — those use the
/// dedicated perimeter-routing pass — and self-edges (which never
/// span layers in the first place).
fn compute_edge_waypoints(
    graph: &Graph,
    layers: &HashMap<String, usize>,
    positions: &HashMap<String, GridPos>,
) -> Vec<EdgeWaypoints> {
    // Per-layer "spine" position (col for LR/RL, row for TD/BT) and
    // the perpendicular range (row span for LR/RL, col span for TD/BT)
    // occupied by real nodes in that layer.
    let layer_anchor = layer_axis_anchors(graph, layers, positions);
    let layer_occupied = layer_perpendicular_ranges(graph, layers, positions);

    let mut out: Vec<EdgeWaypoints> = Vec::new();
    for (edge_idx, edge) in graph.edges.iter().enumerate() {
        let (Some(&from_layer), Some(&to_layer)) =
            (layers.get(&edge.from), layers.get(&edge.to))
        else {
            continue;
        };
        if to_layer <= from_layer + 1 {
            continue; // short edge or back-edge — no waypoints
        }
        let (Some(&src), Some(&tgt)) = (positions.get(&edge.from), positions.get(&edge.to))
        else {
            continue;
        };

        let span = to_layer - from_layer;
        let mut waypoints = Vec::with_capacity(span - 1);
        for slot in 0..(span - 1) {
            let intermediate_layer = from_layer + 1 + slot;
            let Some(&anchor) = layer_anchor.get(&intermediate_layer) else {
                continue;
            };
            let occupied = layer_occupied
                .get(&intermediate_layer)
                .map(Vec::as_slice)
                .unwrap_or(&[]);

            // Ideal perpendicular position: linearly interpolate
            // between source and target so the chain forms a
            // straight-ish diagonal. Then snap *off* any real-node
            // row/column range so the waypoint sits in clear space —
            // otherwise the segment-by-segment router would have to
            // pierce through a real node to reach it.
            let frac = (slot + 1) as f64 / span as f64;
            let waypoint = match graph.direction {
                Direction::LeftToRight | Direction::RightToLeft => {
                    let ideal_row = interpolate(src.1, tgt.1, frac);
                    let row = nearest_clear(ideal_row, occupied);
                    (anchor, row)
                }
                Direction::TopToBottom | Direction::BottomToTop => {
                    let ideal_col = interpolate(src.0, tgt.0, frac);
                    let col = nearest_clear(ideal_col, occupied);
                    (col, anchor)
                }
            };
            waypoints.push(waypoint);
        }
        if !waypoints.is_empty() {
            out.push(EdgeWaypoints {
                edge_idx,
                waypoints,
            });
        }
    }
    out
}

/// For each layer, the position along the flow axis (column for LR/RL,
/// row for TD/BT) shared by every node in that layer. This is the
/// "spine" of the layer — long-edge waypoints anchor here.
fn layer_axis_anchors(
    graph: &Graph,
    layers: &HashMap<String, usize>,
    positions: &HashMap<String, GridPos>,
) -> HashMap<usize, usize> {
    let mut out: HashMap<usize, usize> = HashMap::new();
    for (id, &layer) in layers {
        let Some(&pos) = positions.get(id) else {
            continue;
        };
        // Anchor at the *centre* of the box on the flow axis so the
        // waypoint lines up with the lifeline-equivalent rather than
        // the box's left/top edge.
        let (anchor, half_size) = match graph.direction {
            Direction::LeftToRight | Direction::RightToLeft => {
                (pos.0, node_box_width(graph, id) / 2)
            }
            Direction::TopToBottom | Direction::BottomToTop => {
                (pos.1, node_box_height(graph, id) / 2)
            }
        };
        out.entry(layer).or_insert(anchor + half_size);
    }
    out
}

/// For each layer, the inclusive `(start, end)` ranges occupied by
/// real nodes along the perpendicular axis (rows for LR/RL,
/// columns for TD/BT). Waypoints snap *off* these ranges to avoid
/// piercing real boxes.
fn layer_perpendicular_ranges(
    graph: &Graph,
    layers: &HashMap<String, usize>,
    positions: &HashMap<String, GridPos>,
) -> HashMap<usize, Vec<(usize, usize)>> {
    let mut out: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
    for (id, &layer) in layers {
        let Some(&pos) = positions.get(id) else {
            continue;
        };
        let (start, size) = match graph.direction {
            Direction::LeftToRight | Direction::RightToLeft => (pos.1, node_box_height(graph, id)),
            Direction::TopToBottom | Direction::BottomToTop => (pos.0, node_box_width(graph, id)),
        };
        if size == 0 {
            continue;
        }
        out.entry(layer).or_default().push((start, start + size - 1));
    }
    out
}

/// Snap `target` to the nearest position outside every `(start, end)`
/// range in `occupied` (inclusive). If `target` is already clear,
/// returns it unchanged. When `target` overlaps a range we pick the
/// closer of the two boundaries (`start - 1` if it exists, or
/// `end + 1`); ties go DOWN so waypoints prefer the row below
/// intervening boxes — reads naturally and keeps the path away from
/// the canvas top edge. Iterates until clear of every range —
/// bounded by `occupied.len() + 2` passes (each pass either escapes
/// the current range or moves us monotonically further).
fn nearest_clear(target: usize, occupied: &[(usize, usize)]) -> usize {
    let mut current = target;
    let max_passes = occupied.len() + 2;
    for _ in 0..max_passes {
        let mut moved = false;
        for &(start, end) in occupied {
            if current < start || current > end {
                continue;
            }
            // Distance to escape upward (must have room above the
            // range) vs downward. If the range hugs the top edge
            // (`start == 0`), upward isn't an option — push down.
            let up_target = start.checked_sub(1);
            let down_target = end + 1;
            current = match up_target {
                Some(up) if (current - up) < (down_target - current) => up,
                Some(_) | None => down_target,
            };
            moved = true;
        }
        if !moved {
            return current;
        }
    }
    current
}

/// Linear interpolation between two `usize` positions, rounded to
/// the nearest integer cell. `frac` is clamped to `[0.0, 1.0]`.
fn interpolate(a: usize, b: usize, frac: f64) -> usize {
    let frac = frac.clamp(0.0, 1.0);
    let af = a as f64;
    let bf = b as f64;
    (af + (bf - af) * frac).round() as usize
}

// ---------------------------------------------------------------------------
// Orthogonal subgraph helpers
// ---------------------------------------------------------------------------

/// Return `true` if `direction` is perpendicular (orthogonal) to `parent`.
///
/// LR/RL are horizontal; TD/TB/BT are vertical. Two directions are orthogonal
/// when one is horizontal and the other is vertical.
fn is_orthogonal(parent: Direction, child: Direction) -> bool {
    parent.is_horizontal() != child.is_horizontal()
}

/// Walk the subgraph tree depth-first and collect, for every subgraph whose
/// `direction` override is *orthogonal* to `parent_direction`, the set of
/// **direct** node IDs it owns.
///
/// Only the *direct* `node_ids` of a matching subgraph are included; if a
/// perpendicular subgraph itself contains a nested subgraph that is also
/// perpendicular (relative to the outer graph), that inner subgraph is
/// collected separately so the caller can treat each level independently.
///
/// # Note on deeply-nested alternating directions
///
/// TODO: deeply-nested alternating directions (e.g. LR inside TB inside LR)
/// are not fully supported. Each subgraph is evaluated against the top-level
/// graph direction only. Contributions from inner perpendicular subgraphs
/// collapse their own nodes but do not recursively fix the outer collapse.
fn collect_orthogonal_sets<'a>(
    subs: &'a [Subgraph],
    all_subs: &'a [Subgraph],
    parent_direction: Direction,
    out: &mut Vec<Vec<String>>,
) {
    for sg in subs {
        if sg
            .direction
            .is_some_and(|sg_dir| is_orthogonal(parent_direction, sg_dir))
        {
            // This subgraph's direct children should collapse to one layer.
            out.push(sg.node_ids.clone());
        }
        // Recurse into nested subgraphs regardless — a same-direction wrapper
        // might contain a perpendicular inner subgraph.
        let children: Vec<Subgraph> = sg
            .subgraph_ids
            .iter()
            .filter_map(|id| all_subs.iter().find(|s| &s.id == id).cloned())
            .collect();
        collect_orthogonal_sets(&children, all_subs, parent_direction, out);
    }
}

/// Collect all sets of node IDs that belong to orthogonal (perpendicular)
/// subgraphs relative to the graph's own direction.
fn orthogonal_node_sets(graph: &Graph) -> Vec<Vec<String>> {
    let mut result = Vec::new();
    collect_orthogonal_sets(
        &graph.subgraphs,
        &graph.subgraphs,
        graph.direction,
        &mut result,
    );
    result
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

    // --- Orthogonal subgraph collapsing ---
    //
    // For each subgraph whose direction is perpendicular to the parent's flow
    // axis, all direct child nodes should occupy a single parent layer. Pull
    // them to their minimum layer so they form one "band" in the layout, and
    // then re-run longest-path for the remaining (non-orthogonal) nodes so
    // they stay properly sequenced after the collapsed band.
    let ortho_sets = orthogonal_node_sets(graph);
    if !ortho_sets.is_empty() {
        // Build flat set of all orthogonal node IDs for fast membership tests.
        let all_ortho: std::collections::HashSet<&str> = ortho_sets
            .iter()
            .flat_map(|s| s.iter().map(String::as_str))
            .collect();

        // Collapse each set to min layer.
        for set in &ortho_sets {
            let present: Vec<&str> = set
                .iter()
                .map(String::as_str)
                .filter(|id| layer.contains_key(*id))
                .collect();
            if present.is_empty() {
                continue;
            }
            let min_layer = present.iter().map(|id| layer[*id]).min().unwrap_or(0);
            for id in &present {
                layer.insert((*id).to_owned(), min_layer);
            }
        }

        // Re-run longest-path for non-orthogonal nodes only, so that nodes
        // downstream of the collapsed band get their layers updated correctly.
        // Orthogonal nodes keep their (collapsed) layer; only non-ortho nodes
        // are re-propagated.
        let max_iter2 = graph.nodes.len() + 1;
        let mut changed2 = true;
        let mut iter2 = 0;
        while changed2 && iter2 < max_iter2 {
            changed2 = false;
            iter2 += 1;
            for edge in &graph.edges {
                // Skip propagation INTO orthogonal nodes — their layers are fixed.
                if all_ortho.contains(edge.to.as_str()) {
                    continue;
                }
                let from_layer = layer.get(edge.from.as_str()).copied().unwrap_or(0);
                let to_layer = layer.entry(edge.to.clone()).or_insert(0);
                if from_layer + 1 > *to_layer {
                    *to_layer = from_layer + 1;
                    changed2 = true;
                }
            }
        }
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
fn order_within_layers(
    graph: &Graph,
    layers: &HashMap<String, usize>,
    edges: &[(String, String)],
) -> Vec<Vec<String>> {
    // Find max layer
    let max_layer = layers.values().copied().max().unwrap_or(0);
    let num_layers = max_layer + 1;

    // Bucket nodes into layers (preserve declaration order as initial
    // order). Edges are passed in via `edges` so the barycenter pass
    // doesn't have to reach into `graph.edges` directly — keeps the
    // signature open to future augmented-edge schemes.
    let mut buckets: Vec<Vec<String>> = vec![Vec::new(); num_layers];
    for node in &graph.nodes {
        if let Some(&l) = layers.get(&node.id) {
            buckets[l].push(node.id.clone());
        }
    }

    // Build successor/predecessor maps for barycenter computation —
    // from the augmented edge list (long edges replaced by dummy
    // chains), so dummies receive barycenter "pull" from their owning
    // edge's source and target and naturally form a straight column.
    let mut successors: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut predecessors: HashMap<&str, Vec<&str>> = HashMap::new();
    for (from, to) in edges {
        successors
            .entry(from.as_str())
            .or_default()
            .push(to.as_str());
        predecessors
            .entry(to.as_str())
            .or_default()
            .push(from.as_str());
    }

    // Per-node layer lookup for the crossing counter. Borrows from `layers`
    // rather than `buckets` so that it stays live across mutations of the
    // latter during sweep passes.
    let node_layer: HashMap<&str, usize> = layers.iter().map(|(id, &l)| (id.as_str(), l)).collect();

    // Iterative refinement: alternate barycenter and median sweeps,
    // then a transpose local-refinement pass. Pairing barycenter +
    // median escapes local minima either alone would settle into;
    // transpose mops up adjacent-pair improvements neither sweep
    // catches. Best-seen retention guarantees we never ship a worse
    // ordering than what we found mid-loop.
    const MAX_PASSES: usize = 8;
    const NO_IMPROVEMENT_CAP: usize = 4;

    let mut best = buckets.clone();
    let mut best_crossings = count_crossings(edges, &node_layer, &best);
    let mut no_improvement = 0usize;

    // Alternate the metric per outer iteration so consecutive passes
    // sample both heuristics' search trajectories.
    let metrics = [SortMetric::Barycenter, SortMetric::Median];

    for pass in 0..MAX_PASSES {
        let metric = metrics[pass % metrics.len()];
        sort_by_metric(&mut buckets, &predecessors, SweepDirection::Forward, metric);
        sort_by_metric(&mut buckets, &successors, SweepDirection::Backward, metric);
        // Transpose runs after each sweep pair — cheaper than another
        // global sweep and tends to fix the last 1–2 local crossings.
        transpose_pass(&mut buckets, edges, &node_layer);

        let c = count_crossings(edges, &node_layer, &buckets);
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

    // Enforce topological order for nodes belonging to orthogonal subgraphs
    // that were collapsed into the same layer. Without this, barycenter
    // sorting can place them in arbitrary order, which is fine for crossing
    // minimisation but wrong visually when they must flow along the orthogonal
    // axis (e.g. A→B→C left-to-right inside a top-down parent).
    let ortho_sets = orthogonal_node_sets(graph);
    if !ortho_sets.is_empty() {
        for layer_nodes in &mut best {
            for set in &ortho_sets {
                let in_layer: Vec<usize> = layer_nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, id)| set.contains(id))
                    .map(|(i, _)| i)
                    .collect();
                if in_layer.len() <= 1 {
                    continue;
                }
                // Collect node IDs as owned strings to avoid holding a shared
                // borrow of `layer_nodes` while we later mutate it.
                let internal_ids: Vec<String> =
                    in_layer.iter().map(|&i| layer_nodes[i].clone()).collect();

                // Topological sort (Kahn's) of the subgraph's internal edges.
                let internal_set: std::collections::HashSet<&str> =
                    internal_ids.iter().map(String::as_str).collect();
                let mut successors: HashMap<&str, Vec<&str>> =
                    internal_set.iter().map(|&n| (n, Vec::new())).collect();
                let mut in_degree: HashMap<&str, usize> =
                    internal_set.iter().map(|&n| (n, 0usize)).collect();
                for edge in &graph.edges {
                    if internal_set.contains(edge.from.as_str())
                        && internal_set.contains(edge.to.as_str())
                    {
                        successors
                            .entry(edge.from.as_str())
                            .or_default()
                            .push(edge.to.as_str());
                        *in_degree.entry(edge.to.as_str()).or_default() += 1;
                    }
                }
                let mut queue: std::collections::VecDeque<&str> = in_degree
                    .iter()
                    .filter(|(_, d)| **d == 0)
                    .map(|(&n, _)| n)
                    .collect();
                let mut topo: Vec<String> = Vec::new();
                while let Some(node) = queue.pop_front() {
                    topo.push(node.to_owned());
                    // Clone successor list to avoid borrow conflicts while we
                    // mutate `in_degree` in the same loop body.
                    let succs: Vec<&str> = successors.get(node).cloned().unwrap_or_default();
                    for succ in succs {
                        let d = in_degree.entry(succ).or_default();
                        *d = d.saturating_sub(1);
                        if *d == 0 {
                            queue.push_back(succ);
                        }
                    }
                }
                // Write topo order back into the positions these nodes held in
                // the layer. If Kahn's didn't complete (cycle), fall back to
                // the existing order to avoid producing wrong output silently.
                if topo.len() == in_layer.len() {
                    for (slot, &pos) in in_layer.iter().enumerate() {
                        layer_nodes[pos] = topo[slot].clone();
                    }
                }
            }
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

/// Sort metric used by [`sort_by_metric`] to pick each node's position
/// from its neighbour positions.
///
/// **Barycenter**: arithmetic mean. Smooth, fast, but skewed by
/// outliers (one far-away neighbour can drag the position).
///
/// **Median**: middle value of sorted neighbours (or average of the
/// two middle values for even counts). More robust to outliers; in
/// practice often beats barycenter on crossing count, especially on
/// dense graphs where some nodes have many far-flung neighbours.
///
/// We run both passes alternately in [`order_within_layers`] and keep
/// the best-seen ordering — pairing them tends to escape local minima
/// that either metric alone would settle into.
#[derive(Copy, Clone)]
enum SortMetric {
    Barycenter,
    Median,
}

/// Sort each layer in `buckets` by `metric` applied to each node's
/// neighbours in the adjacent layer (predecessors for `Forward`,
/// successors for `Backward`).
///
/// Nodes without neighbours keep their current position via a stable
/// sort — this prevents the heuristic from shuffling isolated nodes
/// to position 0.
fn sort_by_metric(
    buckets: &mut [Vec<String>],
    neighbors: &HashMap<&str, Vec<&str>>,
    dir: SweepDirection,
    metric: SortMetric,
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

        let mut keyed: Vec<(String, f64)> = buckets[l]
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let mut positions: Vec<f64> = neighbors
                    .get(id.as_str())
                    .map(|ns| {
                        ns.iter()
                            .map(|n| ref_positions.get(n).copied().unwrap_or(i as f64))
                            .collect()
                    })
                    .unwrap_or_default();
                let key = if positions.is_empty() {
                    // Fallback to current position so isolated nodes
                    // don't drift to 0.
                    i as f64
                } else {
                    match metric {
                        SortMetric::Barycenter => {
                            let sum: f64 = positions.iter().sum();
                            sum / positions.len() as f64
                        }
                        SortMetric::Median => median_of_sorted({
                            positions.sort_by(|a, b| {
                                a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                            });
                            &positions
                        }),
                    }
                };
                (id.clone(), key)
            })
            .collect();

        keyed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        buckets[l] = keyed.into_iter().map(|(id, _)| id).collect();
    }
}

/// Median of a slice that's already sorted in ascending order. Returns
/// `0.0` for an empty slice (the caller filters that case out before
/// calling). For even-length slices, averages the two middle values
/// per the standard definition.
fn median_of_sorted(sorted: &[f64]) -> f64 {
    debug_assert!(!sorted.is_empty(), "median of empty slice is undefined");
    let n = sorted.len();
    if n % 2 == 0 {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    } else {
        sorted[n / 2]
    }
}

/// Local-refinement pass: for each pair of adjacent nodes within each
/// layer, try swapping them; keep the swap if it strictly reduces the
/// total crossing count. Repeats per-layer until no swap improves.
///
/// This catches local minima that the global barycenter/median sweeps
/// settle into — e.g. two nodes whose individual barycenters tie but
/// where one ordering produces fewer crossings than the other.
///
/// Returns `true` if any swap was kept (lets the outer loop know
/// progress was made).
fn transpose_pass(
    buckets: &mut [Vec<String>],
    edges: &[(String, String)],
    node_layer: &HashMap<&str, usize>,
) -> bool {
    let mut any_improved = false;
    let mut current_crossings = count_crossings(edges, node_layer, buckets);

    let mut improved_this_pass = true;
    let mut passes_remaining = 4usize; // bound — typically converges in 1–2
    while improved_this_pass && passes_remaining > 0 {
        improved_this_pass = false;
        passes_remaining -= 1;

        for layer_idx in 0..buckets.len() {
            let layer_len = buckets[layer_idx].len();
            if layer_len < 2 {
                continue;
            }
            for i in 0..(layer_len - 1) {
                buckets[layer_idx].swap(i, i + 1);
                let after = count_crossings(edges, node_layer, buckets);
                if after < current_crossings {
                    current_crossings = after;
                    any_improved = true;
                    improved_this_pass = true;
                } else {
                    // Revert the swap.
                    buckets[layer_idx].swap(i, i + 1);
                }
            }
        }
    }
    any_improved
}

/// Count the number of edge crossings implied by the given layer ordering.
///
/// For each pair of edges `(u1, v1)` and `(u2, v2)` that both span the same
/// layer gap (`u.layer → v.layer`), they cross iff the relative positions
/// of `u1,u2` in their layer differ from the relative positions of `v1,v2`.
/// This is the classic inversion test; O(E²) per gap, which is fine for the
/// small graphs this crate targets.
fn count_crossings(
    edges: &[(String, String)],
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
    let edges_with_gaps: Vec<(usize, usize, usize, usize)> = edges
        .iter()
        .filter_map(|(from, to)| {
            let fl = *node_layer.get(from.as_str())?;
            let tl = *node_layer.get(to.as_str())?;
            let fr = *rank.get(from.as_str())?;
            let tr = *rank.get(to.as_str())?;
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
        // Multi-line labels are sized by the widest line — line breaks make
        // the box taller, not wider.
        let label_width = node.label_width();
        let inner = label_width + 4; // 2-char padding each side
        match node.shape {
            // Diamond renders as a plain rectangle.
            NodeShape::Diamond => inner,
            // Circle/Stadium/Hexagon/Asymmetric add 1 extra char on each side
            // for their distinctive markers inside the border.
            NodeShape::Circle | NodeShape::Stadium | NodeShape::Hexagon | NodeShape::Asymmetric => {
                inner + 2
            }
            // Subroutine adds 1 extra char on each side for inner vertical bars.
            NodeShape::Subroutine => inner + 2,
            // Cylinder: standard width — arcs are drawn at top/bottom centre.
            NodeShape::Cylinder => inner,
            // Parallelogram / Trapezoid: add 2 extra chars for slant markers.
            NodeShape::Parallelogram | NodeShape::Trapezoid => inner + 2,
            // DoubleCircle: needs 4 extra chars for the concentric inner border.
            NodeShape::DoubleCircle => inner + 4,
            // Plain shapes (and notes — same width as Rounded).
            NodeShape::Rectangle | NodeShape::Rounded | NodeShape::Note => inner,
            // Fork/join bar: perpendicular to flow. Horizontal bars
            // span 5 cells; vertical bars are a single column.
            NodeShape::Bar(BarOrientation::Horizontal) => 5,
            NodeShape::Bar(BarOrientation::Vertical) => 1,
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
        // Each additional label line adds one interior row to the box.
        let extra = node.label_line_count().saturating_sub(1);
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
            | NodeShape::Subroutine
            | NodeShape::Note => 3 + extra,
            // Cylinder needs 4 rows: top border, lid line, text, bottom border.
            NodeShape::Cylinder => 4 + extra,
            // DoubleCircle needs 5 rows for the concentric inner border.
            NodeShape::DoubleCircle => 5 + extra,
            // Fork/join bar: perpendicular to flow. Vertical bars span
            // 5 rows so multiple parallel branches can attach; horizontal
            // bars are a single row. No label, so no extra rows.
            NodeShape::Bar(BarOrientation::Vertical) => 5,
            NodeShape::Bar(BarOrientation::Horizontal) => 1,
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
    parallel_groups: &[Vec<usize>],
) -> usize {
    // Collect widths of all labels on edges that cross this gap, and
    // remember the edge index alongside so we can match against
    // parallel groups.
    let crossings: Vec<(usize, usize)> = graph // (edge_idx, label_width)
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            let fl = node_layer.get(e.from.as_str()).copied().unwrap_or(0);
            let tl = node_layer.get(e.to.as_str()).copied().unwrap_or(0);
            // Edge crosses the gap in either direction.
            (fl == layer_a && tl == layer_b) || (fl == layer_b && tl == layer_a)
        })
        .filter_map(|(i, e)| {
            e.label.as_deref().map(|l| (i, UnicodeWidthStr::width(l)))
        })
        .collect();

    if crossings.is_empty() {
        return default_gap;
    }

    let max_lbl = crossings.iter().map(|(_, w)| *w).max().unwrap_or(0);
    let needed_for_width = max_lbl + 2;

    // If multiple labels compete for vertical space in the same gap,
    // each occupies 2 rows (label + spacer). Keep that many rows free.
    let mut widths: Vec<usize> = crossings.iter().map(|(_, w)| *w).collect();
    widths.sort_unstable();
    let count = widths.len();
    let needed_for_stacking = count * 2 + 1;

    // Parallel-edge breathing room (Phase 2a of the layout-pass
    // widening work — see `docs/scope-parallel-edges.md`). When the
    // edges crossing this gap include a parallel-edge group of
    // `count >= 2`, the labels would otherwise sit flush against
    // each adjacent box / subgraph border (CI/CD `│pass┌`,
    // Supervisor `└─creates│`). Add extra gap so each label has at
    // least 1 cell of clearance on each side.
    let parallel_extra = parallel_groups
        .iter()
        .filter_map(|group| {
            // How many edges of this parallel group cross this gap?
            let count_in_gap: usize = group
                .iter()
                .filter(|&&edge_idx| crossings.iter().any(|(i, _)| *i == edge_idx))
                .count();
            if count_in_gap < 2 {
                return None;
            }
            // Each additional parallel label past the first needs
            // `max_lbl + 2` cells of its own breathing room.
            Some((count_in_gap - 1) * (max_lbl + 2))
        })
        .max()
        .unwrap_or(0);

    default_gap
        .max(needed_for_width + parallel_extra)
        .max(needed_for_stacking)
}

/// Build the subgraph parent map: child subgraph id → parent subgraph id.
///
/// Subgraphs without a parent entry are top-level. Built once per layout
/// run and used to walk a node's full ancestor chain.
fn build_subgraph_parent_map(graph: &Graph) -> HashMap<&str, &str> {
    let mut m = HashMap::new();
    for parent in &graph.subgraphs {
        for child_id in &parent.subgraph_ids {
            m.insert(child_id.as_str(), parent.id.as_str());
        }
    }
    m
}

/// Return `node_id`'s subgraph ancestor chain, innermost first.
///
/// An empty vector means the node is not inside any subgraph. The chain
/// starts at the node's immediately-enclosing subgraph and walks outward via
/// `parent_map` until it reaches a top-level subgraph.
fn node_subgraph_chain<'a>(
    node_id: &str,
    node_to_sg: &'a HashMap<String, String>,
    parent_map: &'a HashMap<&'a str, &'a str>,
) -> Vec<&'a str> {
    let mut chain = Vec::new();
    let Some(sg_id) = node_to_sg.get(node_id) else {
        return chain;
    };
    let mut cur: &str = sg_id.as_str();
    chain.push(cur);
    while let Some(&parent) = parent_map.get(cur) {
        chain.push(parent);
        cur = parent;
    }
    chain
}

/// Count subgraph borders that must sit between two adjacent same-layer nodes.
///
/// Chains are innermost-first (as returned by [`node_subgraph_chain`]); the
/// common tail is the set of subgraphs that enclose both nodes and therefore
/// do not contribute a boundary between them. The remaining entries in each
/// chain each add one boundary.
///
/// Examples:
/// - `[X]` vs `[X]` → 0 (same subgraph)
/// - `[X]` vs `[]` → 1 (leaving X)
/// - `[X]` vs `[Y]` → 2 (leaving X, entering Y)
/// - `[X, Z]` vs `[Y, Z]` → 2 (leaving X inside Z, entering Y inside Z)
/// - `[X, Z]` vs `[Z]` → 1 (leaving X, Z still encloses both)
fn subgraph_boundary_count(chain_a: &[&str], chain_b: &[&str]) -> usize {
    let a_len = chain_a.len();
    let b_len = chain_b.len();
    let mut shared = 0usize;
    for i in 1..=a_len.min(b_len) {
        if chain_a[a_len - i] == chain_b[b_len - i] {
            shared += 1;
        } else {
            break;
        }
    }
    (a_len - shared) + (b_len - shared)
}

/// Return the minimum gap (in cells) that must sit between two adjacent
/// same-layer nodes given their subgraph memberships.
///
/// For nodes in the same immediate subgraph (or both outside any subgraph),
/// the base `node_gap` is returned. For nodes separated by subgraph
/// boundaries, `SG_GAP_PER_BOUNDARY` cells are added per boundary so that
/// each subgraph's border line and its `SG_BORDER_PAD` of padding on each
/// side all fit without overlapping a neighboring node or sibling subgraph.
fn sibling_gap(
    node_a: &str,
    node_b: &str,
    node_to_sg: &HashMap<String, String>,
    parent_map: &HashMap<&str, &str>,
    base_gap: usize,
) -> usize {
    let chain_a = node_subgraph_chain(node_a, node_to_sg, parent_map);
    let chain_b = node_subgraph_chain(node_b, node_to_sg, parent_map);
    let boundaries = subgraph_boundary_count(&chain_a, &chain_b);
    base_gap + boundaries * SG_GAP_PER_BOUNDARY
}

/// Extra columns to add to a layer's width when one or more of its
/// nodes lives in a subgraph that contains parallel-edge labels.
/// Mirrors the bounds-side calculation so the border wraps cleanly
/// around the labels and external nodes get pushed out by the same
/// amount, avoiding collisions.
fn layer_parallel_label_extra_width(
    graph: &Graph,
    layer_nodes: &[String],
    node_to_sg: &HashMap<String, String>,
) -> usize {
    layer_parallel_label_extra(graph, layer_nodes, node_to_sg, /* axis_w = */ true)
}

fn layer_parallel_label_extra_height(
    graph: &Graph,
    layer_nodes: &[String],
    node_to_sg: &HashMap<String, String>,
) -> usize {
    layer_parallel_label_extra(graph, layer_nodes, node_to_sg, /* axis_w = */ false)
}

/// Take the max parallel-edge-label extra (per `parallel_label_extra`)
/// across the subgraphs that own any of `layer_nodes`. `axis_w` picks
/// the width-axis (`true`) or height-axis (`false`) component of the
/// returned `(extra_w, extra_h)` tuple.
fn layer_parallel_label_extra(
    graph: &Graph,
    layer_nodes: &[String],
    node_to_sg: &HashMap<String, String>,
    axis_w: bool,
) -> usize {
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut max_extra: usize = 0;
    for nid in layer_nodes {
        let Some(sg_id) = node_to_sg.get(nid) else { continue };
        if !seen.insert(sg_id.as_str()) {
            continue;
        }
        let Some(sg) = graph.find_subgraph(sg_id) else { continue };
        let (w, h) = parallel_label_extra(graph, sg);
        let extra = if axis_w { w } else { h };
        max_extra = max_extra.max(extra);
    }
    max_extra
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

    // Pre-compute parallel-edge groups so `label_gap` can widen the
    // inter-layer gap when a parallel group's labels would otherwise
    // sit flush against neighbouring boxes / subgraph borders.
    let parallel_groups = graph.parallel_edge_groups();

    // Subgraph membership lookups — used to widen the gap between two
    // adjacent same-layer nodes when a subgraph boundary sits between them.
    let node_to_sg = graph.node_to_subgraph();
    let sg_parent = build_subgraph_parent_map(graph);

    match graph.direction {
        Direction::LeftToRight | Direction::RightToLeft => {
            // Layers are columns; nodes within a layer are rows.
            let mut col = 0usize;

            for (layer_idx, layer_nodes) in ordered.iter().enumerate() {
                if layer_nodes.is_empty() {
                    continue;
                }

                // Column width = widest node in this layer, plus any
                // extra room a containing subgraph needs for its
                // parallel-edge labels (TB/BT subgraph stacks members
                // vertically — labels run horizontally between them
                // and steal column width).
                let base_layer_width = layer_nodes
                    .iter()
                    .map(|id| node_box_width(graph, id))
                    .max()
                    .unwrap_or(6);
                let extra_w = layer_parallel_label_extra_width(
                    graph,
                    layer_nodes,
                    &node_to_sg,
                );
                let layer_width = base_layer_width + extra_w;

                let mut row = 0usize;
                let mut prev: Option<&str> = None;
                for id in layer_nodes {
                    let h = node_box_height(graph, id);
                    // Widen the gap between this node and the previous one
                    // if a subgraph boundary sits between them. The leading
                    // gap for the first node in the layer is always 0 — the
                    // initial subgraph padding is applied globally by
                    // `offset_positions_for_subgraphs` in lib.rs.
                    if let Some(prev_id) = prev {
                        let gap =
                            sibling_gap(prev_id, id, &node_to_sg, &sg_parent, config.node_gap);
                        // `gap` replaces the node_gap that was added at the
                        // end of the previous iteration. Subtract the already-
                        // applied node_gap to avoid double-counting.
                        row += gap.saturating_sub(config.node_gap);
                    }
                    positions.insert(id.clone(), (col, row));
                    row += h + config.node_gap;
                    prev = Some(id.as_str());
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
                        &parallel_groups,
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

                // Row height = tallest node in this layer, plus any
                // extra room a containing subgraph needs for its
                // parallel-edge labels (LR/RL subgraph stacks members
                // horizontally — labels run vertically between them
                // and steal row height).
                let base_layer_height = layer_nodes
                    .iter()
                    .map(|id| node_box_height(graph, id))
                    .max()
                    .unwrap_or(3);
                let extra_h = layer_parallel_label_extra_height(
                    graph,
                    layer_nodes,
                    &node_to_sg,
                );
                let layer_height = base_layer_height + extra_h;

                let mut col = 0usize;
                let mut prev: Option<&str> = None;
                for id in layer_nodes {
                    let w = node_box_width(graph, id);
                    if let Some(prev_id) = prev {
                        let gap =
                            sibling_gap(prev_id, id, &node_to_sg, &sg_parent, config.node_gap);
                        col += gap.saturating_sub(config.node_gap);
                    }
                    positions.insert(id.clone(), (col, row));
                    col += w + config.node_gap;
                    prev = Some(id.as_str());
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
                        &parallel_groups,
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
        let pos = layout(&g, &cfg).positions;
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
        let pos = layout(&g, &cfg).positions;
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
        let pos = layout(&g, &cfg).positions;
        assert_eq!(pos.len(), 2);
    }

    #[test]
    fn single_node_layout() {
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "Alone", NodeShape::Rectangle));

        let cfg = LayoutConfig::default();
        let pos = layout(&g, &cfg).positions;
        assert_eq!(pos["A"], (0, 0));
    }

    // ---- Long-edge waypoints (Phase A.1) -------------------------------

    #[test]
    fn short_edges_get_no_waypoints() {
        // A -> B (1-layer span) needs no waypoints — the direct router
        // path is already straight.
        let g = simple_lr_graph();
        let result = layout(&g, &LayoutConfig::default());
        assert!(result.edge_waypoints.is_empty());
    }

    #[test]
    fn long_edge_gets_waypoint_per_intermediate_layer() {
        // A -> B -> C -> D plus A -> D (a long edge spanning 3 layers).
        // Should produce 2 waypoints for the long edge — one in B's
        // layer, one in C's layer — anchored to those layer columns.
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
        g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
        g.nodes.push(Node::new("C", "C", NodeShape::Rectangle));
        g.nodes.push(Node::new("D", "D", NodeShape::Rectangle));
        g.edges.push(Edge::new("A", "B", None));
        g.edges.push(Edge::new("B", "C", None));
        g.edges.push(Edge::new("C", "D", None));
        g.edges.push(Edge::new("A", "D", None));

        let result = layout(&g, &LayoutConfig::default());
        // The long A→D edge is index 3; the three short edges produce
        // no waypoints (single-layer span).
        assert_eq!(result.edge_waypoints.len(), 1);
        let w = &result.edge_waypoints[0];
        assert_eq!(w.edge_idx, 3);
        assert_eq!(w.waypoints.len(), 2, "two intermediate layers (B and C)");
        // Both waypoints sit in the same row range, since A and D are
        // in the same row (single-layer chain). The waypoint row must
        // not pierce any real-node row range — `nearest_clear` snaps
        // it off the box.
        let pos_b = result.positions["B"];
        let h_b = node_box_height(&g, "B");
        let row_inside_b = (pos_b.1)..(pos_b.1 + h_b);
        assert!(
            !row_inside_b.contains(&w.waypoints[0].1),
            "waypoint row {} should not be inside B's row range {row_inside_b:?}",
            w.waypoints[0].1
        );
    }

    #[test]
    fn back_edges_get_no_waypoints() {
        // A -> B -> C plus C -> A. The back-edge has layer(C) > layer(A)
        // so technically spans more layers, but we filter out any edge
        // where to-layer <= from-layer + 1 in the FORWARD direction.
        // Here C is at layer 2, A is at layer 0 — to_layer (0) is NOT
        // > from_layer (2) + 1, so the filter catches it.
        let mut g = Graph::new(Direction::LeftToRight);
        g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
        g.nodes.push(Node::new("B", "B", NodeShape::Rectangle));
        g.nodes.push(Node::new("C", "C", NodeShape::Rectangle));
        g.edges.push(Edge::new("A", "B", None));
        g.edges.push(Edge::new("B", "C", None));
        g.edges.push(Edge::new("C", "A", None));

        let result = layout(&g, &LayoutConfig::default());
        // No long forward edges → no waypoints (back-edge handled by
        // the perimeter routing pass).
        assert!(result.edge_waypoints.is_empty());
    }

    #[test]
    fn nearest_clear_no_overlap_returns_target() {
        assert_eq!(nearest_clear(5, &[(10, 12)]), 5);
        assert_eq!(nearest_clear(20, &[(10, 12)]), 20);
        assert_eq!(nearest_clear(5, &[]), 5);
    }

    #[test]
    fn nearest_clear_snaps_off_overlap_to_closer_boundary() {
        // Target 11 in (10, 12): up dist 2, down dist 2 → tie goes
        // down (`13`) by design (see helper docs).
        assert_eq!(nearest_clear(11, &[(10, 12)]), 13);
        // Target 12 at the upper edge — clearly closer to 13 (down).
        assert_eq!(nearest_clear(12, &[(10, 12)]), 13);
        // Target 10 at the lower edge — clearly closer to 9 (up).
        assert_eq!(nearest_clear(10, &[(10, 12)]), 9);
    }

    #[test]
    fn nearest_clear_top_edge_pushes_down() {
        // Range starts at 0 → no room above. Must push down.
        assert_eq!(nearest_clear(1, &[(0, 4)]), 5);
    }

    #[test]
    fn nearest_clear_handles_consecutive_ranges() {
        // Two adjacent ranges: target snaps into the gap.
        assert_eq!(nearest_clear(1, &[(0, 2), (4, 6)]), 3);
    }

    // ---- Median + transpose crossing-min passes (Phase A.3) ------------

    #[test]
    fn median_of_sorted_picks_middle() {
        assert_eq!(median_of_sorted(&[1.0, 2.0, 3.0]), 2.0);
        assert_eq!(median_of_sorted(&[5.0]), 5.0);
    }

    #[test]
    fn median_of_sorted_averages_two_middle_for_even_length() {
        assert_eq!(median_of_sorted(&[1.0, 2.0, 3.0, 4.0]), 2.5);
        assert_eq!(median_of_sorted(&[1.0, 1.0, 5.0, 5.0]), 3.0);
    }

    #[test]
    fn median_resists_outliers_better_than_barycenter() {
        // Demonstrates the algorithmic difference: a single far-out
        // neighbour shifts barycenter dramatically but doesn't move
        // the median. This is the property median exploits to escape
        // crossings barycenter can't.
        let xs = [0.0, 1.0, 2.0, 100.0]; // one wild outlier
        let median = median_of_sorted(&xs);
        let barycenter: f64 = xs.iter().sum::<f64>() / xs.len() as f64;
        assert!((median - 1.5).abs() < 0.01); // tight on the cluster
        assert!(barycenter > 25.0); // dragged way out by the outlier
    }

    #[test]
    fn transpose_swaps_when_it_reduces_crossings() {
        // Construct a deliberate crossing: edges A→C and B→D with
        // layer 0 = [A, B] and layer 1 = [D, C]. EITHER swapping
        // layer 0 to [B, A] OR layer 1 to [C, D] eliminates the
        // crossing — verify by outcome (zero crossings), not by
        // which specific swap won.
        let mut buckets = vec![
            vec!["A".to_string(), "B".to_string()],
            vec!["D".to_string(), "C".to_string()],
        ];
        let edges = vec![
            ("A".to_string(), "C".to_string()),
            ("B".to_string(), "D".to_string()),
        ];
        let mut node_layer: HashMap<&str, usize> = HashMap::new();
        node_layer.insert("A", 0);
        node_layer.insert("B", 0);
        node_layer.insert("C", 1);
        node_layer.insert("D", 1);

        let before = count_crossings(&edges, &node_layer, &buckets);
        assert_eq!(before, 1, "scenario should start with 1 crossing");

        let improved = transpose_pass(&mut buckets, &edges, &node_layer);
        let after = count_crossings(&edges, &node_layer, &buckets);

        assert!(improved, "transpose should report improvement");
        assert_eq!(after, 0, "crossing should be eliminated by the swap");
    }

    #[test]
    fn transpose_leaves_already_optimal_orderings_alone() {
        // [A, B] → [C, D] with edges A→C, B→D has no crossings.
        // Transpose should not swap.
        let mut buckets = vec![
            vec!["A".to_string(), "B".to_string()],
            vec!["C".to_string(), "D".to_string()],
        ];
        let edges = vec![
            ("A".to_string(), "C".to_string()),
            ("B".to_string(), "D".to_string()),
        ];
        let mut node_layer: HashMap<&str, usize> = HashMap::new();
        node_layer.insert("A", 0);
        node_layer.insert("B", 0);
        node_layer.insert("C", 1);
        node_layer.insert("D", 1);

        let improved = transpose_pass(&mut buckets, &edges, &node_layer);
        assert!(!improved, "no swap should be reported when already optimal");
        assert_eq!(buckets[1], vec!["C".to_string(), "D".to_string()]);
    }
}
