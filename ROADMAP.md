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

### Edge-routing improvements

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
