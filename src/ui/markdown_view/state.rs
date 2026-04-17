use crate::markdown::{DocBlock, TableBlockId};
use crate::theme::Palette;
use ratatui::text::Text;
use std::collections::HashMap;
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

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
    /// (table layout, mermaid height update) so click targets and the link
    /// picker stay aligned with what the user sees on screen.
    pub fn recompute_positions(&mut self) {
        let mut abs_links: Vec<AbsoluteLink> = Vec::new();
        let mut abs_anchors: Vec<AbsoluteAnchor> = Vec::new();
        let mut block_offset = 0u32;
        for block in &self.rendered {
            if let DocBlock::Text {
                links,
                heading_anchors,
                ..
            } = block
            {
                for link in links {
                    abs_links.push(AbsoluteLink {
                        line: block_offset + link.line,
                        col_start: link.col_start,
                        col_end: link.col_end,
                        url: link.url.clone(),
                        text: link.text.clone(),
                    });
                }
                for ha in heading_anchors {
                    abs_anchors.push(AbsoluteAnchor {
                        anchor: ha.anchor.clone(),
                        line: block_offset + ha.line,
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
        self.recompute_positions();
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
                let local = (self.cursor_line - offset) as usize;
                let width = match block {
                    DocBlock::Text { text, .. } => text
                        .lines
                        .get(local)
                        .map_or(0, |l| {
                            l.spans
                                .iter()
                                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                                .sum::<usize>()
                        }),
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
