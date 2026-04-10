use crate::app::{App, Focus};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// Render the single-line status bar into `area`.
pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let p = &app.palette;

    let focus_label = match app.focus {
        Focus::Tree => "TREE",
        Focus::Viewer => "VIEWER",
        Focus::Search => "SEARCH",
        Focus::DocSearch => "FIND",
        Focus::Config => "SETTINGS",
        Focus::GotoLine => "GOTO",
    };

    let tab_count = app.tabs.len();
    let file_info = if let Some(tab) = app.tabs.active_tab()
        && !tab.view.file_name.is_empty()
    {
        let pct = if tab.view.total_lines == 0 {
            0u32
        } else {
            (tab.view.scroll_offset * 100 / tab.view.total_lines.max(1)).min(100)
        };
        let tab_idx = app.tabs.active_index().map(|i| i + 1).unwrap_or(0);
        format!(" | [{tab_idx}/{tab_count}] {} ({}%)", tab.view.file_name, pct)
    } else {
        String::new()
    };

    let line = Line::from(vec![
        Span::styled(
            format!(" {focus_label} "),
            Style::default()
                .fg(p.selection_fg)
                .bg(p.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(file_info, Style::default().fg(p.status_bar_fg)),
        Span::raw("  "),
        Span::styled(
            " Tab:panel  /:search  c:settings  q:quit  ?:help ",
            Style::default().fg(p.dim),
        ),
    ]);

    let paragraph = Paragraph::new(line).style(Style::default().bg(p.status_bar_bg));
    f.render_widget(paragraph, area);
}
