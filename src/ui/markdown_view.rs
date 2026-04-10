use crate::app::App;
use crate::theme::Palette;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::borrow::Cow;
use std::path::PathBuf;

/// Runtime state for the markdown preview panel.
#[derive(Debug, Default)]
pub struct MarkdownViewState {
    /// Raw markdown source of the currently displayed file.
    pub content: String,
    /// Pre-rendered ratatui `Text` produced by the markdown renderer.
    pub rendered: Text<'static>,
    /// Current scroll offset in rendered lines.
    pub scroll_offset: u32,
    /// Display name shown in the panel title.
    pub file_name: String,
    /// Absolute path of the loaded file, used for accurate hot-reload matching.
    pub current_path: Option<PathBuf>,
    /// Total number of rendered lines.
    pub total_lines: u32,
    /// Inner height of the panel (rows minus borders), updated each draw call.
    pub view_height: u32,
}

impl MarkdownViewState {
    /// Load a file into the viewer, resetting the scroll position.
    ///
    /// # Arguments
    ///
    /// * `path`      - Absolute path to the file (used for hot-reload matching).
    /// * `file_name` - Display name shown in the panel title.
    /// * `content`   - Raw markdown text to render.
    /// * `palette`   - Active palette used to color the rendered output.
    pub fn load(&mut self, path: PathBuf, file_name: String, content: String, palette: &Palette) {
        self.rendered = crate::markdown::renderer::render_markdown(&content, palette);
        self.total_lines = self.rendered.lines.len() as u32;
        self.content = content;
        self.file_name = file_name;
        self.current_path = Some(path);
        self.scroll_offset = 0;
    }

    pub fn scroll_up(&mut self, n: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n as u32);
    }

    pub fn scroll_down(&mut self, n: u16) {
        let max = self.total_lines.saturating_sub(self.view_height / 2);
        self.scroll_offset = (self.scroll_offset + n as u32).min(max);
    }

    pub fn scroll_half_page_up(&mut self) {
        self.scroll_up((self.view_height / 2) as u16);
    }

    pub fn scroll_half_page_down(&mut self) {
        self.scroll_down((self.view_height / 2) as u16);
    }

    pub fn scroll_page_up(&mut self) {
        self.scroll_up(self.view_height as u16);
    }

    pub fn scroll_page_down(&mut self) {
        self.scroll_down(self.view_height as u16);
    }

    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.total_lines.saturating_sub(self.view_height / 2);
    }
}

/// Render the markdown preview panel into `area`.
pub fn draw(f: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let p = &app.palette;

    let border_style = if focused {
        p.border_focused_style()
    } else {
        p.border_style()
    };

    let title: Cow<str> = if app.viewer.file_name.is_empty() {
        Cow::Borrowed(" Preview ")
    } else {
        Cow::Owned(format!(" {} ", app.viewer.file_name))
    };

    let block = Block::default()
        .title(title.as_ref())
        .title_style(p.title_style())
        .borders(Borders::ALL)
        .border_style(border_style);

    // Update view height for scroll calculations (subtract two border rows).
    app.viewer.view_height = area.height.saturating_sub(2) as u32;

    if app.viewer.content.is_empty() {
        let empty = Paragraph::new("No file selected. Select a markdown file from the tree.")
            .style(p.dim_style().bg(p.background))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    let scroll_row = app.viewer.scroll_offset.min(u16::MAX as u32) as u16;

    let text = if !app.doc_search.query.is_empty() && !app.doc_search.match_lines.is_empty() {
        let current_line = app
            .doc_search
            .match_lines
            .get(app.doc_search.current_match)
            .copied();
        highlight_matches(&app.viewer.rendered, &app.doc_search.query, current_line, p)
    } else {
        app.viewer.rendered.clone()
    };

    if app.show_line_numbers {
        render_with_gutter(f, area, block, text, scroll_row, p);
    } else {
        let paragraph = Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll((scroll_row, 0));
        f.render_widget(paragraph, area);
    }
}

/// Render the document with a line-number gutter on the left.
///
/// The gutter and content paragraphs share the same `scroll` value so they
/// scroll in lockstep without any manual offset tracking.
fn render_with_gutter(
    f: &mut Frame,
    area: Rect,
    block: ratatui::widgets::Block,
    text: Text<'static>,
    scroll_row: u16,
    p: &Palette,
) {
    let total = text.lines.len() as u32;
    // Width needed to display the largest line number, minimum 4 digits.
    let num_digits = if total == 0 {
        4
    } else {
        (total.ilog10() + 1).max(4)
    };
    // Gutter: digits + " │ " separator (3 chars).
    let gutter_width = num_digits + 3;

    // Split the inner area (after the block border) into gutter | content.
    // We render the block first to claim its border, then work inside it.
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::horizontal([Constraint::Length(gutter_width as u16), Constraint::Min(0)])
        .split(inner);

    // Build gutter lines: right-aligned 1-indexed numbers.
    let gutter_style = Style::new().fg(p.gutter);
    let gutter_lines: Vec<Line<'static>> = (1..=total)
        .map(|n| {
            Line::from(Span::styled(
                format!("{:>width$} │ ", n, width = num_digits as usize),
                gutter_style,
            ))
        })
        .collect();

    let gutter_para = Paragraph::new(Text::from(gutter_lines)).scroll((scroll_row, 0));
    f.render_widget(gutter_para, chunks[0]);

    let content_para = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((scroll_row, 0));
    f.render_widget(content_para, chunks[1]);
}

/// Produce a new `Text` with search matches highlighted.
fn highlight_matches(
    text: &Text<'static>,
    query: &str,
    current_line: Option<u32>,
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

            let is_current = current_line == Some(line_idx as u32);
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
