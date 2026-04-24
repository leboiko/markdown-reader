# Roadmap

Tracked list of planned features for `markdown-tui-explorer` (the TUI app)
and `mermaid-text` (the standalone library). Items here are
**considered** or **in progress**; shipped work moves to the relevant
crate's `CHANGELOG.md`.

When picking what to work on next: skim this file first. When agreeing
on a new feature, add it here. When shipping, remove the entry (and let
the CHANGELOG be the historical record).

---

## In progress

_Nothing actively in progress._

---

## Next up (ordered roughly by ROI)

### Mermaid rendering bugs found in 2026-04-24 gallery audit (remaining)

Audit of 43 charts surfaced 12 bugs + 3 missing-test patterns.
Phase 1 (B1+B2+S1) and Phase 2 (B4+B6+B10) shipped in
mermaid-text 0.16.4 / parent 1.26.1. Remaining items in priority
order:

**Subgraph-border + edge-label cluster (likely shared root cause):**

- **B8** `crates/mermaid-text/src/layout/grid.rs` — Edge labels in
  the README Supervisor chart bleed through subgraph walls
  (`│└─────creates`, `│┌────panics┼┘`, `│▸│ Worker ││      beat│`).
  Cells adjacent to subgraph borders aren't cleared/padded.
- **B11** `…/grid.rs` — Wrapped multi-line edge labels escape the
  subgraph right border (Personal_05). Likely same root cause as B8.
- **B5** `…/grid.rs` — Cross-subgraph edge label writes into the
  closing `╰─╯` of a subgraph. No guard for "this cell is a
  subgraph border."

One careful pass may fix all three. Add snapshots for S3
(cross-subgraph label placement) afterwards.

**Subgraph layout (its own design pass):**

- **B7** `…/render/flowchart.rs` or `…/layout/grid.rs` — Adjacent
  sibling subgraphs in TB layout collide on the same `y` row when
  combined width approaches terminal width (Personal_01, Personal_10).
  Add snapshot for S2 once fixed.

**Route-attach issues (defer — high regression risk):**

- **B12** `…/layout/route.rs` — Back-edge source-attach pierces the
  bottom of a rounded box (`╰─────────┬────────╯`).
- **B9** `…/layout/route.rs` — Back-edge route deposits `├` on the
  right wall of the `Idle` state box.
- **B3** `…/layout/route.rs` — `App` box top border broken in the
  dependency graph (`┌─────┐────┐`); edge exits through box top row.

⚠️ **All three touch the source-attach anchor logic that took 3
iterations to stabilize in 1.22.x.** Defer until someone can carefully
review against past iterations + add a focused regression-test plan
before touching the routing code.

**Suggested attack order for remaining work:**

1. **B8+B11+B5** (label-vs-subgraph-border cluster).
2. **B7** (subgraph crowding).
3. **B3+B9+B12** (route-attach trio — careful review needed).

### Pie chart slice colours

`pie` charts ship monochrome in 0.9.4. Wiring the existing 24-bit
ANSI colour pipeline through `render::pie::render` would let users
`--color` their pies (auto-assign distinct colours per slice, or
honour an explicit `pieDef <name> fill:#…` directive if Mermaid
adopts one). Small extension once someone asks.

### Sequence-polish follow-ups (deferred)

The four-part sequence-diagram polish series shipped over 0.9.0–
0.9.3. Smaller follow-ups still on the table:

- **`rect <colour>` background highlight blocks** — Mermaid's
  grammar can't express hex colours easily, and ANSI bg-tinting
  fights the layered colour system; defer.
- **Wider activation bars / block-frame fills** — both currently
  render single-cell-thick borders. A real "filled thick bar /
  rectangle" needs a multi-row block-fill primitive (also wanted
  for the fork/join wider-bars roadmap entry below).
- **Width-aware canvas widening** when a note (or block label)
  exceeds the rightmost participant column. Today the content
  clips silently at the canvas right edge. Defer until someone
  reports clipping.
- **Word-wrap for long note lines.** Today users add `<br>`
  manually. Auto-wrap would need a width budget per anchor type.
- `note over X,Y,Z` (3+ anchor) — Mermaid's grammar doesn't
  actually support this; out by spec.

---

## Bigger ideas

### Concurrent regions `--` — `mermaid-text`

State-diagram `state X { region1; --; region2 }` for orthogonal
sub-state-machines. Needs a new layout primitive ("two layouts
side-by-side in one container"). The hardest item on this list —
deserves its own design pass. Multi-day.

### `erDiagram` — `mermaid-text`

Most-requested missing diagram type for documentation use. Entities
with attribute lists, relationship cardinalities (`||--o{`),
foreign-key arrows. Substantial — entity boxes have internal
structure (header + attribute rows), and the relationship notation
is its own mini-language. Multi-day.

### `gantt` / `journey` / `classDiagram` — `mermaid-text`

Each is its own decent chunk of work. Lower priority than the above
unless someone asks. Multi-day each.

---

## Quality / polish backlog

### erDiagram grid layout (deferred from Phase 3)

erDiagram ships in 0.11.0 / 0.11.1 with a single-row source-order
layout. For diagrams with >5 entities the row gets too wide for a
typical 80-column terminal. The planned Phase 3 (a
`ceil(sqrt(n))`-column grid layout) was deferred because
cross-row relationship routing is substantially more complex than
the visible win justifies — the single-row layout already works
cleanly when the terminal is wide enough, and users with large
ER diagrams can widen their terminal or scroll horizontally.

Pick this up when someone reports the wide-ER problem with a real
schema. Reference: `layout::subgraph` already handles nested
bounding boxes; a similar per-row "sub-grid" abstraction could
carry the box coordinates through the current single-row renderer.

### Brandes-Köpf coordinate assignment (deferred from Phase A.2)

The layered-layout improvements series planned a Phase A.2 to
replace `compute_positions`'s evenly-spaced layout with
Brandes-Köpf's compact placement. The 2026-04-22 ship cycles for
Phase A.1 (long-edge waypoints, 0.10.0) and Phase A.3 (median +
transpose crossing-min, 0.10.1) revealed that on the current
gallery, our existing positioning is already near-optimal —
Brandes-Köpf's compaction win would be marginal for the small
graphs we typically render.

Defer until: a real-world large/wide flowchart shows the layout
visibly too sprawly. Reference implementation: `rust-sugiyama`'s
~300-line port, readable Rust. The full algorithm is documented
in Brandes & Köpf (2002), "Fast and Simple Horizontal Coordinate
Assignment".



### Rendering issues found in the 2026-04-21 gallery review

Reviewing `docs/mermaid-gallery.md` against GitHub's native Mermaid
rendering surfaced a cluster of rendering-quality issues. Grouped
by root cause and ordered by ROI (small-targeted-fix first, deep
rework last). Most of these were discovered together and share
some underlying machinery, so picking the right order of attack
matters.

#### 1. Bidirectional edges share label space — **SHIPPED IN 0.9.5** ✓

**Symptom (Supervisor pattern):** `F[Factory] -->|creates| W[Worker]`
plus `W -->|panics| F` rendered as `└──panics──┘` *inside*
Factory's bottom border, with `creates` bleeding through the
subgraph border.

**Fix shipped:** label-placement Pass A now treats node top/bottom
border rows AND subgraph border cells (`╭╮╰╯─│`) as protected
regions. Two new helpers in `render/unicode.rs`:
`overlaps_node_border_row` and `overlaps_subgraph_border`.
Pass B (last-resort relaxation) still allows them so labels
never disappear. Five existing state-diagram snapshots updated
with box-integrity improvements; 10 new unit tests + 2 reproducer
snapshots (`supervisor_bidirectional_in_subgraph` and
`cicd_parallel_styles_to_same_target`).

#### 2. Multiple labelled edges between same node pair — **SHIPPED IN 0.12.0 + 0.13.0** ✓

**What 0.9.5 first did:** subgraph borders stopped getting
punctured — in the CI/CD case `pass` landed at col 41
(immediately right of CI's `│` at col 40) instead of overwriting
the border. Box outlines stayed intact, but labels still glued
to each other.

**What 0.12.0 fixes (full fix):** `Graph::parallel_edge_groups()`
detects edges sharing an unordered endpoint pair (so `A→B` and
`B→A` group together). The layered layout's `label_gap` then
widens the inter-layer gap by `(N − 1) × (max_label_width + 2)`
when N parallel labels cross it — each label gets its own row
(LR/RL) or column (TD/BT). No path-routing changes were needed
("Phase 2a" of the parallel-edges scope doc).

Visible wins: CI/CD pipeline `pass`/`skip` now stack on adjacent
rows; CircuitClosed↔CircuitOpen state diagram has clear breathing
room between `5 errors` and `timeout reached` labels;
`done`/`task` style transitions in composite states render
cleanly. Four existing snapshots updated, all visual improvements.

#### 3. Arrow termination doesn't visually merge with destination box — **SHIPPED IN 0.9.6** ✓

**Symptom (state machine):** `▾` lands on the row directly above
each destination box's top border. In raw text they're adjacent;
in TUI display the line-height creates a visible gap, especially
when back-edge perimeter columns insert vertical `│` lines that
fragment the destination row visually.

**Root cause:** `draw_arrow` terminates the arrowhead one row
above the destination box. Convention is correct but reads as
"floating" in proportional-line-height contexts.

**Fix scope:** change arrow termination to land *on* the
destination box's top border, replacing one `─` with `▾`:
```
   │              │
   │   instead    │
   ▾   of         ▾   →  ┌──▾──┐
┌─────┐           ┌─────┐│ Box │
│ Box │           │ Box │└─────┘
└─────┘           └─────┘
```

**Why ★★:** small change, makes every arrow read more clearly,
benefits every diagram type. Risk: changes existing snapshots
broadly — need to bulk-update.

#### 4. Sequence-block tag style mismatches Mermaid — **SHIPPED IN 0.9.7** ✓

**Symptom (sequence with alt block):** we render
`╔═[alt: cache hit]═══...═╗` (kind name and first-branch label
combined inside one bracket). Real Mermaid renders the kind name
as a separate small tag in the top-left corner with the condition
label floating inside the box:
```
Real Mermaid:        Ours:
┌ alt ┬─────────┐    ╔═[alt: cache hit]═══════╗
│ ··· [cache hit] │   ║   …                    ║
│ ········  ····· │   ╠┄[cache miss]┄┄┄┄┄┄┄┄┄┄┄╣
│ ········· ····· │   ║   …                    ║
│       [cache miss] │ ╚═══════════════════════╝
│ ········  ····· │
│ ········  ····· │
└─────────────────┘
```

**Fix scope:** restructure `draw_block_frame` to render the kind
name as a small inset tag in the top-left corner only, and place
the first-branch label as a free-floating tag below the top
border (similar to how continuation labels already work). Keeps
all existing data-model and parse logic; pure renderer change.

**Why ★★:** matches user expectation from real Mermaid, sequence
diagrams are heavily used.

#### 5. Missing bottom participant boxes in sequence diagrams — **SHIPPED IN 0.9.7** ✓

**Symptom:** real Mermaid renders the participant boxes at BOTH
top AND bottom of the lifelines. We only render the top boxes.
Visually subtle but missing — and it's where the eye looks to
"close" a sequence diagram.

**Fix scope:** extend `render` to draw a mirror set of
participant boxes at the bottom of the canvas, after the last
message row. Affects canvas height calculation
(`+ BOX_HEIGHT`), block-frame row range (mustn't extend into the
bottom-box area), and lifeline drawing (must terminate one row
above the bottom box). Bounded change, but touches several height
budgets.

**Why ★★:** matches Mermaid convention; visually completes
sequence diagrams.

#### 6. Edge crossings not minimised in dense graphs — **PHASE 1 SHIPPED IN 0.14.0 (opt-in)** ★

**Symptom (Dependency graph):** App→PostgreSQL, App→RabbitMQ,
RabbitMQ→Worker, Worker→PostgreSQL all cross each other in
visually busy ways. GitHub's Mermaid uses smarter layout (curved
edges, crossing minimisation, long-edge dummy nodes) and the same
graph reads cleanly there.

**Root cause:** our layered-layout pipeline:
- Doesn't run a crossing-minimisation pass during node ordering
  within layers.
- Doesn't insert dummy nodes for edges spanning multiple layers
  (so `App → PostgreSQL` skipping past `Worker`'s layer takes a
  long detour rather than threading cleanly).
- A* edge router is greedy per-edge rather than globally
  optimising.

**Fix scope (the big rework):**
1. Add long-edge dummy node insertion (standard sugiyama step
   between layer assignment and crossing minimisation).
2. Add a barycenter or median-based crossing-minimisation pass
   over node-orderings within each layer.
3. Optional: smarter edge router that uses the dummy-node grid
   to thread parallel edges along reserved channels.

**Why ★ (not higher):** "hard, mostly-invisible-when-it-works"
applies here. Big effort with subtle wins on simple diagrams; the
payoff shows up on the gallery's complex examples but those
already half-work. Already on roadmap as "Edge-routing
improvements" (see below) — this section is the detailed
diagnosis.

**Phase 1 (shipped in 0.14.0):** opt-in `LayoutBackend::Sugiyama`
that wraps the [`ascii-dag`] crate. Gives clean 4-layer output on
the README architecture case (App | Cache+Queue | Worker | DB)
with the long App→DB edge routed through dummy nodes — exactly
what the diagnosis above prescribed. CLI: `--sugiyama`. Embedded:
`RenderOptions { backend: Sugiyama, ..Default::default() }`.

**Phase 2 (deferred):** flip the default once the wrapper grows:
- Subgraph cluster support (so opt-in isn't required for diagrams
  with `subgraph ... end` blocks).
- Parallel-edge group passthrough (so #2/#4 work survives the
  switch — today the wrapper would lose Phase 2a's widening).
- Direction overrides on nested clusters (TB-inside-LR etc.).
- Tunable spacing (ascii-dag uses hardcoded 3-cell separation
  regardless of our `node_gap`/`layer_gap`; would inflate or
  shrink ~all snapshots when flipped).
- Snapshot triage of the 26 cases that change when sugiyama is
  used unconditionally for non-subgraph graphs (most are likely
  improvements; a few simple cases regress on spacing).

[`ascii-dag`]: https://crates.io/crates/ascii-dag

#### 7. Back-edge perimeter routing fragments forward edges (needs deeper A* work — investigated 2026-04-22)

**Symptom (state machine):** the back-edge `Failed → Idle` (going
UP in TD direction) routes via the right perimeter, inserting a
vertical `│` column that threads between Done and Failed.

**Investigation finding (2026-04-22):** tried the simplest fix —
pushing `exit_point_back_edge` / `entry_point_back_edge` further
from the source/target so A* would prefer the far-right corridor.
This only shifts the fragmenting column by 1-2 cells; A* still
picks the shortest path through the diagram body because it has
no cost signal to avoid the "node-dense region."

**Proper fix (deferred):** introduce a new `Obstacle::InnerArea`
variant that marks cells inside the convex hull of real node
bounding boxes (but not ON a node box itself, not edge-occupied).
When routing back-edges specifically, give A* a cost penalty for
`InnerArea` cells so it routes around the outside. Requires:
1. A pre-pass in `render_inner` that marks `InnerArea` cells.
2. A back-edge-aware variant of `route_edge` (or a per-call
   cost-modifier hint) that charges extra for `InnerArea`.
3. Tuning the penalty relative to `EDGE_SOFT_COST` so back-edges
   take the outside route WITHOUT refusing to take shortcuts
   when a clean corridor doesn't exist.

Estimated effort: 1-1.5 days (bigger than the "~half day" the
original ROADMAP entry suggested). The complexity surfaced once
the simple fix was tried — reversing out as a lesson about
picking the right level of intervention.

**Why deferred:** visible only in specific TD/BT cyclic diagrams;
doesn't regress the typical user's experience. The proven-
complexity note here lets a future session pick it up with the
right scope from the start.

---

### Edge-routing improvements (legacy entry — see #6 above for detailed diagnosis)

Dense graphs (e.g. the circuit-breaker FSM with 5 states + 3
back-edges) still produce visually busy outputs because A* is
greedy and doesn't optimise for label space. Better routing would
benefit every diagram. Hard, mostly-invisible-when-it-works.

### Composite-edge attach-to-border (state diagrams)

Today `Composite --> X` is rewritten at parse time to point at the
composite's synthesised inner `[*]` end. Works, but the arrow lands
on the inner marker rather than the composite border. A renderer
extension that lets edges target subgraph IDs (currently silently
dropped — see `crates/mermaid-text/src/render/unicode.rs:477`)
would let the arrow attach to the border like Mermaid's own
renderer does. Medium effort, would need new edge-routing logic.

### Per-composite direction-aware fork/join orientation

`mermaid-text` 0.7.2 ships fork/join shapes resolved from the
top-level graph direction. A composite with `direction TB` inside
an LR top-level diagram gets the wrong orientation (LR's vertical
bars instead of TB's horizontal). Fix needs the parser to track
each fork/join's enclosing composite path and resolve orientation
from the relevant composite's direction (or fall back to the
top-level). Small change, low priority unless someone hits it.

### Wider fork/join bars

Real Mermaid renders fork/join bars as filled rectangles several
cells thick. v1 uses single-cell-thick `━` / `┃`. A real "filled
thick bar" would need a new primitive (multi-row block fills) and
edge-routing changes to attach edges along the bar's long edge
rather than its midpoint.

### `<<choice>>` rendering without label for unnamed choices

Mermaid hides labels for unnamed choices; we still render the
state ID inside the diamond. Detect "synthetic" / placeholder IDs
and skip the label, or accept an explicit empty-label hint.

---

### `classDef DEFAULT` special semantics — `mermaid-text`

Mermaid treats `classDef DEFAULT …` as a base class merged into every
other class. We currently treat it as a normal classDef named
"DEFAULT" with no special semantics. Implement the merge if someone
asks. ~half day.

### Subgraph interior fill — `mermaid-text`

Today only `stroke` is honoured for subgraph styles (border colour);
`fill` and `color` are accepted in the schema but not rendered. A
real "fill the composite interior with a tint" pass would conflict
with inner node backgrounds — needs a layered-paint design (paint
subgraph fill first, then node fills overlay). Defer.

### `click` / hyperlink directives — `mermaid-text`

Mermaid `click NodeId "https://…"` makes the node clickable. In a
text terminal we'd render a footnote-style link reference, or use
OSC 8 hyperlinks where supported. Separate ticket.

### Real dashed-border note shape — `mermaid-text`

v1 of notes uses a solid rounded box; the dotted connector
distinguishes it from regular states. A real Mermaid-style note
would have a dashed border too. Needs a new `Grid::draw_note_box`
primitive mixing rounded corners with dotted top/bottom and dotted
vertical sides. Add later if anyone asks.

### `note over X,Y` multi-anchor — `mermaid-text`

Mermaid's `note over X,Y` spans two anchors. v1 silently skips
multi-anchor forms. Adding it needs either: a new "spanning" edge
that anchors to multiple targets, or a renderer pass that draws
two separate dotted lines from one note. Defer.

### Floating notes (`note "text" as N1`) — `mermaid-text`

Mermaid's no-anchor form. Rendering is ill-specified upstream;
defer until someone files a real use case.

## Done since 1.7.1 (recent history — see CHANGELOGs for detail)

- **0.11.2**: back-edge perimeter routing fix (ROADMAP item #7).
  Cyclic state diagrams now route their back-edges around the
  perimeter instead of threading through the diagram body — forward
  edges keep their clean channels, labels read uncrammed. New
  `Obstacle::InnerArea` cell classification + a `route_back_edge`
  variant that charges extra for crossing it. Closes the visible
  state-machine fragmentation reported in the 2026-04-21 gallery
  review.
- **0.11.1**: erDiagram Phase 2 — entity attribute tables render
  inside the boxes with aligned type/name/keys columns and a
  header divider; relationship endpoints carry single-character
  cardinality glyphs (`1`, `?`, `+`, `*`). Ships the visual
  polish that makes erDiagram feel like a first-class diagram
  type.
- **0.11.0**: `erDiagram` (entity-relationship) support — Phase 1
  of a 3-phase series. Parser handles the full Mermaid grammar
  (cardinality codes, identifying vs non-identifying, attribute
  blocks with PK/FK/UK modifiers, quoted labels and comments).
  Renderer ships a single-row source-order layout with name-only
  entity boxes and labelled arrows between. The most-requested
  missing diagram type per earlier ROADMAP.
- **0.10.1**: median + transpose crossing-minimisation passes
  (Phase A.3 of the layered-layout improvements series). Median
  is more robust to outlier neighbours than barycenter alone;
  transpose is a local-refinement pass that swaps adjacent nodes
  when it strictly reduces crossings. Refactored
  `sort_by_barycenter` into a generic `sort_by_metric` taking a
  `SortMetric` enum — same code path, zero duplication. No
  snapshot changes on the current gallery (barycenter alone was
  already at optimum) but hedges against future dense graphs.
  6 new unit tests prove the algorithms work in isolation.
- **0.10.0**: long-edge waypoint routing (Phase A.1 of the
  layered-layout improvements series). Edges spanning >1 layer
  now thread through per-layer waypoints anchored on each
  intermediate layer's spine, with row/column snapped off
  real-node ranges. Marginal-but-clean visual win on dense
  graphs; sets up Phase A.2 (Brandes-Köpf coordinate assignment,
  due 0.10.1) for the bigger compaction win. Source-breaking:
  `layered::layout` now returns `LayoutResult`; `render::render`
  gains a `&[EdgeWaypoints]` parameter.


- **0.9.4**: `pie` chart support — first new diagram type since
  `sequenceDiagram` in 0.9.0. Accepts the standard `pie [showData]
  [title <text>]` header plus `"label" : value` slice lines.
  Renders as a horizontal bar chart in monospace text (more
  legible than any ASCII pie attempt) — `█` filled / `░` unfilled,
  percentage column, optional value column when `showData` is set.
  Bar width auto-scales to the `--width` budget. Slice colours are
  deferred (monochrome v1).
- **0.9.3**: sequence-diagram block statements — `loop`, `alt`/
  `else`, `opt`, `par`/`and`, `critical`/`option`, `break`. Stack-
  based parser with proper validation (orphan `end`, wrong
  continuation keyword, unclosed-at-EOF all error clearly). Renders
  as labelled rectangles using heavy double-line glyphs (`╔╗╚╝═║`)
  to differentiate from participant boxes (square) and notes
  (rounded). Nested blocks inset by 1 cell per nesting level.
  Continuations draw `╠┄[label]┄╣` dividers. Completes the four-
  part sequence-diagram polish series — autonumber, notes,
  activation bars, and blocks all compose cleanly in one diagram.
- **0.9.2**: sequence-diagram activation bars — both explicit
  `activate X` / `deactivate X` directives and inline `A->>+B` /
  `A-->>-B` shorthand. Stack-based pairing supports nested
  activations on the same participant. Renderer overlays heavy
  `┃` on the participant's lifeline column for the duration of
  each span; orphan deactivate is a hard parse error, unclosed
  activate auto-closes at the last message.
- **0.9.1**: sequence-diagram notes — `note left of X : text`,
  `note right of X`, `note over X`, and the multi-anchor
  `note over X,Y` span form. `<br>` / `<br/>` in note text become
  real line breaks. Defensive parse error if a state-diagram-style
  `end note` is written, pointing the user at `<br>`. Note
  interior is cleared so dashed lifelines don't bleed through.
- **0.9.0**: sequence-diagram `autonumber` directive (bare,
  `autonumber N`, `autonumber off`, mid-diagram re-base). New
  foundation types on `SequenceDiagram` (`notes`, `activations`,
  `blocks`, `autonumber_changes`) ready for the upcoming 0.9.x
  releases. Lifted `strip_keyword_prefix` into `parser/common.rs`
  to retire a duplicate.
- **0.8.1**: notes anchored to states (`note left|right|over of X`,
  single + multi-line). Each note synthesises a `NodeShape::Note`
  connected by a dotted, no-arrow edge. Also fixed a latent bug in
  `rewrite_composite_edges` that was silently dropping edge style
  fields.
- **0.8.0**: `classDef` + `class` + `:::className` shorthand for
  flowcharts and state diagrams. New `Graph::class_defs` /
  `subgraph_styles` registries. Subgraph border colouring. State
  diagrams pick up `style` / `linkStyle` (no longer silently
  skipped). Shared `parser/common.rs` module eliminates the prior
  parser-helper duplication.
- **0.7.2**: `<<choice>>` / `<<fork>>` / `<<join>>` shape modifiers
  for state diagrams (Diamond / direction-perpendicular Bar).
- **0.7.1**: edge-label collision avoidance (labels stop overwriting
  node interiors).
- **0.7.0**: state diagrams default to LR (was TB) for better text
  output.
- **0.6.0**: composite states `state X { … }` with recursive nesting,
  per-composite `[*]` scope, external-edge rewrite, back-edge
  perimeter connectors, orphan-marker GC.
- **0.5.0**: `stateDiagram` / `stateDiagram-v2` support.
- **0.4.0**: ANSI 24-bit color output (opt-in).
- **markdown-tui-explorer 1.8.1 / 1.8.2 / 1.9.0 / 1.9.1**: layout-height
  fix, scroll-inside-mermaid fix, transitive bumps for the above.
