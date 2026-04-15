# Changelog

All notable changes to `markdown-tui-explorer` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.3.0] - 2026-04-15

### Fixed
- **Doc-search navigation now moves the viewer cursor.** `n`/`N` and the
  auto-jump to the first match were mutating `scroll_offset` directly,
  leaving `cursor_line` stranded at its old position. Press `j`/`k`
  after `n` now moves the cursor from the match row, as expected.
- **Cursor highlight no longer disappears over tables and mermaid
  blocks.** The highlight code now runs for `DocBlock::Text`,
  `DocBlock::Table`, and the source-text fallback of `DocBlock::Mermaid`
  via a shared `patch_cursor_highlight()` helper. Mermaid blocks in
  image mode render a 1-row background bar beneath the image so the
  cursor is still visible around the image padding.
- **Entering edit mode inside a table or mermaid block lands on the
  correct source line.** `source_line_at` previously returned only the
  block's opening line, so `i` from the middle of a 20-row table dropped
  you on the header. Tables now track per-row source lines via a new
  `TableBlock::row_source_lines` vector populated from
  pulldown-cmark's `OffsetIter`. Mermaid blocks interpolate as
  `fence + 1 + K`, clamped to the content length — same pattern code
  blocks already use for their content rows.

### Added
- **Cursor position in the viewer status bar.** The status bar now
  shows `(cursor_line / total_lines, percentage)` instead of the old
  scroll-based percentage, so `d`/`u`/`gg`/`G`/`PageDown`/`PageUp`
  navigation is reflected immediately even when the cursor stays
  on-screen.

## [1.2.0] - 2026-04-15

### Added
- **Visible viewer cursor.** The viewer now shows a highlighted cursor row
  (background from `palette.selection_bg`, carries through line wrapping)
  that moves with `j`/`k`/`d`/`u`/`PageDown`/`PageUp`/`gg`/`G`. Scroll
  follows the cursor when it would leave the viewport, so the observable
  behaviour of "press `j` to scroll down" is preserved while unlocking a
  proper notion of "where I am" for future features.
- **Vim-style edit mode** via
  [edtui](https://crates.io/crates/edtui) 0.11.2. Press `i` in the viewer
  to drop into a modal editor at the exact source line of the viewer
  cursor. Normal/Insert/Visual modes with vim motions (`w`, `b`, `e`,
  `gg`, `G`, `0`, `$`, `dd`, `yy`, `p`, etc.). `:w` saves atomically
  (tempfile + rename), `:q` returns to the rendered view, `:wq` does
  both, `:q!` force-discards unsaved changes. Undo/redo via `u` /
  `Ctrl+r`. The editor theme tracks the active UI palette.
- **Source-line plumbing through the renderer.** pulldown-cmark byte
  offsets are now threaded through `MdRenderer` so every rendered logical
  line reports its originating source line. `DocBlock::Text` carries a
  parallel `source_lines: Vec<u32>`; `DocBlock::Mermaid` and `TableBlock`
  carry `source_line: u32`. This is what powers exact cursor-to-editor
  positioning and unlocks future line-aware features.
- **Git status refresh on save.** Editing a file and pressing `:w` now
  recolors its entry in the file tree immediately — new files turn
  yellow (modified) as soon as the write lands, no git poll wait.

### Changed
- `j`/`k`/`d`/`u`/`PageDown`/`PageUp`/`gg`/`G` in the viewer now move a
  cursor rather than the scroll offset directly. Scroll follows cursor,
  so the visible effect is the same — but the cursor is the new primary
  concept for "where I am".
- `edtui` is pulled in with `default-features = false` to avoid the
  `arboard` clipboard dependency. Our app handles mouse and clipboard
  separately, and this keeps the binary smaller and headless-safe.

### Fixed
- Mouse events are now ignored while `Focus::Editor` is active, so clicks
  in the tree panel during editing no longer select and open files.

## [1.1.0] - 2026-04-14

### Added
- **Syntax highlighting for fenced code blocks.** Fenced blocks with a
  language tag (`rust`, `python`, `javascript`, `go`, `json`, `bash`, and
  many more) are now tokenised and colored inline. Implemented via
  [syntect](https://crates.io/crates/syntect) with the pure-Rust
  `default-fancy` feature — no C dependencies, no onig. Each UI theme
  maps to a bundled syntect theme so colors track the active palette.
- **Table modal mouse support.** The full-screen table viewer (`Enter`
  on a table) now responds to the mouse wheel: plain scroll pans rows,
  `Shift`+scroll pans columns, and clicking outside the modal closes it.
- **Column-boundary horizontal panning in the table modal.** `h` and `l`
  now snap to the previous/next column boundary rather than moving a
  single cell at a time. `H` and `L` pan half a page instead of a fixed
  ten cells, making wide tables dramatically faster to navigate.
- **`scroll_left` / `scroll_right` (`MouseEventKind::ScrollLeft` /
  `ScrollRight`)** are handled where terminals emit them, mapping to
  one-column-boundary pans.

### Fixed
- **Code block right-border alignment.** Lines containing multi-byte
  characters (em dashes, CJK, emoji) no longer push the box frame out of
  alignment. Width measurement now uses `unicode-width` display cells
  throughout instead of byte length.

### Changed
- `render_markdown` and `MarkdownViewState::load` now take the active
  `Theme` so fenced code blocks can be highlighted with a matching
  syntect theme. Callers inside the crate are updated accordingly.

[1.1.0]: https://github.com/leboiko/markdown-reader/releases/tag/v1.1.0
