use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::ConfigPopupState;
use crate::config::TreePosition;
use crate::theme::{Palette, Theme};

const ACTIVE_BULLET: &str = "●";
const INACTIVE_BULLET: &str = "○";

/// Render the settings popup centered over the full terminal area.
///
/// # Arguments
///
/// * `f`       - Ratatui frame.
/// * `state`   - Cursor position in the flat option list.
/// * `config`  - Reference to the live config for reading current values.
/// * `palette` - Active palette for border and accent colors.
pub fn render_config_popup(
    f: &mut Frame,
    state: &ConfigPopupState,
    theme: Theme,
    show_line_numbers: bool,
    tree_position: TreePosition,
    palette: &Palette,
) {
    let area = centered_rect(46, 18, f.area());
    f.render_widget(Clear, area);

    let lines = build_lines(state, theme, show_line_numbers, tree_position, palette);

    let block = Block::default()
        .title(" Settings ")
        .title_style(palette.title_style())
        .borders(Borders::ALL)
        .border_style(Style::new().fg(palette.border_focused))
        .style(Style::default().bg(palette.help_bg));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn build_lines<'a>(
    state: &ConfigPopupState,
    theme: Theme,
    show_line_numbers: bool,
    tree_position: TreePosition,
    palette: &Palette,
) -> Vec<Line<'a>> {
    let section_style = Style::new()
        .fg(palette.accent_alt)
        .add_modifier(Modifier::BOLD);
    let text_style = Style::new().fg(palette.foreground);
    let cursor_style = Style::new().fg(palette.accent).add_modifier(Modifier::BOLD);
    let dim_style = palette.dim_style();
    let active_style = Style::new()
        .fg(palette.accent_alt)
        .add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    let mut row = 0usize;

    // --- Theme section ---
    lines.push(Line::from(vec![
        Span::styled("  ", text_style),
        Span::styled(ConfigPopupState::SECTIONS[0].0, section_style),
    ]));
    for &t in Theme::ALL {
        lines.push(option_line(
            row == state.cursor,
            t == theme,
            t.label(),
            cursor_style,
            active_style,
            text_style,
            dim_style,
        ));
        row += 1;
    }

    lines.push(Line::from(""));

    // --- Markdown section ---
    lines.push(Line::from(vec![
        Span::styled("  ", text_style),
        Span::styled(ConfigPopupState::SECTIONS[1].0, section_style),
    ]));
    lines.push(option_line(
        row == state.cursor,
        show_line_numbers,
        "Show line numbers",
        cursor_style,
        active_style,
        text_style,
        dim_style,
    ));
    row += 1;

    lines.push(Line::from(""));

    // --- Panels section ---
    lines.push(Line::from(vec![
        Span::styled("  ", text_style),
        Span::styled(ConfigPopupState::SECTIONS[2].0, section_style),
    ]));
    lines.push(option_line(
        row == state.cursor,
        tree_position == TreePosition::Left,
        "Tree left",
        cursor_style,
        active_style,
        text_style,
        dim_style,
    ));
    row += 1;
    lines.push(option_line(
        row == state.cursor,
        tree_position == TreePosition::Right,
        "Tree right",
        cursor_style,
        active_style,
        text_style,
        dim_style,
    ));

    lines.push(Line::from(""));

    // --- Help footer ---
    lines.push(Line::from(vec![
        Span::styled("  ", text_style),
        Span::styled("↑↓ / j k", cursor_style),
        Span::styled(" Navigate  ", dim_style),
        Span::styled("Enter", cursor_style),
        Span::styled(" Apply  ", dim_style),
        Span::styled("Esc/c", cursor_style),
        Span::styled(" Close", dim_style),
    ]));

    lines
}

fn option_line<'a>(
    is_cursor: bool,
    is_active: bool,
    label: &'a str,
    cursor_style: Style,
    active_style: Style,
    text_style: Style,
    dim_style: Style,
) -> Line<'a> {
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
