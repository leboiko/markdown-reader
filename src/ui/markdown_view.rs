use crate::app::App;
use crate::markdown::{DocBlock, TableBlockId, update_mermaid_heights};
use crate::theme::Palette;

/// How many display lines above and below the viewport to prefetch mermaid
/// renders. Large enough that normal scrolling rarely hits an unrendered
/// placeholder; small enough that unused diagrams don't waste CPU.
const LAZY_RENDER_LOOKAHEAD: u32 = 50;
use crate::ui::table_render::layout_table;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;

/// Cached rendering of a single table at a given layout width.
#[derive(Debug)]
pub struct TableLayout {
    pub text: Text<'static>,
}

/// Anchor and cursor bounds of a visual-line selection in the viewer.
///
/// Both values are absolute logical line indices in the rendered output
/// (same coordinate space as `cursor_line`).  The selected range is
/// `min(anchor, cursor)..=max(anchor, cursor)` — inclusive on both ends,
/// normalising the direction of selection so callers need not distinguish
/// upward from downward selections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualRange {
    /// The line where the selection began (does not move during extension).
    pub anchor: u32,
    /// The current end of the selection, tracking the cursor.
    pub cursor: u32,
}

impl VisualRange {
    /// Smaller of `anchor` and `cursor` — the top of the selection.
    pub fn top(&self) -> u32 {
        self.anchor.min(self.cursor)
    }

    /// Larger of `anchor` and `cursor` — the bottom of the selection.
    pub fn bottom(&self) -> u32 {
        self.anchor.max(self.cursor)
    }

    /// `true` if the absolute logical `line` is inside the selection (inclusive).
    pub fn contains(&self, line: u32) -> bool {
        line >= self.top() && line <= self.bottom()
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
        self.total_lines = blocks.iter().map(|b| b.height()).sum();
        self.rendered = blocks;
        self.recompute_positions();
        self.content = content;
        self.file_name = file_name;
        self.current_path = Some(path);
        self.scroll_offset = 0;
        self.cursor_line = 0;
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
    /// track the new cursor position; the `anchor` stays fixed.
    ///
    /// Does not update `scroll_offset`; call [`scroll_to_cursor`] afterward.
    pub fn cursor_down(&mut self, n: u32) {
        let max = self.total_lines.saturating_sub(1);
        self.cursor_line = self.cursor_line.saturating_add(n).min(max);
        if let Some(range) = self.visual_mode.as_mut() {
            range.cursor = self.cursor_line;
        }
    }

    /// Move the cursor up by `n` logical lines, saturating at 0.
    ///
    /// When visual mode is active, the selection's `cursor` end is extended to
    /// track the new cursor position; the `anchor` stays fixed.
    ///
    /// Does not update `scroll_offset`; call [`scroll_to_cursor`] afterward.
    pub fn cursor_up(&mut self, n: u32) {
        self.cursor_line = self.cursor_line.saturating_sub(n);
        if let Some(range) = self.visual_mode.as_mut() {
            range.cursor = self.cursor_line;
        }
    }

    /// Jump the cursor to the first line and reset the scroll to the top.
    ///
    /// When visual mode is active, extends the selection to the top of the document.
    pub fn cursor_to_top(&mut self) {
        self.cursor_line = 0;
        self.scroll_offset = 0;
        if let Some(range) = self.visual_mode.as_mut() {
            range.cursor = 0;
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
        if let Some(range) = self.visual_mode.as_mut() {
            range.cursor = self.cursor_line;
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

/// Render the markdown preview panel into `area`.
pub fn draw(f: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let p = app.palette;

    let border_style = if focused {
        p.border_focused_style()
    } else {
        p.border_style()
    };

    let active_tab = app.tabs.active_tab();
    let file_name = active_tab.map(|t| t.view.file_name.as_str()).unwrap_or("");

    let title: Cow<str> = if file_name.is_empty() {
        Cow::Borrowed(" Preview ")
    } else {
        Cow::Owned(format!(" {file_name} "))
    };

    let block = Block::default()
        .title(title.as_ref())
        .title_style(p.title_style())
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(p.background));

    app.tabs.view_height = area.height.saturating_sub(2) as u32;

    let has_content = app
        .tabs
        .active_tab()
        .map(|t| !t.view.content.is_empty())
        .unwrap_or(false);

    if !has_content {
        let empty = Paragraph::new("No file selected. Select a markdown file from the tree.")
            .style(p.dim_style().bg(p.background))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    let view_height = app.tabs.view_height;
    let inner = block.inner(area);
    f.render_widget(block, area);

    // When line numbers are on, the gutter steals cells from the left of the
    // content area. Tables must be laid out against the actual content width
    // or their rows wrap inside ratatui's Paragraph and the grid breaks.
    let effective_width = if app.show_line_numbers {
        let estimate = app
            .tabs
            .active_tab()
            .map(|t| t.view.total_lines.max(10))
            .unwrap_or(10);
        let num_digits = (estimate.ilog10() + 1).max(4) as u16;
        let gutter_width = num_digits + 3;
        inner.width.saturating_sub(gutter_width)
    } else {
        inner.width
    };

    // If the effective width has changed, all table layout caches are stale.
    // Recompute heights for every table block and update total_lines.
    {
        let tab = app.tabs.active_tab_mut().unwrap();
        if tab.view.layout_width != effective_width {
            tab.view.layout_width = effective_width;
            tab.view.table_layouts.clear();

            for doc_block in &mut tab.view.rendered {
                if let DocBlock::Table(table) = doc_block {
                    let (text, height, _was_truncated) = layout_table(table, effective_width, &p);
                    table.rendered_height = height;
                    tab.view
                        .table_layouts
                        .insert(table.id, TableLayout { text });
                }
            }

            // Width changed — all block heights may have shifted; always recompute.
            update_mermaid_heights(&tab.view.rendered, &app.mermaid_cache);
            tab.view.total_lines = tab.view.rendered.iter().map(|b| b.height()).sum();
            tab.view.recompute_positions();
            let max_scroll = tab.view.total_lines.saturating_sub(view_height / 2);
            tab.view.scroll_offset = tab.view.scroll_offset.min(max_scroll);
        } else {
            // Populate cache for any tables not yet laid out (e.g. first draw).
            // Track whether any new table was added so we know to recompute positions.
            let mut layout_changed = false;
            for doc_block in &mut tab.view.rendered {
                if let DocBlock::Table(table) = doc_block
                    && let std::collections::hash_map::Entry::Vacant(e) =
                        tab.view.table_layouts.entry(table.id)
                {
                    let (text, height, _was_truncated) = layout_table(table, effective_width, &p);
                    table.rendered_height = height;
                    e.insert(TableLayout { text });
                    layout_changed = true;
                }
            }
            // update_mermaid_heights returns true when any block's height changed.
            // Only recompute positions (O(blocks)) when something actually moved —
            // calling it unconditionally every frame was the source of UI freezes on
            // large documents.
            let mermaid_changed = update_mermaid_heights(&tab.view.rendered, &app.mermaid_cache);
            if layout_changed || mermaid_changed {
                tab.view.total_lines = tab.view.rendered.iter().map(|b| b.height()).sum();
                tab.view.recompute_positions();
                let max_scroll = tab.view.total_lines.saturating_sub(view_height / 2);
                tab.view.scroll_offset = tab.view.scroll_offset.min(max_scroll);
            }
        }
    }

    let tab = app.tabs.active_tab().unwrap();
    let scroll_offset = tab.view.scroll_offset;
    let cursor_line = tab.view.cursor_line;
    // Copy the visual selection so we can use it while iterating over blocks
    // without holding a borrow into `app.tabs`.
    let visual_mode = tab.view.visual_mode;

    let doc_search_query =
        if !tab.doc_search.query.is_empty() && !tab.doc_search.match_lines.is_empty() {
            Some((
                tab.doc_search.query.clone(),
                tab.doc_search
                    .match_lines
                    .get(tab.doc_search.current_match)
                    .copied(),
            ))
        } else {
            None
        };

    // Build a flat list of (block_start_line, block) to find which blocks
    // intersect [scroll_offset, scroll_offset + view_height).
    let viewport_end = scroll_offset + view_height;

    // Mermaid blocks within this extended window are queued for rendering even
    // if not yet visible, so that scrolling rarely hits an unrendered placeholder.
    let lookahead_start = scroll_offset.saturating_sub(LAZY_RENDER_LOOKAHEAD);
    let lookahead_end = viewport_end + LAZY_RENDER_LOOKAHEAD;

    // We can't hold a borrow into `app.tabs` while also accessing
    // `app.mermaid_cache`, so we collect rendering instructions first.
    struct TextDraw {
        y: u16,
        height: u16,
        text: Text<'static>,
        first_line_number: u32,
    }
    struct MermaidDraw {
        y: u16,
        height: u16,
        fully_visible: bool,
        id: crate::markdown::MermaidBlockId,
        source: String,
        /// Absolute logical-line index where this block starts in the document.
        block_start: u32,
        /// Total height of this block in logical lines.
        block_height: u32,
        /// Visual selection at the time of the draw instruction capture.
        visual_mode: Option<VisualRange>,
    }

    let mut text_draws: Vec<TextDraw> = Vec::new();
    let mut mermaid_draws: Vec<MermaidDraw> = Vec::new();
    let mut mermaid_to_queue: Vec<(crate::markdown::MermaidBlockId, String)> = Vec::new();

    {
        let tab = app.tabs.active_tab().unwrap();
        let mut block_start = 0u32;

        for doc_block in &tab.view.rendered {
            let block_height = doc_block.height();
            let block_end = block_start + block_height;

            // Queue mermaid blocks within the lookahead window.
            if let DocBlock::Mermaid { id, source, .. } = doc_block
                && block_end > lookahead_start
                && block_start < lookahead_end
            {
                mermaid_to_queue.push((*id, source.clone()));
            }

            if block_end > scroll_offset && block_start < viewport_end {
                // Lines within this block that are visible.
                let clip_start = scroll_offset.saturating_sub(block_start);
                let clip_end = (viewport_end - block_start).min(block_height);
                let visible_lines = clip_end.saturating_sub(clip_start);

                // Y offset in the inner rect.
                let y_in_viewport = block_start.saturating_sub(scroll_offset);
                let rect_y = inner.y.saturating_add(y_in_viewport as u16);

                if rect_y < inner.y + inner.height && visible_lines > 0 {
                    let draw_height =
                        visible_lines.min((inner.y + inner.height - rect_y) as u32) as u16;

                    match doc_block {
                        DocBlock::Text { text, .. } => {
                            // Slice only the visible lines from this Text block.
                            let start = clip_start as usize;
                            let end =
                                (clip_start + visible_lines).min(text.lines.len() as u32) as usize;
                            let mut visible_text = if let Some((query, current_line)) =
                                &doc_search_query
                            {
                                let full_text =
                                    highlight_matches(text, query, *current_line, block_start, &p);
                                let sliced_lines = full_text.lines[start..end].to_vec();
                                Text::from(sliced_lines)
                            } else {
                                let sliced_lines = text.lines[start..end].to_vec();
                                Text::from(sliced_lines)
                            };

                            // Apply highlight(s) when the viewer has focus.
                            // In visual mode every line in the selection gets highlighted;
                            // in normal mode only the single cursor row is highlighted.
                            let block_end = block_start + block_height;
                            if focused {
                                apply_block_highlight(
                                    &mut visible_text.lines,
                                    visual_mode,
                                    cursor_line,
                                    block_start,
                                    block_end,
                                    start,
                                    p.selection_bg,
                                );
                            }

                            text_draws.push(TextDraw {
                                y: rect_y,
                                height: draw_height,
                                text: visible_text,
                                first_line_number: block_start + clip_start + 1,
                            });
                        }
                        DocBlock::Mermaid { id, source, .. } => {
                            // Render the image when the block is as visible as
                            // it can get: fully visible for small blocks, or
                            // filling the viewport for blocks taller than the
                            // viewport. Show a placeholder only while the block
                            // is entering/exiting the viewport edges.
                            let max_renderable = block_height.min(inner.height as u32);
                            let fully_visible = visible_lines >= max_renderable
                                && draw_height as u32 >= max_renderable;
                            mermaid_draws.push(MermaidDraw {
                                y: rect_y,
                                height: draw_height,
                                fully_visible,
                                id: *id,
                                source: source.clone(),
                                block_start,
                                block_height,
                                visual_mode,
                            });
                        }
                        DocBlock::Table(table) => {
                            // Slice visible lines from the cached rendered text.
                            if let Some(cached) = tab.view.table_layouts.get(&table.id) {
                                let start = clip_start as usize;
                                let end = (clip_start + visible_lines)
                                    .min(cached.text.lines.len() as u32)
                                    as usize;
                                let mut visible_text =
                                    if let Some((query, current_line)) = &doc_search_query {
                                        let full = highlight_matches(
                                            &cached.text,
                                            query,
                                            *current_line,
                                            block_start,
                                            &p,
                                        );
                                        Text::from(full.lines[start..end].to_vec())
                                    } else {
                                        Text::from(cached.text.lines[start..end].to_vec())
                                    };
                                // Apply highlight(s) when the viewer has focus.
                                // In visual mode every line in the selection range is
                                // highlighted; in normal mode only the cursor row.
                                let block_end = block_start + block_height;
                                if focused {
                                    apply_block_highlight(
                                        &mut visible_text.lines,
                                        visual_mode,
                                        cursor_line,
                                        block_start,
                                        block_end,
                                        start,
                                        p.selection_bg,
                                    );
                                }
                                text_draws.push(TextDraw {
                                    y: rect_y,
                                    height: draw_height,
                                    text: visible_text,
                                    first_line_number: block_start + clip_start + 1,
                                });
                            }
                        }
                    }
                }
            }

            block_start = block_end;
            if block_start >= lookahead_end {
                break;
            }
        }
    }

    // Queue any mermaid diagrams in the lookahead window that haven't been
    // rendered yet. This is the only site that calls ensure_queued — rendering
    // is fully lazy and driven by viewport proximity.
    if let Some(tx) = &app.action_tx {
        let in_tmux = std::env::var("TMUX").is_ok();
        let tx = tx.clone();
        let bg_rgb = match p.background {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => (0, 0, 0),
        };
        for (id, source) in mermaid_to_queue {
            app.mermaid_cache
                .ensure_queued(id, &source, app.picker.as_ref(), &tx, in_tmux, bg_rgb);
        }
    }

    let total_doc_lines = app
        .tabs
        .active_tab()
        .map(|t| t.view.total_lines)
        .unwrap_or(0);

    // Render text blocks.
    for td in text_draws {
        let rect = Rect {
            x: inner.x,
            y: td.y,
            width: inner.width,
            height: td.height,
        };
        if app.show_line_numbers {
            render_text_with_gutter(f, rect, td.text, td.first_line_number, total_doc_lines, &p);
        } else {
            let para = Paragraph::new(td.text).wrap(Wrap { trim: false });
            f.render_widget(para, rect);
        }
    }

    // Render mermaid blocks.
    for md in mermaid_draws {
        let rect = Rect {
            x: inner.x,
            y: md.y,
            width: inner.width,
            height: md.height,
        };
        let params = MermaidDrawParams {
            fully_visible: md.fully_visible,
            id: md.id,
            source: &md.source,
            focused,
            cursor_line,
            block_start: md.block_start,
            block_end: md.block_start + md.block_height,
            visual_mode: md.visual_mode,
        };
        draw_mermaid_block(f, app, rect, &p, &params);
    }
}

/// All parameters needed to draw a single mermaid block.
///
/// Bundles the per-block rendering state and cursor context into one struct so
/// [`draw_mermaid_block`] stays within clippy's 7-argument limit.
struct MermaidDrawParams<'a> {
    /// Whether the image is fully visible in the viewport.
    fully_visible: bool,
    /// Opaque block identifier used to look up the cache entry.
    id: crate::markdown::MermaidBlockId,
    /// Raw mermaid source, displayed when the image is not available.
    source: &'a str,
    /// Whether the viewer panel currently has keyboard focus.
    focused: bool,
    /// Absolute logical-line index of the cursor.
    cursor_line: u32,
    /// Inclusive start of the block in absolute logical lines.
    block_start: u32,
    /// Exclusive end of the block in absolute logical lines.
    block_end: u32,
    /// Active visual-line selection, or `None` in normal mode.
    visual_mode: Option<VisualRange>,
}

/// Draw a mermaid block at the given rect, looking up the cache entry.
///
/// When `params.fully_visible` is false (the block is partially scrolled on-
/// or off-screen), skip image rendering and show a placeholder; otherwise the
/// image widget would re-fit to the shrinking rect and visibly jitter.
fn draw_mermaid_block(
    f: &mut Frame,
    app: &mut App,
    rect: Rect,
    p: &Palette,
    params: &MermaidDrawParams,
) {
    use crate::mermaid::MermaidEntry;

    let entry = app.mermaid_cache.get_mut(&params.id);

    // Helper: true when the cursor is inside this block and the viewer is focused.
    let cursor_in_block = params.focused
        && params.cursor_line >= params.block_start
        && params.cursor_line < params.block_end;

    match entry {
        None => {
            render_mermaid_placeholder(f, rect, "mermaid diagram", p);
        }
        Some(MermaidEntry::Pending) => {
            render_mermaid_placeholder(f, rect, "rendering\u{2026}", p);
        }
        Some(MermaidEntry::Ready { protocol, .. }) => {
            if params.fully_visible {
                use ratatui_image::{Resize, StatefulImage};
                f.render_widget(
                    Block::default().style(Style::default().bg(p.background)),
                    rect,
                );
                // Render background bars BEFORE the image so they sit underneath.
                // In visual mode draw a bar for every selected row; in normal mode
                // draw one bar for the cursor row.  The image overwrites most of
                // each bar, leaving only a thin coloured strip around the padding.
                let highlighted_rows: Vec<u32> = match params.visual_mode {
                    Some(range) => {
                        let top = range.top().max(params.block_start) - params.block_start;
                        let bottom = range.bottom().min(params.block_end.saturating_sub(1))
                            - params.block_start;
                        (top..=bottom).collect()
                    }
                    None if cursor_in_block => {
                        vec![params.cursor_line - params.block_start]
                    }
                    None => vec![],
                };
                for row_offset in highlighted_rows {
                    let row_offset = row_offset as u16;
                    if row_offset < rect.height {
                        let bar_rect = Rect {
                            x: rect.x,
                            y: rect.y + row_offset,
                            width: rect.width,
                            height: 1,
                        };
                        f.render_widget(
                            Block::default().style(Style::default().bg(p.selection_bg)),
                            bar_rect,
                        );
                    }
                }
                let padded = padded_rect(rect, 4, 1);
                let image = StatefulImage::new().resize(Resize::Fit(None));
                f.render_stateful_widget(image, padded, protocol.as_mut());
            } else {
                render_mermaid_placeholder(f, rect, "scroll to view diagram", p);
            }
        }
        Some(MermaidEntry::Failed(msg)) => {
            let footer = format!("[mermaid \u{2014} {}]", truncate(msg, 60));
            let mut text = render_mermaid_source_text(params.source, &footer, p);
            // Apply cursor/selection highlight to the source-text fallback.
            if params.focused {
                apply_block_highlight(
                    &mut text.lines,
                    params.visual_mode,
                    params.cursor_line,
                    params.block_start,
                    params.block_end,
                    0,
                    p.selection_bg,
                );
            }
            render_mermaid_source_styled(f, rect, text, p);
        }
        Some(MermaidEntry::SourceOnly(reason)) => {
            let footer = format!("[mermaid \u{2014} {}]", reason);
            let mut text = render_mermaid_source_text(params.source, &footer, p);
            // Apply cursor/selection highlight to the source-text fallback.
            if params.focused {
                apply_block_highlight(
                    &mut text.lines,
                    params.visual_mode,
                    params.cursor_line,
                    params.block_start,
                    params.block_end,
                    0,
                    p.selection_bg,
                );
            }
            render_mermaid_source_styled(f, rect, text, p);
        }
    }
}

/// Shrink `rect` by `h` cells on the left/right and `v` cells on the top/bottom.
/// If the rect is smaller than the total padding, returns it unchanged.
fn padded_rect(rect: Rect, h: u16, v: u16) -> Rect {
    if rect.width <= h * 2 || rect.height <= v * 2 {
        return rect;
    }
    Rect {
        x: rect.x + h,
        y: rect.y + v,
        width: rect.width - h * 2,
        height: rect.height - v * 2,
    }
}

fn render_mermaid_placeholder(f: &mut Frame, rect: Rect, msg: &str, p: &Palette) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(p.border_style())
        .style(Style::default().bg(p.background));
    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if inner.height > 0 {
        let line = Line::from(Span::styled(msg.to_string(), p.dim_style()));
        let para =
            Paragraph::new(Text::from(vec![line])).alignment(ratatui::layout::Alignment::Center);
        // Center vertically.
        let y_offset = inner.height / 2;
        let target = Rect {
            y: inner.y + y_offset,
            height: 1,
            ..inner
        };
        f.render_widget(para, target);
    }
}

/// Build the styled `Text` for a mermaid source-fallback display.
///
/// Separating text construction from rendering lets callers mutate the lines
/// (e.g., apply cursor highlight) before committing to the frame buffer.
fn render_mermaid_source_text(source: &str, footer: &str, p: &Palette) -> Text<'static> {
    let code_style = Style::default().fg(p.code_fg).bg(p.code_bg);
    let dim_style = p.dim_style();

    let mut lines: Vec<Line<'static>> = source
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), code_style)))
        .collect();
    lines.push(Line::from(Span::styled(footer.to_string(), dim_style)));
    Text::from(lines)
}

/// Render a pre-built mermaid source `Text` with a border block.
fn render_mermaid_source_styled(f: &mut Frame, rect: Rect, text: Text<'static>, p: &Palette) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(p.border_style())
        .style(Style::default().bg(p.background));
    let para = Paragraph::new(text).block(block).wrap(Wrap { trim: false });
    f.render_widget(para, rect);
}

/// Render a slice of text with an absolute-line-number gutter.
///
/// `first_line_number` is the 1-based absolute display line of the slice's first row;
/// `total_doc_lines` is used to size the gutter so width is stable across blocks.
fn render_text_with_gutter(
    f: &mut Frame,
    rect: Rect,
    text: Text<'static>,
    first_line_number: u32,
    total_doc_lines: u32,
    p: &Palette,
) {
    let num_digits = if total_doc_lines == 0 {
        4
    } else {
        (total_doc_lines.ilog10() + 1).max(4)
    };
    let gutter_width = num_digits + 3;

    let chunks = Layout::horizontal([Constraint::Length(gutter_width as u16), Constraint::Min(0)])
        .split(rect);

    // The content pane uses `Paragraph::wrap(Wrap { trim: false })`, so a
    // single logical `Line` can occupy multiple visual rows on narrow
    // terminals. The gutter must match that per-row layout: emit the line
    // number on the row where the logical line starts and blank padding on
    // each continuation row, so the number stays visually adjacent to its
    // content.
    let content_width = chunks[1].width;
    let gutter_style = Style::new().fg(p.gutter);
    let mut gutter_lines: Vec<Line<'static>> = Vec::with_capacity(text.lines.len());
    let blank_span = Span::styled(
        format!("{:>width$} | ", "", width = num_digits as usize),
        gutter_style,
    );
    for (i, line) in text.lines.iter().enumerate() {
        gutter_lines.push(Line::from(Span::styled(
            format!(
                "{:>width$} | ",
                first_line_number + i as u32,
                width = num_digits as usize
            ),
            gutter_style,
        )));
        let wraps = line_visual_rows(line, content_width);
        for _ in 1..wraps {
            gutter_lines.push(Line::from(blank_span.clone()));
        }
    }

    f.render_widget(Paragraph::new(Text::from(gutter_lines)), chunks[0]);
    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), chunks[1]);
}

/// Decide which lines in a visible block slice need highlighting and apply the
/// background colour to each.
///
/// In **visual mode** every absolute logical line that falls inside the
/// `VisualRange` and is also within the visible clip is highlighted.  In
/// **normal mode** only the single cursor row is highlighted.
///
/// # Arguments
///
/// * `lines`       – mutable slice of visible lines already clipped to the viewport.
/// * `visual_mode` – current visual selection, or `None` for normal mode.
/// * `cursor_line` – absolute logical cursor position.
/// * `block_start` – absolute logical line where this block starts.
/// * `block_end`   – exclusive end of the block in absolute logical lines.
/// * `clip_start`  – index within the block of the first visible line (same as
///   the `start` variable used when slicing `visible_text`).
/// * `bg`          – background colour to apply.
fn apply_block_highlight(
    lines: &mut [Line<'static>],
    visual_mode: Option<VisualRange>,
    cursor_line: u32,
    block_start: u32,
    block_end: u32,
    clip_start: usize,
    bg: Color,
) {
    match visual_mode {
        Some(range) => {
            // Highlight every visible line inside the selection range.
            // Iterate over absolute logical lines that belong to this block
            // and fall within the visible clip.
            let block_visible_start = block_start + clip_start as u32;
            let block_visible_end = block_start + clip_start as u32 + lines.len() as u32;
            for abs in block_visible_start..block_visible_end {
                if range.contains(abs) {
                    let idx = (abs - block_visible_start) as usize;
                    patch_cursor_highlight(lines, idx, bg);
                }
            }
        }
        None => {
            // Normal mode: highlight only the cursor row.
            if cursor_line >= block_start && cursor_line < block_end {
                let cursor_relative = (cursor_line - block_start) as usize;
                if cursor_relative >= clip_start {
                    let idx = cursor_relative - clip_start;
                    patch_cursor_highlight(lines, idx, bg);
                }
            }
        }
    }
}

/// Apply the cursor-highlight background to one row inside a visible slice.
///
/// `lines` is the mutable slice of rendered lines (already clipped to the
/// viewport). `idx` is the 0-based index within that slice that should be
/// highlighted. `bg` is the selection background color.
///
/// Behaviour:
/// - If `idx` is out of bounds, the function is a no-op (no panic).
/// - If the target line has no spans (blank line), a single space span with
///   the background color is injected so the highlight row is still visible.
/// - Otherwise every existing span on that line is patched with `.bg(bg)`.
///
/// All three block types (Text, Table, Mermaid-source) share this helper so
/// the highlight logic lives in exactly one place.
fn patch_cursor_highlight(lines: &mut [Line<'static>], idx: usize, bg: Color) {
    let Some(line) = lines.get_mut(idx) else {
        return;
    };
    if line.spans.is_empty() {
        // Blank line — inject a space so the colored row is visible.
        *line = Line::from(Span::styled(" ".to_string(), Style::default().bg(bg)));
    } else {
        for span in line.spans.iter_mut() {
            span.style = span.style.patch(Style::default().bg(bg));
        }
    }
}

/// Produce a new `Text` with search matches highlighted.
///
/// `block_start` is the absolute display-line offset of `text`'s first row.
/// It is added to the local line index before comparing against
/// `current_line` (which is absolute), so the "current match" color lands
/// on the right row regardless of which block the match lives in.
fn highlight_matches(
    text: &Text<'static>,
    query: &str,
    current_line: Option<u32>,
    block_start: u32,
    p: &Palette,
) -> Text<'static> {
    let query_lower = query.to_lowercase();
    let match_style = Style::default()
        .bg(p.search_match_bg)
        .fg(p.match_fg)
        .add_modifier(Modifier::BOLD);
    let current_style = Style::default()
        .bg(p.current_match_bg)
        .fg(p.match_fg)
        .add_modifier(Modifier::BOLD);

    let lines: Vec<Line<'static>> = text
        .lines
        .iter()
        .enumerate()
        .map(|(line_idx, line)| {
            let line_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if !line_text.to_lowercase().contains(&query_lower) {
                return line.clone();
            }

            let is_current = current_line == Some(block_start + line_idx as u32);
            let hl_style = if is_current {
                current_style
            } else {
                match_style
            };

            let mut new_spans: Vec<Span<'static>> = Vec::new();
            for span in &line.spans {
                split_and_highlight(
                    &span.content,
                    &query_lower,
                    span.style,
                    hl_style,
                    &mut new_spans,
                );
            }
            Line::from(new_spans)
        })
        .collect();

    Text::from(lines)
}

fn split_and_highlight(
    text: &str,
    query_lower: &str,
    base_style: Style,
    highlight_style: Style,
    out: &mut Vec<Span<'static>>,
) {
    let text_lower = text.to_lowercase();
    let mut start = 0;

    while let Some(pos) = text_lower[start..].find(query_lower) {
        let abs_pos = start + pos;

        if abs_pos > start {
            out.push(Span::styled(text[start..abs_pos].to_string(), base_style));
        }

        let match_end = abs_pos + query_lower.len();
        out.push(Span::styled(
            text[abs_pos..match_end].to_string(),
            highlight_style,
        ));

        start = match_end;
    }

    if start < text.len() {
        out.push(Span::styled(text[start..].to_string(), base_style));
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}

/// Compute the number of terminal rows a single rendered `Line` occupies when
/// wrapped to `content_width` columns.
///
/// ratatui's `Paragraph::wrap` word-wraps at `content_width`. A line that is
/// shorter than or equal to `content_width` occupies exactly 1 row. Lines wider
/// than `content_width` overflow into additional rows; we calculate the count
/// with ceiling division. Empty lines (zero width) still occupy 1 row.
fn line_visual_rows(line: &Line, content_width: u16) -> u32 {
    if content_width == 0 {
        return 1;
    }
    let width: usize = line
        .spans
        .iter()
        .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    if width == 0 {
        return 1;
    }
    let cw = content_width as usize;
    width.div_ceil(cw) as u32
}

/// Translate a visual row within the viewport to the absolute logical document
/// line it corresponds to.
///
/// The viewer uses `Paragraph::wrap(Wrap { trim: false })`, so a single logical
/// `Line` that is wider than the content area wraps to multiple visual rows.
/// This means the naive formula `scroll_offset + visual_row_offset` is only
/// correct when every logical line fits on exactly one visual row.
///
/// This function walks the rendered blocks starting at `scroll_offset` and
/// counts visual rows (accounting for wrapping) until it reaches `visual_row`,
/// then returns the logical line index at that position.
///
/// # Arguments
///
/// * `blocks` – the rendered document blocks
/// * `scroll_offset` – the logical document line at the top of the viewport
/// * `visual_row` – 0-based row offset from the top of the content area
/// * `content_width` – width in terminal columns available for text (excluding
///   the gutter when line numbers are shown)
pub fn visual_row_to_logical_line(
    blocks: &[DocBlock],
    scroll_offset: u32,
    visual_row: u32,
    content_width: u16,
) -> u32 {
    let mut remaining_visual = visual_row;
    let mut block_offset = 0u32;

    for block in blocks {
        let block_height = block.height();
        let block_end = block_offset + block_height;

        // Skip blocks that end before the scroll offset.
        if block_end <= scroll_offset {
            block_offset = block_end;
            continue;
        }

        // The first logical line within this block that is visible.
        let clip_start = scroll_offset.saturating_sub(block_offset) as usize;

        match block {
            DocBlock::Text { text, .. } => {
                for (idx, line) in text.lines.iter().enumerate().skip(clip_start) {
                    let rows = line_visual_rows(line, content_width);
                    if remaining_visual < rows {
                        // The clicked row is inside this logical line.
                        return block_offset + idx as u32;
                    }
                    remaining_visual -= rows;
                }
            }
            // Mermaid and Table blocks are opaque (no internal logical lines
            // that can hold links), so treat each visible row as 1 unit.
            DocBlock::Mermaid { cell_height, .. } => {
                let visible_rows = cell_height.get().saturating_sub(clip_start as u32);
                if remaining_visual < visible_rows {
                    // Inside a mermaid block — no links here; return a sentinel
                    // that won't match any link line.
                    return u32::MAX;
                }
                remaining_visual -= visible_rows;
            }
            DocBlock::Table(t) => {
                let visible_rows = t.rendered_height.saturating_sub(clip_start as u32);
                if remaining_visual < visible_rows {
                    return u32::MAX;
                }
                remaining_visual -= visible_rows;
            }
        }

        block_offset = block_end;
    }

    // Fell off the end — return a value that won't match any link.
    u32::MAX
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{DocBlock, HeadingAnchor, LinkInfo};
    use ratatui::text::{Line, Span, Text};

    // ── VisualRange ──────────────────────────────────────────────────────────

    /// A selection anchored at 3 with cursor at 5 should contain 3, 4, 5 and
    /// exclude lines outside the range.
    #[test]
    fn visual_range_contains_inclusive() {
        let r = VisualRange {
            anchor: 3,
            cursor: 5,
        };
        assert!(r.contains(3), "should contain anchor");
        assert!(r.contains(4), "should contain middle");
        assert!(r.contains(5), "should contain cursor");
        assert!(!r.contains(2), "should not contain below anchor");
        assert!(!r.contains(6), "should not contain above cursor");
    }

    /// A reversed selection (anchor > cursor) should behave identically because
    /// `top()`/`bottom()` normalise the direction.
    #[test]
    fn visual_range_contains_reversed() {
        let r = VisualRange {
            anchor: 5,
            cursor: 3,
        };
        assert!(r.contains(3));
        assert!(r.contains(4));
        assert!(r.contains(5));
        assert!(!r.contains(2));
        assert!(!r.contains(6));
    }

    // ── load clears visual_mode ──────────────────────────────────────────────

    #[test]
    fn load_clears_visual_mode() {
        use crate::theme::{Palette, Theme};
        let palette = Palette::from_theme(Theme::Default);
        let mut view = MarkdownViewState {
            visual_mode: Some(VisualRange {
                anchor: 2,
                cursor: 4,
            }),
            ..Default::default()
        };
        view.load(
            std::path::PathBuf::from("/fake/test.md"),
            "test.md".to_string(),
            "hello\nworld\n".to_string(),
            &palette,
            Theme::Default,
        );
        assert_eq!(view.visual_mode, None, "load() must clear visual_mode");
    }

    // ── cursor_down / cursor_up extend visual range ─────────────────────────

    #[test]
    fn cursor_down_in_visual_mode_extends_range() {
        let mut v = MarkdownViewState {
            total_lines: 10,
            cursor_line: 3,
            visual_mode: Some(VisualRange {
                anchor: 3,
                cursor: 3,
            }),
            ..Default::default()
        };
        v.cursor_down(2);
        let range = v.visual_mode.unwrap();
        assert_eq!(range.anchor, 3, "anchor must stay fixed");
        assert_eq!(range.cursor, 5, "cursor must extend down");
    }

    #[test]
    fn cursor_up_in_visual_mode_extends_range() {
        let mut v = MarkdownViewState {
            total_lines: 10,
            cursor_line: 5,
            visual_mode: Some(VisualRange {
                anchor: 5,
                cursor: 5,
            }),
            ..Default::default()
        };
        v.cursor_up(3);
        let range = v.visual_mode.unwrap();
        assert_eq!(range.anchor, 5, "anchor must stay fixed");
        assert_eq!(range.cursor, 2, "cursor must move up");
    }

    /// Helper: build a MarkdownViewState with a given total_lines and
    /// default scroll/cursor at 0.
    fn view_with_lines(total: u32) -> MarkdownViewState {
        MarkdownViewState {
            total_lines: total,
            ..Default::default()
        }
    }

    // ── cursor_down / cursor_up ──────────────────────────────────────────────

    /// Moving down then back up the same amount must return to line 0.
    #[test]
    fn cursor_down_then_up_returns_home() {
        let mut v = view_with_lines(5);
        v.cursor_down(3);
        assert_eq!(v.cursor_line, 3);
        v.cursor_up(3);
        assert_eq!(v.cursor_line, 0);
    }

    /// Moving down more lines than the document has must clamp to the last line.
    #[test]
    fn cursor_down_clamps_to_last_line() {
        let mut v = view_with_lines(3);
        v.cursor_down(100);
        // Last valid line index = total_lines - 1 = 2.
        assert_eq!(v.cursor_line, 2);
    }

    // ── scroll_to_cursor ────────────────────────────────────────────────────

    /// When the cursor is below the viewport, `scroll_to_cursor` scrolls just
    /// enough to bring it to the bottom row of the viewport.
    ///
    /// Document: 10 lines.  view_height: 5.  scroll_offset: 0.  cursor: 7.
    /// Expected: scroll_offset = 7 - (5 - 1) = 3.
    #[test]
    fn cursor_scroll_follows_when_off_screen() {
        let mut v = view_with_lines(10);
        v.scroll_offset = 0;
        v.cursor_line = 7;
        v.scroll_to_cursor(5);
        assert_eq!(v.scroll_offset, 3);
    }

    /// When the cursor is already inside the viewport, `scroll_to_cursor` must
    /// not change `scroll_offset`.
    #[test]
    fn cursor_scroll_unchanged_when_already_visible() {
        let mut v = view_with_lines(20);
        v.scroll_offset = 5;
        v.cursor_line = 7;
        v.scroll_to_cursor(10);
        // cursor (7) is in [5, 15) — no adjustment needed.
        assert_eq!(v.scroll_offset, 5);
    }

    // ── source_line_at ───────────────────────────────────────────────────────

    fn make_text_block_with_sources(source_lines: Vec<u32>) -> DocBlock {
        let n = source_lines.len();
        let text_lines: Vec<Line<'static>> = (0..n)
            .map(|i| Line::from(Span::raw(format!("line {i}"))))
            .collect();
        DocBlock::Text {
            text: Text::from(text_lines),
            links: Vec::<LinkInfo>::new(),
            heading_anchors: Vec::<HeadingAnchor>::new(),
            source_lines,
        }
    }

    /// Querying each logical line in a Text block returns the expected source line.
    #[test]
    fn source_line_at_text_block_exact() {
        use crate::markdown::source_line_at;
        let block = make_text_block_with_sources(vec![0, 1, 2]);
        let blocks = vec![block];
        assert_eq!(source_line_at(&blocks, 0), 0);
        assert_eq!(source_line_at(&blocks, 1), 1);
        assert_eq!(source_line_at(&blocks, 2), 2);
    }

    /// Every logical line within a Table block must return the table's source line.
    /// A table with no `row_source_lines` data (empty) falls back to
    /// `source_line` for every row position (defensive stub path).
    #[test]
    fn source_line_at_table_block_returns_table_start() {
        use crate::markdown::source_line_at;
        use crate::markdown::{TableBlock, TableBlockId};
        let block = DocBlock::Table(TableBlock {
            id: TableBlockId(0),
            headers: vec![],
            rows: vec![],
            alignments: vec![],
            natural_widths: vec![],
            rendered_height: 4,
            source_line: 5,
            row_source_lines: vec![],
        });
        let blocks = vec![block];
        // With no row_source_lines, all positions fall back to source_line = 5.
        assert_eq!(source_line_at(&blocks, 0), 5);
        assert_eq!(source_line_at(&blocks, 3), 5);
    }

    // ── patch_cursor_highlight ───────────────────────────────────────────────

    /// Build a slice of simple `Line`s for highlight tests.
    fn make_lines(count: usize) -> Vec<Line<'static>> {
        (0..count)
            .map(|i| Line::from(Span::raw(format!("line {i}"))))
            .collect()
    }

    /// Patching a middle line must set `bg` on all its spans and leave other
    /// lines unchanged.
    #[test]
    fn patch_cursor_highlight_patches_given_line() {
        use ratatui::style::Color;
        let bg = Color::Rgb(30, 30, 100);
        let mut lines = make_lines(3);
        patch_cursor_highlight(&mut lines, 1, bg);

        // Line 1 spans must carry the bg color.
        for span in &lines[1].spans {
            assert_eq!(span.style.bg, Some(bg), "line 1 span must have bg color");
        }
        // Lines 0 and 2 must be untouched.
        for span in &lines[0].spans {
            assert_eq!(span.style.bg, None, "line 0 must be untouched");
        }
        for span in &lines[2].spans {
            assert_eq!(span.style.bg, None, "line 2 must be untouched");
        }
    }

    /// An empty line at the target index must be replaced with a space span
    /// carrying the bg color so the highlight row is visible.
    #[test]
    fn patch_cursor_highlight_fills_empty_line() {
        use ratatui::style::Color;
        let bg = Color::Rgb(50, 50, 150);
        let mut lines = vec![
            Line::from(Span::raw("before")),
            Line::from(vec![]), // empty — no spans
            Line::from(Span::raw("after")),
        ];
        patch_cursor_highlight(&mut lines, 1, bg);
        assert_eq!(
            lines[1].spans.len(),
            1,
            "empty line must have a filler span injected"
        );
        assert_eq!(
            lines[1].spans[0].content.as_ref(),
            " ",
            "filler span must be a single space"
        );
        assert_eq!(lines[1].spans[0].style.bg, Some(bg));
    }

    /// An out-of-bounds `idx` must not panic or mutate anything.
    #[test]
    fn patch_cursor_highlight_out_of_bounds_noop() {
        use ratatui::style::Color;
        let bg = Color::Rgb(10, 10, 10);
        let mut lines = make_lines(2);
        // idx == 2 is one past the end.
        patch_cursor_highlight(&mut lines, 2, bg);
        // Both lines must be unchanged.
        for line in &lines {
            for span in &line.spans {
                assert_eq!(span.style.bg, None);
            }
        }
    }

    // ── source_line_at — Table with row_source_lines ─────────────────────────

    /// Build a `TableBlock` with explicit `row_source_lines` and verify that
    /// `source_line_at` maps each rendered row to the correct source line.
    ///
    /// Layout (2 body rows):
    ///   0: top border  → header source (5)
    ///   1: header row  → 5
    ///   2: separator   → 5
    ///   3: body[0]     → 7
    ///   4: body[1]     → 8
    ///   5: bottom border → last body (8)
    #[test]
    fn source_line_at_table_block_per_row() {
        use crate::markdown::{TableBlock, TableBlockId, source_line_at};
        let block = DocBlock::Table(TableBlock {
            id: TableBlockId(0),
            headers: vec![vec![Span::raw("H")]],
            rows: vec![vec![vec![Span::raw("a")]], vec![vec![Span::raw("b")]]],
            alignments: vec![pulldown_cmark::Alignment::None],
            natural_widths: vec![1],
            rendered_height: 6,
            source_line: 5,
            row_source_lines: vec![5, 7, 8],
        });
        let blocks = vec![block];
        // top border → header fallback
        assert_eq!(source_line_at(&blocks, 0), 5, "top border -> header");
        // header row
        assert_eq!(source_line_at(&blocks, 1), 5, "header row");
        // separator
        assert_eq!(source_line_at(&blocks, 2), 5, "separator -> header");
        // body[0]
        assert_eq!(source_line_at(&blocks, 3), 7, "body[0]");
        // body[1]
        assert_eq!(source_line_at(&blocks, 4), 8, "body[1]");
        // bottom border → last body fallback
        assert_eq!(source_line_at(&blocks, 5), 8, "bottom border -> last body");
    }

    /// Edge cases: table with only a header (no body rows).
    #[test]
    fn table_row_source_line_helper_boundary_cases() {
        use crate::markdown::{TableBlock, TableBlockId, source_line_at};

        // Header-only: rendered_height = 3 (top border, header, bottom border).
        let header_only = DocBlock::Table(TableBlock {
            id: TableBlockId(1),
            headers: vec![vec![Span::raw("H")]],
            rows: vec![],
            alignments: vec![pulldown_cmark::Alignment::None],
            natural_widths: vec![1],
            rendered_height: 3,
            source_line: 10,
            row_source_lines: vec![10],
        });
        let blocks = vec![header_only];
        // Row 0 = top border → header (10)
        assert_eq!(source_line_at(&blocks, 0), 10);
        // Row 1 = header → 10
        assert_eq!(source_line_at(&blocks, 1), 10);
        // Row 2 = bottom border → last (10)
        assert_eq!(source_line_at(&blocks, 2), 10);

        // Empty row_source_lines: must not panic, must fall back to source_line.
        let empty_rsl = DocBlock::Table(TableBlock {
            id: TableBlockId(2),
            headers: vec![vec![Span::raw("H")]],
            rows: vec![vec![vec![Span::raw("a")]]],
            alignments: vec![pulldown_cmark::Alignment::None],
            natural_widths: vec![1],
            rendered_height: 4,
            source_line: 99,
            row_source_lines: vec![],
        });
        let blocks2 = vec![empty_rsl];
        // All positions must fall back to source_line without panicking.
        for i in 0..4 {
            assert_eq!(source_line_at(&blocks2, i), 99, "empty rsl row {i}");
        }
    }
}
