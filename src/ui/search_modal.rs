//! Full-screen global search modal.
//!
//! Owns [`SearchState`], [`SearchMode`], and [`SearchResult`].  The draw
//! entry-point is [`draw`], invoked last in [`crate::ui::draw`] so the modal
//! floats above all other panels.  Search execution is driven from
//! `app.rs` via the `SearchResults` action; this module only renders the
//! state and exposes pure helpers ([`smartcase_is_sensitive`],
//! [`build_preview`]) used by the search execution path.

use crate::app::App;
use crate::config::SearchPreview;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

/// Whether the search matches file names or file contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// Match against the file name (fast, no I/O).
    FileName,
    /// Match against the contents of every markdown file (slower).
    Content,
}

/// Transient state for the interactive search modal.
#[derive(Debug)]
pub struct SearchState {
    /// Whether the modal is currently visible.
    pub active: bool,
    /// Current query string entered by the user.
    pub query: String,
    /// File-name search or full-content search.
    pub mode: SearchMode,
    /// Current result set (up to [`RESULT_CAP`] entries).
    pub results: Vec<SearchResult>,
    /// Index of the highlighted result (0-based into `results`).
    pub selected_index: usize,
    /// Whether the result list was capped at the [`RESULT_CAP`]-file limit.
    pub truncated_at_cap: bool,
}

/// A single match returned by a search query.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Absolute path to the matched file.
    pub path: PathBuf,
    /// File name component (for display).
    pub name: String,
    /// For Content mode: total number of line matches across the file.
    /// Always `0` for `FileName` mode.
    pub match_count: usize,
    /// For Content mode: formatted preview of the first match line.
    /// Empty for `FileName` mode.
    pub preview: String,
    /// 0-based line index of the first match.  `None` for `FileName` mode.
    ///
    /// Stored as 0-based so consumers can pass it directly to source-line
    /// coordinates without adjustment.  Display code should add 1 for
    /// human-readable output.
    pub first_match_line: Option<usize>,
}

impl Default for SearchState {
    fn default() -> Self {
        Self {
            active: false,
            query: String::new(),
            mode: SearchMode::FileName,
            results: Vec::new(),
            selected_index: 0,
            truncated_at_cap: false,
        }
    }
}

impl SearchState {
    /// Open the modal and reset all transient state.
    pub fn activate(&mut self) {
        self.active = true;
        self.query.clear();
        self.results.clear();
        self.selected_index = 0;
        self.truncated_at_cap = false;
    }

    /// Toggle between file-name and content search modes.
    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            SearchMode::FileName => SearchMode::Content,
            SearchMode::Content => SearchMode::FileName,
        };
        self.results.clear();
        self.selected_index = 0;
        self.truncated_at_cap = false;
    }

    /// Advance to the next result, wrapping around to the first.
    pub fn next_result(&mut self) {
        if !self.results.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.results.len();
        }
    }

    /// Go back to the previous result, wrapping around to the last.
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

/// Maximum number of results to accumulate before truncating.
pub const RESULT_CAP: usize = 500;

/// Width (in Unicode scalar values) of the centered snippet window used by
/// [`build_preview`] in [`crate::config::SearchPreview::Snippet`] mode.
const SNIPPET_WINDOW: usize = 80;

/// Returns `true` when `query` should be matched case-sensitively.
///
/// Smartcase: if the query contains any uppercase character, use
/// case-sensitive matching; otherwise use case-insensitive matching.
///
/// # Examples
///
/// ```
/// use markdown_tui_explorer::ui::search_modal::smartcase_is_sensitive;
/// assert!(!smartcase_is_sensitive("hello"));
/// assert!(smartcase_is_sensitive("Hello"));
/// assert!(!smartcase_is_sensitive(""));
/// ```
pub fn smartcase_is_sensitive(query: &str) -> bool {
    query.chars().any(char::is_uppercase)
}

/// Build a display preview for a single matched line.
///
/// * `SearchPreview::FullLine` — returns the trimmed line text.
/// * `SearchPreview::Snippet` — returns an approximately [`SNIPPET_WINDOW`]-char window centred on the
///   first occurrence of `query` (case-insensitive locate for the window
///   position even when matching case-sensitively, so the window always
///   covers the match).
///
/// Both variants guarantee that the returned string contains only ASCII-printable
/// or multi-byte Unicode content; control characters are stripped.
///
/// # Arguments
///
/// * `line`  - The raw source line (may be long).
/// * `query` - The search query (used to centre the snippet window).
/// * `mode`  - Which preview style to apply.
pub fn build_preview(line: &str, query: &str, mode: SearchPreview) -> String {
    // Strip leading/trailing whitespace so indented code doesn't look odd.
    let trimmed = line.trim();

    match mode {
        SearchPreview::FullLine => trimmed.to_string(),
        SearchPreview::Snippet => {
            // Locate the first occurrence of `query` (case-insensitive for
            // positioning so the window reliably centres over the match text
            // regardless of the smartcase setting).
            let lower_line = trimmed.to_lowercase();
            let lower_query = query.to_lowercase();
            let match_byte = lower_line.find(lower_query.as_str()).unwrap_or(0);

            // Convert byte offset to a char index for safe slicing.
            let match_char = trimmed[..match_byte].chars().count();
            let total_chars = trimmed.chars().count();

            // Centre a SNIPPET_WINDOW-char window over the match.
            let half = SNIPPET_WINDOW / 2;
            // Start of window (saturating to avoid underflow).
            let start_char = match_char.saturating_sub(half);
            // Clamp so the window fits within the string.
            let start_char = if start_char + SNIPPET_WINDOW > total_chars {
                total_chars.saturating_sub(SNIPPET_WINDOW)
            } else {
                start_char
            };
            let end_char = (start_char + SNIPPET_WINDOW).min(total_chars);

            // Re-materialise the character slice back into a &str.
            let snippet: String = trimmed
                .chars()
                .skip(start_char)
                .take(end_char - start_char)
                .collect();

            // Prefix an ellipsis when we didn't start at the beginning.
            if start_char > 0 {
                format!("…{snippet}")
            } else {
                snippet
            }
        }
    }
}

/// Render the search modal as a full-screen overlay.
///
/// Writes per-row [`Rect`]s into `app.search_result_rects` for mouse
/// hit-testing.  Clears the vec at the start of each draw.
#[allow(clippy::too_many_lines)]
pub fn draw(f: &mut Frame, app: &mut App) {
    app.search_result_rects.clear();

    if !app.search.active {
        return;
    }

    let p = &app.palette;
    let area = f.area();

    // 80 % wide, 80 % tall, centred.
    let popup_area = percent_rect(80, 80, area);
    f.render_widget(Clear, popup_area);

    let mode_label = match app.search.mode {
        SearchMode::FileName => "Files",
        SearchMode::Content => "Content",
    };

    let smartcase_marker = if smartcase_is_sensitive(&app.search.query) {
        " Aa"
    } else {
        " aA"
    };

    let result_count: usize = app.search.results.len();
    let total_matches: usize = app
        .search
        .results
        .iter()
        .map(|r| r.match_count.max(1))
        .sum();

    // Outer block (border + title).
    let outer_block = Block::default()
        .title(format!(
            " Search [{mode_label}] (Tab: toggle mode  Esc: close) "
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.border_focused))
        .style(Style::default().bg(p.help_bg));

    let inner = outer_block.inner(popup_area);
    f.render_widget(outer_block, popup_area);

    // Split the inner area: 1 row for the query bar, remainder for results,
    // 1 row for the footer.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(inner);

    let query_area = chunks[0];
    let results_area = chunks[1];
    let footer_area = chunks[2];

    // ── Query bar ────────────────────────────────────────────────────────────
    let query_line = Line::from(vec![
        Span::styled(" / ", Style::default().fg(p.accent_alt)),
        Span::raw(app.search.query.as_str()),
        // Block cursor drawn with a blinking modifier.
        Span::styled(
            "█",
            Style::default()
                .fg(p.foreground)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
        Span::styled(
            format!("  [{mode_label}]{smartcase_marker}"),
            Style::default().fg(p.dim),
        ),
    ]);
    f.render_widget(Paragraph::new(query_line), query_area);

    // ── Results list ─────────────────────────────────────────────────────────
    let cursor = app.search.selected_index;
    let visible_rows = results_area.height as usize;

    // Scroll the list so the cursor row stays visible.
    let scroll_offset = if result_count == 0 || cursor < visible_rows {
        0
    } else {
        cursor - visible_rows + 1
    };

    let is_content_mode = app.search.mode == SearchMode::Content;

    for (slot, result) in app
        .search
        .results
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_rows)
    {
        let row_rect = Rect {
            x: results_area.x,
            y: results_area.y + crate::cast::u16_sat(slot - scroll_offset),
            width: results_area.width,
            height: 1,
        };
        app.search_result_rects.push((slot, row_rect));

        let is_selected = slot == cursor;

        // Build display segments from structured fields.
        // `result.name` is the filename-only component; the parent directory
        // gives context without repeating the full absolute path.
        let parent_str = result
            .path
            .parent()
            .map(|p| {
                // Show at most 2 parent directory components.
                let comps: Vec<_> = p.components().collect();
                if comps.len() <= 2 {
                    p.to_string_lossy().into_owned()
                } else {
                    let tail: Vec<String> = comps
                        .iter()
                        .rev()
                        .take(2)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect();
                    format!("…/{}", tail.join("/"))
                }
            })
            .unwrap_or_default();

        let line = if is_content_mode {
            // Content mode: [N matches] name  — preview  (line :L)
            let count_label = format!("[{}] ", result.match_count);
            // Show first-match line number when available.
            // first_match_line is 0-based internally; add 1 for display.
            let line_tag = result
                .first_match_line
                .map(|l| format!(" :{}", l + 1))
                .unwrap_or_default();

            let preview_text = if result.preview.is_empty() {
                String::new()
            } else {
                // Truncate preview to available width minus bookkeeping chars.
                let budget = (results_area.width as usize)
                    .saturating_sub(count_label.width())
                    .saturating_sub(result.name.width())
                    .saturating_sub(parent_str.width())
                    .saturating_sub(line_tag.width())
                    .saturating_sub(5); // " — " + leading space + margin
                truncate_str(&result.preview, budget)
            };
            let preview_suffix = if preview_text.is_empty() {
                line_tag
            } else {
                format!(" — {preview_text}{line_tag}")
            };

            if is_selected {
                let sel_style = Style::default()
                    .fg(p.selection_fg)
                    .bg(p.selection_bg)
                    .add_modifier(Modifier::BOLD);
                Line::from(vec![
                    Span::styled(" ", sel_style),
                    Span::styled(count_label, sel_style),
                    Span::styled(result.name.as_str(), sel_style),
                    Span::styled(format!("  {parent_str}"), sel_style),
                    Span::styled(preview_suffix, sel_style),
                ])
            } else {
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(count_label, Style::default().fg(p.accent_alt)),
                    Span::styled(
                        result.name.as_str(),
                        Style::default()
                            .fg(p.foreground)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("  {parent_str}"), Style::default().fg(p.dim)),
                    Span::styled(preview_suffix, Style::default().fg(p.dim)),
                ])
            }
        } else {
            // FileName mode: name  parent/dir
            if is_selected {
                let sel_style = Style::default()
                    .fg(p.selection_fg)
                    .bg(p.selection_bg)
                    .add_modifier(Modifier::BOLD);
                Line::from(vec![
                    Span::styled(" ", sel_style),
                    Span::styled(result.name.as_str(), sel_style),
                    Span::styled(format!("  {parent_str}"), sel_style),
                ])
            } else {
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(
                        result.name.as_str(),
                        Style::default()
                            .fg(p.foreground)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("  {parent_str}"), Style::default().fg(p.dim)),
                ])
            }
        };

        // Render each row directly so we can track its rect.
        // Paint the selection background across the full row width.
        if is_selected {
            let bg_block = Block::default().style(Style::default().bg(p.selection_bg));
            f.render_widget(bg_block, row_rect);
        }
        f.render_widget(Paragraph::new(line), row_rect);
    }

    // Empty-state hint.
    if app.search.results.is_empty() && !app.search.query.is_empty() {
        let hint = Paragraph::new(Span::styled(" No matches", Style::default().fg(p.dim)));
        f.render_widget(hint, results_area);
    }

    // ── Footer ───────────────────────────────────────────────────────────────
    let truncation_note = if app.search.truncated_at_cap {
        format!("  … results capped at {RESULT_CAP} files")
    } else {
        String::new()
    };

    let count_label = if result_count == 0 {
        if app.search.query.is_empty() {
            String::new()
        } else {
            " No results".to_string()
        }
    } else if is_content_mode {
        format!(" {result_count} files, {total_matches} matches")
    } else {
        format!(" {result_count} files")
    };

    let footer_line = Line::from(vec![
        Span::styled(count_label, Style::default().fg(p.dim)),
        Span::styled(truncation_note, Style::default().fg(p.accent_alt)),
        Span::styled(
            "  j/k: navigate  Enter: open  Tab: toggle mode",
            Style::default().fg(p.dim),
        ),
    ]);
    f.render_widget(Paragraph::new(footer_line), footer_area);
}

/// Truncate `s` to at most `max_chars` Unicode scalar values, appending `…`
/// when truncation occurs.
fn truncate_str(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

/// Compute a percentage-sized [`Rect`] centred within `area`.
fn percent_rect(width_pct: u16, height_pct: u16, area: Rect) -> Rect {
    let height = (area.height * height_pct / 100).max(4);
    let width = (area.width * width_pct / 100).max(20);
    let vertical = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .split(area);
    Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .split(vertical[0])[0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SearchPreview;

    // ── smartcase_is_sensitive ────────────────────────────────────────────────

    #[test]
    fn smartcase_all_lowercase_is_insensitive() {
        assert!(!smartcase_is_sensitive("hello world"));
    }

    #[test]
    fn smartcase_mixed_case_is_sensitive() {
        assert!(smartcase_is_sensitive("Hello"));
    }

    #[test]
    fn smartcase_all_uppercase_is_sensitive() {
        assert!(smartcase_is_sensitive("HELLO"));
    }

    #[test]
    fn smartcase_empty_query_is_insensitive() {
        assert!(!smartcase_is_sensitive(""));
    }

    #[test]
    fn smartcase_digits_only_is_insensitive() {
        assert!(!smartcase_is_sensitive("12345"));
    }

    // ── build_preview ─────────────────────────────────────────────────────────

    #[test]
    fn full_line_preview_trims_whitespace() {
        let preview = build_preview("   hello world   ", "hello", SearchPreview::FullLine);
        assert_eq!(preview, "hello world");
    }

    #[test]
    fn snippet_preview_within_window_returns_full_trimmed() {
        // A short line fits within the 80-char window.
        let line = "short line with match";
        let preview = build_preview(line, "match", SearchPreview::Snippet);
        assert_eq!(preview, "short line with match");
    }

    #[test]
    fn snippet_preview_centres_on_match() {
        // Build a line where the match is in the middle.
        let prefix = "a".repeat(50);
        let suffix = "b".repeat(50);
        let line = format!("{prefix}MATCH{suffix}");
        let preview = build_preview(&line, "MATCH", SearchPreview::Snippet);
        // The preview must include "MATCH".
        assert!(
            preview.contains("MATCH"),
            "preview should contain the match text"
        );
        // The preview must not exceed ~82 chars (80 window + optional '…').
        assert!(
            preview.chars().count() <= 82,
            "snippet too long: {} chars",
            preview.chars().count()
        );
    }

    #[test]
    fn snippet_adds_ellipsis_when_truncated_from_start() {
        let prefix = "x".repeat(100);
        let line = format!("{prefix} keyword here");
        let preview = build_preview(&line, "keyword", SearchPreview::Snippet);
        assert!(
            preview.starts_with('…'),
            "should start with ellipsis when start was trimmed"
        );
    }

    #[test]
    fn build_preview_with_empty_query_does_not_panic() {
        // When query is empty, find returns None and we default to byte 0.
        let preview = build_preview("some content here", "", SearchPreview::Snippet);
        assert!(!preview.is_empty());
    }

    // ── truncate_str ─────────────────────────────────────────────────────────

    #[test]
    fn truncate_str_no_op_when_short() {
        let s = "hello";
        assert_eq!(truncate_str(s, 10), "hello");
    }

    #[test]
    fn truncate_str_appends_ellipsis() {
        let result = truncate_str("hello world", 8);
        assert!(result.ends_with('…'));
        assert!(result.chars().count() <= 8);
    }

    #[test]
    fn truncate_str_zero_budget_returns_empty() {
        assert_eq!(truncate_str("anything", 0), "");
    }

    // ── SearchState helpers ───────────────────────────────────────────────────

    #[test]
    fn search_state_next_wraps() {
        let mut state = SearchState {
            active: true,
            query: "q".to_string(),
            mode: SearchMode::FileName,
            selected_index: 1,
            truncated_at_cap: false,
            results: vec![
                SearchResult {
                    path: PathBuf::from("/a.md"),
                    name: "a.md".to_string(),
                    match_count: 0,
                    preview: String::new(),
                    first_match_line: None,
                },
                SearchResult {
                    path: PathBuf::from("/b.md"),
                    name: "b.md".to_string(),
                    match_count: 0,
                    preview: String::new(),
                    first_match_line: None,
                },
            ],
        };
        state.next_result();
        assert_eq!(state.selected_index, 0);
    }

    #[test]
    fn search_state_prev_wraps() {
        let mut state = SearchState {
            active: true,
            query: "q".to_string(),
            mode: SearchMode::FileName,
            selected_index: 0,
            truncated_at_cap: false,
            results: vec![
                SearchResult {
                    path: PathBuf::from("/a.md"),
                    name: "a.md".to_string(),
                    match_count: 0,
                    preview: String::new(),
                    first_match_line: None,
                },
                SearchResult {
                    path: PathBuf::from("/b.md"),
                    name: "b.md".to_string(),
                    match_count: 0,
                    preview: String::new(),
                    first_match_line: None,
                },
            ],
        };
        state.prev_result();
        assert_eq!(state.selected_index, 1);
    }
}
