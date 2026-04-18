# Changelog

All notable changes to `mermaid-text` are documented in this file.
This project adheres to [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

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
