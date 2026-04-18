# Changelog

All notable changes to `mermaid-text` are documented in this file.
This project adheres to [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

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
