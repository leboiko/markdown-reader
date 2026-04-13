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
    style::{Modifier, Style},
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
}

impl MarkdownViewState {
    /// Load a file into the viewer, resetting the scroll position.
    pub fn load(&mut self, path: PathBuf, file_name: String, content: String, palette: &Palette) {
        let blocks = crate::markdown::renderer::render_markdown(&content, palette);
        self.total_lines = blocks.iter().map(|b| b.height()).sum();

        // Walk blocks once to build absolute-line link and anchor tables.
        let mut abs_links: Vec<AbsoluteLink> = Vec::new();
        let mut abs_anchors: Vec<AbsoluteAnchor> = Vec::new();
        let mut block_offset = 0u32;
        for block in &blocks {
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
        self.rendered = blocks;
        self.content = content;
        self.file_name = file_name;
        self.current_path = Some(path);
        self.scroll_offset = 0;
        // Invalidate table layout cache. The fresh DocBlock::Table values carry
        // a pessimistic rendered_height that only becomes accurate once the
        // draw loop runs layout_table; forcing a rebuild keeps the hint line
        // and doc-search line numbers in sync after re-renders (e.g. on theme
        // change, live reload, or session restore).
        self.layout_width = 0;
        self.table_layouts.clear();
    }

    pub fn scroll_up(&mut self, n: u16, _view_height: u32) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n as u32);
    }

    pub fn scroll_down(&mut self, n: u16, view_height: u32) {
        let max = self.total_lines.saturating_sub(view_height / 2);
        self.scroll_offset = (self.scroll_offset + n as u32).min(max);
    }

    pub fn scroll_half_page_up(&mut self, view_height: u32) {
        self.scroll_up((view_height / 2) as u16, view_height);
    }

    pub fn scroll_half_page_down(&mut self, view_height: u32) {
        self.scroll_down((view_height / 2) as u16, view_height);
    }

    pub fn scroll_page_up(&mut self, view_height: u32) {
        self.scroll_up(view_height as u16, view_height);
    }

    pub fn scroll_page_down(&mut self, view_height: u32) {
        self.scroll_down(view_height as u16, view_height);
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn scroll_to_bottom(&mut self, view_height: u32) {
        self.scroll_offset = self.total_lines.saturating_sub(view_height / 2);
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

            update_mermaid_heights(&tab.view.rendered, &app.mermaid_cache);
            tab.view.total_lines = tab.view.rendered.iter().map(|b| b.height()).sum();
            let max_scroll = tab.view.total_lines.saturating_sub(view_height / 2);
            tab.view.scroll_offset = tab.view.scroll_offset.min(max_scroll);
        } else {
            // Populate cache for any tables not yet laid out (e.g. first draw).
            for doc_block in &mut tab.view.rendered {
                if let DocBlock::Table(table) = doc_block
                    && let std::collections::hash_map::Entry::Vacant(e) =
                        tab.view.table_layouts.entry(table.id)
                {
                    let (text, height, _was_truncated) = layout_table(table, effective_width, &p);
                    table.rendered_height = height;
                    e.insert(TableLayout { text });
                }
            }
            // Sync mermaid heights from cache (no-op when nothing has changed).
            update_mermaid_heights(&tab.view.rendered, &app.mermaid_cache);
            // Recompute total_lines in case any table or mermaid heights changed.
            tab.view.total_lines = tab.view.rendered.iter().map(|b| b.height()).sum();
            let max_scroll = tab.view.total_lines.saturating_sub(view_height / 2);
            tab.view.scroll_offset = tab.view.scroll_offset.min(max_scroll);
        }
    }

    let tab = app.tabs.active_tab().unwrap();
    let scroll_offset = tab.view.scroll_offset;

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
                            let visible_text = if let Some((query, current_line)) =
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
                            });
                        }
                        DocBlock::Table(table) => {
                            // Slice visible lines from the cached rendered text.
                            if let Some(cached) = tab.view.table_layouts.get(&table.id) {
                                let start = clip_start as usize;
                                let end = (clip_start + visible_lines)
                                    .min(cached.text.lines.len() as u32)
                                    as usize;
                                let visible_text =
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
            ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
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
        draw_mermaid_block(f, app, rect, md.fully_visible, md.id, &md.source, &p);
    }
}

/// Draw a mermaid block at the given rect, looking up the cache entry.
///
/// When `fully_visible` is false (the block is partially scrolled on- or
/// off-screen), skip image rendering and show a placeholder; otherwise the
/// image widget would re-fit to the shrinking rect and visibly jitter.
fn draw_mermaid_block(
    f: &mut Frame,
    app: &mut App,
    rect: Rect,
    fully_visible: bool,
    id: crate::markdown::MermaidBlockId,
    source: &str,
    p: &Palette,
) {
    use crate::mermaid::MermaidEntry;

    let entry = app.mermaid_cache.get_mut(&id);

    match entry {
        None => {
            render_mermaid_placeholder(f, rect, "mermaid diagram", p);
        }
        Some(MermaidEntry::Pending) => {
            render_mermaid_placeholder(f, rect, "rendering\u{2026}", p);
        }
        Some(MermaidEntry::Ready { protocol, .. }) => {
            if fully_visible {
                use ratatui_image::{Resize, StatefulImage};
                f.render_widget(
                    Block::default().style(Style::default().bg(p.background)),
                    rect,
                );
                let padded = padded_rect(rect, 4, 1);
                let image = StatefulImage::new().resize(Resize::Fit(None));
                f.render_stateful_widget(image, padded, protocol.as_mut());
            } else {
                render_mermaid_placeholder(f, rect, "scroll to view diagram", p);
            }
        }
        Some(MermaidEntry::Failed(msg)) => {
            let footer = format!("[mermaid \u{2014} {}]", truncate(msg, 60));
            render_mermaid_source(f, rect, source, &footer, p);
        }
        Some(MermaidEntry::SourceOnly(reason)) => {
            let footer = format!("[mermaid \u{2014} {}]", reason.clone());
            render_mermaid_source(f, rect, source, &footer, p);
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

fn render_mermaid_source(f: &mut Frame, rect: Rect, source: &str, footer: &str, p: &Palette) {
    let code_style = Style::default().fg(p.code_fg).bg(p.code_bg);
    let dim_style = p.dim_style();

    let mut lines: Vec<Line<'static>> = source
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), code_style)))
        .collect();
    lines.push(Line::from(Span::styled(footer.to_string(), dim_style)));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(p.border_style())
        .style(Style::default().bg(p.background));
    let para = Paragraph::new(Text::from(lines))
        .block(block)
        .wrap(Wrap { trim: false });
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
    let slice_len = text.lines.len() as u32;
    let num_digits = if total_doc_lines == 0 {
        4
    } else {
        (total_doc_lines.ilog10() + 1).max(4)
    };
    let gutter_width = num_digits + 3;

    let chunks = Layout::horizontal([Constraint::Length(gutter_width as u16), Constraint::Min(0)])
        .split(rect);

    let gutter_style = Style::new().fg(p.gutter);
    let gutter_lines: Vec<Line<'static>> = (first_line_number..first_line_number + slice_len)
        .map(|n| {
            Line::from(Span::styled(
                format!("{:>width$} | ", n, width = num_digits as usize),
                gutter_style,
            ))
        })
        .collect();

    f.render_widget(Paragraph::new(Text::from(gutter_lines)), chunks[0]);
    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), chunks[1]);
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
