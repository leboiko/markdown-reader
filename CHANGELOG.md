# Changelog

All notable changes to `markdown-tui-explorer` are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.4.1] - 2026-04-16

### Fixed
- **`Enter` now expands the table under the cursor** rather than the first
  table that happens to intersect the viewport.  Falls back to the
  first-visible table when the cursor is on prose, preserving the old
  "click anywhere to expand" behaviour.
- **Table modal contrast** — the expanded-table modal's grid borders
  were rendered with a colour tuned for the main viewer background but
  drawn against the modal's tinted background, which made the grid
  barely visible on light themes (GitHub Light in particular).  The
  modal body now uses the viewer background directly; the focused-border
  colour around the outer frame still signals "this is a modal".

### Changed
- README now includes screenshots (viewer overview, global search,
  GitHub Light with settings) and lists all eight themes in the
  Features section (Solarized Light and Gruvbox Light were missing from
  the count).  The settings-modal keybinding description mentions the
  new "search preview" option.

## [1.4.0] - 2026-04-16

### Added
- **Global search modal.** Press `/` in the Tree or Viewer to open a
  full-screen search pane. Results are grouped per file with a match
  count and a preview of the first match (full-line or ~80-char
  snippet, selectable in Settings). `j`/`k`/arrows/`Ctrl+n`/`Ctrl+p`
  navigate; `Enter` opens the selected file in a new tab; `Tab`
  toggles between Files and Content modes; `Esc` dismisses. Click a
  row to open it, click outside to dismiss.
- **Smartcase search.** Lowercase query = case-insensitive match;
  any uppercase character in the query = case-sensitive. An `Aa`
  / `aA` indicator in the modal footer shows the active mode. No
  manual toggle required.
- **Jump to match line on open.** Confirming a content-search result
  opens the file and places the viewer cursor on the first-match
  source line, centred in the viewport.
- **Tree auto-expand on open.** Whenever a file is opened
  programmatically (search, link follow, session restore), the file
  tree expands any collapsed ancestor directories so the file's row
  is visible and selected.
- **Vim-style visual-line mode in the viewer.** Press `V` to start a
  line-wise selection; `j`/`k`/`d`/`u`/`gg`/`G`/`PageDown`/`PageUp`
  extend the range; `y` yanks the selection to the clipboard via
  OSC 52 and exits; `Esc` or `V` cancels. Status bar shows
  `VISUAL` while active. `yy` in normal mode copies the current
  cursor line.
- **Search preview setting.** New `Search preview` section in the
  Settings modal toggles between Full line (default) and Snippet
  (~80 chars) previews in the search modal. Persisted in
  `config.toml` as `search_preview`.
- **Cursor position in the status bar.** The status bar now shows
  `(cursor_line / total_lines, percentage)` so `d`/`u`/`gg`/`G`
  navigation is reflected immediately. (Already shipped in 1.3.0;
  this release adds the `VISUAL` label override.)

### Fixed
- **GitHub Light theme: invisible tab and status-bar labels.** The
  `accent` and `selection_fg` colors in the GitHub Light palette
  were both the same blue, so text drawn on an accent background
  (active tab name, focus indicator) rendered blue-on-blue and
  vanished. A new `Palette::on_accent_fg` field disambiguates the
  two roles; for GitHub Light it's set to white.
- **Search-jump to the right source line inside lists and
  paragraphs.** Previously the inverse source-to-logical mapping
  assumed `source_lines` was monotonically non-decreasing, but
  pulldown-cmark's End-of-list events can cause dips (e.g.
  `[..., 165, 160, 167, ...]`), leading to wrong jumps for any
  match whose target line lived after a list. The scan now walks
  the full vector and returns the last candidate whose source
  `<= target`.
- **Gutter line numbers now align with wrapped content.** The
  gutter paragraph previously rendered one number per logical
  line against a wrapping content paragraph, so the two drifted
  vertically on long lines. The gutter now emits blank
  continuation rows that match the content's wrap count, so a
  line number always sits next to its content.
- **Table header source-line tracking.** pulldown-cmark does not
  emit `Tag::TableRow` for a table's header — cells live directly
  inside `Tag::TableHead` — so the header's source line was
  recorded as 0 regardless of the table's actual position. Now
  captured from `Tag::TableHead`'s own span.
- **`pending_jump` no longer leaks on read failure.** A new
  `Action::FileLoadFailed` variant fires when the async read
  fails, clearing the pending jump so a later unrelated file
  load cannot inherit a stale target.
- **Misleading search-truncation footer.** The "N more" count was
  derived by subtracting a file cap from a match count. Replaced
  with a clear `"results capped at N files"` message.

### Changed
- **`:N` go-to-line now centres the target** to match the UX of
  search-result jumps. Both are long-distance jumps; neither
  should park the cursor at the viewport edge.
- **Content search counts all matches per file.** Previously the
  search broke after the first match in each file; the new
  modal needs the count for its per-file display.
- **`edtui` upgraded to 0.11.2** (already in 1.2.0) now with
  `default-features = false` to drop the `arboard` clipboard
  dependency we do not use. Smaller binary, headless-safe.

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
