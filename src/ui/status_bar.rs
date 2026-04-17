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

    // Override the label when the viewer has an active visual selection.
    // Char-wise mode (`v`) shows "VISUAL"; line-wise mode (`V`) shows "VISUAL LINE".
    // This matches vim's convention so users can tell the two modes apart.
    let focus_label = if app.focus == Focus::Viewer {
        use crate::ui::markdown_view::VisualMode;
        match app
            .tabs
            .active_tab()
            .and_then(|t| t.view.visual_mode.as_ref())
            .map(|r| r.mode)
        {
            Some(VisualMode::Char) => "VISUAL",
            Some(VisualMode::Line) => "VISUAL LINE",
            None => focus_label,
        }
    } else {
        focus_label
    };

    let tab_count = app.tabs.len();
    let file_info = if let Some(tab) = app.tabs.active_tab()
        && !tab.view.file_name.is_empty()
    {
        let total = tab.view.total_lines.max(1);
        let pct = (tab.view.cursor_line * 100 / total).min(100);
        let tab_idx = app.tabs.active_index().map_or(0, |i| i + 1);
        // Show line (1-indexed), column (1-indexed), total lines, and percentage so
        // users can see j/k/h/l navigation reflected immediately in the status bar.
        format!(
            " | [{tab_idx}/{tab_count}] {} ({}/{}, col {}, {}%)",
            tab.view.file_name,
            tab.view.cursor_line + 1,
            tab.view.total_lines,
            tab.view.cursor_col + 1,
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
