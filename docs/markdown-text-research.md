# Markdown-to-Text Research

Date: 2026-04-23

This note compares `markdown-reader`'s current markdown-to-text pipeline with a few external projects that solve related terminal-rendering problems, then turns that into concrete suggestions for this repo.

## Scope

I focused on:

- Markdown parsing strategy
- Width-aware text wrapping
- Table layout
- Inline styling preservation
- Source-position handling
- Whether the observed ideas look broadly useful here, instead of only helping one or two current bugs

## Current `markdown-reader` shape

Observed locally:

- Parsing is event-driven with `pulldown-cmark` in [src/markdown/renderer.rs](/Users/leboiko/Documents/temp/temp2/temp3/markdown-reader/src/markdown/renderer.rs).
- Rendering already targets an intermediate block IR: `DocBlock::{Text, Mermaid, Table}` in [src/markdown/mod.rs](/Users/leboiko/Documents/temp/temp2/temp3/markdown-reader/src/markdown/mod.rs).
- Source-line mapping is a first-class concern and is threaded through text, tables, and mermaid blocks.
- Width handling is split across layers:
  - text uses ratatui `Paragraph::wrap`
  - tables are laid out explicitly in [src/ui/table_render.rs](/Users/leboiko/Documents/temp/temp2/temp3/markdown-reader/src/ui/table_render.rs)
  - visual row mapping is reproduced in [src/ui/markdown_view/visual_rows.rs](/Users/leboiko/Documents/temp/temp2/temp3/markdown-reader/src/ui/markdown_view/visual_rows.rs)

Inference:

- The current architecture is already better than a naive "markdown straight to terminal string" pipeline.
- The main risk area is not lack of features. It is duplicated layout logic across parsing, block rendering, ratatui wrapping, and source-line mapping.

## External repos and what matters

### 1. `pulldown-cmark`

Source:

- https://github.com/pulldown-cmark/pulldown-cmark

Observed:

- It explicitly supports event streams with source offsets (`into_offset_iter()`).
- Its design favors a pull-parser pipeline instead of forcing an AST.

Why it matters here:

- Your current renderer is using one of the strongest features of `pulldown-cmark`: offset-aware event parsing.
- That part should probably stay unless you decide you need a richer structural representation than events can comfortably provide.

### 2. `termimad`

Sources:

- https://github.com/Canop/termimad
- inspected locally in `/tmp/markdown-text-survey/termimad`

Observed:

- It has a dedicated wrapping layer (`src/fit/wrap.rs`) that wraps styled composites while preserving list and quote prefixes.
- It treats tables as a separate normalization/layout pass (`src/tbl.rs`):
  - discover table runs
  - measure columns
  - fit widths
  - wrap oversized cells into extra physical rows
  - then apply alignment and borders

Why it matters here:

- This is the clearest example of a reusable algorithmic idea for your repo.
- The important part is not termimad's parser. It is the table pipeline and the "wrap styled content with prefix-aware continuation lines" approach.

### 3. `presenterm`

Source:

- https://github.com/mfontanini/presenterm

Observed:

- It parses markdown into an explicit AST-backed domain model using `comrak` in `src/markdown/parse.rs`.
- It carries source positions through the parse layer.
- It uses `WeightedText` / `WeightedLine` in `src/markdown/text.rs` and `src/render/text.rs` to split styled text by display width while preserving style runs.
- Its table rendering is simple and fixed-width for slide use, which is less directly reusable than its text-splitting model.

Why it matters here:

- The strongest idea here is precomputing width metadata for styled text chunks instead of recomputing widths during multiple later passes.
- The AST-first parser is also relevant if your renderer keeps accumulating state-machine complexity around nested lists, tables, footnotes, and block quotes.

### 4. `glamour` / `glow`

Sources:

- https://github.com/charmbracelet/glamour
- https://github.com/charmbracelet/glow

Observed:

- `glow` delegates markdown rendering to `glamour`.
- `glamour` builds rendering around block elements and style-aware render buffers rather than flattening too early.
- It exposes independent controls for word wrap, table wrap, and inline-table-link behavior.
- The project emphasizes a pure renderer shape: same input + same options => same output.

Why it matters here:

- The useful idea is not the Go implementation itself. It is the separation between:
  - structural rendering
  - style policy
  - terminal/environment policy
- That separation tends to make wrapping and testing easier.

### 5. `mdcat` / `pulldown-cmark-mdcat`

Sources:

- https://github.com/swsnr/mdcat
- https://docs.rs/pulldown-cmark-mdcat/latest/pulldown_cmark_mdcat/
- https://docs.rs/crate/mdcat/2.7.1

Observed:

- `mdcat` split its core renderer into a library (`pulldown-cmark-mdcat`).
- The docs are explicit that table support is limited: inline markup and text wrapping in table cells are not fully supported.

Why it matters here:

- This is a useful warning sign. Even mature terminal markdown renderers often stop short at table-cell wrapping because it is a separate layout problem.
- That matches what I saw elsewhere: broad correctness usually requires a dedicated table engine, not a paragraph wrapper plus borders.

### 6. `pandoc`

Sources:

- https://github.com/jgm/pandoc
- https://pandoc.org/MANUAL.html

Observed:

- Pandoc uses the classic reader -> native AST -> writer architecture.
- Its docs call out width-sensitive plain-text table formatting and cell wrapping behavior.

Why it matters here:

- The main lesson is architectural, not implementation-level: plain-text output gets much easier once the renderer consumes a normalized document model instead of raw parser events.

### 7. `markdown-rs`

Source:

- https://github.com/wooorm/markdown-rs

Observed:

- It exposes a positional AST and is oriented toward more complex markdown transforms than event-stream-only renderers.

Why it matters here:

- It is a plausible alternative if you ever outgrow `pulldown-cmark` events for structural reasons.
- I would not switch just to switch, but it is a credible option if you want AST richness without leaving Rust-native parsing.

## Patterns that show up repeatedly

Across the repos above, the broader patterns are:

1. Strong renderers do not flatten too early.
2. Text wrapping and table layout are treated as different problems.
3. Width is usually computed on styled chunks, not on already-rendered terminal lines.
4. Source positions are easiest to preserve when they are attached before the last rendering pass.
5. Table-cell wrapping is where many implementations either simplify heavily or stop.

## Suggestions for `markdown-reader`

These are ordered by value relative to implementation cost.

### 1. Keep `DocBlock`, but add one more normalization layer inside `Text`

Recommendation:

- Keep the existing top-level IR (`Text`, `Table`, `Mermaid`).
- Inside text rendering, stop relying so heavily on raw `Line<Span>` plus ratatui wrapping as the main source of truth.
- Introduce a small width-aware internal text model for paragraphs/list items/block quotes before final ratatui conversion.

Why:

- Right now visual row math, cursor mapping, highlighting, and rendering all need to agree with ratatui wrapping.
- That creates cross-module coupling and repeated width computation.
- A normalized text model would make wrapping deterministic in your own code and reduce "mirror ratatui exactly" maintenance.

Broadness:

- This is a general renderer-quality improvement, not a mermaid-specific fix.

### 2. Borrow the `WeightedText` idea, not necessarily the whole parser stack

Recommendation:

- Add a local width cache for styled inline chunks:
  - original text
  - style
  - display width
  - optional char/grapheme split points

Why:

- You repeatedly need width for wrapping, cursor mapping, truncation, and visual-row calculations.
- `presenterm`'s approach is a good model for avoiding recomputation and preserving style runs during splits.

Broadness:

- Helps paragraphs, headings, block quotes, lists, code spans, table cells, and search highlighting.

### 3. Upgrade table layout from "truncate when tight" to "row expansion when useful"

Recommendation:

- Keep the current fair-share logic for compact inline rendering.
- Add a second layout mode that can wrap cell content into multiple physical table rows instead of only truncating.

Why:

- This is the most broadly useful table improvement I found in the survey.
- `termimad` shows a practical model:
  - compute column widths
  - split oversized cells
  - materialize extra row fragments
  - apply alignment after splitting

Where to apply it:

- Expanded table view first.
- Inline view later, if desired.

Broadness:

- This is a general markdown-table fix, not a repo-specific optimization.

### 4. Centralize wrapping rules instead of partially delegating to ratatui

Recommendation:

- Choose one source of truth for wrapping behavior.
- Either:
  - fully own wrapping in your renderer, or
  - isolate a tiny compatibility layer that computes visual rows from the exact same splitting algorithm used for drawing

Why:

- The current setup requires [src/ui/markdown_view/visual_rows.rs](/Users/leboiko/Documents/temp/temp2/temp3/markdown-reader/src/ui/markdown_view/visual_rows.rs) to predict what ratatui `Paragraph::wrap` will do.
- That is workable, but fragile when styling, unicode width, or prefix behavior evolves.

Broadness:

- This improves scrolling, link hit-testing, highlights, and edit-mode source mapping across the whole viewer.

### 5. Be conservative about switching parsers

Recommendation:

- Do not replace `pulldown-cmark` immediately.
- Reassess only if one of these becomes frequent:
  - nested structure bugs keep growing
  - parser-state code becomes harder to reason about than a tree walk
  - you want richer block transformations before render

Why:

- Your current use of `pulldown-cmark` offsets is a real advantage.
- The migration cost to `comrak` or `markdown-rs` is justified only if structural complexity, not just layout complexity, becomes the main bottleneck.

### 6. Add width-regression tests the same way termimad does

Recommendation:

- Add parameterized tests that render the same markdown at many widths.
- Assert invariants like:
  - no overflow
  - stable source-line mapping
  - list continuation alignment
  - table borders remain valid
  - no style-span loss after wrapping/truncation

Why:

- The best surveyed projects test across width ranges, not only one golden width.
- Width bugs are usually algorithm bugs, so this catches broad regressions early.

## Concrete adoption plan

### Phase 1: Low-risk improvements

- Introduce a styled width-cache type for inline chunks.
- Use it in visual-row counting and text truncation.
- Add width-sweep tests for paragraphs, lists, block quotes, and tables.

### Phase 2: Better table engine

- Add a wrapped-cell table layout path for expanded tables.
- Reuse existing `CellSpans` and alignments.
- Materialize physical row fragments explicitly instead of asking ratatui to wrap table text.

### Phase 3: Text normalization

- Introduce an internal wrapped text representation for non-table markdown blocks.
- Convert that to ratatui `Text` only at the end.

### Phase 4: Revisit parser choice only if needed

- Prototype `comrak` or `markdown-rs` only if renderer state complexity keeps growing despite the layout cleanup.

## What I would not do

- I would not replace the whole renderer with another project.
- I would not switch parsers just because ASTs look cleaner on paper.
- I would not optimize around mermaid-specific cases when the underlying problem is generic label/text layout.
- I would not push wrapped tables into the main inline path first; expanded view is the safer proving ground.

## Bottom line

The biggest reusable ideas are:

1. A width-cached styled text model for splitting and mapping.
2. A dedicated table layout pass that can wrap cells into extra physical rows.
3. Less dependence on ratatui wrapping as hidden renderer behavior.

If I had to pick one direction with the best payoff, it would be:

- keep `pulldown-cmark`
- keep `DocBlock`
- add a stronger local layout model for text and tables

That is the most broadly useful path I found, and it aligns with how the stronger terminal markdown renderers are structured.
