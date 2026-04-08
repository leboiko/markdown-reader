use crate::app::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

/// Render the in-document find bar at the bottom of the viewer area.
pub fn draw(f: &mut Frame, app: &App, area: Rect) {
    // Place a 3-row bar at the bottom of the given area.
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

    let match_info = if app.doc_search.match_lines.is_empty() {
        if app.doc_search.query.is_empty() {
            String::new()
        } else {
            " No matches".to_string()
        }
    } else {
        format!(
            " [{}/{}]",
            app.doc_search.current_match + 1,
            app.doc_search.match_lines.len()
        )
    };

    let line = Line::from(vec![
        Span::styled(" Find: ", Style::default().fg(Color::Yellow)),
        Span::raw(&app.doc_search.query),
        Span::styled(
            "█",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
        Span::styled(match_info, Style::default().fg(Color::DarkGray)),
    ]);

    let block = Block::default()
        .title(" Document Search (Enter to confirm, Esc to cancel) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::Rgb(20, 20, 30)));

    let paragraph = Paragraph::new(line).block(block);

    f.render_widget(Clear, bar_area);
    f.render_widget(paragraph, bar_area);
}
