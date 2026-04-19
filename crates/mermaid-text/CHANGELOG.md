# Changelog

All notable changes to `mermaid-text` are documented in this file.
This project adheres to [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

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
