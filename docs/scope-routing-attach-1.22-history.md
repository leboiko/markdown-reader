# Routing-attach archaeology: 1.22.x iterations

## Summary

- The source-attach anchor logic was introduced from scratch in mermaid-text **0.16.1**
  (parent 1.22.1) and took exactly **three consecutive commits** across ~52 minutes to
  stabilise, producing versions 0.16.1 → 0.16.2 → 0.16.3.
- All three iterations touched a single function in a single file:
  `route_all()` in `crates/mermaid-text/src/layout/router.rs`. No other routing-attach
  file was modified across the three iterations.
- The failure mode was identical across all three attempts: **too-broad an anchor
  condition** produced spurious corner glyphs (`┐ ┘ ┌ └`) on routes that were already
  rendering correctly without the anchor.
- The surviving rule (0.16.3 "final form") is intentionally narrow: apply the anchor
  **only** to TD/BT layouts when the route's first step is horizontal. LR/RL layouts
  are skipped entirely. This narrowness is the root cause of B3, B9, and B12 — cases
  the rule deliberately excluded but where attach correctness is still needed.
- A parallel archaeological thread runs through pre-1.22 work (0.11.2, 0.12.2) that
  fixed the back-edge *perimeter routing* and *border glyph*; those are distinct from
  the source-attach anchor and are not implicated in B3/B9/B12.

---

## Iteration timeline

### Iteration 0 — `8876d92` (2026-04-23) — "Phase 4 — pure A\* edge routing"

- Mermaid-text version: **0.15.0** (parent 1.21.1)
- Mechanism: Created `layout/router.rs` from scratch. Replaced ~450 LOC of
  waypoint-based routing (in `layered.rs` + `unicode.rs`) with a single `route_all()`
  dispatcher: try straight-line → try L-route → fall back to A\*. No source-attach
  anchor existed. `Obstacle::EdgeOccupied` split into
  `EdgeOccupiedHorizontal / EdgeOccupiedVertical` to distinguish same-axis overlaps
  (cost 10) from perpendicular crossings (cost 3). `try_l_route` returned `None` on
  equal H/V cost, letting A\* break ties via the obstacle map.
- Files touched: `src/layout/router.rs` (new, 458 LOC), `src/layout/grid.rs`,
  `src/render/unicode.rs`, `src/layout/layered.rs` (large deletions), `sugiyama.rs`.
- Outcome: **Shipped; becomes the baseline that 0.16.1 patches.**
- Test impact: Added `tests/crossings.rs` with 19 crossing-counter regression tests.

### Iteration 1 — `f85a847` (2026-04-23) — "edge label br + subgraph border crossings"

- Mermaid-text version: **0.16.1** (parent 1.22.1)
- Mechanism: Post-processing pass added at the end of `route_all()` — for every
  successfully routed edge, OR the "back into the source box" direction bit
  (`DIR_UP` for TD, `DIR_DOWN` for BT, `DIR_LEFT` for LR, `DIR_RIGHT` for RL)
  unconditionally into the route's first cell via `grid.add_dirs(src.col, src.row,
  anchor)`. The intent was to produce a proper corner glyph (`└ ┘ ┌ ┐`) at the
  attach cell, solving "edge looks detached" when source and target box widths differ
  by one column. Direction-bit constants `DIR_UP/DOWN/LEFT/RIGHT` were promoted from
  private to `pub(crate)` so `router.rs` could reference them. `Grid::add_dirs`
  was also relaxed to OR through protected cells that already carry direction bits
  (subgraph borders), enabling a companion fix for junction glyphs at subgraph
  border crossings.
- Files touched: `src/layout/router.rs` (+29 lines), `src/layout/grid.rs` (+35
  lines; `add_dirs` visibility + protection logic; new `seed_border_dirs`).
  35 snapshot files regenerated.
- Outcome: **Partially reverted by iteration 2.** Fixed the intended case (detached
  edge on different-width boxes) but produced spurious corner glyphs on:
  (a) back-edges going anti-parallel to natural flow,
  (b) multi-edge fan-outs with straight exits,
  (c) LR layouts containing internal TB subgraphs (Supervisor pattern —
  `│┐`/`│┘` appeared on bidirectional subgraph edges).
- Test impact: 35 snapshots regenerated. Crossing snapshots re-accepted because the
  new junction glyph at subgraph border crossings counts as a crossing (expected).

### Iteration 2 — `233445b` (2026-04-24) — "source-attach correctness"

- Mermaid-text version: **0.16.2** (parent 1.22.2)
- Mechanism: Replaced the unconditional anchor with a **perpendicular-axis guard**.
  After routing, if the path has ≥ 2 cells, read the first two: `(c0, r0)` and
  `(_, r1)`. If `r0 == r1` the first step is horizontal. If the first step is
  perpendicular to the layout's natural axis (`route_first_step_horizontal !=
  natural_horizontal`), apply the anchor. Otherwise skip it. Also changed
  `try_l_route` tie-breaking: on equal H/V cost, prefer the bend near the target
  (`horizontal_first` flag — V-first for TB/BT, H-first for LR/RL) so the source
  side stays as a clean straight segment. Removed the LR/RL anchor cases from the
  `match` but did not exclude LR/RL entirely — vertical first steps in LR layouts
  still triggered the anchor via the `!=` condition.
- Files touched: `src/layout/router.rs` (99 additions, 56 deletions). 4 crossing
  snapshots updated. No grid.rs changes.
- Outcome: **Partially reverted by iteration 3.** Fixed fan-out and pure-parallel
  back-edges (crossing counts improved: `dense_bipartite` 8→5, `dense_td_crossing`
  6→3, `dense_fan_out` 7→6). But LR layouts with vertical first steps (mid-side
  attach points in LR+internal-TB subgraphs) still triggered the anchor because
  `route_first_step_horizontal = false` and `natural_horizontal = true`, making
  `!=` true — the exact Supervisor `│┐`/`│┘` symptom from 0.16.1 remained.
- Test impact: 4 crossing snapshots updated (dense_bipartite, dense_fan_out,
  dense_multiple_back_edges, dense_td_crossing).

### Iteration 3 — `c1f0419` (2026-04-24) — "source-attach final form"

- Mermaid-text version: **0.16.3** (parent 1.22.3)
- Mechanism: Replaced the perpendicular-axis guard with a **layout-axis positive
  check**: apply the anchor only when `route_first_step_horizontal == true` AND the
  layout is TD or BT (i.e., `anchor` returns `Some(DIR_UP/DOWN)` for TD/BT and
  `None` for LR/RL). The LR/RL branch now explicitly returns `None`, skipping the
  anchor entirely. The reasoning documented in the commit: for LR/RL the anchor bit
  would land on the same axis as the first step, OR-ing into a visual no-op `─`
  while still polluting `directions[]`, which subtly increases `edge_occupied`
  weights and worsens crossings on dense LR graphs. Imports reduced from
  `DIR_DOWN, DIR_LEFT, DIR_RIGHT, DIR_UP` to only `DIR_DOWN, DIR_UP`; `DIR_LEFT`
  and `DIR_RIGHT` removed from router imports entirely.
- Files touched: `src/layout/router.rs` (53 additions, 22 deletions). 7 snapshot
  files updated. `crates/mermaid-text/README.md` had two stale static-text examples
  regenerated.
- Outcome: **Shipped; current production state.** Supervisor `││` renders cleanly.
  `dense_fan_out` 6→5 crossings. Intentional exclusion of LR/RL and vertical-step
  TD/BT cases is the precondition for B3, B9, B12.
- Test impact: 7 snapshots regenerated (all_edge_styles, edge_crosses_subgraph_
  boundary, perpendicular_subgraph_direction, state_circuit_breaker, state_nested_
  composites, state_self_transition, supervisor_bidirectional_in_subgraph, plus one
  crossings snapshot).

---

## Pre-1.22 back-edge context (not the source-attach anchor, but adjacent)

Two earlier commits are part of the back-edge rendering history and document
regression vectors that will recur when B3/B9/B12 are touched:

**`76ca6c6` (2026-04-21, mermaid-text 0.11.2) — "back-edge perimeter routing fix"**

Introduced `Obstacle::InnerArea` to classify cells in the convex hull of node
bounding boxes, and `Grid::route_back_edge` (a variant of `route_edge` with an
extra inner-area cost of 8.0 to bias A\* toward the perimeter corridor). This is
the mechanism by which B9's `├` glyph deposit is produced: the perimeter path
exits the source node's right border (`back_edge_border_cells` for TD/BT returns
`border_col = c + geom.width - 1`) and a junction glyph is stamped there. If the
source node is a rounded box, `c + geom.width - 1` lands inside the rounded corner
cell zone.

**`8fbceae` (2026-04-21, mermaid-text 0.12.2) — "TD/BT back-edge corner glyph fix"**

Fixed a uniform glyph table that produced `├┤` adjacent on the same row for TD/BT
back-edges. Changed the path-cell glyph from `┴` (correct for LR where adjacency
is vertical) to `┘` (TD) / `┐` (BT) to indicate "path comes from left, turns
up/down." This `├` + `┘/┐` pair at the source border is what B9 is observing in
a degraded form — the `├` is stamped at `back_edge_border_cells.border_col` which,
for a `Running` state-diagram box, is adjacent to the border cell.

---

## Patterns observed

- **Pattern: broadening the anchor condition fixes the narrow case but breaks
  straight-exit edges.** Each time the anchor was applied to more routes (0.16.1:
  all routes; 0.16.2: perpendicular-axis routes), edges that were already rendering
  correctly acquired spurious corner glyphs. The only safe anchor application is one
  where the anchor bit is strictly additive (the cell has one direction bit and
  needs the opposite-axis bit to form a corner).

- **Pattern: LR/RL layouts with internal TB subgraphs (Supervisor) are the
  canary.** This diagram type has edges with vertical first steps inside an LR
  layout, which looks perpendicular to the LR axis — exactly the case that
  iteration 2's guard misidentified as "needing an anchor." Any change to the
  anchor condition must be verified against the `supervisor_bidirectional_in_subgraph`
  snapshot.

- **Pattern: direction-bit pollution worsens crossing counts on dense LR graphs.**
  Iteration 3's commit message documents this explicitly: OR-ing `DIR_LEFT` or
  `DIR_RIGHT` into a source cell in an LR layout increases the `edge_occupied`
  weight seen by downstream L-route cost calculations. Changes to `add_dirs` in the
  source-cell anchor path have measurable side effects on the crossing corpus
  (dense_fan_out, dense_bipartite, dense_td_crossing are the sensitive benchmarks).

- **Pattern: `back_edge_border_cells` uses fixed offsets that are insensitive to
  node shape.** Both `exit_point_back_edge` and `back_edge_border_cells` compute
  their positions from `geom.width` and `geom.height` without consulting the node's
  shape variant. For rounded boxes (`╭╮╰╯`), the corner cells are at
  `(c, r)`, `(c + width - 1, r)`, `(c, r + height - 1)`, `(c + width - 1,
  r + height - 1)`. The `border_col = c + geom.width - 1` in `back_edge_border_cells`
  for TD/BT exactly hits the top-right corner cell of a rounded box — which is `╮`,
  not a plain `─` or `│` that the junction stamping code expects. This is the
  mechanism behind B12 (bottom piercing) and B9 (misplaced `├`).

- **Pattern: the `exit_point` for forward edges in TD layout places the attach one
  row BELOW the bottom border (`row: r + geom.height`).** For the `App` box in a
  dependency graph, this is the row of the bottom `─` border. If the route then
  turns immediately sideways (L-route, horizontal first step), the first path cell
  is on that same row as the bottom border, and without the anchor it renders as `─`
  stacking against the border `─`. But for long edges whose A\* path exits upward
  through the top (a "long-edge route exits through the top" symptom, per B3's
  description), the issue is upstream in A\*'s cost map, not in the anchor logic.
  B3 may be a distinct mechanism from B9/B12.

---

## Implications for Phase 3

### B3 — `App` box top border broken (`┌─────┐────┐`)

- **Likely shared code path:** `exit_point()` in `unicode.rs` + `route_all()` in
  `router.rs`, specifically the L-route or A\* fallback path for the specific edge
  that exits through the top. The symptom ("long-edge route exits through box top
  row") suggests A\* found a path going up through the source box's top border,
  meaning the obstacle map did not mark the source box interior as `NodeBox` on all
  rows, OR the A\* start cell is placed inside the box rather than outside it. The
  `exit_point` for TD puts the attach at `row: r + geom.height` (one below the
  bottom border), which is correct; the bug may be that a long-edge A\* route
  backtracks through the source box's interior when the direct route is blocked.
- **Known regression vector:** The `back_edge_avoids_diagram_interior_in_td_cycle`
  and `cicd_parallel_styles_to_same_target` snapshots. If the obstacle-map or
  `InnerArea` logic is modified, those two are the first to break. The `all_edge_styles`
  snapshot also changed in all three 1.22.x iterations and is sensitive to any
  routing-path change.
- **Minimum-risk attack angle:** B3 is likely distinct from B9/B12 (it is a forward
  edge, not a back-edge; the broken border is on the source box top, not the
  perpendicular attach side). Archaeology does not surface a prior iteration that
  attempted this specific fix. Safest approach: add a targeted regression snapshot
  for the dependency-graph `App` box case first, then investigate whether A\*'s
  start-cell protection on the source box is complete (all rows of the NodeBox
  obstacle should be populated before routing begins).

### B9 — Back-edge deposits `├` on the `Running` state box right wall

- **Likely shared code path:** `back_edge_border_cells()` → `back_edge_border_joins`
  stamping loop in `render_inner()` in `unicode.rs`. The `border_col` for a TD/BT
  back-edge is `c + geom.width - 1`; for the `Running` box this is the rightmost
  column of the box border. The loop stamps a junction glyph there via `grid.set()`.
  For a plain rectangle the rightmost border cell is `│`, and the `├` glyph is
  correct (a stub going right out of the box). For a node whose right border is
  adjacent to the diagram interior — or where the junction stamping is running on the
  wrong cell — the `├` appears on a cell that visually reads as "inside" the box.
- **Known regression vector:** `state_circuit_breaker` and
  `back_edge_avoids_diagram_interior_in_td_cycle` snapshots. The 0.12.2 fix
  (`├┘` instead of `├┤`) was specifically about the glyph AFTER the border cell;
  B9 is about the border cell itself being on the wrong column. Adjusting
  `geom.width - 1` vs `geom.width` in `back_edge_border_cells` for TD/BT is the
  most contained change, but it must be verified that the path-cell's junction
  glyph logic (which relies on `(border_cell, first_path_cell)` being one apart)
  is updated in sync.
- **Minimum-risk attack angle:** Iteration 2 (`233445b`) came closest to the
  related area but did not touch `back_edge_border_cells` at all. The 0.12.2 commit
  (`8fbceae`) is the closest prior fix — it changed the path-cell glyph, not the
  border cell. Phase 3 should revisit 0.12.2's diff as the template and ask: should
  `border_col` be `c + geom.width - 1` (ON the border) or `c + geom.width` (one
  past the border, matching the back-edge exit-point logic)?

### B12 — Back-edge source-attach pierces the bottom of a rounded box (`╰─────────┬────────╯`)

- **Likely shared code path:** `exit_point_back_edge()` and `entry_point_back_edge()`
  in `unicode.rs`, plus `back_edge_border_cells()`. For LR/RL flow, the back-edge
  exits from the bottom centre (`row: r + geom.height` — one row below the bottom
  border). `back_edge_border_cells` for LR/RL returns `border_row = r + geom.height - 1`
  (the bottom border row) and stamps a junction glyph there. For a rounded box,
  `r + geom.height - 1` is the row of `╰─────────╯` — the bottom rounded border.
  The `┬` glyph is being OR-ed into a `─` cell on that row, which is technically
  correct for a plain rectangle but visually looks like the edge is piercing the box
  because the `╰` and `╯` corners are protected (no direction bits) while the
  interior `─` cells are not.
- **Known regression vector:** `state_circuit_breaker` snapshot (shows LR state
  machine with rounded boxes and back-edges). Also `back_edge_lr` snapshot. Changes
  to `entry_point_back_edge` for LR/RL must be tested against these. The 1.22.3
  iteration (`c1f0419`) updated `state_circuit_breaker` — that snapshot's current
  state is the baseline to hold.
- **Minimum-risk attack angle:** The `┬` on the bottom row is produced by the
  `back_edge_border_joins` stamping loop. For rounded boxes, the bottom border
  row is `╰─────────╯`; the `─` cells between the corners are not protected and
  accept junction bits. The fix may be to check whether the target cell is
  part of a rounded-box bottom row (i.e., whether the row is `r + geom.height - 1`
  AND the node has a rounded/subgraph shape) and in that case offset the junction
  stamp by one row down (into the perimeter corridor row) rather than ON the border.
  Alternatively, `entry_point_back_edge` for LR/RL could land the tip one row below
  the border (currently `row: r + geom.height - 1`) to avoid the bottom rounded
  border row, matching the TD/BT pattern where the tip stays one column OUTSIDE
  the border for a similar reason.

---

## Open questions

1. **Is B3 a routing bug or an obstacle-map bug?** The symptom ("exits through box
   top row") suggests A\* traversed through the source box rather than starting
   outside it. It is unclear whether `exit_point` for the specific edge places the
   start cell correctly (one row below bottom border), or whether a long-range A\*
   path backtracks upward through the box when the bottom corridor is congested.
   Phase 3 needs a minimal reproduction diagram before touching any code.

2. **Does `Running`'s `├` in B9 appear on the right border or one cell to the right?**
   `back_edge_border_cells` for TD/BT returns `border_col = c + geom.width - 1`
   (on the border) and `path_col = c + geom.width` (one past). The snapshot shows
   `├` adjacent to the box — but "adjacent" in a text cell grid could mean either
   location. Needs a char-level inspection of the snapshot or a targeted unit test
   to confirm which column the `├` occupies.

3. **Are B9 and B12 the same root cause?** Both involve the `back_edge_border_cells`
   stamping loop writing a junction glyph onto a rounded border cell. B9 is TD/BT
   (right border, `├`); B12 is LR/RL (bottom border, `┬`). If they share the root
   cause, a single shape-aware guard in the stamping loop would fix both. Phase 3
   should confirm this by attempting the B9 fix and observing whether B12 disappears
   transitively.

4. **Does the 0.16.3 "final form" anchor rule need to be extended for B3, or is
   B3 independent of the anchor logic?** The anchor post-processing in `route_all`
   only runs on the first cell of the route. B3's symptom is on the top border of
   the source box — which for a TD dependency graph would be the ENTRY side of the
   source (not the exit side). If A\* is routing the edge backwards through the
   source box, the anchor post-processing would not catch it. B3 may require a
   fix in the A\* obstacle map rather than in the source-attach anchor.

5. **What does the dependency graph `App` diagram look like as a concrete input?**
   The roadmap describes the symptom (`┌─────┐────┐`) but does not name a snapshot
   or test fixture. Phase 3 should create a minimal snapshot for this diagram before
   any code is touched, to serve as the regression guard.

---

## Phase 3 Step 3 attempt notes (re-dispatch, 2026-04-28)

### Hypothesis tried

**Root-cause confirmed by tracing (no code changed):** The B3 bug is a routing
*quality* issue — not an obstacle-map correctness bug. The exact chain:

1. `spread_sources` (LR/RL, 3 forward-edges from App) assigns exit rows using
   step=1 centered on the node: rows R, R+1, R+2 for a height-3 box.  Edge 0
   (App→PostgreSQL, the longest edge) receives row R — the TOP border row of App.

2. At exit cell `(app_width, R)`:
   - **H-first L-route**: horizontal segment at row R hits RabbitMQ's NodeBox
     (same row, columns to the right) → `l_cost` returns `None`.
   - **V-first L-route**: corner at `(app_width, pg_row)`, vertical segment
     then horizontal at pg_row — the horizontal segment passes through RabbitMQ's
     NodeBox body → `l_cost` returns `None`.
   - Both L-routes `None` → A\* takes over.

3. A\* from `(app_width, R)` evaluates:
   - UP to `(app_width, R-1)`: 1 step, free perimeter corridor — total UP cost ~1.
   - DOWN to `(app_width, R+1)`: crosses EdgeOccupiedHorizontal from App→Cache
     (cost 1+CROSS_AXIS=4), then App→Queue exit row (similar) — total DOWN cost ~4+
     before reaching the below-RabbitMQ corridor (R+4).
   - A\* rationally picks UP (total path: 1 up + ~40 right + ~3 down) over
     DOWN (4 down + ~40 right + ~3 up), saving ~6 g-cost units.

4. Result: the App→PostgreSQL route wraps OVER the top of the diagram via the
   perimeter corridor, producing the `┌┼───────────────┐│` row above App.

### Scope it expanded to

Every candidate fix examined exceeded the 50-line / 5-snapshot limit:

- **UP-penalty in A\***: adding a cost for going UP in LR forward-edge routing
  biases A\* against the top perimeter for ALL LR diagrams, not just B3.
  Estimated ≥15 LR corpus snapshots would change.  Scope: OVER LIMIT.

- **Source-spread border-skip**: clamping spread to interior rows only
  (skip top/bottom border rows) collapses all 3 exits to 1 interior row for
  a height-3 box — no spread possible.  Doesn't fix B3 even in theory.
  Scope: irrelevant.

- **New U-route option in `try_l_route`**: adding a 3-segment "go below
  RabbitMQ" route option is a new routing strategy requiring new logic in
  `try_l_route` (or a new `try_u_route` helper) plus A\* fallback handling.
  Scope: easily > 50 lines, unknown snapshot count.

- **Routing order change (long edges first)**: reversing the shortest-first
  ordering so long edges claim corridors before short edges is a global change
  to `order_edges`.  Known to increase crossings on dense graphs (confirmed in
  Phase 2 archaeology).  Scope: REGRESSION risk.

### Why stopped

No targeted fix within the 50-line / 5-snapshot scope boundary fixes B3 without
simultaneously affecting many other LR diagrams.  The previous re-dispatch
failure (170-line refactor) is explained: fixing B3 correctly requires one of:

1. A new multi-segment route strategy (beyond the current straight/L/A\*
   progression).
2. An obstacle-map enhancement that makes the perimeter above a forward-edge
   source more expensive *contextually* (i.e., only when that perimeter is
   not on the natural flow path).
3. A post-routing correction pass that detects "route wraps over source box"
   and re-routes the specific edge with a different starting hint.

### Recommendation

Defer B3 to a **multi-day scoped initiative** with the following plan:

- **Step 1**: Add a "U-route" candidate in `router.rs` that tries the 3-segment
  path `exit → down N rows → right → up M rows → target` for LR/RL layouts when
  both L-routes are None (blocked by NodeBox).  This is ~30 lines in `router.rs`
  and is the most targeted mechanical fix for B3 specifically.

- **Step 2**: Measure snapshot impact of Step 1 alone.  If ≤ 5 snapshots differ,
  ship as B3 fix.  If more, investigate per-snapshot whether each change is an
  Improvement or Regression before deciding.

- **Step 3**: Only if Step 1 produces > 5 changes or regressions: consider the
  perimeter-penalty approach with an additional obstacle classification
  `PerimeterAbove`/`PerimeterBelow` that forward edges pay a moderate cost for
  (say 4.0 — cheaper than SAME_AXIS but enough to prefer the below corridor
  over the above when both are equidistant).

The U-route approach (Step 1) is the minimum-risk angle because:
- It only activates when BOTH L-routes return `None` (blocked by NodeBox) AND
  the preferred path (H-first for LR) leads upward.
- It does not change A\* cost weights, so dense-graph crossing counts are
  unaffected.
- It is additive: if the U-route fails, A\* still runs as fallback.

Estimated effort: 1–2 focused hours for a careful implementation and harness run.
