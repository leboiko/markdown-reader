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

---

## Next up

### Quick wins (under a session each)

- **`--check-external` HTTP link checker** — flag exists today as a
  stub (see `src/checklinks/mod.rs:118` and the doc comment at
  `src/checklinks/mod.rs:436`). Wire up an actual HTTP HEAD/GET probe
  with concurrency cap, timeout, and configurable retry.
- **Per-composite direction-aware fork/join orientation** — `mermaid-text`
  currently resolves fork/join shape from the top-level graph direction
  (since 0.7.2). A composite with `direction TB` inside an LR top-level
  diagram gets the wrong orientation. Fix needs the parser to track each
  fork/join's enclosing composite path and resolve orientation from the
  nearest composite. Small change, low priority unless someone hits it.

### Medium projects (1-3 sessions)

- **Composite-edge attach-to-border (state diagrams)** — Today
  `Composite --> X` is rewritten at parse time to point at the composite's
  synthesised inner `[*]` end. Works, but the arrow lands on the inner
  marker rather than the composite border. A renderer extension that lets
  edges target subgraph IDs (currently silently dropped — see
  `crates/mermaid-text/src/render/unicode.rs:477`) would let the arrow
  attach to the border like Mermaid's own renderer does. Needs new
  edge-routing logic.
- **Sequence-diagram polish follow-ups** — All small, all gated on user
  reports:
  - **Width-aware canvas widening** when a note (or block label) exceeds
    the rightmost participant column. Today the content clips silently
    at the canvas right edge.
  - **Word-wrap for long note lines.** Today users add `<br>` manually.
    Auto-wrap would need a width budget per anchor type.
  - **Wider activation bars / block-frame fills** — both currently render
    single-cell-thick borders. A real "filled thick bar / rectangle"
    needs a multi-row block-fill primitive.
  - **`rect <colour>` background highlight blocks** — Mermaid's grammar
    can't express hex colours easily, and ANSI bg-tinting fights the
    layered colour system; defer until a clear request comes in.

---

## Bigger ideas (multi-session)

- **Concurrent regions `--` (state diagrams)** — `state X { region1; --;
  region2 }` for orthogonal sub-state-machines. Needs a new layout
  primitive ("two layouts side-by-side in one container"). The hardest
  item on this list — deserves its own design pass. Multi-day.
- **More Mermaid diagram types** — In rough order of usefulness:
  `mindmap`, `requirementDiagram`, `quadrantChart`, `sankey-beta`,
  `block-beta`, `xyChart-beta`, `architecture-beta`, `packet-beta`. Each
  is its own decent chunk of work; pick up by demand. Multi-day each.
  (Note: a parallel agent is currently implementing `mindmap`.)

---

## Deferred / parked

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
