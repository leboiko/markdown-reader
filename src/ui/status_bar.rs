use crate::app::{App, Focus};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// Render the single-line status bar into `area`.
///
/// Displays the current focus mode, the open file name, scroll percentage,
/// and a brief key-binding legend.
pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let focus_label = match app.focus {
        Focus::Tree => "TREE",
        Focus::Viewer => "VIEWER",
        Focus::Search => "SEARCH",
    };

    let file_info = if app.viewer.file_name.is_empty() {
        String::new()
    } else {
        // Both scroll_offset and total_lines are u32, so no casts needed.
        let pct = if app.viewer.total_lines == 0 {
            0u32
        } else {
            (app.viewer.scroll_offset * 100 / app.viewer.total_lines.max(1)).min(100)
        };
        format!(" │ {} ({}%)", app.viewer.file_name, pct)
    };

    let line = Line::from(vec![
        Span::styled(
            format!(" {focus_label} "),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(file_info, Style::default().fg(Color::Gray)),
        Span::raw("  "),
        Span::styled(
            " Tab:panel  /:search  q:quit  ?:help ",
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let paragraph = Paragraph::new(line).style(Style::default().bg(Color::Rgb(30, 30, 30)));
    f.render_widget(paragraph, area);
}
