use crate::app::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

/// Render the in-document find bar at the bottom of the viewer area.
pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    let p = &app.palette;

    let bar_height = 3u16;
    if area.height < bar_height + 2 {
        return;
    }
    let bar_area = Rect {
        x: area.x,
        y: area.y + area.height - bar_height,
        width: area.width,
        height: bar_height,
    };

    let Some(ds) = app.doc_search() else {
        return;
    };

    let match_info = if ds.match_lines.is_empty() {
        if ds.query.is_empty() {
            String::new()
        } else {
            " No matches".to_string()
        }
    } else {
        format!(" [{}/{}]", ds.current_match + 1, ds.match_lines.len())
    };

    let line = Line::from(vec![
        Span::styled(" Find: ", Style::default().fg(p.accent_alt)),
        Span::raw(ds.query.clone()),
        Span::styled(
            "█",
            Style::default()
                .fg(p.foreground)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
        Span::styled(match_info, p.dim_style()),
    ]);

    let block = Block::default()
        .title(" Document Search (Enter to confirm, Esc to cancel) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.accent_alt))
        .style(Style::default().bg(p.help_bg));

    let paragraph = Paragraph::new(line).block(block);

    f.render_widget(Clear, bar_area);
    f.render_widget(paragraph, bar_area);
}
