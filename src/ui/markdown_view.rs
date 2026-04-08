use crate::app::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
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
    ///
    /// `u32` avoids silent truncation for large documents while remaining
    /// compatible with `u16` arithmetic used by ratatui's `Paragraph::scroll`.
    pub scroll_offset: u32,
    /// Display name shown in the panel title.
    pub file_name: String,
    /// Absolute path of the loaded file, used for accurate hot-reload matching.
    pub current_path: Option<PathBuf>,
    /// Total number of rendered lines (u32 — avoids silent u16 truncation).
    pub total_lines: u32,
    /// Inner height of the panel (rows minus borders), updated each draw call.
    pub view_height: u32,
}

impl MarkdownViewState {
    /// Load a file into the viewer, resetting the scroll position.
    ///
    /// # Arguments
    ///
    /// * `path`     - Absolute path to the file (used for hot-reload matching).
    /// * `file_name` - Display name shown in the panel title.
    /// * `content`  - Raw markdown text to render.
    pub fn load(&mut self, path: PathBuf, file_name: String, content: String) {
        self.rendered = crate::markdown::renderer::render_markdown(&content);
        self.total_lines = self.rendered.lines.len() as u32;
        self.content = content;
        self.file_name = file_name;
        self.current_path = Some(path);
        self.scroll_offset = 0;
    }

    /// Scroll up by `n` rendered lines, clamping at the top.
    pub fn scroll_up(&mut self, n: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n as u32);
    }

    /// Scroll down by `n` rendered lines, clamping so the document end stays visible.
    pub fn scroll_down(&mut self, n: u16) {
        let max = self.total_lines.saturating_sub(self.view_height / 2);
        self.scroll_offset = (self.scroll_offset + n as u32).min(max);
    }

    /// Scroll up by half the current panel height.
    pub fn scroll_half_page_up(&mut self) {
        let half = (self.view_height / 2) as u16;
        self.scroll_up(half);
    }

    /// Scroll down by half the current panel height.
    pub fn scroll_half_page_down(&mut self) {
        let half = (self.view_height / 2) as u16;
        self.scroll_down(half);
    }

    /// Scroll up by a full page.
    pub fn scroll_page_up(&mut self) {
        let page = self.view_height as u16;
        self.scroll_up(page);
    }

    /// Scroll down by a full page.
    pub fn scroll_page_down(&mut self) {
        let page = self.view_height as u16;
        self.scroll_down(page);
    }

    /// Jump to the top of the document.
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
    }

    /// Jump to the bottom of the document.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.total_lines.saturating_sub(self.view_height / 2);
    }
}

/// Render the markdown preview panel into `area`.
pub fn draw(f: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Use Cow<str> to avoid allocating when no file is loaded.
    let title: Cow<str> = if app.viewer.file_name.is_empty() {
        Cow::Borrowed(" Preview ")
    } else {
        Cow::Owned(format!(" {} ", app.viewer.file_name))
    };

    let block = Block::default()
        .title(title.as_ref())
        .borders(Borders::ALL)
        .border_style(border_style);

    // Update view height for scroll calculations (subtract two border rows).
    app.viewer.view_height = area.height.saturating_sub(2) as u32;

    if app.viewer.content.is_empty() {
        let empty = Paragraph::new("No file selected. Select a markdown file from the tree.")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    // ratatui's Paragraph::scroll takes (u16, u16); clamp to u16::MAX to be safe.
    let scroll_row = app.viewer.scroll_offset.min(u16::MAX as u32) as u16;

    // Apply search highlighting if there's an active doc search query.
    let text = if !app.doc_search.query.is_empty() && !app.doc_search.match_lines.is_empty() {
        let current_line = app
            .doc_search
            .match_lines
            .get(app.doc_search.current_match)
            .copied();
        highlight_matches(&app.viewer.rendered, &app.doc_search.query, current_line)
    } else {
        app.viewer.rendered.clone()
    };

    let paragraph = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll_row, 0));

    f.render_widget(paragraph, area);
}

/// Produce a new `Text` with search matches highlighted.
///
/// All matches get a yellow background. The match on `current_line` gets
/// an orange/red background to distinguish the "active" match.
fn highlight_matches(text: &Text<'static>, query: &str, current_line: Option<u32>) -> Text<'static> {
    let query_lower = query.to_lowercase();
    let match_style = Style::default()
        .bg(Color::Yellow)
        .fg(Color::Black)
        .add_modifier(Modifier::BOLD);
    let current_style = Style::default()
        .bg(Color::Rgb(255, 120, 0))
        .fg(Color::Black)
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
            let hl_style = if is_current { current_style } else { match_style };

            // Rebuild spans with highlighting injected.
            let mut new_spans: Vec<Span<'static>> = Vec::new();
            for span in &line.spans {
                split_and_highlight(&span.content, &query_lower, span.style, hl_style, &mut new_spans);
            }
            Line::from(new_spans)
        })
        .collect();

    Text::from(lines)
}

/// Split `text` on case-insensitive occurrences of `query` and push spans
/// with alternating normal/highlight styles.
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

        // Text before the match.
        if abs_pos > start {
            out.push(Span::styled(text[start..abs_pos].to_string(), base_style));
        }

        // The matched portion (preserve original casing).
        let match_end = abs_pos + query_lower.len();
        out.push(Span::styled(
            text[abs_pos..match_end].to_string(),
            highlight_style,
        ));

        start = match_end;
    }

    // Remaining text after the last match.
    if start < text.len() {
        out.push(Span::styled(text[start..].to_string(), base_style));
    }
}
