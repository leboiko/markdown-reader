# Changelog

All notable changes to `mermaid-text` are documented in this file.
This project adheres to [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

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
