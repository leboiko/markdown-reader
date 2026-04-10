use crate::app::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

/// Whether the search matches file names or file contents.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchMode {
    /// Match against the file name (fast, no I/O).
    FileName,
    /// Match against the contents of every markdown file (slower).
    Content,
}

/// Transient state for the interactive search overlay.
#[derive(Debug)]
pub struct SearchState {
    pub active: bool,
    pub query: String,
    pub mode: SearchMode,
    pub results: Vec<SearchResult>,
    pub selected_index: usize,
}

/// A single search match returned by a query.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SearchResult {
    pub path: std::path::PathBuf,
    pub name: String,
    pub line_number: Option<usize>,
    pub snippet: Option<String>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            active: false,
            query: String::new(),
            mode: SearchMode::FileName,
            results: Vec::new(),
            selected_index: 0,
        }
    }
}

impl SearchState {
    pub fn activate(&mut self) {
        self.active = true;
        self.query.clear();
        self.results.clear();
        self.selected_index = 0;
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            SearchMode::FileName => SearchMode::Content,
            SearchMode::Content => SearchMode::FileName,
        };
    }

    pub fn next_result(&mut self) {
        if !self.results.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.results.len();
        }
    }

    pub fn prev_result(&mut self) {
        if !self.results.is_empty() {
            self.selected_index = if self.selected_index == 0 {
                self.results.len() - 1
            } else {
                self.selected_index - 1
            };
        }
    }
}

/// Render the search bar overlay into `area`.
pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let p = &app.palette;

    let mode_label = match app.search.mode {
        SearchMode::FileName => "Files",
        SearchMode::Content => "Content",
    };

    let result_info = if app.search.results.is_empty() {
        if app.search.query.is_empty() {
            String::new()
        } else {
            " No matches".to_string()
        }
    } else {
        format!(
            " [{}/{}]",
            app.search.selected_index + 1,
            app.search.results.len()
        )
    };

    let line = Line::from(vec![
        Span::styled(" / ", Style::default().fg(p.accent_alt)),
        Span::raw(&app.search.query),
        Span::styled(
            "█",
            Style::default()
                .fg(p.foreground)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
        Span::styled(result_info, p.dim_style()),
    ]);

    let block = Block::default()
        .title(format!(" Search [{mode_label}] (Tab to toggle) "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.accent_alt));

    let paragraph = Paragraph::new(line).block(block);
    f.render_widget(paragraph, area);
}
