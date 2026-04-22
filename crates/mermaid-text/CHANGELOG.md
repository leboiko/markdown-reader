# Changelog

All notable changes to `mermaid-text` are documented in this file.
This project adheres to [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## 0.14.2 — 2026-04-22

### Fixed

- **Edge labels for parallel-edge groups and multi-outgoing nodes
  place much more cleanly.** When two edges run between the same
  node pair (CI/CD `pass`/`skip`) the labels used to crowd the
  arrow row, with one label often clipping the arrow itself. They
  now stack on opposite sides of the channel — one label above the
  arrow, one below — using a `LabelPlacementContext` that tracks
  prior path cells per source-target pair.

  When a single node fans out to multiple successors (the canonical
  TD state machine's `Running` → `Done` / `Failed`), labels for
  sibling outgoing edges now share a row instead of stacking
  vertically along the arrow corridor (`done │ │ error` instead of
  the old `│done` / `│ │error` two-row layout).

  Three snapshots updated (all clear improvements):
  - `cicd_parallel_styles_to_same_target`
  - `state_circuit_breaker`
  - `back_edge_avoids_diagram_interior_in_td_cycle`

### Internal

- Tiny clippy cleanup in `sugiyama.rs` test
  (`!waypoints.is_empty()` instead of `len() >= 1`).

## 0.14.1 — 2026-04-22

### Fixed

- **Sugiyama backend chrome glitches narrowed.** Previous output
  glued junction glyphs (`├┐`) flush against node borders because
  ascii-dag's hardcoded 3-cell inter-layer spacing didn't leave
  room for our edge-routing A* corners. Now the wrapper applies
  per-layer offsets (`level × (config.layer_gap − 3)`) to expand
  the layout to match our preferred spacing, and looks up each
  long-edge waypoint's level via dummy-node coordinates so the
  expansion stays consistent. Visible improvement on the README
  architecture case: gap between App and RabbitMQ widens from 0
  to 4 cells; the cross-junction now reads as a real `┼` instead
  of a glued `├┐`.

  Snapshot updated:
  `architecture_diagram_with_sugiyama_backend`.

### Changed

- `is_multiple_of(2)` instead of `n % 2 == 0` (now that MSRV is
  1.92, the more idiomatic API is available — keeps clippy happy
  on Rust 1.93's `manual_is_multiple_of` lint).

## 0.14.0 — 2026-04-22

### Added

- **Sugiyama layout backend (opt-in)** — first phase of ROADMAP item
  #6 ("Edge crossings on dependency graphs"). The existing in-house
  layered layout collapses long edges into the wrong layer (Worker
  ends up beside Cache/RabbitMQ in the README architecture example,
  with App→PostgreSQL drawn as zig-zag detours). The new
  [`LayoutBackend::Sugiyama`] backend adapts the [`ascii-dag`] crate
  (0.9.1, MIT/Apache-2.0, zero-dep, `no_std`-compatible) to give us:
  - Proper crossing minimisation (median + adjacent-exchange).
  - Brandes-Köpf coordinate assignment (compact straight long edges).
  - Long-edge dummy nodes with multi-segment waypoint routing — our
    long-edge router threads the existing A* through ascii-dag's
    waypoints, so paths stay clean across multiple layers.
  - Proper layering (App, then Cache+Queue, then Worker, then DB —
    4 distinct columns instead of 3).

  Wired everywhere `RenderOptions` reaches: `RenderOptions::backend`
  defaults to `Native` (no behaviour change for existing callers).
  CLI gains `--sugiyama` flag for one-shot rendering.

  ```text
  Native (default):                        Sugiyama:
  ┌─────┐─────┐╭───────╮         ┌────────┐                ╭───────╮
  │ App │────┐│├───────┤        ▸│ Worker │              ┌▸│ Redis │
  └─────┘───┐└▸│ Redis │        │└────────┘              │ ╰───────╯
            │ │╰───────╯        │     ──────────▸               ┌──────────┐
            │ └─────────────────┼─────┘             ┌─────┐    ▸│ Worker   │
            │                   │                   │ App │    │└──────────┘
            │  ╭──────────╮     │                   └─────┘─▸… │
            │  ├──────────┤     │                              ▸│ PostgreSQL│
            └─▸│ RabbitMQ │─────┘
               ╰──────────╯
  ```

  Coverage gaps the wrapper does NOT yet handle (the in-house
  layered backend remains the right choice when you hit any of
  these): subgraph clusters, parallel-edge groups, direction
  overrides on nested clusters, tunable spacing (ascii-dag uses
  hardcoded 3-cell separation regardless of our `node_gap`/`layer_gap`).

- `LayoutBackend` enum + `LayoutConfig::with_gaps(layer_gap, node_gap)`
  convenience constructor for forward-compatible struct initialisation.

### Changed

- **MSRV bumped to 1.92** to match `ascii-dag`'s requirement. (Previous
  floor was 1.85 from edition 2024.)

[`ascii-dag`]: https://crates.io/crates/ascii-dag

## 0.13.0 — 2026-04-22

### Fixed

- **Subgraphs containing parallel-edge labels now have visible
  breathing room** (Phase 3 of the parallel-edges scope, ROADMAP
  item #2 second-pass). The Supervisor reproducer used to render
  with `creates` and `panics` labels glued flush against the
  subgraph's right border:
  ```
  Before:                       After:
  ╭─Supervisor──╮               ╭─Supervisor───────────╮
  │└─────creates│               │└─────creates         │
  │┌────panics┼┘│               │┌────panics┼┘         │
  ```

  The fix lives in two coordinated places so layout and bounds
  agree: `parallel_label_extra` in `layout/subgraph.rs` computes
  per-subgraph extra room, and the layered layout's per-layer
  width/height calculation pre-allocates the same amount. Result:
  the subgraph border wraps the labels, AND external nodes get
  pushed out by the matching amount instead of overlapping the
  grown border (which was the broken first attempt — easy trap).

  Only fires when the **subgraph overrides the parent direction**
  (e.g., `direction TB` inside an `LR` graph). When directions
  match, `label_gap`'s existing inter-layer widening already
  handles the labels — adding extra would inflate the subgraph
  with empty rows.

  ### Added

  - `parallel_label_extra(graph, sg) -> (extra_w, extra_h)` —
    public helper in `layout::subgraph` so downstream consumers
    building their own bounds can stay in sync.

## 0.12.2 — 2026-04-22

### Fixed

- **TD/BT back-edge source connector renders as a clean L-corner
  instead of garbled `├┤`**. When a back-edge exits a node's right
  side and turns to climb (TD) or descend (BT) the perimeter, the
  source border was stamped `├` (correct) followed by the first
  path cell stamped `┤` (wrong — `┤` is a tee, not a corner, and
  the two glyphs glued together looked like a chrome glitch).
  The `┤` choice was left over from a uniform `┬`/`┴`-vs-`├`/`┤`
  glyph table inherited from the LR/RL case, where the LR pair
  works because they sit on separate rows.

  Now the path-cell glyph is direction-aware:
  - LR/RL: keeps `┴` (vertical adjacency reads fine on its row)
  - TD: `┘` — the path arrives from the left and turns up
  - BT: `┐` — the path arrives from the left and turns down

  Visible on the canonical TD state machine (`Idle → Running →
  Done/Failed → Idle`): the `Idle` cell at the back-edge entry
  now reads `│ Idle ├┘` instead of `│ Idle ├┤`. One snapshot
  updated (`back_edge_avoids_diagram_interior_in_td_cycle`).

## 0.12.1 — 2026-04-22

### Fixed

- **erDiagram relationships now visually connect their entity boxes**.
  The cardinality glyphs and label used to float in a detached row
  *below* both boxes — readers had to mentally connect "the line
  with `1──*` and `places`" to "the entities above it". The
  rendering looked broken, and the gallery's CUSTOMER↔ORDER and
  PARENT↔CHILD examples both showed the symptom prominently.

  The relationship line now sits on the entity-name row of both
  boxes, merging into the side borders via `┤` (source's right
  edge) and `├` (target's left edge) tee glyphs, with cardinality
  markers at each end and the label centred on a row above the
  boxes. The inter-entity gap widens dynamically to fit the
  relationship label plus its cardinality glyphs and breathing
  room. Dashed (non-identifying) relationships keep `│` borders
  since `┤`/`├` are solid-only — the dashed line still touches
  both borders cleanly.

  ```
  Before:                          After:
  ┌──────────┐    ┌─────────┐                    places
  │  PARENT  │    │  CHILD  │      ┌──────────┐          ┌─────────┐
  └──────────┘    └─────────┘      │  PARENT  ┤1────────*├  CHILD  │
              optional             └──────────┘          └─────────┘
              1┄┄*
  ```

  Three existing snapshots updated, all visual improvements.
  Self-relationships are skipped in this pass — they need a loop
  visual that's deferred to a future Phase 3.

## 0.12.0 — 2026-04-22

### Fixed

- **Parallel labelled edges between the same node pair no longer
  cram their labels into one shared cell** (ROADMAP issues #2 +
  #4). Previously, when two or more labelled edges connected the
  same pair of nodes (Supervisor's `creates`/`panics`,
  CI/CD's `pass`/`skip`), all labels competed for one inter-layer
  channel and visually collided — `pass│skip` glued together,
  `creates`/`panics` overlapping the subgraph border, etc.

  `Graph::parallel_edge_groups()` (added in the 0.11.x line as
  pure infrastructure) now feeds `label_gap` in the layered layout
  pass: when N labels share an inter-layer crossing, the gap
  widens by `(N − 1) × (max_label_width + 2)` so each label can
  occupy its own row (LR/RL flow) or column (TD/BT flow). Labels
  remain in the same channel — the channel just gets wider — so
  no path-routing changes are needed (Phase 2a of the scope doc).

  Visible improvements: CI/CD pipeline shows `pass` and `skip`
  on adjacent rows with full breathing room. State diagrams with
  bidirectional transitions (CircuitClosed↔CircuitOpen,
  Working↔Idle's `done`/`task` pair) also benefit — labels that
  had been clipped against subgraph borders or overwritten by
  other labels now render cleanly.

  Snapshot triage: 4 existing snapshots updated, all confirmed as
  visual improvements (the canonical `cicd_parallel_styles_to_same_target`
  is the target-case snapshot). No regressions.

### Added

- `Graph::parallel_edge_groups()` — public method returning groups
  of edge indices that share an unordered endpoint pair (so
  `A → B` and `B → A` group together). Powers Phase 2a above; also
  available to downstream consumers building their own layout
  passes.

## 0.11.2 — 2026-04-22

### Fixed

- **Back-edge perimeter routing no longer fragments forward edges**
  (ROADMAP item #7 — flagged in the gallery review and finally
  resolved). Previously, in cyclic diagrams (most TD state
  machines), the back-edge would A* the shortest path between
  source and target — which often cut through the diagram body
  between forward-edge target nodes, inserting a vertical `│`
  column that crammed the forward-edge labels (`done`/`error`/
  etc.) into narrow channels.

  Now the renderer marks the convex hull of node bounding boxes
  as a new `Obstacle::InnerArea` classification, and back-edge
  A* charges an extra cost (~2× `EDGE_SOFT_COST`) for crossing
  these cells. The back-edge takes the perimeter corridor that
  `compute_canvas_bounds` already reserves, leaving the diagram
  interior clean for forward edges.

  Visible win on the canonical 4-state state machine (`Idle →
  Running → Done/Failed → Idle`): forward edges to `Done`/`Failed`
  now have unobstructed vertical channels; the `done`/`error`
  labels read cleanly; the back-edge routes around the right
  side and re-enters from outside.

### Changed

- Refactored `Grid::route_edge` to share its A* core with the new
  `Grid::route_back_edge` via a private `route_edge_with_inner_cost`
  helper. The two public methods differ only by the
  `inner_area_cost` they pass — same code path, no duplication.
- `Grid::mark_inner_area(col, row, w, h)` is the new public method
  the renderer uses to flag the convex-hull cells.

### Notes

- 1 existing state-diagram snapshot updated (`state_composite_keyboard_lock`)
  — back-edges in that diagram also now route around the perimeter
  rather than through the box-row interior. Verified as a visible
  improvement (cleaner separation between the two NumLock /
  CapsLock state pairs).
- 1 new snapshot test guards against regression
  (`back_edge_avoids_diagram_interior_in_td_cycle`).
- This fix took the InnerArea-obstacle approach diagnosed in the
  ROADMAP entry — the simpler "push exit point further" approach
  tried earlier today only shifted the fragmenting column by 1-2
  cells.

## 0.11.1 — 2026-04-22

### Added

- **erDiagram Phase 2**: entity attributes render inside the boxes
  as aligned columns (type / name / keys), with crisp dividers
  between the header and the attribute table:
  ```
  ┌───────────────────┐
  │     CUSTOMER      │
  ├───────────────────┤
  │  string name      │
  │  string email PK  │
  └───────────────────┘
  ```
  Column widths are computed per-entity so every box is snug — no
  wasted horizontal space.
- **Cardinality glyphs at relationship endpoints**: each end of
  the arrow now carries a single-character cardinality marker:
  `1` (exactly one), `?` (zero or one), `+` (one or many),
  `*` (zero or many). Relationship labels sit on their own row
  above the arrow:
  ```
                places
                1──*
  ```
  Chose single-char glyphs over multi-cell crow's-foot
  approximations because they read unambiguously in any monospace
  font and keep the horizontal footprint small.

### Changed

- The `1:0..N` compound cardinality summary in the label (Phase 1)
  is replaced by per-endpoint glyphs. The user-supplied label now
  carries only the relationship name.

### Notes

- Phase 3 (grid layout for diagrams with more than ~4 entities)
  is the remaining piece of the erDiagram series. The single-row
  source-order layout shipped here works for up to 4-5 entities
  on an 80-column terminal.

## 0.11.0 — 2026-04-22

### Added

- **`erDiagram` (entity-relationship) support — Phase 1**. The
  most-requested missing diagram type per our ROADMAP is now
  rendered natively:
  - Parser accepts the full Mermaid erDiagram grammar: the header,
    relationship lines (`ENTITY1 ||--o{ ENTITY2 : "label"`) with
    all cardinality codes (`||`, `|o`, `}|`, `}o`), both line
    styles (`--` identifying and `..` non-identifying), entity
    blocks (`ENTITY { ... }`) with `type name [KEY,KEY] "comment"`
    attribute rows, and the three recognised key modifiers (`PK`,
    `FK`, `UK`).
  - Phase 1 renderer: single-row source-order layout. Entities
    render as small name-only boxes; relationships render as
    horizontal arrows below the row, labelled with
    `from-cardinality:to-cardinality` plus the user-supplied label.
    Non-identifying relationships use dashed `┄` glyphs.
  - Public types re-exported from the crate root: `ErDiagram`,
    `Entity`, `Attribute`, `AttributeKey`, `Cardinality`,
    `LineStyle`, `Relationship`.
  - `DiagramKind::Er` added to the detection enum.
- **16 parser unit tests** covering: minimal header-only,
  missing-header error, all four cardinality codes round-trip,
  identifying vs non-identifying, labels with and without quotes,
  entity blocks with multiple attributes and key modifiers,
  attribute comments, forward references, unclosed blocks, stray
  `}`, missing connector, invalid cardinality, comment/blank skip.
- **5 data-model unit tests** + **3 renderer unit tests** +
  **3 snapshot tests** (`er_minimal_two_entities`,
  `er_canonical_three_entities`,
  `er_non_identifying_renders_dashed_line`).

### Notes

- Phase 1 intentionally ships minimal: no attribute rows in the
  rendered boxes, no crow's-foot cardinality glyphs at line
  endpoints, single-row layout only. All three polish items land
  in 0.11.x follow-ups (Phase 2: attributes + cardinality glyphs;
  Phase 3: grid layout).
- Hyphenated entity names (`LINE-ITEM` in the canonical Mermaid
  example) are parsed as single tokens — Mermaid permits them.
- This is the first new diagram type since pie (0.9.4). **0.11.0
  minor bump** because the `DiagramKind` enum gained a variant —
  source-breaking for external consumers matching exhaustively on
  it.

## 0.10.1 — 2026-04-22

### Added

- **Median crossing-min metric** — alternates with the existing
  barycenter pass in `order_within_layers`. Median is more robust
  to outlier neighbours (one far-away node can drag a barycenter
  position dramatically; median ignores it). Pairing the two in
  alternating passes escapes local minima either alone would
  settle into.
- **Transpose local-refinement pass** — after each barycenter/
  median sweep pair, iterate over each layer's adjacent node
  pairs and swap any that strictly reduce the global crossing
  count. Catches per-pair improvements neither global sweep
  detects (e.g. tied barycenters that produce different crossing
  counts depending on order).
- **6 new unit tests** covering: median picks the middle, median
  averages for even-length, median resists outliers vs barycenter
  on a known case, transpose swaps when it reduces crossings,
  transpose leaves already-optimal orderings alone.

### Changed

- Refactored `sort_by_barycenter` into a single `sort_by_metric`
  function that takes a `SortMetric` enum (`Barycenter` |
  `Median`). Same code path, cleaner factoring per the "code as
  art" rule — the median variant added zero duplication.

### Notes

- **No snapshot changes** in the existing gallery — barycenter
  alone (with iterative best-seen retention over 8 passes) was
  already reaching the optimum on every test diagram. Median +
  transpose buy us insurance against future complex graphs that
  barycenter alone would settle into a sub-optimal local minimum.
- This completes the layered-layout improvements series (Phase
  A.1 long-edge waypoints in 0.10.0; Phase A.3 crossing-min
  passes here in 0.10.1; Phase A.2 Brandes-Köpf compaction was
  in scope but the gain on our specific gallery is marginal —
  rolled back into ROADMAP as a deferred polish item).

## 0.10.0 — 2026-04-22

### Added

- **Long-edge waypoint routing** (Phase A.1 of the layered-layout
  improvements series). Edges that span more than one layer in a
  flowchart or state diagram now get per-layer waypoints anchored
  on each intermediate layer's spine, with the perpendicular axis
  (row in LR/RL, column in TD/BT) interpolated between source and
  target and snapped off any real-node row range so the waypoint
  sits in clear space. The edge router segments through these
  waypoints as a chain of short A* runs, giving long edges a
  near-straight channel rather than a detour around intervening
  real nodes.
- **`LayoutResult` struct** as the new `layered::layout` return
  type, carrying both `positions` (the existing per-node grid
  coordinates) and `edge_waypoints` (the new per-long-edge
  waypoint trails). Old callers that destructured the position
  map directly need to swap to `result.positions`.
- **`Grid::set_unless_protected`** complemented by a new public
  `route_via_waypoints` helper in the renderer — the segment-by-
  segment chain builder.
- 5 new unit tests covering: short edges produce no waypoints,
  long edges produce one waypoint per intermediate layer,
  back-edges are skipped (they use perimeter routing),
  `nearest_clear` snap behaviour at edges and ties.

### Changed

- **Public API**: `layered::layout` now returns `LayoutResult`
  instead of `HashMap<String, GridPos>`. `render::render` /
  `render::render_color` gain a fourth parameter
  `&[EdgeWaypoints]`. **Source-breaking** for any external
  consumer; the only known consumer (`markdown-tui-explorer`)
  ships in lockstep at 1.12.0.
- **Refactored** `order_within_layers` and `count_crossings` to
  take an explicit `&[(String, String)]` edge list instead of
  reaching into `graph.edges`. Cleaner separation of concerns and
  paves the way for Phase A.2's barycenter-with-dummies pass.

### Notes

- This is a **0.10.0 minor** because the public layout / render
  signatures changed.
- 3 state-diagram snapshots updated with minor edge-routing
  re-routings (single-character changes, all marginal).
- The visible win is incremental: the dependency-graph gallery
  example's `App→PostgreSQL` long edge now threads through the
  layer columns instead of detouring around the bottom. Phase A.2
  (Brandes-Köpf coordinate assignment) and Phase A.3 (median +
  transpose crossing-min passes) ship in 0.10.x follow-ups and
  will deliver the bigger compaction win.
- Honest scope: this fix addresses the *channel-reservation*
  half of long-edge handling. The full sugiyama benefit needs
  the upcoming Brandes-Köpf compaction.

## 0.9.7 — 2026-04-22

### Changed

- **Sequence-diagram block tags split into two `[…]` brackets**
  matching Mermaid's badge-plus-condition style. The kind name
  (`alt`, `loop`, `opt`, `par`, `critical`, `break`) renders as a
  small corner badge and the opener-branch condition floats beside
  it, separated by `═`. Was `╔═[alt: cache hit]═══╗`; is
  `╔═[alt]══[cache hit]═══╗`. Continuation labels (`else`/`and`/
  `option`) keep their existing `╠┄[label]┄┄┄╣` placement on the
  divider row — same `draw_tag` helper used everywhere for
  consistency.

### Added

- **Bottom participant boxes** in sequence diagrams (Mermaid
  convention — lifelines are bracketed *both* top and bottom).
  `draw_participant_box` now takes a `top_row` parameter and is
  called twice per render (header at row 0, footer at
  `height - BOX_HEIGHT`). Lifelines terminate one row above the
  footer so the box outline reads as a clean bracket.

- **`draw_tag` helper** in `render/sequence.rs` — single source of
  truth for the `[label]` glyph format used by block top borders,
  block branch dividers, and any future label site. Returns the
  column past the tag so callers can chain (e.g. block top border
  draws `[kind]` then `[opener]` adjacent).

### Notes

- All 13 sequence snapshots updated. Every change is either the
  bottom-box mirror (additive) or the new two-tag style (visible
  improvement). No regressions.
- ROADMAP items #4 and #5 from the 2026-04-21 gallery review are
  now retired. Items #6 (sugiyama improvements) and #7 (back-edge
  perimeter routing) remain — both are deeper rework that needs
  its own design pass.

## 0.9.6 — 2026-04-22

### Changed

- **Arrow tips now merge into the destination box border for TD/BT
  flows.** Previously the `▾` (or `▴` for BT and back-edges) sat
  on the row directly above the box's `┌────┐` top border, creating
  a visible gap in TUI display ("arrow not touching the box"). It
  now lands *on* the border row, replacing one `─` with the arrow
  glyph: `┌─▾─┐`. The horizontal border has plenty of `─` cells, so
  dropping one preserves the outline.

  LR/RL flows are unchanged: their box left/right borders are a
  single `│` per row, and replacing it would visibly remove the
  border. Monospace cell-grid rendering already places `▸│` and
  `│◂` adjacent with zero gap, so the merge gain is moot there.

  Back-edge tip placement follows the same rule by perpendicularity:
  LR/RL back-edges enter from the bottom (horizontal `─` border) →
  tip merges in. TD/BT back-edges enter from the right (vertical
  `│` border) → tip stays one column outside.

### Added

- `Grid::set_unless_protected(col, row, ch)` — protection-respecting
  variant of `set` so the box-drawing primitives don't overwrite
  arrow tips that landed on the border in pass 2. The unconditional
  `set` is preserved for primitives that legitimately need to stomp
  (corners, recomputed glyphs).

- New regression-guard snapshot test
  `arrow_tip_merges_into_destination_box_top_td` plus an updated
  `back_edge_lr_exits_bottom` unit test that asserts `▴` lands on
  the same line as `└` (the destination box's bottom border row).

### Notes

- 11 existing flowchart/state snapshots updated — every change is
  a visible improvement (arrows now read as connecting to boxes
  rather than floating above them).

## 0.9.5 — 2026-04-22

### Fixed

- **Edge labels no longer overwrite node border rows or subgraph
  borders.** Two related bugs from the 0.9.4 gallery review:
  - The Supervisor pattern's `panics` label sat on Factory's bottom
    border row between `└` and `┘`, rendering as `└──panics──┘` —
    visually part of Factory rather than labelling the back-edge.
  - State-diagram labels (`EvNumLockPressed`, `start`, `done`,
    `positive`, `negative`, `retry`) frequently landed on adjacent
    nodes' border rows or subgraph border cells, breaking the box
    outlines.

  The label-placement guard now treats node top/bottom border rows
  and subgraph border cells (`╭╮╰╯─│`) as protected regions in
  Pass A. Pass B (last-resort relaxation) still allows them so
  labels never disappear entirely. Two new helpers in
  `render/unicode.rs`: `overlaps_node_border_row` and
  `overlaps_subgraph_border` — 10 new unit tests cover each.

- **TD/BT label candidate range expanded** from 5 row offsets
  (`[0, ±1, ±2]`) to 8 (`[0, ±1, ±2, ±3, -4]`) to match LR/RL.
  Gives the placer more breathing room when Pass A's stricter
  guards filter near positions.

### Notes

- Five existing state-diagram snapshots updated — every change is
  a box-integrity improvement (labels moved away from borders).
- Two new snapshot tests guard against regression:
  `supervisor_bidirectional_in_subgraph` (the `panics` bug) and
  `cicd_parallel_styles_to_same_target` (the parallel-edge case;
  this one stays cramped — the underlying layout-level fix to
  widen gaps for parallel edges is tracked in ROADMAP item #6).

## 0.9.4 — 2026-04-21

### Added

- **`pie` chart support** — first new diagram type since
  `sequenceDiagram` in 0.9.0. Mermaid syntax accepted:
  ```mermaid
  pie [showData] [title <text>]
      "Label1" : 386
      "Label2" : 85
  ```
  - Optional `showData` keyword (case-insensitive) toggles the raw
    value column on each row.
  - Optional `title <text>` rendered centred above the chart.
  - Slice values may be integer or decimal; non-positive values
    are rejected with a clear `Error::ParseError`.
- **Renderer**: horizontal bar chart per slice — much more legible
  in monospace text than any ASCII pie attempt. Bars use `█`
  (filled) and `░` (unfilled); columns are: label, bar, percentage,
  optional value-in-parens. Width auto-scales to the `--width`
  budget (default 80 cols). Integer values render without trailing
  `.0` (`(386)`, not `(386.0)`).
- New public types `PieChart` and `PieSlice` re-exported from the
  crate root. `DiagramKind::Pie` added to the detection enum.
- **4 new snapshot tests** (`pie_minimal`, `pie_with_title`,
  `pie_with_show_data`, `pie_many_slices_with_decimals`) plus 13
  parser unit tests and 6 renderer tests.

### Notes

- Slice colours are deferred: pie renders monochrome in v1. Wiring
  the existing 24-bit ANSI color pipeline through the bar renderer
  would let users `--color` their pies — tracked as a follow-up.
- `gantt`, `journey`, `erDiagram`, and `classDiagram` remain the
  most-requested unsupported diagram types.

## 0.9.3 — 2026-04-21

### Added

- **Sequence-diagram block statements** — the last of the four
  sequence-polish sub-features. Supported kinds:
  - `loop <label>` … `end`
  - `alt <cond>` … `else <cond>` … `end`
  - `opt <label>` … `end`
  - `par <label>` … `and <label>` … `end`
  - `critical <label>` … `option <label>` … `end`
  - `break <label>` … `end`
- **Stack-based parser** with arbitrary nesting and proper
  validation: orphan `end`, continuation keyword inside the wrong
  block kind (e.g. `and` inside `alt`), and unclosed blocks at EOF
  all return clear `Error::ParseError` messages.
- **Renderer**: each block draws as a labelled rectangle using
  heavy double-line glyphs (`╔╗╚╝═║`) — visually distinct from
  participant boxes (`┌┐└┘`) and notes (`╭╮╰╯`). The label tag
  appears inset from the top-left as `[loop: forever]`. Branch
  continuations draw a dashed `╠┄[else: …]┄╣` divider. Nested
  blocks inset by one cell per nesting level so they read
  distinctly. Frame glyphs paint into space / lifeline /
  activation-bar cells only — never overwrite arrow heads or
  message labels.
- **5 new snapshot tests** (`sequence_with_loop_block`,
  `sequence_with_alt_else_block`, `sequence_with_opt_block`,
  `sequence_with_nested_loop_alt`,
  `sequence_with_par_and_critical_blocks`) plus 12 new parser unit
  tests and 2 new helper tests in `parser/common.rs` for
  `block_kind_from_keyword` / `continuation_keyword_for`.

### Notes

- This completes the **sequence-diagram polish series** started in
  0.9.0. All four sub-features (autonumber, notes, activation bars,
  block statements) are now shipped. They compose cleanly: a single
  diagram can mix all four constructs.
- `rect <colour>` background highlight blocks remain silently
  skipped — Mermaid's hex-colour grammar isn't expressible without a
  bigger colour-system rework, and ANSI bg-tinting fights the rest of
  the layered colour rendering. Tracked as a deferred follow-up.
- Single-cell-thick rectangle borders (`║`/`═`) are used because the
  text grid lacks a multi-row filled-block primitive. Real Mermaid
  draws thicker bars; tracked alongside the activation-bar width
  follow-up.

## 0.9.2 — 2026-04-21

### Added

- **Sequence-diagram activation bars** — both forms supported:
  - **Explicit**: `activate X` / `deactivate X` directives.
  - **Inline shorthand**: `+` / `-` on the message target — `A->>+B`
    activates B at the call; `A-->>-B` deactivates the SOURCE at the
    reply, preserving the canonical call/reply pattern
    `A->>+B; B-->>-A`.
- **Stack-based pairing** with per-participant LIFO stacks so nested
  activations on the same participant render as separate `Activation`
  spans. Orphan `deactivate` is a hard `Error::ParseError` with the
  participant name; unclosed `activate` auto-closes at the last
  message (matches Mermaid).
- **Renderer overlay**: heavy `┃` (U+2503) drawn on the participant's
  lifeline column for the duration of each activation span. Skips
  cells already holding arrow / junction glyphs so the bar reads as
  "behind" the arrow. Range includes the activating message's label
  row so single-message activations stay visible.
- **3 new snapshot tests** (`sequence_with_explicit_activation`,
  `sequence_with_inline_call_reply_activation`,
  `sequence_with_nested_activations`) plus 7 new parser unit tests
  and 3 new helper tests for `strip_activation_marker`.

### Notes

- The bar is single-cell-thick (`┃`) — Mermaid renders a wider
  filled rectangle, but that needs a multi-cell primitive. Tracked
  in ROADMAP.
- Activation rendering uses `arrow_row - 1` as the start row to
  ensure visibility when the underlying arrow row is fully
  overwritten by message glyphs.

## 0.9.1 — 2026-04-19

### Added

- **Sequence-diagram notes** — `note left of X : text`,
  `note right of X : text`, `note over X : text`, and the
  multi-anchor span form `note over X,Y : text`. Notes appear at
  their source position (after the preceding message) as rounded
  boxes (`╭─╮ … ╰─╯`) anchored to the relevant participant column;
  multi-anchor notes auto-widen to bridge both columns. The note
  interior is cleared so dashed lifelines don't bleed through.
- **`<br>` and `<br/>` in note text** become real line breaks,
  producing a multi-line note box. Mermaid sequence diagrams have
  no `end note` form (state diagrams do); writing one now returns
  a clear `Error::ParseError` pointing the user at `<br>` instead
  of silently misparsing.
- **3 new snapshot tests** (`sequence_with_note_right_of`,
  `sequence_with_note_over_pair`, `sequence_with_multiline_note`)
  plus 8 new parser unit tests and 5 new helper tests in
  `parser/common.rs` for `parse_sequence_note_anchor`.

### Notes

- Defers width-aware canvas widening (long notes that exceed the
  rightmost participant column are still clipped by the existing
  `Canvas::put_str` bounds check) and word-wrap (use `<br>`).
  Both are tracked in ROADMAP follow-ups.
- Floating notes (`note "text" as N1`) remain silently skipped —
  Mermaid's rendering rules for them are ill-specified upstream.

## 0.9.0 — 2026-04-20

### Added

- **Sequence-diagram `autonumber` directive.** Forms supported:
  bare `autonumber` (start at 1), `autonumber <N>` (start at N),
  and `autonumber off` (halt numbering mid-diagram). Multiple
  directives in one diagram re-base or toggle. The renderer
  prefixes message labels with `[N] ` when numbering is active —
  `[1] POST /order`, `[2] 201 Created`, etc. Notes and block
  markers do not increment the counter (matches Mermaid).
- **Foundation data model for sequence-diagram polish.** New
  public types on `SequenceDiagram`:
  `notes: Vec<NoteEvent>`, `activations: Vec<Activation>`,
  `blocks: Vec<Block>`, `autonumber_changes: Vec<AutonumberChange>`,
  plus their associated enums (`NoteAnchor`, `BlockKind`,
  `BlockBranch`, `AutonumberState`). Only `autonumber_changes` is
  populated in 0.9.0; the others are wired in subsequent 0.9.x
  releases (notes, activations, block brackets — see ROADMAP for
  the planned ordering).
- **Snapshot tests for sequence diagrams** (the first such tests
  in the project). `sequence_minimal` establishes the baseline;
  `sequence_with_autonumber` verifies prefixes; an additional
  test covers the off/re-base flow.

### Changed

- **Lifted `strip_keyword_prefix` into `parser/common.rs`.** The
  sequence and state parsers had drifted-but-equivalent copies;
  the canonical version uses the ASCII-fast path
  (`eq_ignore_ascii_case`) rather than allocating via
  `to_lowercase()`. No behavioural difference for ASCII keywords.
- **Sequence parser now uses shared `strip_inline_comment`** so
  inline `%% comment` after a directive is properly stripped (the
  prior naive `starts_with("%%")` check missed inline cases).

### Notes

- This is the first of several sequence-diagram polish releases.
  Order per ROADMAP: 0.9.0 = autonumber + foundation; subsequent
  0.9.x = notes, activation bars, block brackets. Each lands as
  soon as it's stable.
- Adding fields to `pub` `SequenceDiagram` is mildly
  source-breaking for external consumers that use struct-literal
  construction. The only known consumer (`markdown-tui-explorer`)
  goes through `SequenceDiagram::default()` and is unaffected.
  If you see a "missing field" error after upgrading, switch to
  `SequenceDiagram::default()` and mutate.

## 0.8.1 — 2026-04-20

### Added

- **Notes anchored to states** for state diagrams. Both single-line
  (`note left of X : text`) and multi-line (`note left of X / lines /
  end note`) forms supported, with `left of`, `right of`, and `over`
  positions. Each note synthesises a `NodeShape::Note` (rounded box)
  connected to its anchor by a dotted, no-arrow edge. Position is
  encoded via edge direction so the existing layered layout places
  the note appropriately. Notes inside composite states are
  registered as members of the enclosing composite. Multi-line note
  text is joined with `\n` into the note's label and renders via
  the existing multi-line label path.
- **New `NodeShape::Note` variant** — same dimensions as Rounded;
  the dotted connector visually distinguishes it from regular
  rounded states.
- **New shared helper `parser::common::parse_note_anchor`** plus
  `NoteSide` enum, available to any future diagram parser that
  wants to support Mermaid notes.

### Fixed

- **`rewrite_composite_edges` in the state parser was silently
  resetting edge `style` / `end` / `start` fields** to defaults
  (Solid, Arrow) when rebuilding edges via `Edge::new`. Discovered
  while wiring synthetic note edges (which need Dotted + None
  endpoints). Now preserves all fields via direct struct
  construction. No user-visible change for pre-0.8.1 inputs.

### Notes

- Out of scope (intentional follow-ups, see ROADMAP): real
  dashed-border note shape (custom primitive); `note over X,Y`
  multi-anchor; floating notes (`note "text" as N1` — silently
  skipped); notes for sequence diagrams; `<br>` line-break
  conversion inside note text.
- Layout placement honours the `left of` / `right of` / `over`
  hint via edge direction, but the barycenter heuristic may
  compromise to minimise crossings — the dotted connector keeps
  the relationship readable regardless of distance.

## 0.8.0 — 2026-04-20

### Added

- **`classDef` + `class` directives and `:::className` inline
  shorthand** — both flowcharts and state diagrams. Define a named
  style class once (`classDef cache fill:#234,stroke:#9cf,color:#fff`),
  then apply it across many ids in one statement (`class A,B,C cache`)
  or inline (`A:::cache --> B:::warn`). Inline modifiers stack:
  `A:::base:::overlay` applies both. Forward references work —
  `class A foo` may appear before its `classDef foo …` definition.
- **`Graph::class_defs` and `Graph::subgraph_styles`** — two new
  public registries on `Graph`. `class_defs` holds the parsed named
  palette; `subgraph_styles` holds per-subgraph border colors when
  `class CompositeId styleName` targets a subgraph id.
- **Subgraph border colouring** — the renderer now paints subgraph
  borders with the matched class's `stroke` colour. `fill` and
  `color` for subgraphs are accepted in the schema for consistency
  but only `stroke` is honoured today (filling a composite's
  interior would conflict with inner node fills).
- **`style` and `linkStyle` directives now work for state diagrams**
  too (previously silently skipped — see CHANGELOG 0.4.0 for the
  flowchart story).
- **New `parser/common.rs` module** holding helpers shared by both
  parsers (`strip_inline_comment`, `matches_keyword`,
  `apply_color_pairs`, `parse_node_style_payload`,
  `parse_edge_color_payload`, `extract_class_modifier`,
  `merge_node_style`, `parse_style_directive`,
  `parse_link_style_directive`, `parse_class_def_directive`,
  `parse_class_directive`, `apply_pending_classes`). Eliminates
  prior copy-paste duplication between `parser/flowchart.rs` and
  `parser/state.rs`.

### Changed

- **`NodeShape` and `Graph` gain new public fields.** Adding
  fields to `pub` types is mildly source-breaking for external
  consumers that construct `Graph` via struct-literal syntax. The
  only known consumer (`markdown-tui-explorer`) goes through
  `Graph::new` and is unaffected. If you see a "missing field"
  compile error after upgrading, switch to `Graph::new(direction)`
  and mutate the fields you need.

### Notes

- Class names are matched case-sensitively. Multiple `classDef`
  with the same name are last-wins (matches Mermaid).
- Class application order: `style id …` → all classes from `class`
  / `:::` (in source order) layered on top via `merge_node_style`.
  Per-id `style` provides the base, classes overlay attributes
  they explicitly set; attributes the class doesn't set are
  preserved.
- Out of scope (intentional follow-ups): `classDef DEFAULT`
  special semantics, `click`, sequence-diagram colours,
  subgraph interior fill.

## 0.7.2 — 2026-04-20

### Added

- **`<<choice>>`, `<<fork>>`, `<<join>>` shape modifiers for state
  diagrams** (and the `[[…]]` alternative spellings). Previously
  parsed but silently rendered as plain rounded states; now:
  - `<<choice>>` renders as the existing decision `Diamond` shape.
  - `<<fork>>` and `<<join>>` collapse to a new `NodeShape::Bar`
    variant — visually identical, only the semantic role differs.
- **`NodeShape::Bar(BarOrientation)` variant** with `Horizontal`
  (`━━━`, used in TD/BT flows) and `Vertical` (`┃` stacked, used
  in LR/RL flows) orientations. The orientation is resolved at
  parse time from the graph's flow direction so the bar is always
  perpendicular to flow, matching UML / Mermaid convention.
- **`Grid::draw_horizontal_bar` and `Grid::draw_vertical_bar`** —
  two small public primitives in the `draw_*` family for
  single-row / single-column thick lines. No direction-bit canvas
  integration; bars are static character runs.

### Changed

- **`NodeShape` gains a struct variant.** Adding `Bar(BarOrientation)`
  to a `pub` enum is mildly source-breaking for external consumers
  that exhaustively match on `NodeShape`. The only known consumer
  (`markdown-tui-explorer`) doesn't match on it directly, so no
  observable break in practice. If you see a "non-exhaustive
  pattern" compile error after upgrading, add a new arm for
  `NodeShape::Bar(_)`.

### Notes

- Bar shapes don't render their state ID as a label — drawing
  "ForkPoint" on top of a single `┃` column would be visually
  confusing, and Mermaid's renderer also hides labels for
  fork/join. The state ID is still parsed and addressable from
  edges; it's just not visible inside the bar.
- v1 always uses the top-level graph direction for fork/join
  orientation. Per-composite `direction` overrides changing
  fork/join orientation inside that composite is a follow-up.

## 0.7.1 — 2026-04-20

### Fixed

- **Edge labels no longer overwrite node interiors.** The label
  placement candidate loop only checked against other labels, never
  against node bounding boxes. A label whose preferred position
  happened to fall inside another node would silently overwrite that
  node's content (e.g. a stray `5` from `5 consecutive failures`
  appearing inside the OPEN box of a circuit-breaker FSM). The
  candidate generator now yields 24 LR positions / 15 TB positions
  (extended row offsets ±1..±4 plus 1/3 and 2/3 column anchors), and
  `label_position` runs a two-pass selection: prefer candidates that
  avoid both labels and node interiors; only when none exists, fall
  back to candidates that avoid label collisions but may sit on a
  node border. Universal renderer fix — flowcharts and state diagrams
  with dense edge-label layouts both benefit.

## 0.7.0 — 2026-04-20

### Changed

- **State diagrams now default to `LR` direction** (was `TB` to match
  Mermaid). In a text canvas, TB inserts `layer_gap` (6) blank rows
  between every layer of nodes, so a typical 4-state machine balloons
  into 40+ rows. LR keeps the chain horizontal and the diagram one
  node-height tall, matching how authors would naturally lay out
  state machines for a terminal. Users who want the original Mermaid
  default can still write `direction TB` explicitly.
- Snapshot tests updated to reflect the new default. Diagrams without
  explicit direction render dramatically shorter and wider.

## 0.6.0 — 2026-04-20

### Fixed

- **Back-edge perimeter paths now visibly connect to their source and
  destination boxes.** Previously, back-edges (`C --> A` when `A` is
  upstream of `C`) rendered the horizontal perimeter line and arrow
  tip but left a 1-cell gap between the line and each node's
  bottom/right border, producing a "line coming from nowhere" effect.
  A new post-pass stamps `┬`/`┴` (LR/RL) or `├`/`┤` (TD/BT) junction
  glyphs on the source and destination border cells so the connection
  reads cleanly. Pre-existing since 0.3.0; surfaced frequently in
  state-diagram retry loops.
- **Orphan `[*]` markers from composite-edge rewrites are dropped.**
  When the user writes `Active --> [*]` on a composite whose inner
  flow never transitions to an end marker, the parse-time rewrite
  synthesises an `__end__Active --> __end__` pair that has no
  connection to the composite's real states. Those floating
  double-circles now get garbage-collected before rendering, leaving
  only the markers that visibly participate in the diagram. Applies
  symmetrically to orphaned `__start__` chains.

### Added

- **Composite states for state diagrams** (`state X { … }`). Recursive
  nesting is supported; each composite renders as a rounded rectangle
  enclosing its inner states. The state-diagram parser was refactored
  into a recursive walker that opens a new scope on `state Id {` and
  closes it on `}`. Per Mermaid spec, both `state Id { … }` and
  `state "Display" as Id { … }` openers are accepted.
- **Per-composite `[*]` scope.** Each composite has its own start and
  end markers, mangled as `__start__<ancestor_path>` /
  `__end__<ancestor_path>` so `[*] --> Inner` inside `state Active {…}`
  is a different node from a top-level `[*] --> X`. Top-level marker
  IDs (`__start__`, `__end__`) are preserved exactly to keep
  0.5.0 snapshots byte-identical.
- **External edges to/from composite IDs are rewritten at parse time.**
  `OuterState --> Composite` becomes `OuterState --> __start__Composite`
  (the synthesised inner start marker), and `Composite --> Done`
  becomes `__end__Composite --> Done`. The arrow lands visibly inside
  the composite border on its start/end marker — a sensible
  approximation of Mermaid's "arrow to composite border" rendering.
- **Per-composite `direction` overrides** (`direction LR` inside a
  composite body) populate the existing `Subgraph::direction` field.
- **Stricter error handling** for state diagrams: an unterminated
  composite (`state X { …` with no closing `}`) and a stray `}` at
  top level both return a clear `Error::ParseError` with line number,
  rather than silently consuming or dropping content.

### Notes

- Out of scope (intentional follow-ups): concurrent regions (`--`),
  real shapes for `<<choice>>` / `<<fork>>` / `<<join>>` (still
  silently rendered as plain rounded states), notes (silently
  skipped), `classDef` / `class` / `style` / `click` (silently
  skipped), cross-composite transitions (Mermaid itself doesn't
  support these).
- One-line composite syntax `state X { [*] --> Inner }` is not
  supported — the closing `}` must be on its own line.

## 0.5.0 — 2026-04-19

### Added

- **`stateDiagram` and `stateDiagram-v2` support.** Both keywords share the
  same grammar in upstream Mermaid; we accept either. State diagrams are
  parsed into the existing flowchart `Graph` type and ride the same
  layered layout, A* edge routing, ANSI color, ASCII fallback, and
  `--width` compaction pipeline as flowcharts. No CLI changes — pass a
  `.mmd` containing a state diagram to `mermaid-text` and it Just Works.
- Supported syntax (the "Always" / "Common" tiers per the Mermaid spec):
  `[*]` start (rendered as a small `Circle` with `●`) and end (rendered
  as a `DoubleCircle` with `●`); `A --> B` and `A --> B : label`
  transitions including self-transitions; `STATE : description` lines
  that accumulate into a multi-line label; `state "Display Name" as Id`
  with `\n` line-break support; bare `state Id` declarations;
  `direction LR/TB/BT/RL`; `%%` comments; colons inside labels and
  descriptions.
- `'●' → '*'` mapping in `to_ascii` so state diagrams compose with `--ascii`.

### Notes

- Out of scope (intentional follow-ups): composite states `state X { … }`
  return a clear parse error; concurrent regions (`--`); `<<choice>>` /
  `<<fork>>` / `<<join>>` shape modifiers (silently treated as plain
  states for now); notes (silently skipped); `classDef` / `class` /
  `style` / `click` (silently skipped); `accTitle`, `accDescr`, `scale`,
  `hide empty description`.
- The default direction for state diagrams is `TB` (top-to-bottom),
  matching upstream Mermaid. Add `direction LR` near the top of the
  source for a wider, shorter layout.

## 0.4.0 — 2026-04-19

### Added

- **ANSI 24-bit color output (opt-in)**. Mermaid `style <id>
  fill:#…,stroke:#…,color:#…` and `linkStyle <indexes>
  stroke:#…[,color:#…]` directives — previously silently ignored — are
  now parsed and emitted as truecolor SGR sequences when color output is
  requested. New CLI flag `--color` / `-c` and a new public entry point
  `render_with_options(input, &RenderOptions { color: true, .. })` opt
  into the new behaviour. The default behaviour of every existing entry
  point is unchanged: zero ANSI bytes, byte-identical to v0.3.x.
- New public `RenderOptions { max_width, ascii, color }` struct with a
  `Default` impl that matches the historical `render` behaviour.
- New public types `Rgb`, `NodeStyle`, `EdgeStyleColors` plus
  `Graph::node_styles` / `Graph::edge_styles` registries that hold the
  parsed color metadata.

### Notes

- `--color` composes with `--ascii`: ANSI escapes are pure ASCII and
  pass through the box-drawing-to-ASCII conversion untouched, useful for
  terminals that strip Unicode but speak ANSI.
- Out of scope for this release (intentional follow-ups): `classDef` /
  `:::className` shorthand, subgraph border styling, 256-color and
  16-color fallbacks, and color support in `sequenceDiagram` rendering.

## 0.3.4 — 2026-04-18

### Fixed

- `sequenceDiagram` self-messages (`A->>A: text`) now read as visually
  attached to their source lifeline. Previously the loop's top and bottom
  legs started with the dashed lifeline char `┆`, which on most fonts
  doesn't visually join the solid horizontal `─` — users reported the
  loop looked like a floating box disconnected from the lifeline.
  Replaced the lifeline cell at both leg rows with `├` (solid T-junction
  branching right) and moved the return arrow `◂` one cell inward so
  both connection points read clearly.

## 0.3.3 — 2026-04-18

### Fixed

- `sequenceDiagram` rendering no longer over-widens gaps between
  participants. v0.3.2 fixed box-overlap by switching `MIN_GAP` to an
  edge-to-edge measure but left the value at 6, producing 200-300 column
  diagrams that wrapped inside the markdown-reader viewer pane.
  Reduced `MIN_GAP` to 2 (just enough breathing room for edges to not
  touch) and tightened the per-label padding from 6 to 2. Multi-span
  message labels now drive gap width via `(label + 2) / spans` instead of
  `(label + 6) / spans`.

## 0.3.2 — 2026-04-18

### Fixed

- `sequenceDiagram` participant boxes no longer visually overlap when two
  adjacent participants have wide labels. `MIN_GAP` was interpreted as a
  centre-to-centre distance, so boxes wider than half that gap had their
  borders touching or overlapping. It is now the minimum clearance between
  adjacent box *edges*; the centre-to-centre distance adds half of each
  neighbor's width on top.
- `sequenceDiagram` self-messages (`A->>A: text`) no longer collide with the
  following message's label. The self-loop renders as a two-leg box (top
  leg + bottom leg) but the previous row-budget treated it as a single-row
  event, so the next message's label landed on the self-loop's bottom leg.
  Self-messages now consume 3 rows; the next message starts on the row
  after.

## 0.3.1 — 2026-04-18

### Fixed

- `sequenceDiagram` parser no longer errors on `else`, `and`, and `option`
  block keywords. The v0.3.0 parser skipped block-opening keywords
  (`alt`, `loop`, `opt`, `par`, etc.) and their closing `end`, but missed
  the separators (`else`, `and`) and `option`, which caused real-world
  diagrams using `alt … else … end` to return `ParseError` and the
  markdown-reader TUI to fall back to raw source display. Inner messages
  inside these blocks are preserved.

## 0.3.0 — 2026-04-18

### Added

- **`sequenceDiagram` support (MVP)** — `sequenceDiagram` with
  `participant`/`actor` declarations (optional `as Alias`) and message
  arrows (`->>`, `-->>`, `->`, `-->`) now renders as a sequence diagram
  with participant boxes across the top, dashed `┆` lifelines, and
  horizontally-drawn message arrows. Block statements, activation bars,
  notes, and `autonumber` are parsed-and-skipped with TODO markers.
- **ASCII rendering mode** — `render_ascii(input)` and
  `render_ascii_with_width(input, max_width)` produce output in which every
  character is in the ASCII range.  The Unicode renderer runs unchanged; a
  post-processing pass (`to_ascii`) substitutes each box-drawing or arrow glyph
  with a plain ASCII equivalent (`+`, `-`, `|`, `>`, `<`, `v`, `^`, `*`, `o`,
  `x`).  All three functions are exported at the crate root.
- **`--ascii` CLI flag** — `mermaid-text --ascii [--width N] [FILE]` invokes
  `render_ascii_with_width` instead of `render_with_width`.  `--help` text
  updated.
- **Back-edge perimeter routing** — edges that travel against the primary flow
  direction (e.g. `W --> F` in an `LR` graph where `F` is upstream of `W`)
  now exit the source from the perpendicular side (bottom for `LR`/`RL`,
  right for `TD`/`BT`), travel along a corridor outside the node grid, and
  enter the target from the same perpendicular side.  The arrow tip reflects
  the entry direction (`▴` for `LR`/`RL` back-edges, `◂` for `TD`/`BT`).
  This prevents back-edges from cutting through the middle of the diagram and
  makes feedback loops (circuit breakers, supervisor/worker restart arcs)
  visually distinct from forward edges.

## 0.2.5 — 2026-04-18

### Fixed

- Edges crossing subgraph borders no longer collapse onto a single trunk.
  Subgraph borders were previously marked as hard `NodeBox` obstacles,
  which forced A* to give up on any edge whose source or destination
  lived inside a subgraph — the fallback Manhattan router then drew
  straight lines ignoring every obstacle, visually stacking all edges
  onto one column. Borders are now only *protected* (so their glyph
  survives edge routing) and A* can route through them, producing
  distinct parallel corridors per edge.

## 0.2.4 — 2026-04-18

### Fixed

- Cylinder shape (`[(label)]`) now renders as a continuous rounded
  rectangle with a horizontal T-junction "lid line" below the top
  border. The previous double-arc design rendered as three detached
  boxes in monospace fonts (top arc + text + bottom arc with no side
  walls connecting them). Height reduced from 5 to 4 rows for the base
  case; multi-line labels still add rows in the middle.

## 0.2.3 — 2026-04-18

### Added

- Multi-line node labels. The parser now converts HTML line-break tags
  (`<br/>`, `<br>`, `<br />`, case-insensitive) into `\n` separators, and
  the renderer draws each segment on its own row inside the node box.
  Example: `A[first<br/>second<br/>third]` renders as a 3-row-tall box.
- Automatic soft-wrap for long label lines. Any single line wider than
  40 terminal cells is wrapped at the last comma or space within the
  budget. Single long words without separators (e.g. a long identifier)
  stay unwrapped to avoid mangling. This prevents large node labels from
  stretching the whole diagram horizontally off-screen.

## 0.2.2 — 2026-04-17

### Fixed

- Sibling subgraphs at the same nesting level no longer overlap each other.
  `layout::layered::compute_positions` now widens the gap between two
  adjacent same-layer nodes by `SG_BORDER_PAD + 1` cells per subgraph
  boundary that separates them, so each subgraph's border line and padding
  fit without colliding with the neighboring subgraph.

## 0.2.1 — 2026-04-17

### Changed

- `repository` and `homepage` metadata now deep-link to the crate's
  subdirectory in the monorepo so crates.io users land at the right path.

## 0.2.0 — 2026-04-17

First fully-featured release. Complete rewrite of the rendering pipeline
and a fleshed-out flowchart feature set.

### Added

- **Flowchart parser** — hand-rolled parser for `graph`/`flowchart` syntax
  supporting both newline and semicolon statement separators.
- **All twelve node shapes** — Rectangle, Rounded, Diamond, Circle, Stadium,
  Subroutine, Cylinder, Hexagon, Asymmetric, Parallelogram, Trapezoid,
  Double-circle.
- **All edge styles** — Solid (`-->`), plain (`---`), dotted (`-.->` / `-.->`),
  thick (`==>`).
- **All edge endpoint types** — Arrow, None (plain line), Circle (`--o`),
  Cross (`--x`), Bidirectional (`<-->`).
- **Edge labels** — pipe-style (`-->|label|`) and dash-style (`-- label -->`).
- **Subgraphs** — nested subgraphs with rounded-corner borders and labels;
  edges may freely cross subgraph boundaries.
- **Per-subgraph `direction` override** — perpendicular subgraph directions
  (e.g. `direction LR` inside a `graph TD` parent) cause the subgraph's
  nodes to be collapsed onto a single parent layer.
- **Sugiyama-inspired layered layout** — longest-path layer assignment,
  iterative barycenter heuristic (8 passes, 4-pass early-exit, best-seen
  retention), and configurable `layer_gap`/`node_gap` spacing.
- **A\* obstacle-aware edge routing** — hard obstacles (node boxes, subgraph
  borders), soft obstacles (existing edge cells), and a corner penalty to
  prefer straight paths.
- **Direction-bit canvas** — 4-bit per-cell direction mask with a lookup
  table that produces correct junction glyphs (`┼ ├ ┤ ┬ ┴`) automatically.
- **Width-constrained rendering** — `render_with_width(src, Some(n))` tries
  progressively more compact gap configurations until the output fits within
  `n` columns.
- **CLI binary** — `mermaid-text [--width N] [FILE]` reads Mermaid from
  stdin or a file and prints the rendered diagram.
- **Rustdoc** — full doc comments and runnable doctests on all public items.
- **README.md** and **CHANGELOG.md** at the crate root.

## 0.1.0

Initial crate placeholder. Superseded by 0.2.0.
