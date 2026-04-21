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

#### 2. Multiple labelled edges between same node pair — **PARTIAL FIX in 0.9.5; full fix needs layout-pass work**

**What 0.9.5 fixed:** subgraph borders no longer get punctured —
in the CI/CD case `pass` lands at col 41 (immediately right of
CI's `│` at col 40) instead of overwriting the border. Box
outlines stay intact.

**What's still cramped:** the labels themselves remain visually
glued to nearby chrome because the layout pipeline doesn't widen
the gap between `T` (inside CI subgraph) and `D` (outside) to
account for parallel labels. Empirical finding from the 0.9.5
development cycle: trying a label-placement-only patch with
1-cell padding around protected regions just detaches labels
from their edges entirely. Real fix needs layout-pass work —
during layered layout, detect parallel-edge pairs and add
column/row spacing between their endpoints proportional to the
combined label widths.

**Decision:** rolled into item #6 (sugiyama improvements) below.
Don't try to solve this purely in the label-placement pass.

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

#### 6. Edge crossings not minimised in dense graphs (deep rework, multi-day) ★

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

#### 7. Back-edge perimeter routing fragments forward edges (medium, ~half day) ★

**Symptom (state machine):** the back-edge `Failed → Idle` (going
UP in TD direction) routes via the right perimeter, inserting a
vertical `│` column that threads between Done and Failed. The
forward edges `Running → Done` and `Running → Failed` then have
to share narrow channels in the middle, and labels like
`done`/`error` get crammed into tight rows that read as
disconnected.

**Root cause:** perimeter routing for back-edges (shipped in
0.6.0) inserts columns/rows greedily without considering the
visual impact on forward edges through the same area.

**Fix scope:** during perimeter routing, prefer perimeter slots
that are FURTHER from the dense-forward-edge area; if no slot is
clean, route the back-edge through a fresh added column at the
canvas edge rather than through the active diagram body.

**Why ★:** mostly affects the visual quality of mixed-direction
diagrams. Diagnosis should prove out before committing to the
fix.

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

- **0.11.0**: `erDiagram` (entity-relationship) support — Phase 1
  of a 3-phase series. Parser handles the full Mermaid grammar
  (cardinality codes, identifying vs non-identifying, attribute
  blocks with PK/FK/UK modifiers, quoted labels and comments).
  Renderer ships a single-row source-order layout with name-only
  entity boxes and labelled arrows between. Phase 2 adds
  attribute tables + crow's-foot glyphs; Phase 3 adds grid layout.
  The most-requested missing diagram type per earlier ROADMAP.
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
