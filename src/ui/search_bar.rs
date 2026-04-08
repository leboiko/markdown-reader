use crate::app::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
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
    /// Whether the search bar is currently visible.
    pub active: bool,
    /// The current search query string.
    pub query: String,
    /// Whether to search file names or contents.
    pub mode: SearchMode,
    /// All matches found for the current query.
    pub results: Vec<SearchResult>,
    /// The index of the highlighted result.
    pub selected_index: usize,
}

/// A single search match returned by a query.
///
/// `line_number` and `snippet` are populated for content searches and are
/// reserved for future display in the results list.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SearchResult {
    /// Absolute path to the matching file.
    pub path: std::path::PathBuf,
    /// Display name (file name component).
    pub name: String,
    /// Line number of the match when searching by content.
    pub line_number: Option<usize>,
    /// Trimmed snippet of the matching line.
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
    /// Activate the search bar, clearing any previous query and results.
    pub fn activate(&mut self) {
        self.active = true;
        self.query.clear();
        self.results.clear();
        self.selected_index = 0;
    }

    /// Toggle between [`SearchMode::FileName`] and [`SearchMode::Content`].
    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            SearchMode::FileName => SearchMode::Content,
            SearchMode::Content => SearchMode::FileName,
        };
    }

    /// Advance to the next result, wrapping around.
    pub fn next_result(&mut self) {
        if !self.results.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.results.len();
        }
    }

    /// Move to the previous result, wrapping around.
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
        Span::styled(" / ", Style::default().fg(Color::Yellow)),
        Span::raw(&app.search.query),
        Span::styled(
            "█",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
        Span::styled(result_info, Style::default().fg(Color::DarkGray)),
    ]);

    let block = Block::default()
        .title(format!(" Search [{mode_label}] (Tab to toggle) "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let paragraph = Paragraph::new(line).block(block);
    f.render_widget(paragraph, area);
}
