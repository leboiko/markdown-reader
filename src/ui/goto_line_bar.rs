use crate::app::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

/// Render the go-to-line prompt at the bottom of the viewer area.
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

    let total_info = if app.viewer.total_lines == 0 {
        String::new()
    } else {
        format!(" / {}", app.viewer.total_lines)
    };

    let line = Line::from(vec![
        Span::styled(" Go to line: ", Style::default().fg(p.accent_alt)),
        Span::raw(&app.goto_line.input),
        Span::styled(
            "█",
            Style::default()
                .fg(p.foreground)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
        Span::styled(total_info, p.dim_style()),
    ]);

    let block = Block::default()
        .title(" Go to Line (Enter to jump, Esc to cancel) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.accent_alt))
        .style(Style::default().bg(p.help_bg));

    let paragraph = Paragraph::new(line).block(block);

    f.render_widget(Clear, bar_area);
    f.render_widget(paragraph, bar_area);
}
