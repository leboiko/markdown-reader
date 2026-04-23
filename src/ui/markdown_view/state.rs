use crate::markdown::{DocBlock, TableBlockId};
use crate::theme::Palette;
use ratatui::text::Text;
use std::collections::HashMap;
use std::path::PathBuf;

/// Whether the visual selection is character-wise or line-wise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualMode {
    /// `v` — selection spans a character range across lines.
    Char,
    /// `V` — selection spans full logical lines.
    Line,
}

/// Anchor and cursor bounds of a visual selection in the viewer.
///
/// Absolute logical line indices are in the same coordinate space as
/// `cursor_line`. For line mode the selected range is
/// `min(anchor_line, cursor_line)..=max(anchor_line, cursor_line)` covering
/// every column. For char mode the first and last lines may be partial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualRange {
    /// Whether the selection is character-wise or line-wise.
    pub mode: VisualMode,
    /// The line where the selection began (does not move during extension).
    pub anchor_line: u32,
    /// The display column where the selection began (0-based terminal cells).
    pub anchor_col: u16,
    /// The current end line, tracking the cursor.
    pub cursor_line: u32,
    /// The current end column, tracking the cursor (0-based terminal cells).
    pub cursor_col: u16,
}

impl VisualRange {
    /// Smaller of `anchor_line` and `cursor_line` — the top line of the selection.
    pub fn top_line(&self) -> u32 {
        self.anchor_line.min(self.cursor_line)
    }

    /// Larger of `anchor_line` and `cursor_line` — the bottom line of the selection.
    pub fn bottom_line(&self) -> u32 {
        self.anchor_line.max(self.cursor_line)
    }

    /// `true` if the absolute logical `line` is inside the selection range
    /// (inclusive on both ends).
    ///
    /// This checks line containment only — use [`char_range_on_line`] for
    /// column-precise testing in char mode.
    pub fn contains(&self, line: u32) -> bool {
        line >= self.top_line() && line <= self.bottom_line()
    }

    /// Return the `(start_col, end_col_exclusive)` range on `line` that is
    /// selected, or `None` if `line` is outside the selection.
    ///
    /// `line_width` is the display width (in terminal cells) of the logical
    /// line at `line`; it is used to compute the end of a full-line selection.
    pub fn char_range_on_line(&self, line: u32, line_width: u16) -> Option<(u16, u16)> {
        if line < self.top_line() || line > self.bottom_line() {
            return None;
        }
        match self.mode {
            VisualMode::Line => Some((0, line_width)),
            VisualMode::Char => {
                let (start_line, start_col, end_line, end_col) = self.ordered();
                if self.top_line() == self.bottom_line() {
                    // Same-line selection: highlight [start_col, end_col] inclusive.
                    Some((start_col, end_col + 1))
                } else if line == start_line {
                    Some((start_col, line_width))
                } else if line == end_line {
                    Some((0, end_col + 1))
                } else {
                    Some((0, line_width)) // middle line — fully selected
                }
            }
        }
    }

    /// Return `(start_line, start_col, end_line, end_col)` ordered so that
    /// `(start_line, start_col)` is positionally before `(end_line, end_col)`.
    fn ordered(&self) -> (u32, u16, u32, u16) {
        let anchor_before = self.anchor_line < self.cursor_line
            || (self.anchor_line == self.cursor_line && self.anchor_col <= self.cursor_col);
        if anchor_before {
            (
                self.anchor_line,
                self.anchor_col,
                self.cursor_line,
                self.cursor_col,
            )
        } else {
            (
                self.cursor_line,
                self.cursor_col,
                self.anchor_line,
                self.anchor_col,
            )
        }
    }
}

/// A hyperlink with an absolute display-line position (after block offsets are applied).
#[derive(Debug, Clone)]
pub struct AbsoluteLink {
    /// Absolute 0-indexed display line within the document.
    pub line: u32,
    pub col_start: u16,
    pub col_end: u16,
    pub url: String,
    pub text: String,
}

/// A heading anchor with an absolute display-line position.
#[derive(Debug, Clone)]
pub struct AbsoluteAnchor {
    pub anchor: String,
    /// Absolute 0-indexed display line within the document.
    pub line: u32,
}

/// Runtime state for the markdown preview panel.
#[derive(Debug, Default)]
pub struct MarkdownViewState {
    /// Raw markdown source of the currently displayed file.
    pub content: String,
    /// Pre-rendered block sequence produced by the markdown renderer.
    pub rendered: Vec<DocBlock>,
    /// Current scroll offset in display lines.
    pub scroll_offset: u32,
    /// Current cursor position as an absolute rendered logical-line index.
    /// Same coordinate space as `scroll_offset`. Defaults to 0.
    pub cursor_line: u32,
    /// Horizontal cursor position within the current logical line, measured in
    /// terminal display cells (0-based). Used for char-wise visual mode (`v`)
    /// and displayed in the status bar. Clamped to the line width on vertical
    /// moves. Defaults to 0.
    pub cursor_col: u16,
    /// Display name shown in the panel title.
    pub file_name: String,
    /// Absolute path of the loaded file.
    pub current_path: Option<PathBuf>,
    /// Total number of display lines across all blocks.
    pub total_lines: u32,
    /// The inner width used for the last layout pass; cached layouts are invalid
    /// when this changes.
    pub layout_width: u16,
    /// Per-table rendering cache keyed by `TableBlockId`.
    pub table_layouts: HashMap<TableBlockId, TableLayout>,
    /// All hyperlinks in the document with absolute display-line positions.
    pub links: Vec<AbsoluteLink>,
    /// All heading anchors in the document with absolute display-line positions.
    pub heading_anchors: Vec<AbsoluteAnchor>,
    /// Active visual-line selection; `None` when the viewer is in normal mode.
    ///
    /// Reset to `None` by [`load`] so switching files always clears any
    /// dangling selection from the previous document.
    pub visual_mode: Option<VisualRange>,
}

impl MarkdownViewState {
    /// Recompute absolute link and heading-anchor positions from the current
    /// block heights. Call this after any operation that changes block heights
    /// (table layout, mermaid height update, text wrap recompute) so click
    /// targets and the link picker stay aligned with what the user sees on
    /// screen.
    ///
    /// `content_width` is the effective viewer width (excluding the gutter
    /// when line numbers are on); used to translate logical line indices
    /// stored on `LinkInfo` / `HeadingAnchor` into the visual rows actually
    /// occupied by those lines after wrapping.
    pub fn recompute_positions(&mut self, content_width: u16) {
        use super::visual_rows::line_visual_rows;
        let mut abs_links: Vec<AbsoluteLink> = Vec::new();
        let mut abs_anchors: Vec<AbsoluteAnchor> = Vec::new();
        let mut block_offset = 0u32;
        for block in &self.rendered {
            if let DocBlock::Text {
                text,
                links,
                heading_anchors,
                ..
            } = block
            {
                // Cumulative visual-row offset of each logical line within
                // this block. `visual_offset_of_logical[i]` is the visual
                // row at which `text.lines[i]` begins.
                let mut visual_offset_of_logical: Vec<u32> =
                    Vec::with_capacity(text.lines.len() + 1);
                let mut acc = 0u32;
                visual_offset_of_logical.push(acc);
                for line in &text.lines {
                    acc = acc.saturating_add(line_visual_rows(line, content_width));
                    visual_offset_of_logical.push(acc);
                }
                for link in links {
                    let visual_row = visual_offset_of_logical
                        .get(link.line as usize)
                        .copied()
                        .unwrap_or(link.line);
                    abs_links.push(AbsoluteLink {
                        line: block_offset + visual_row,
                        col_start: link.col_start,
                        col_end: link.col_end,
                        url: link.url.clone(),
                        text: link.text.clone(),
                    });
                }
                for ha in heading_anchors {
                    let visual_row = visual_offset_of_logical
                        .get(ha.line as usize)
                        .copied()
                        .unwrap_or(ha.line);
                    abs_anchors.push(AbsoluteAnchor {
                        anchor: ha.anchor.clone(),
                        line: block_offset + visual_row,
                    });
                }
            }
            block_offset += block.height();
        }
        self.links = abs_links;
        self.heading_anchors = abs_anchors;
    }

    /// Load a file into the viewer, resetting the scroll position.
    ///
    /// # Arguments
    ///
    /// * `path`      – filesystem path of the file being loaded.
    /// * `file_name` – display name shown in the tab bar.
    /// * `content`   – raw markdown source.
    /// * `palette` – color palette for the active UI theme.
    /// * `theme` – the active UI theme; forwarded to the markdown renderer
    ///   to select the matching syntect highlighting theme for fenced code blocks.
    pub fn load(
        &mut self,
        path: PathBuf,
        file_name: String,
        content: String,
        palette: &Palette,
        theme: crate::theme::Theme,
    ) {
        let blocks = crate::markdown::renderer::render_markdown(&content, palette, theme);
        self.total_lines = blocks.iter().map(crate::markdown::DocBlock::height).sum();
        self.rendered = blocks;
        // No layout width yet — pass 0 so `line_visual_rows` short-circuits
        // to "1 row per logical line", matching the pre-wrap heights stored
        // by the renderer. The first `draw` call will recompute with the
        // real width once the viewport rect is known.
        self.recompute_positions(0);
        self.content = content;
        self.file_name = file_name;
        self.current_path = Some(path);
        self.scroll_offset = 0;
        self.cursor_line = 0;
        self.cursor_col = 0;
        // Always clear visual-line selection when loading a new file so the
        // previous document's selection doesn't appear in the new one.
        self.visual_mode = None;
        // Invalidate table layout cache. The fresh DocBlock::Table values carry
        // a pessimistic rendered_height that only becomes accurate once the
        // draw loop runs layout_table; forcing a rebuild keeps the hint line
        // and doc-search line numbers in sync after re-renders (e.g. on theme
        // change, live reload, or session restore).
        self.layout_width = 0;
        self.table_layouts.clear();
    }

    /// Move the cursor down by `n` logical lines, clamped to the last line.
    ///
    /// When visual mode is active, the selection's `cursor` end is extended to
    /// track the new cursor position; the `anchor` stays fixed. `cursor_col`
    /// is clamped to the new line's display width (vim-style column clamping).
    ///
    /// Does not update `scroll_offset`; call [`scroll_to_cursor`] afterward.
    pub fn cursor_down(&mut self, n: u32) {
        let max = self.total_lines.saturating_sub(1);
        self.cursor_line = self.cursor_line.saturating_add(n).min(max);
        self.clamp_cursor_col();
        if let Some(range) = self.visual_mode.as_mut() {
            range.cursor_line = self.cursor_line;
            range.cursor_col = self.cursor_col;
        }
    }

    /// Move the cursor up by `n` logical lines, saturating at 0.
    ///
    /// When visual mode is active, the selection's `cursor` end is extended to
    /// track the new cursor position; the `anchor` stays fixed. `cursor_col`
    /// is clamped to the new line's display width.
    ///
    /// Does not update `scroll_offset`; call [`scroll_to_cursor`] afterward.
    pub fn cursor_up(&mut self, n: u32) {
        self.cursor_line = self.cursor_line.saturating_sub(n);
        self.clamp_cursor_col();
        if let Some(range) = self.visual_mode.as_mut() {
            range.cursor_line = self.cursor_line;
            range.cursor_col = self.cursor_col;
        }
    }

    /// Jump the cursor to the first line and reset the scroll to the top.
    ///
    /// When visual mode is active, extends the selection to the top of the document.
    pub fn cursor_to_top(&mut self) {
        self.cursor_line = 0;
        self.scroll_offset = 0;
        self.clamp_cursor_col();
        if let Some(range) = self.visual_mode.as_mut() {
            range.cursor_line = 0;
            range.cursor_col = self.cursor_col;
        }
    }

    /// Jump the cursor to the last line and scroll so it is visible.
    ///
    /// When visual mode is active, extends the selection to the bottom of the document.
    ///
    /// # Arguments
    ///
    /// * `view_height` – visible viewport height in display lines.
    pub fn cursor_to_bottom(&mut self, view_height: u32) {
        self.cursor_line = self.total_lines.saturating_sub(1);
        self.scroll_to_cursor(view_height);
        self.clamp_cursor_col();
        if let Some(range) = self.visual_mode.as_mut() {
            range.cursor_line = self.cursor_line;
            range.cursor_col = self.cursor_col;
        }
    }

    /// Clamp `cursor_col` to `min(cursor_col, line_width - 1)` for the
    /// current `cursor_line`. Called after every vertical move to match vim's
    /// column-clamping behaviour.
    pub fn clamp_cursor_col(&mut self) {
        let width = self.current_line_width();
        if width == 0 {
            self.cursor_col = 0;
        } else {
            self.cursor_col = self.cursor_col.min(width - 1);
        }
    }

    /// Display width (in terminal cells) of the logical line at `cursor_line`.
    ///
    /// Walks the rendered blocks to find the [`Line`] at `cursor_line` and sums
    /// the display width of each span's content via `UnicodeWidthStr`. Returns 0
    /// for empty lines or when `cursor_line` is out of range.
    ///
    /// Not cached — only called on key events, not per frame.
    pub fn current_line_width(&self) -> u16 {
        let mut offset = 0u32;
        for block in &self.rendered {
            let h = block.height();
            if self.cursor_line < offset + h {
                let local_visual = self.cursor_line - offset;
                let width = match block {
                    DocBlock::Text { text, .. } => {
                        // `cursor_line` is in visual rows now (1.18.4) but
                        // `text.lines` is still indexed by logical line —
                        // walk the lines to convert. Without this, a cursor
                        // sitting on a wrapped paragraph mapped past the
                        // end of `text.lines`, returning width 0, which
                        // forced `clamp_cursor_col` to reset `cursor_col`
                        // to 0 on every `j`/`k` and clamped the Right arrow
                        // to a no-op.
                        let logical_idx = super::visual_rows::visual_row_to_logical_in_block(
                            text,
                            self.layout_width,
                            local_visual,
                        ) as usize;
                        text.lines
                            .get(logical_idx)
                            .map_or(0, |l| crate::text_layout::measure(&l.spans) as usize)
                    }
                    // Mermaid and Table blocks have opaque content — treat them
                    // as having no horizontal extent for cursor purposes.
                    DocBlock::Mermaid { .. } | DocBlock::Table(_) => 0,
                };
                return crate::cast::u16_sat(width);
            }
            offset += h;
        }
        0
    }

    /// Joined span text of the logical line under the cursor, or `None` for
    /// Mermaid / Table blocks (which have no per-cell cursor concept) and
    /// blank/out-of-range positions.
    ///
    /// Used by the word-jump helpers to find whitespace-separated word
    /// boundaries. Indexed by char position (display column on
    /// ASCII-only lines, which covers the vast majority of prose markdown).
    fn current_line_text(&self) -> Option<String> {
        let mut offset = 0u32;
        for block in &self.rendered {
            let h = block.height();
            if self.cursor_line < offset + h {
                let local_visual = self.cursor_line - offset;
                return match block {
                    DocBlock::Text { text, .. } => {
                        let logical_idx = super::visual_rows::visual_row_to_logical_in_block(
                            text,
                            self.layout_width,
                            local_visual,
                        ) as usize;
                        text.lines.get(logical_idx).map(|l| {
                            l.spans
                                .iter()
                                .map(|s| s.content.as_ref())
                                .collect::<String>()
                        })
                    }
                    DocBlock::Mermaid { .. } | DocBlock::Table(_) => None,
                };
            }
            offset += h;
        }
        None
    }

    /// Move the cursor to the start of the next word on the current line, or
    /// to end-of-line if no further word exists. Mirrors Option+Right on
    /// macOS and vim's `w` for whitespace-segmented words.
    ///
    /// Updates the visual-mode cursor end too so range selection extends
    /// naturally with word jumps.
    pub fn cursor_word_forward(&mut self) {
        let Some(text) = self.current_line_text() else {
            return;
        };
        self.cursor_col = next_word_col(&text, self.cursor_col);
        if let Some(range) = self.visual_mode.as_mut() {
            range.cursor_col = self.cursor_col;
        }
    }

    /// Move the cursor to the start of the previous word on the current
    /// line, or to col 0 if already at the first word. Mirrors Option+Left
    /// on macOS and vim's `b`.
    pub fn cursor_word_backward(&mut self) {
        let Some(text) = self.current_line_text() else {
            return;
        };
        self.cursor_col = prev_word_col(&text, self.cursor_col);
        if let Some(range) = self.visual_mode.as_mut() {
            range.cursor_col = self.cursor_col;
        }
    }

    /// Jump cursor to col 0 of the current line. Mirrors Home / Cmd+Left
    /// (where the terminal forwards Cmd+Left as Home, e.g. macOS Terminal).
    pub fn cursor_line_start(&mut self) {
        self.cursor_col = 0;
        if let Some(range) = self.visual_mode.as_mut() {
            range.cursor_col = 0;
        }
    }

    /// Jump cursor to the last column of the current line. Mirrors End /
    /// Cmd+Right.
    pub fn cursor_line_end(&mut self) {
        let max = self.current_line_width().saturating_sub(1);
        self.cursor_col = max;
        if let Some(range) = self.visual_mode.as_mut() {
            range.cursor_col = max;
        }
    }

    /// Adjust `scroll_offset` so the cursor sits as close to the vertical
    /// centre of the viewport as possible.
    ///
    /// Intended for long-distance cursor jumps (search-result open, go-to-line)
    /// where the user wants to see context around the target line rather than
    /// landing at the viewport edge.  Short-distance movement (`j`/`k`/etc.)
    /// should continue to use [`scroll_to_cursor`] so the scroll only tracks
    /// the cursor when the cursor would otherwise leave the screen.
    ///
    /// # Arguments
    ///
    /// * `view_height` – visible viewport height in display lines.
    pub fn scroll_to_cursor_centered(&mut self, view_height: u32) {
        let vh = view_height.max(1);
        let half = vh / 2;
        self.scroll_offset = self.cursor_line.saturating_sub(half);
        let max = self.total_lines.saturating_sub(vh / 2);
        self.scroll_offset = self.scroll_offset.min(max);
    }

    /// Adjust `scroll_offset` so the cursor is visible in the viewport.
    ///
    /// Matches vim's default `scrolloff=0` behaviour: the scroll moves only
    /// as much as needed to bring the cursor onto the screen.
    ///
    /// # Arguments
    ///
    /// * `view_height` – visible viewport height in display lines.
    pub fn scroll_to_cursor(&mut self, view_height: u32) {
        let vh = view_height.max(1);
        if self.cursor_line < self.scroll_offset {
            // Cursor went above the viewport top — scroll up.
            self.scroll_offset = self.cursor_line;
        } else if self.cursor_line >= self.scroll_offset + vh {
            // Cursor went below the viewport bottom — scroll down.
            self.scroll_offset = self.cursor_line.saturating_sub(vh - 1);
        }
        // Clamp scroll so we never show an entirely blank viewport at the end.
        let max = self.total_lines.saturating_sub(vh / 2);
        self.scroll_offset = self.scroll_offset.min(max);
    }
}

/// Cached rendering of a single table at a given layout width.
#[derive(Debug)]
pub struct TableLayout {
    pub text: Text<'static>,
}

/// Find the start of the next whitespace-separated word at or after
/// `current_col` in `text`. If no further word exists, return the column
/// just past the last char (so the cursor parks at end-of-line).
///
/// Word definition: a maximal run of non-whitespace chars. A "word jump"
/// from inside a word skips the rest of that word + any whitespace,
/// landing on the first char of the next word. From whitespace, it skips
/// the whitespace and lands on the next word.
///
/// Indexed by char position; on ASCII-only lines this matches the display
/// column space `cursor_col` lives in. Multi-byte / wide chars are out of
/// scope (rare in prose markdown; the existing `h`/`l` arrows have the
/// same approximation).
fn next_word_col(text: &str, current_col: u16) -> u16 {
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut col = current_col as usize;
    if col >= len {
        return crate::cast::u16_sat(len.saturating_sub(1));
    }
    // Skip the rest of the current word (if we're inside one).
    while col < len && !chars[col].is_whitespace() {
        col += 1;
    }
    // Skip the whitespace gap.
    while col < len && chars[col].is_whitespace() {
        col += 1;
    }
    if col >= len {
        crate::cast::u16_sat(len.saturating_sub(1))
    } else {
        crate::cast::u16_sat(col)
    }
}

/// Mirror of [`next_word_col`] in the other direction. Returns 0 when
/// already at or before the first word.
fn prev_word_col(text: &str, current_col: u16) -> u16 {
    let chars: Vec<char> = text.chars().collect();
    if current_col == 0 || chars.is_empty() {
        return 0;
    }
    let mut col = (current_col as usize).min(chars.len()).saturating_sub(1);
    // If we started in whitespace, skip back to the previous word's end.
    while col > 0 && chars[col].is_whitespace() {
        col -= 1;
    }
    // If we started inside a word but not at its first char, jump to the
    // word's start. Otherwise we're already at a word boundary; keep
    // walking back through the gap to the previous word.
    if col > 0 && !chars[col - 1].is_whitespace() {
        // Inside a word; back up to its first char.
        while col > 0 && !chars[col - 1].is_whitespace() {
            col -= 1;
        }
    } else {
        // At the first char of a word (or the boundary). Skip the gap…
        while col > 0 && chars[col - 1].is_whitespace() {
            col -= 1;
        }
        // …then back up to the start of that previous word.
        while col > 0 && !chars[col - 1].is_whitespace() {
            col -= 1;
        }
    }
    crate::cast::u16_sat(col)
}

#[cfg(test)]
mod word_jump_tests {
    use super::{next_word_col, prev_word_col};

    /// `next_word_col` from inside a word jumps past the word and gap to the
    /// next word's start.
    #[test]
    fn next_word_from_inside_word() {
        // "hello world foo"
        //       ^   col=4 → next word "world" starts at col 6
        assert_eq!(next_word_col("hello world foo", 4), 6);
    }

    /// From whitespace, `next_word_col` lands on the next word's first char.
    #[test]
    fn next_word_from_whitespace() {
        // "abc   def"
        //     ^^^   col=3 (whitespace) → "def" starts at col 6
        assert_eq!(next_word_col("abc   def", 3), 6);
    }

    /// From inside the last word, `next_word_col` parks at end-of-line.
    #[test]
    fn next_word_from_last_word() {
        // "abc def"  len=7, last index=6 → end-of-line is col 6
        assert_eq!(next_word_col("abc def", 5), 6);
    }

    /// `prev_word_col` from inside a word jumps to that word's start.
    #[test]
    fn prev_word_from_mid_word() {
        // "hello world foo"
        //         ^ col=8 (inside "world") → start of "world" is col 6
        assert_eq!(prev_word_col("hello world foo", 8), 6);
    }

    /// From the first char of a word, `prev_word_col` jumps back to the
    /// previous word's start.
    #[test]
    fn prev_word_from_word_start() {
        // "hello world foo"
        //       ^ col=6 (start of "world") → previous word "hello" starts at 0
        assert_eq!(prev_word_col("hello world foo", 6), 0);
    }

    /// From col 0, `prev_word_col` stays at 0.
    #[test]
    fn prev_word_at_start_stays() {
        assert_eq!(prev_word_col("hello world", 0), 0);
    }

    /// From whitespace, `prev_word_col` jumps to the previous word's start.
    #[test]
    fn prev_word_from_whitespace() {
        // "abc   def"
        //      ^ col=4 (in the gap) → "abc" starts at 0
        assert_eq!(prev_word_col("abc   def", 4), 0);
    }
}
