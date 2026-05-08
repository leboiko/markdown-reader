# Scope: composite-edge attach-to-border

## Summary

- **Problem.** In Mermaid stateDiagram-v2, `Composite --> X` and
  `X --> Composite` should visually attach the arrow to the
  composite's OUTER rectangle border. Today our parser rewrites these
  edges at parse time to point at a synthesised inner `[*]` marker
  (`__start__Composite` / `__end__Composite`), so arrows land on a
  `(  ●  )` circle drawn INSIDE the composite, contradicting Mermaid's
  visual contract.
- **First attempt (2026-05-08, reverted).** Disabled the parser rewrite
  + added an `endpoint_geom` helper in the renderer that consults
  `sg_bounds` when an edge endpoint is a composite id (not a node).
  The parser fix was clean; the renderer fix was clean; CANARY
  assertion (`composite_edge_attaches_to_outer_border_not_inner_marker`)
  passed.
- **Why it failed.** Layout-level interaction. The layered layout
  pipeline assigns positions only to actual nodes, deriving subgraph
  bounds post-hoc from member-node positions. With composite-targeted
  edges, the layout doesn't treat the composite as a layer-position
  proxy, so:
  - `__start__` markers (from outer `[*]`) get placed without
    reference to the composite's position.
  - The composite is positioned wherever its inner nodes happen to
    land.
  - The result is geometrically incoherent — markers float above the
    composite, paths route through unrelated cells, label rows
    duplicate, side rails get stamped twice.
- **Status.** Reverted. Test
  `composite_edge_attaches_to_outer_border_not_inner_marker` is
  `#[ignore]`'d as a tracking artefact. Original parser-rewrite
  behaviour restored.

## What was tried

### Attempt 1 — disable parser rewrite + add `endpoint_geom` fallback

Diff sketch (parser):

```rust
// Before: rewrote `Composite --> X` to `__end__Composite --> X` and
// synthesised the marker inside the composite.
// After: no-op — composite-attached edges flow through unchanged.
fn rewrite_composite_edges(&mut self) {}
```

Diff sketch (renderer, `crates/mermaid-text/src/render/unicode.rs`):

```rust
fn endpoint_geom(
    id: &str,
    positions: &HashMap<String, GridPos>,
    geoms: &HashMap<String, NodeGeom>,
    sg_bounds: &[SubgraphBounds],
) -> Option<(GridPos, NodeGeom)> {
    if let (Some(&pos), Some(&geom)) = (positions.get(id), geoms.get(id)) {
        return Some((pos, geom));
    }
    sg_bounds.iter().find(|sg| sg.id == id).map(|sg| {
        let pos = (sg.col, sg.row);
        let geom = NodeGeom { width: sg.width, height: sg.height, text_row: 1 };
        (pos, geom)
    })
}
```

Plus 8 callsite ports in `unicode.rs` to thread `sg_bounds` through:
- `compute_spread_attaches` signature gets `sg_bounds: &[SubgraphBounds]`.
- `edge_effective_direction` consults `sg_bounds`.
- The `has_back_edge` closure in `grid_size`.
- The `back_edge_border_cells` callsite (lookup via `endpoint_geom`).
- The `spread_sources` / `spread_destinations` group-leader lookups.
- The `is_back_edge` per-edge evaluation.

### Why the renderer fix wasn't enough

The render pipeline can correctly *route* an edge once given source/dest
attach points on the composite border. What's missing is the
*layout-level* knowledge that the composite is a logical node for layering
purposes. Without that, sibling nodes (`__start__`, `__end__`, peers like
`X` / `Y` in `X --> Composite --> Y`) get placed without reference to the
composite's spatial position. The composite "floats" wherever its
member nodes happen to land, with no constraint that it be downstream of
its predecessors or upstream of its successors.

Concretely, on `state_composite_external_edges` (`[*] --> Active` and
`Active --> [*]`): pre-fix, the inner markers visually replaced the
external edges (wrong but coherent). Post-fix, the outer `__start__`
and `__end__` markers float above the Active composite and the
connecting paths route through unrelated cells, producing duplicated
label rows and stamped side rails. Bucket-C diff on every state-
composite fixture in the corpus.

## Snapshot triage outcome

9 snapshots changed under the attempted fix. Sampled diffs:

| Fixture | Bucket | Notes |
| --- | --- | --- |
| `state_composite_external_edges` | C | Outer markers float above Active; routing paths cross unrelated cells; double border on Active. |
| `state_composite_keyboard_lock` | C | EvNumLockPressed / EvCapsLockPressed labels duplicated; double border on Active. |
| `state_composite_nested` | C (presumed) | Same class of issue; not sampled in detail. |
| `state_diagram_classdef` | C (presumed) | Same. |

Hard scope ceiling was ≤15 substantive snapshot diffs / 0 class-C; we
measured 9 diffs but multiple class-C → revert.

## Path forward

The fix needs **layout-level support for composite ids as virtual
nodes for layering purposes**. Sketch:

1. **Parser change** (already validated) — disable the rewrite so
   composite-targeted edges keep their composite endpoints.
2. **Layout change** — when assigning layers, treat a composite id
   as if it were a node at its centroid (or at the bounding box of
   its member nodes). Edges into/out of a composite contribute to
   the composite's layer rank, the same way edges into/out of a
   node contribute to that node's rank.
3. **Position synthesis** — after the layered pass, emit a virtual
   "anchor" position for each composite that the renderer can
   consult via `endpoint_geom`. The composite's actual border
   geometry comes from `compute_subgraph_bounds` post-pass; the
   anchor is just for routing.
4. **Edge routing** — once layout produces correct attach points,
   the renderer fallback (`endpoint_geom`) is sufficient. The
   border attach is on the side of the composite closest to the
   counterpart node, with `spread_destinations` / `spread_sources`
   distributing multi-edge attaches.

Estimated additional scope: 1-2 sessions on the layered layout
itself. The renderer-side work (`endpoint_geom` + 8 callsite ports) is
already mapped and small (~150 lines).

## Decision log

- **2026-05-08.** First attempt (parser + renderer only) reverted
  within one hour after class-C snapshot churn surfaced on every
  state-composite fixture. Test left `#[ignore]`'d. No CHANGELOG
  entry and no version bump — the bug is unchanged from 0.52.0.
- **Owner.** Open. The next attempt should land the layered-layout
  change first, validate it doesn't disturb non-composite state
  fixtures, and only then re-introduce the renderer fallback.
