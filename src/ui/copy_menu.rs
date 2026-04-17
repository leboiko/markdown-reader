use crate::app::CopyMenuState;
use crate::theme::Palette;
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

const ACTIVE_BULLET: &str = "●";
const INACTIVE_BULLET: &str = "○";

/// Render the copy-path popup centered over the full terminal area.
///
/// # Arguments
///
/// * `f`       - Ratatui frame.
/// * `state`   - Current cursor position and the path/name to copy.
/// * `palette` - Active palette for border and accent colors.
pub fn draw(f: &mut Frame, state: &CopyMenuState, palette: &Palette) {
    let area = centered_rect(26, 6, f.area());
    f.render_widget(Clear, area);

    let cursor_style = Style::new().fg(palette.accent).add_modifier(Modifier::BOLD);
    let active_style = Style::new()
        .fg(palette.accent_alt)
        .add_modifier(Modifier::BOLD);
    let text_style = Style::new().fg(palette.foreground);
    let dim_style = palette.dim_style();

    let lines = vec![
        Line::from(""),
        option_line(
            state.cursor == 0,
            state.cursor == 0,
            "Full path",
            cursor_style,
            active_style,
            text_style,
            dim_style,
        ),
        option_line(
            state.cursor == 1,
            state.cursor == 1,
            "Filename",
            cursor_style,
            active_style,
            text_style,
            dim_style,
        ),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ", text_style),
            Span::styled("Enter", cursor_style),
            Span::styled(": copy  ", dim_style),
            Span::styled("Esc", cursor_style),
            Span::styled(": cancel", dim_style),
        ]),
    ];

    let block = Block::default()
        .title(" Copy ")
        .title_style(palette.title_style())
        .borders(Borders::ALL)
        .border_style(Style::new().fg(palette.border_focused))
        .style(Style::default().bg(palette.help_bg));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn option_line(
    is_cursor: bool,
    is_active: bool,
    label: &str,
    cursor_style: Style,
    active_style: Style,
    text_style: Style,
    dim_style: Style,
) -> Line<'_> {
    let arrow = if is_cursor { "> " } else { "  " };
    let bullet = if is_active {
        ACTIVE_BULLET
    } else {
        INACTIVE_BULLET
    };
    let bullet_style = if is_active { active_style } else { dim_style };
    let label_style = if is_cursor { cursor_style } else { text_style };

    Line::from(vec![
        Span::styled("  ", text_style),
        Span::styled(arrow, cursor_style),
        Span::styled(bullet, bullet_style),
        Span::styled(" ", text_style),
        Span::styled(label, label_style),
    ])
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .split(area);
    Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .split(vertical[0])[0]
}
