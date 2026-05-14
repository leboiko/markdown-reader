# Roadmap

Tracked list of planned features for `markdown-tui-explorer` (the TUI app)
and `mermaid-text` (the standalone library). Items here are
**considered** or **in progress**; shipped work moves to the relevant
crate's `CHANGELOG.md`.

When picking what to work on next: skim this file first. When agreeing
on a new feature, add it here. When shipping, remove the entry (and let
the CHANGELOG be the historical record).

---

## Recently shipped (concise pointers — see CHANGELOGs for detail)

High-level summary of major projects/features shipped recently. The
CHANGELOGs in `CHANGELOG.md` and `crates/mermaid-text/CHANGELOG.md` are
the historical source of truth.

- **Hybrid live-preview editing** — Obsidian-style block-level reveal,
  9 sub-phases shipped in `markdown-tui-explorer` 1.29.1 → 1.33.0; the
  per-element granularity follow-up landed in 1.34.0.
- **CLI / app features** — HTML export, link validator (`--check-links`),
  outline navigator (`o`), stdin support, `--section` extraction,
  pre-built binaries via GitHub Actions release pipeline.
- **Mermaid diagram types added** — `journey` (0.19.0), `gantt` (0.20.0),
  `gitGraph` (0.21.0), `timeline` (0.22.0), `classDiagram` (0.16.0),
  `pie` (0.9.4), `erDiagram` Phases 1–3 (0.11.0 → 0.23.0 incl. grid
  layout).
- **Mermaid render polish** — edge-label midpoint placement (LR/RL),
  `classDef DEFAULT` base-class semantics, anonymous `<<choice>>` label
  hiding, `<<fork>>` / `<<join>>` wider bars, width-budget label
  wrapping (0.28.0), parallel-edge widening (0.12.0–0.13.0),
  bidirectional edge label space (0.9.5), arrow-tip merges into
  destination box (0.9.6), bottom participant boxes + Mermaid-style
  sequence-block tags (0.9.7), back-edge perimeter routing via
  `Obstacle::InnerArea` (0.11.2).
- **Mermaid layout engine** — Sugiyama backend default (0.17.0) with
  long-edge waypoints (0.10.0), median + transpose crossing-min (0.10.1).
- **Mermaid routing-attach bug fixes** — B7 (0.19.1), B9 (0.27.2),
  B12 (0.27.3), B3 (0.28.1), subgraph-title pierce (0.28.2). Routing
  regression-test harness landed in 0.27.1.
- **Mermaid bug audit** — gitGraph fork/merge arc connector,
  erDiagram inline attribute syntax, ER spine, LR labels (0.27.0).
- **18 Mermaid diagram types covered** — beyond the original 10, added
  `mindmap` (0.27.0), `quadrantChart` (0.29.0), `requirementDiagram`
  (0.31.0), `xychart-beta` (0.32.0), `block-beta` (0.34.0), `sankey-beta`
  (0.35.0), `packet-beta` (0.37.0), `architecture-beta` (0.38.0).
- **Beta diagram visual upgrades (April 30, 2026)** — architecture-beta
  Path A spatial-edge routing via the flowchart Sugiyama pipeline (0.40.0),
  sankey-beta proportional bars with sub-cell precision (0.41.0),
  block-beta inline spatial edges in grid gaps with adjacency detection
  (0.42.0). Three of the four "structured-text" beta types upgraded to
  actual visual diagrams in one session.
- **ER polish (April 30, 2026)** — cross-row spine connects rightmost-in-row
  entities (0.39.2), cross-row labels stagger across gap rows to avoid
  collision (0.39.3).
- **App stability (April 30, 2026)** — xychart-beta x-axis label
  alignment (0.39.1), config-popup mouse-scroll passthrough guard
  (1.34.39), applying-theme preserves viewer cursor and scroll
  (1.34.40), mermaid heights preserved across cache invalidation so
  the cursor doesn't jump during theme refresh (1.34.41).
- **Sequence-diagram note polish (April 29, 2026)** — automatic
  word-wrap for long notes plus width-aware canvas widening for
  unbreakable words (0.39.0).
- **Version-check on exit + release-pipeline patch** — quit-time crates.io
  upgrade banner (1.34.34), `release.yml` `workflow_dispatch` back-fill
  path fixed so manual tag rebuilds actually produce binaries (1.34.43).
- **Block-frame interior fill primitive (May 8, 2026)** — `loop`/`alt`/`opt`/
  `par`/`critical`/`break` block-frame interiors now filled with `░`
  (U+2591) on otherwise-empty cells (mermaid-text 0.54.0).
- **`rect <colour>` background highlight blocks (May 11, 2026)** —
  `rect rgb(R,G,B)` and `rect rgba(R,G,B,A)` in sequence diagrams rendered
  as borderless fills with luminance-keyed 3-step shade palette (mermaid-text
  0.55.0).
- **Sequence-diagram polish basket (May 11, 2026)** — Self-message U-shape
  arrows, stacked nested activations (outer anchors at lifeline, inner
  offsets right), and `box [colour] "label" ... end` participant grouping
  with optional CSS colour (mermaid-text 0.56.0).
- **Text-backend selector exposed in config (May 14, 2026)** — The
  `mermaid_text_backend` config field (Sugiyama default, opt-in Native,
  new opt-in `Auto`) surfaces in `c` settings. `Auto` resolves to Native
  only for the documented Sugiyama coverage gap (subgraph with inner
  `direction` override), and to Sugiyama everywhere else. Scope and
  shipping notes in `docs/scope-mermaid-backend-selection.md`
  (markdown-tui-explorer 1.34.70).
- **Composite-edge attach-to-border (May 8, 2026)** — Edges from outside
  a composite state attach to the composite's outer rectangle border
  instead of the synthesised inner marker, via render-time intercept
  (`compute_externally_attached_markers` + `endpoint_geom` redirect to
  `SubgraphBounds`). Parser rewrite kept for layered-layout coherence
  (mermaid-text 0.53.0).
- **xychart-beta mixed-width label centering (May 7, 2026)** — Labels of
  mixed display width (e.g. `c0..c9` then `c10..c14`) are pre-padded to
  the maximum width before slot centering, eliminating ±1 cell drift
  from integer-division (mermaid-text 0.52.0).

---

## Next up

### Quick wins (under a session each)

_(Empty — most recent quick-wins shipped in 1.34.x. Per-composite
direction-aware fork/join orientation was already delivered in
mermaid-text 0.30.0; the regression tests live at
`crates/mermaid-text/src/parser/state.rs`
(`fork_inside_tb_composite_in_lr_diagram_uses_horizontal_bar` /
`fork_at_top_level_lr_diagram_keeps_vertical_bar`).)_

### Medium projects (1-3 sessions)

_(Empty — composite-edge attach-to-border, the sequence-diagram polish
basket, and xychart-beta mixed-width label centering all shipped between
0.52.0 and 0.56.0. Pick from "Bigger ideas" below or surface a new
feature from CHANGELOG follow-ups.)_

---

## Bigger ideas (multi-session)

- **Concurrent regions `--` (state diagrams)** — `state X { region1; --;
  region2 }` for orthogonal sub-state-machines. Needs a new layout
  primitive ("two layouts side-by-side in one container"). The hardest
  item on this list — deserves its own design pass. Multi-day.
- **True proportional sankey** — current 0.41.0 is per-source
  bars-next-to-flows. The "real" version stacks node bars with heights
  proportional to total inflow, draws bands as filled stripes between
  layers (computed by Sugiyama-style layering with band-crossing
  minimisation), and conserves flow visually. 2-3 sessions.

---

## Deferred / parked

- **architecture-beta Path B — port-aware edge attachment** — `ArchEdge`
  carries `source_port` and `target_port` (`L`/`R`/`T`/`B`) that indicate
  which side of the service box each edge must attach to. Path A (mermaid-text
  0.40.0) stores these but ignores them, letting the Sugiyama router choose
  attach points freely. Path B would add a constrained attach-point mode to
  the A\* router so that e.g. `db:L -- R:server` reliably exits the left face
  of `db` and enters the right face of `server`. This requires non-trivial
  changes to `layout/router.rs` and `layout/grid.rs`; deferred until a
  real-world architecture diagram visibly suffers from unconstrained routing.

- **Brandes-Köpf compact coordinate assignment** — Phase A.2 of the
  layered-layout improvements series. Deferred because on the current
  gallery, our existing positioning (Sugiyama default since 0.17.0,
  median + transpose crossing-min from 0.10.1) is already near-optimal.
  Brandes-Köpf's compaction win would be marginal for the small graphs we
  typically render. Pick up when a real-world large/wide flowchart shows
  the layout visibly too sprawly. Reference implementation:
  `rust-sugiyama`'s ~300-line port; algorithm in Brandes & Köpf (2002),
  "Fast and Simple Horizontal Coordinate Assignment".
- **Subgraph interior fill** — Today only `stroke` is honoured for
  subgraph styles (border colour); `fill` and `color` are accepted in
  the schema but not rendered. A real "fill the composite interior with
  a tint" pass would conflict with inner node backgrounds — needs a
  layered-paint design (paint subgraph fill first, then node fills
  overlay). Defer.
- **Real dashed-border note shape** — v1 of notes uses a solid rounded
  box; the dotted connector distinguishes it from regular states. A real
  Mermaid-style note would have a dashed border too. Needs a new
  `Grid::draw_note_box` primitive mixing rounded corners with dotted
  top/bottom and dotted vertical sides. Add later if anyone asks.
- **`note over X,Y` multi-anchor (state diagrams)** — Mermaid's `note
  over X,Y` spans two anchors. v1 silently skips multi-anchor forms.
  Adding it needs either a new "spanning" edge that anchors to multiple
  targets, or a renderer pass that draws two separate dotted lines from
  one note. Defer.
- **Floating notes (`note "text" as N1`)** — Mermaid's no-anchor form.
  Rendering is ill-specified upstream; defer until someone files a real
  use case.
- **Sugiyama sub-phase 4b** — When upstream `ascii-dag` adds proper
  `level_spacing` / `node_spacing` config support, remove our
  `extra_per_layer` post-pass and let ascii-dag control spacing
  directly. Tracked against the upstream crate.
- **Sugiyama sub-phase 6** — Per-edge effective-direction tracking for
  parallel-edge widening inside orthogonal-override subgraphs; also
  fixes long edge-label placement when A→B runs through a tight
  subgraph (the `wrapped_edge_label` regression noted in 0.17.0).
  Requires per-node effective-direction tracking that is non-trivial to
  thread through the current pipeline.
- **block-beta Tier 2 — Manhattan routing for non-adjacent edges** — 0.42.0
  ships Tier 1 (adjacent inline arrows) + Tier 3 (text fallback for
  non-adjacent). Tier 2 would route non-adjacent edges through the gap rows
  and columns using `─ │ ┌ ┐ └ ┘` corner glyphs with crossing handling.
  Deferred because it requires a 2-D character canvas pass and risks
  Bucket-C regressions on existing block-beta diagrams. Pick up if users
  report non-adjacent edges are confusing in the text summary.
