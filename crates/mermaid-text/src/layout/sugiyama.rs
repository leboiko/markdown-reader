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
//! - **Direction overrides on nested subgraphs** — `subgraph X; direction TB`
//!   inside `graph LR` (the Supervisor pattern). Sub-phase 3 of Sugiyama
//!   Phase 2. Two-step approach:
//!   1. **Pre-pass**: intra-orthogonal-set edges are hidden from ascii-dag
//!      so it collapses the override-subgraph members into one parent layer.
//!   2. **Post-pass**: walk override subgraphs DFS post-order, reassigning
//!      member positions in topological order along the override axis.
//!
//! ## Gaps to fill in follow-ups
//!
//! - Edge styles (dashed/thick/etc.) — render-side concern, but
//!   we should keep `edge_index` consistent for downstream lookup.
//! - Parallel-edge widening for groups whose both endpoints are inside an
//!   orthogonal-override subgraph applies along the parent axis (wrong for
//!   the override). Accepted v1 limitation; see `apply_direction_overrides`
//!   doc for the tradeoff note.

use std::collections::{HashMap, HashSet};

use ascii_dag::{Graph as AGraph, LayoutConfig as ALayoutConfig};
use unicode_width::UnicodeWidthStr;

use crate::layout::layered::{LayoutConfig, LayoutResult, node_box_height, node_box_width};
use crate::layout::subgraph::parallel_label_extra;
use crate::types::{Direction, Graph, Subgraph};

// ---------------------------------------------------------------------------
// Direction-override helpers (sub-phase 3)
// ---------------------------------------------------------------------------

/// Collect the set of node-ID pairs that are both members of the same
/// orthogonal-override subgraph and connected by an intra-subgraph edge.
///
/// We drop these edges before handing to ascii-dag so it places all members
/// of the override subgraph in a single parent layer — no intra-group ordering
/// signal means ascii-dag treats them as siblings in the same level band.
/// The edges themselves live in `graph.edges` and are never removed; the A*
/// router in `lib.rs` re-routes them from the final node positions automatically.
///
/// Pairs are stored as `(min_id, max_id)` (canonical form) so a single
/// `HashSet` lookup handles both edge directions.
fn intra_orthogonal_edges(graph: &Graph) -> HashSet<(String, String)> {
    let mut result = HashSet::new();
    for sg in &graph.subgraphs {
        collect_intra_edges(sg, graph, graph.direction, &mut result);
    }
    result
}

/// Recursive helper for [`intra_orthogonal_edges`]: walk `sg` and its
/// descendants, collecting edge pairs for any subgraph whose direction is
/// orthogonal to `parent_dir` (the effective direction of the enclosing context).
fn collect_intra_edges(
    sg: &Subgraph,
    graph: &Graph,
    parent_dir: Direction,
    out: &mut HashSet<(String, String)>,
) {
    let effective_child_dir = sg.direction.unwrap_or(parent_dir);
    // Recurse children with sg's effective direction as their parent context.
    for child_id in &sg.subgraph_ids {
        if let Some(child) = graph.find_subgraph(child_id) {
            collect_intra_edges(child, graph, effective_child_dir, out);
        }
    }
    let Some(sg_dir) = sg.direction else { return };
    if sg_dir.is_horizontal() == parent_dir.is_horizontal() {
        return; // same axis as effective parent — not orthogonal, no collapse needed
    }

    // All edges where both endpoints are direct node_ids of this subgraph.
    let member_set: HashSet<&str> = sg.node_ids.iter().map(String::as_str).collect();
    for edge in &graph.edges {
        if member_set.contains(edge.from.as_str()) && member_set.contains(edge.to.as_str()) {
            let (a, b) = if edge.from <= edge.to {
                (edge.from.clone(), edge.to.clone())
            } else {
                (edge.to.clone(), edge.from.clone())
            };
            out.insert((a, b));
        }
    }
}

/// Re-assign positions for members of each direction-override subgraph in
/// DFS post-order (inner subgraphs first, outer last), so every level of
/// nesting receives correctly re-ordered positions before the parent
/// consumes them.
///
/// **Why post-order?** An inner override transposes its members relative to
/// the inner anchor.  If the outer override were processed first, it would
/// anchor on stale (pre-inner-transpose) row values and produce wrong offsets.
///
/// **Parallel-edge widening interaction (v1 tradeoff):** `apply_parallel_edge_widening`
/// runs before this pass (step 4.6) and operates along the parent graph's flow axis.
/// For parallel-edge groups where *both* endpoints are direct members of an
/// orthogonal-override subgraph, the widening shifts along the parent axis —
/// which is the *within-layer* axis after the override transpose, not the new
/// flow axis.  In practice this means those groups may not get full breathing
/// room after the transpose.  The safe fix is to run widening *after* transposes
/// (so it sees the per-subgraph effective direction), but that requires per-node
/// effective-direction tracking which is sub-phase 6 work.  Accepted limitation
/// for v1; pure-override-subgraph parallel groups are rare.
fn apply_direction_overrides(
    positions: &mut HashMap<String, (usize, usize)>,
    graph: &Graph,
    config: &LayoutConfig,
) {
    if graph.subgraphs.is_empty() {
        return;
    }
    // We need a collected list for the recursive helper — borrow checker doesn't
    // allow passing `&graph.subgraphs` alongside `&mut positions` through a
    // shared reference, but we only need the IDs/direction fields, so clone them.
    let sgs: Vec<Subgraph> = graph.subgraphs.clone();
    apply_overrides_recursive(positions, graph, &sgs, graph.direction, config);
}

/// Recurse DFS post-order over `sgs`, applying transposes from inner to outer.
///
/// `parent_dir` is the effective flow direction of the *parent* context —
/// initially the top-level `graph.direction`, updated to `sg_dir` as we
/// recurse into each override subgraph.  This lets alternating-direction
/// nesting (e.g. LR → TB → LR) compose correctly at each level: each
/// subgraph is evaluated against its *immediate* parent, not the root.
fn apply_overrides_recursive(
    positions: &mut HashMap<String, (usize, usize)>,
    graph: &Graph,
    sgs: &[Subgraph],
    parent_dir: Direction,
    config: &LayoutConfig,
) {
    for sg in sgs {
        let Some(sg_dir) = sg.direction else {
            // No override on this subgraph; recurse with same parent_dir.
            let children: Vec<Subgraph> = sg
                .subgraph_ids
                .iter()
                .filter_map(|id| graph.find_subgraph(id).cloned())
                .collect();
            apply_overrides_recursive(positions, graph, &children, parent_dir, config);
            continue;
        };

        // Children first (post-order), passing sg_dir as their parent_dir so
        // nested overrides are evaluated relative to this subgraph's direction.
        let children: Vec<Subgraph> = sg
            .subgraph_ids
            .iter()
            .filter_map(|id| graph.find_subgraph(id).cloned())
            .collect();
        apply_overrides_recursive(positions, graph, &children, sg_dir, config);

        if sg_dir.is_horizontal() == parent_dir.is_horizontal() {
            continue; // same axis as effective parent — no transpose needed
        }
        transpose_subgraph_positions(positions, graph, sg, sg_dir, config);
    }
}

/// Transpose the positions of `sg`'s direct members so they flow along
/// `sg_dir` rather than the parent graph's direction.
///
/// **Mechanism:** after ascii-dag layout (+ global transpose for LR/RL),
/// all direct members of an orthogonal-override subgraph share the same
/// flow-axis coordinate (they were placed in one layer because we dropped
/// their intra edges).  We recompute their positions as a mini-layout along
/// the *override* axis, anchored at the subgraph's current top-left:
///
/// - Topological-sort the members using only intra-subgraph edges.
/// - Assign positions in topo order: step along the override flow axis by
///   `node_box_{width,height} + config.node_gap`, keeping the perpendicular
///   axis at the anchor.
/// - Mirror if `sg_dir` is RL or BT.
fn transpose_subgraph_positions(
    positions: &mut HashMap<String, (usize, usize)>,
    graph: &Graph,
    sg: &Subgraph,
    sg_dir: Direction,
    config: &LayoutConfig,
) {
    let members: Vec<&str> = sg
        .node_ids
        .iter()
        .filter(|id| positions.contains_key(*id))
        .map(String::as_str)
        .collect();
    if members.is_empty() {
        return;
    }

    // Anchor = top-left of current bounding box.
    let anchor_col = members.iter().map(|id| positions[*id].0).min().unwrap();
    let anchor_row = members.iter().map(|id| positions[*id].1).min().unwrap();

    // Topological sort of subgraph members via Kahn's algorithm.
    let topo = topo_sort_members(&members, graph);

    // Re-assign positions in topo order along the override-direction axis.
    // `sg_dir` is the override: TB/BT → col stays at anchor, row increases;
    //                            LR/RL → row stays at anchor, col increases.
    let override_is_horizontal = sg_dir.is_horizontal();
    let mut flow_offset = 0usize;
    let mut new_positions: HashMap<String, (usize, usize)> = HashMap::with_capacity(topo.len());
    for id in &topo {
        let (col, row) = if override_is_horizontal {
            // Override is LR/RL: advance along the col axis.
            (anchor_col + flow_offset, anchor_row)
        } else {
            // Override is TB/BT: advance along the row axis.
            (anchor_col, anchor_row + flow_offset)
        };
        new_positions.insert((*id).to_owned(), (col, row));
        // Step = node size along the override flow axis + gap.
        let step = if override_is_horizontal {
            node_box_width(graph, id) + config.node_gap
        } else {
            node_box_height(graph, id) + config.node_gap
        };
        flow_offset += step;
    }

    // Mirror for RL or BT within the subgraph's new extent.
    if override_is_horizontal {
        let max_col = new_positions
            .values()
            .map(|(c, _)| *c)
            .max()
            .unwrap_or(anchor_col);
        if matches!(sg_dir, Direction::RightToLeft) {
            for (col, _) in new_positions.values_mut() {
                *col = anchor_col + (max_col - *col);
            }
        }
    } else {
        let max_row = new_positions
            .values()
            .map(|(_, r)| *r)
            .max()
            .unwrap_or(anchor_row);
        if matches!(sg_dir, Direction::BottomToTop) {
            for (_, row) in new_positions.values_mut() {
                *row = anchor_row + (max_row - *row);
            }
        }
    }

    // Write back.
    for (id, pos) in new_positions {
        positions.insert(id, pos);
    }
}

/// Topological sort (Kahn's) of `members` using only intra-subgraph edges
/// from `graph`. Returns all members in topological order; if a cycle is
/// detected (shouldn't happen in valid Mermaid flowcharts), returns members
/// in declaration order as a fallback.
fn topo_sort_members(members: &[&str], graph: &Graph) -> Vec<String> {
    let member_set: HashSet<&str> = members.iter().copied().collect();
    let mut succ: HashMap<&str, Vec<&str>> = members.iter().map(|&m| (m, Vec::new())).collect();
    let mut in_degree: HashMap<&str, usize> = members.iter().map(|&m| (m, 0usize)).collect();

    for edge in &graph.edges {
        let (f, t) = (edge.from.as_str(), edge.to.as_str());
        if member_set.contains(f) && member_set.contains(t) && f != t {
            succ.entry(f).or_default().push(t);
            *in_degree.entry(t).or_default() += 1;
        }
    }

    let mut queue: std::collections::VecDeque<&str> = members
        .iter()
        .filter(|&&m| in_degree.get(m).copied().unwrap_or(0) == 0)
        .copied()
        .collect();
    let mut order: Vec<String> = Vec::with_capacity(members.len());
    while let Some(node) = queue.pop_front() {
        order.push(node.to_owned());
        let succs: Vec<&str> = succ.get(node).cloned().unwrap_or_default();
        for s in succs {
            let d = in_degree.entry(s).or_default();
            *d = d.saturating_sub(1);
            if *d == 0 {
                queue.push_back(s);
            }
        }
    }
    // Cycle fallback: return declaration order for any node not in the topo result.
    if order.len() < members.len() {
        let in_order: HashSet<String> = order.iter().cloned().collect();
        for &m in members {
            if !in_order.contains(m) {
                order.push(m.to_owned());
            }
        }
    }
    order
}

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
///   3. For LR/RL, transposing each node's `(x, y)` to `(y, x)`.
///   4. Applying per-layer gap expansion (`layer_gap − 3` extra cells per
///      level) and parallel-edge widening.
///   5. Applying per-subgraph direction overrides (DFS post-order).
///   6. For RL/BT, mirroring the transposed axis.
///
/// The `LayoutConfig`'s `node_gap` / `layer_gap` are passed
/// through ascii-dag's spacing controls so behaviour matches
/// our native pipeline.
/// Recursive helper for Bug 1 layer-width post-pass: walk the subgraph
/// tree and accumulate `parallel_label_extra` per ascii-dag layer.
///
/// `parent_axis_horizontal` is `true` for parent direction LR/RL (the
/// flow-axis is `col`), `false` for TB/BT. `parallel_label_extra`
/// returns `(extra_w, extra_h)` where extra_w is non-zero when the
/// subgraph's direction is TB/BT and extra_h is non-zero when LR/RL.
/// We only consume the component along the PARENT flow axis.
fn collect_subgraph_extras(
    graph: &Graph,
    sg: &Subgraph,
    parent_axis_horizontal: bool,
    id_to_level: &HashMap<String, usize>,
    layer_extra: &mut HashMap<usize, usize>,
) {
    let (extra_w, extra_h) = parallel_label_extra(graph, sg);
    let extra = if parent_axis_horizontal {
        extra_w
    } else {
        extra_h
    };
    if extra > 0 {
        // All direct members share a level after the override pre-pass.
        // Take the min level among members as the "boundary" — extras
        // shift everything STRICTLY beyond that level.
        let member_level = sg
            .node_ids
            .iter()
            .filter_map(|nid| id_to_level.get(nid).copied())
            .min();
        if let Some(level) = member_level {
            let cur = layer_extra.entry(level).or_insert(0);
            *cur = (*cur).max(extra);
        }
    }
    for child_id in &sg.subgraph_ids {
        if let Some(child) = graph.find_subgraph(child_id) {
            collect_subgraph_extras(
                graph,
                child,
                parent_axis_horizontal,
                id_to_level,
                layer_extra,
            );
        }
    }
}

/// Recenter singleton visual layers against the median of their real
/// neighbours' centres on the perpendicular axis.
///
/// `ascii-dag`'s coordinate assignment legitimately uses long-edge dummy
/// nodes when computing within-layer order. Once we project back to "real
/// nodes only" positions, a layer can end up containing a single visible
/// node that still carries the vertical offset induced by those hidden
/// dummies. This shows up as dependency-graph intermediates that look
/// needlessly "kinked" even though the layer ordering is correct.
///
/// Smoothing only singleton visual layers is a conservative fix:
/// - it never changes layer ordering or crossing structure;
/// - it cannot create same-layer overlap because there are no visible peers;
/// - it works for any chart where a lone visible node in a layer should align
///   more naturally with its incident neighbourhood.
fn smooth_singleton_layers(positions: &mut HashMap<String, (usize, usize)>, graph: &Graph) {
    let flow_is_horizontal = graph.direction.is_horizontal();
    let original_positions = positions.clone();
    let mut layers: HashMap<usize, Vec<&str>> = HashMap::new();
    for node in &graph.nodes {
        let Some(&(col, row)) = original_positions.get(&node.id) else {
            continue;
        };
        let layer_key = if flow_is_horizontal { col } else { row };
        layers.entry(layer_key).or_default().push(node.id.as_str());
    }

    let mut layer_keys: Vec<usize> = layers.keys().copied().collect();
    layer_keys.sort_unstable();

    let mut updates: Vec<(&str, usize)> = Vec::new();
    for layer_key in layer_keys {
        let ids = &layers[&layer_key];
        if ids.len() != 1 {
            continue;
        }
        let id = ids[0];
        let Some(target_center) =
            median_neighbor_center(&original_positions, graph, id, flow_is_horizontal)
        else {
            continue;
        };

        let size = if flow_is_horizontal {
            node_box_height(graph, id)
        } else {
            node_box_width(graph, id)
        };
        let half_span = (size.saturating_sub(1) as f64) / 2.0;
        let new_start = (target_center - half_span).round().max(0.0) as usize;
        updates.push((id, new_start));
    }

    for (id, new_start) in updates {
        if let Some((col, row)) = positions.get_mut(id) {
            if flow_is_horizontal {
                *row = new_start;
            } else {
                *col = new_start;
            }
        }
    }
}

fn median_neighbor_center(
    positions: &HashMap<String, (usize, usize)>,
    graph: &Graph,
    id: &str,
    flow_is_horizontal: bool,
) -> Option<f64> {
    let mut neighbors: HashSet<&str> = HashSet::new();
    let mut has_incoming = false;
    let mut has_outgoing = false;
    for edge in &graph.edges {
        if edge.from == id && edge.to != id {
            neighbors.insert(edge.to.as_str());
            has_outgoing = true;
        } else if edge.to == id && edge.from != id {
            neighbors.insert(edge.from.as_str());
            has_incoming = true;
        }
    }
    if neighbors.is_empty() || !has_incoming || !has_outgoing {
        return None;
    }

    let mut centers: Vec<f64> = neighbors
        .into_iter()
        .filter_map(|neighbor| {
            let &(col, row) = positions.get(neighbor)?;
            let start = if flow_is_horizontal { row } else { col };
            let size = if flow_is_horizontal {
                node_box_height(graph, neighbor)
            } else {
                node_box_width(graph, neighbor)
            };
            Some(start as f64 + (size.saturating_sub(1) as f64) / 2.0)
        })
        .collect();
    if centers.is_empty() {
        return None;
    }

    centers.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = centers.len() / 2;
    Some(if centers.len() % 2 == 1 {
        centers[mid]
    } else {
        (centers[mid - 1] + centers[mid]) / 2.0
    })
}

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

    // Pre-pass: collect intra-orthogonal-set edge pairs so we can hide them
    // from ascii-dag. Hiding them forces ascii-dag to place all members of
    // an override subgraph into one parent layer (no ordering signal →
    // all members end up as layer-siblings). The edges are NOT removed from
    // `graph.edges` — the A* router in lib.rs re-routes them from the final
    // node positions automatically.
    let skip_edges = intra_orthogonal_edges(graph);

    for edge in &graph.edges {
        // Skip intra-orthogonal edges — they would give ascii-dag a false
        // ordering constraint that spreads override-subgraph members across
        // multiple layers, preventing the post-pass transpose from working.
        let canonical = if edge.from <= edge.to {
            (edge.from.clone(), edge.to.clone())
        } else {
            (edge.to.clone(), edge.from.clone())
        };
        if skip_edges.contains(&canonical) {
            continue;
        }
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

    // 4.4. B1 — Terminal-state-marker promotion. State diagrams use
    //      a synthetic `__end__` (or `__end__<scope>`) node for the
    //      `[*]` final marker; longest-path layering puts it at
    //      `max(predecessor_level) + 1`, which for short paths lands
    //      mid-graph while real states sit further right via longer
    //      paths. Promote any sink whose id starts with `__end__` so
    //      the final marker always renders in the rightmost layer.
    //
    //      The promotion is intentionally narrow: a generic "promote
    //      all sinks" rule would also relocate flowchart leaves
    //      (e.g. `App --> Cache` where Cache has no successors) and
    //      break their topologically-correct longest-path placement.
    //      `__end__` is a state-diagram-only synthetic id, so this
    //      path is exclusive to state diagrams without needing a
    //      diagram-type tag on `Graph`.
    //
    //      Network simplex layering (Gansner 1993) does NOT fix this
    //      symptom — research-doc analysis shows it ALSO assigns the
    //      sink to layer 1 for short paths. The correct fix is a
    //      post-pass regardless of which ranker is used.
    //
    //      Within-layer (perpendicular axis) coordinate is shifted
    //      to clear the lowest existing max-level box so the
    //      promoted marker does not overlap. Multi-`__end__` fixtures
    //      slot consecutively; their relative ordering is the IR
    //      iteration order, which is stable across runs.
    {
        let max_level = raw_positions
            .iter()
            .map(|(_, _, _, l)| *l)
            .max()
            .unwrap_or(0);
        if max_level > 0 {
            let has_outgoing: HashSet<&str> = graph.edges.iter().map(|e| e.from.as_str()).collect();
            let sinks: HashSet<String> = graph
                .nodes
                .iter()
                .filter(|n| !has_outgoing.contains(n.id.as_str()) && n.id.starts_with("__end__"))
                .map(|n| n.id.clone())
                .collect();

            // Find the flow-axis coordinate of any node already at
            // max_level so promoted sinks can adopt it. Flow axis
            // depends on transpose: for LR/RL it's `col`, for TB/BT
            // it's `row`.
            let max_level_flow = raw_positions.iter().find_map(|(_, c, r, l)| {
                if *l == max_level {
                    Some(if transpose { *c } else { *r })
                } else {
                    None
                }
            });

            // Find the lowest within-layer extent already occupied at
            // max_level. Promoted sinks slot in BELOW that extent (or
            // RIGHT for TB) so their boxes don't overlap with existing
            // max-level nodes' boxes.
            //
            // Within-layer axis is `row` for LR/RL (transpose=true),
            // `col` for TB/BT. We need to advance past the box's full
            // perpendicular extent, not just its top-left coordinate.
            let max_level_within_extent = raw_positions
                .iter()
                .filter(|(_, _, _, l)| *l == max_level)
                .map(|(id, c, r, _)| {
                    let perp = if transpose { *r } else { *c };
                    let perp_size = if transpose {
                        node_box_height(graph, id)
                    } else {
                        node_box_width(graph, id)
                    };
                    perp + perp_size
                })
                .max();

            if let Some(target_flow) = max_level_flow {
                let mut next_within = max_level_within_extent.unwrap_or(0);
                for (id, col, row, level) in raw_positions.iter_mut() {
                    if *level < max_level && sinks.contains(id) {
                        *level = max_level;
                        if transpose {
                            *col = target_flow;
                            *row = next_within + 1;
                            next_within = *row + node_box_height(graph, id);
                        } else {
                            *row = target_flow;
                            *col = next_within + 1;
                            next_within = *col + node_box_width(graph, id);
                        }
                    }
                }
            }
        }
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
    //
    //      NOTE: widening runs BEFORE direction-override transposes (step 4.7).
    //      Parallel groups where both endpoints are inside an orthogonal-override
    //      subgraph will be widened along the parent axis — which is the
    //      within-layer axis after the override transpose, not the new flow
    //      axis.  This is acceptable v1 behaviour; see module doc for rationale.
    apply_parallel_edge_widening(&mut positions, &id_to_level, graph);

    // 4.7. Apply per-subgraph direction overrides.  For each subgraph whose
    //      `direction` is orthogonal to the parent's flow axis (e.g. `direction TB`
    //      inside `graph LR`), re-assign its direct members' positions so they
    //      flow along the override axis.  Inner subgraphs are processed before
    //      outer ones (DFS post-order) so nested overrides compose correctly.
    apply_direction_overrides(&mut positions, graph, _config);

    // 4.8. Bug 1 — Subgraph layer-width post-pass (mirrors Native LR's
    //      `layer_parallel_label_extra_width` invariant). When a
    //      subgraph overrides the parent direction (e.g. `direction TB`
    //      inside `graph LR`) AND has parallel-edge groups, its
    //      bounding-box width is inflated by `parallel_label_extra` so
    //      the labels have breathing room. ascii-dag has no concept of
    //      cluster-width feedback, so without this post-pass the
    //      Supervisor-style subgraph's right border lands inside the
    //      next layer's node box.
    //
    //      Algorithm:
    //        1. Per subgraph with extra_w/h > 0, compute the parent
    //           flow-axis level its members occupy (after direction-
    //           override pre-pass, all members share one level).
    //        2. For each occupied level L, accumulate `max(extra)`
    //           across all subgraphs at that level.
    //        3. Shift every node whose level > L right by that extra
    //           (LR/RL: along col; TB/BT: along row).
    //
    //      Idempotent across multiple subgraphs at different levels:
    //      level-2's extras stack on top of level-1's.
    {
        let parent_axis_horizontal = matches!(
            graph.direction,
            Direction::LeftToRight | Direction::RightToLeft
        );
        let mut layer_extra: HashMap<usize, usize> = HashMap::new();
        for sg in &graph.subgraphs {
            collect_subgraph_extras(
                graph,
                sg,
                parent_axis_horizontal,
                &id_to_level,
                &mut layer_extra,
            );
        }
        if !layer_extra.is_empty() {
            let mut levels: Vec<usize> = layer_extra.keys().copied().collect();
            levels.sort();
            for boundary_level in levels {
                let extra = layer_extra[&boundary_level];
                for (id, (col, row)) in positions.iter_mut() {
                    if let Some(&node_level) = id_to_level.get(id)
                        && node_level > boundary_level
                    {
                        if parent_axis_horizontal {
                            *col += extra;
                        } else {
                            *row += extra;
                        }
                    }
                }
            }
        }
    }

    // 4.9. Smooth singleton visual layers after all flow-axis spacing passes.
    //      This preserves the layer ordering chosen by ascii-dag while
    //      removing dummy-induced perpendicular offsets from lone visible
    //      nodes like the README dependency graph's `Worker`.
    smooth_singleton_layers(&mut positions, graph);

    // Recompute max_x / max_y after widening and overrides so step 5's
    // mirror arithmetic uses the updated extents.
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
    fn singleton_dependency_layer_tracks_neighbor_median() {
        let src = "graph LR\n    App --> DB[(PostgreSQL)]\n    App --> Cache[(Redis)]\n    App --> Queue[(RabbitMQ)]\n    Queue --> Worker[Worker]\n    Worker --> DB";
        let g = crate::parser::flowchart::parse(src).unwrap();
        let out = sugiyama_layout(&g, &LayoutConfig::default());

        let center_row = |id: &str| -> f64 {
            let (_, row) = out.positions[id];
            row as f64 + (node_box_height(&g, id).saturating_sub(1) as f64) / 2.0
        };

        let worker = center_row("Worker");
        let queue = center_row("Queue");
        let db = center_row("DB");
        let target = (queue + db) / 2.0;
        assert!(
            (worker - target).abs() <= 0.5,
            "Worker should stay within half a cell of its real-neighbor median: \
             queue={queue}, worker={worker}, db={db}, target={target}"
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

    // ---- direction-override tests (sub-phase 3) ------------------------------

    /// Outer TB, inner LR: subgraph members must be arranged left-to-right
    /// (increasing col) while the parent flow stays top-down (rows increase).
    #[test]
    fn subgraph_direction_override_lr_in_tb() {
        use crate::types::Subgraph;

        // graph TD
        //   subgraph SG
        //     direction LR
        //     A --> B --> C
        //   end
        let mut g = Graph::new(Direction::TopToBottom);
        for id in ["A", "B", "C"] {
            g.nodes.push(rect(id));
        }
        g.edges.push(Edge::new("A", "B", None));
        g.edges.push(Edge::new("B", "C", None));

        let mut sg = Subgraph::new("SG", "SG");
        sg.direction = Some(Direction::LeftToRight);
        sg.node_ids = vec!["A".into(), "B".into(), "C".into()];
        g.subgraphs.push(sg);

        let out = sugiyama_layout(&g, &LayoutConfig::default());

        for id in ["A", "B", "C"] {
            assert!(out.positions.contains_key(id), "{id} missing");
        }
        // LR override: col must increase A → B → C.
        assert!(
            out.positions["A"].0 < out.positions["B"].0,
            "A must be left of B (LR override): {:?} {:?}",
            out.positions["A"],
            out.positions["B"]
        );
        assert!(
            out.positions["B"].0 < out.positions["C"].0,
            "B must be left of C (LR override): {:?} {:?}",
            out.positions["B"],
            out.positions["C"]
        );
        // All in the same row (no vertical spread within the override subgraph).
        assert_eq!(
            out.positions["A"].1, out.positions["B"].1,
            "A and B should share the same row in LR override"
        );
    }

    /// Supervisor reproducer: outer LR, inner TB.
    /// A → B → C inside the subgraph must flow top-down (row increases).
    #[test]
    fn subgraph_direction_override_tb_in_lr_supervisor_pattern() {
        let src =
            "graph LR\n    subgraph X\n        direction TB\n        A-->B\n        B-->C\n    end";
        let g = crate::parser::flowchart::parse(src).unwrap();
        let out = sugiyama_layout(&g, &LayoutConfig::default());

        for id in ["A", "B", "C"] {
            assert!(out.positions.contains_key(id), "{id} missing");
        }
        // TB override: row must increase A → B → C.
        assert!(
            out.positions["A"].1 < out.positions["B"].1,
            "A must be above B (TB override in LR): {:?} {:?}",
            out.positions["A"],
            out.positions["B"]
        );
        assert!(
            out.positions["B"].1 < out.positions["C"].1,
            "B must be above C (TB override in LR): {:?} {:?}",
            out.positions["B"],
            out.positions["C"]
        );
        // All in the same column band (shared col, flowing TB).
        assert_eq!(
            out.positions["A"].0, out.positions["B"].0,
            "A and B should share the same column in TB-in-LR override"
        );
    }

    /// Same-axis is a no-op: outer LR with `direction RL` subgraph.
    /// Both are horizontal — no axis swap, positions should be determined
    /// by ascii-dag's layer ordering (not transposed).
    #[test]
    fn subgraph_direction_override_same_axis_is_noop() {
        use crate::types::Subgraph;

        // graph LR
        //   subgraph SG
        //     direction RL   ← same axis as parent (both horizontal)
        //     A --> B
        //   end
        let mut g = Graph::new(Direction::LeftToRight);
        for id in ["A", "B"] {
            g.nodes.push(rect(id));
        }
        g.edges.push(Edge::new("A", "B", None));

        let mut sg = Subgraph::new("SG", "SG");
        sg.direction = Some(Direction::RightToLeft);
        sg.node_ids = vec!["A".into(), "B".into()];
        g.subgraphs.push(sg);

        let out = sugiyama_layout(&g, &LayoutConfig::default());

        // Both nodes must be present.
        assert!(out.positions.contains_key("A"), "A missing");
        assert!(out.positions.contains_key("B"), "B missing");

        // The same-axis override should NOT transpose: A→B in LR graph must
        // keep A to the left of B (no axis flip applied).
        assert!(
            out.positions["A"].0 < out.positions["B"].0,
            "same-axis RL override must not transpose: A should still be left of B"
        );
    }

    /// Three-level nesting: outer LR → mid TB (orthogonal) → inner LR (orthogonal to mid).
    ///
    /// Each override level is evaluated relative to its immediate parent's
    /// effective direction (not the root), so alternating directions compose
    /// correctly at every depth.
    #[test]
    fn subgraph_direction_override_nested_recursive() {
        use crate::types::Subgraph;

        // graph LR
        //   subgraph Mid [direction TB]
        //     subgraph Inner [direction LR]
        //       A --> B
        //     end
        //     C
        //   end
        //
        // Expected:
        //   - Inner (LR inside Mid's TB context): A left of B (col increases)
        //   - Mid (TB inside root LR): C above Inner cluster (row increases)
        let mut g = Graph::new(Direction::LeftToRight);
        for id in ["A", "B", "C"] {
            g.nodes.push(rect(id));
        }
        g.edges.push(Edge::new("A", "B", None));
        g.edges.push(Edge::new("C", "A", None)); // C feeds into Inner

        let mut inner = Subgraph::new("Inner", "Inner");
        inner.direction = Some(Direction::LeftToRight);
        inner.node_ids = vec!["A".into(), "B".into()];

        let mut mid = Subgraph::new("Mid", "Mid");
        mid.direction = Some(Direction::TopToBottom);
        mid.node_ids = vec!["C".into()];
        mid.subgraph_ids = vec!["Inner".into()];

        g.subgraphs.push(mid);
        g.subgraphs.push(inner);

        let out = sugiyama_layout(&g, &LayoutConfig::default());

        for id in ["A", "B", "C"] {
            assert!(out.positions.contains_key(id), "{id} missing");
        }
        // Inner is LR inside Mid's TB: A must be left of B.
        assert!(
            out.positions["A"].0 < out.positions["B"].0,
            "A must be left of B in LR-inside-TB override: {:?} {:?}",
            out.positions["A"],
            out.positions["B"]
        );
        // A and B share a row within the inner LR subgraph.
        assert_eq!(
            out.positions["A"].1, out.positions["B"].1,
            "A and B must share a row within the inner LR subgraph"
        );
    }

    /// Snapshot: `perpendicular_subgraph_direction` under Sugiyama backend.
    ///
    /// Side-by-side with the Native baseline in
    /// `snapshots__perpendicular_subgraph_direction.snap` to let reviewers
    /// verify that the TB-in-LR direction override works under Sugiyama.
    #[test]
    fn perpendicular_subgraph_direction_sugiyama() {
        let src = r#"graph LR
        subgraph Sub
            direction TD
            X-->Y-->Z
        end
        A-->Sub"#;
        let out = crate::render_with_options(
            src,
            &crate::RenderOptions {
                backend: crate::layout::LayoutBackend::Sugiyama,
                ..Default::default()
            },
        )
        .unwrap();
        for node in ["X", "Y", "Z"] {
            assert!(out.contains(node), "node {node} missing:\n{out}");
        }
        insta::assert_snapshot!("perpendicular_subgraph_direction_sugiyama", out);
    }

    /// Snapshot: `supervisor_bidirectional_in_subgraph` under Sugiyama backend.
    ///
    /// The Supervisor pattern: outer LR, inner TB. Factory and Worker must
    /// flow top-to-bottom within their cluster; labels must not overwrite
    /// node border rows.
    #[test]
    fn supervisor_bidirectional_in_subgraph_sugiyama() {
        let src = "graph LR
    subgraph Supervisor
        direction TB
        F[Factory] -->|creates| W[Worker]
        W -->|panics| F
    end
    W -->|beat| HB[Heartbeat]
    HB --> WD[Watchdog]";
        let out = crate::render_with_options(
            src,
            &crate::RenderOptions {
                backend: crate::layout::LayoutBackend::Sugiyama,
                ..Default::default()
            },
        )
        .unwrap();
        for node in ["Factory", "Worker", "Heartbeat", "Watchdog"] {
            assert!(out.contains(node), "node {node} missing:\n{out}");
        }
        assert!(
            !out.contains("└───panics┘") && !out.contains("└─────creates─────┘"),
            "labels overwrote node border rows under Sugiyama:\n{out}"
        );
        insta::assert_snapshot!("supervisor_bidirectional_in_subgraph_sugiyama", out);
    }
}
