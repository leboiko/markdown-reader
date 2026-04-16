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
        Focus::TabPicker => "TABS",
        Focus::TableModal => "TABLE",
        Focus::CopyMenu => "COPY",
        Focus::LinkPicker => "LINKS",
        Focus::Editor => "EDIT",
    };

    // Override the label with "VISUAL" when the viewer has an active visual-line
    // selection, so the user always knows they are in selection mode.
    let focus_label = if app.focus == Focus::Viewer
        && app
            .tabs
            .active_tab()
            .and_then(|t| t.view.visual_mode.as_ref())
            .is_some()
    {
        "VISUAL"
    } else {
        focus_label
    };

    let tab_count = app.tabs.len();
    let file_info = if let Some(tab) = app.tabs.active_tab()
        && !tab.view.file_name.is_empty()
    {
        let total = tab.view.total_lines.max(1);
        let pct = (tab.view.cursor_line * 100 / total).min(100);
        let tab_idx = app.tabs.active_index().map(|i| i + 1).unwrap_or(0);
        // Show both the absolute cursor line (1-indexed for humans) and the
        // percentage through the document so users can see `j`/`k`/`d`/`u`
        // navigation reflected immediately.
        format!(
            " | [{tab_idx}/{tab_count}] {} ({}/{}, {}%)",
            tab.view.file_name,
            tab.view.cursor_line + 1,
            tab.view.total_lines,
            pct
        )
    } else {
        String::new()
    };

    let line = Line::from(vec![
        Span::styled(
            format!(" {focus_label} "),
            Style::default()
                .fg(p.on_accent_fg)
                .bg(p.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(file_info, Style::default().fg(p.status_bar_fg)),
        Span::raw("  "),
        Span::styled(
            " Tab:panel  t:new-tab  T:picker  x:close-tab  f:links  /:search  c:settings  q:quit  ?:help ",
            Style::default().fg(p.dim),
        ),
    ]);

    let paragraph = Paragraph::new(line).style(Style::default().bg(p.status_bar_bg));
    f.render_widget(paragraph, area);
}
