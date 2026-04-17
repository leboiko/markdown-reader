use crate::app::App;
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

/// Render a centered help overlay listing all keyboard shortcuts.
#[allow(clippy::too_many_lines)]
pub fn draw(f: &mut Frame, app: &App) {
    let p = &app.palette;

    let header_style = Style::default().fg(p.accent).add_modifier(Modifier::BOLD);
    let key_style = Style::default()
        .fg(p.accent_alt)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(p.foreground);
    let dim_style = p.dim_style();

    let lines = vec![
        Line::from(Span::styled("Keyboard Shortcuts", header_style)),
        Line::from(""),
        Line::from(Span::styled("── Navigation ──", dim_style)),
        shortcut_line("j / Down", "Move down", key_style, desc_style),
        shortcut_line("k / Up", "Move up", key_style, desc_style),
        shortcut_line("Enter / l", "Open file / expand dir", key_style, desc_style),
        shortcut_line("h / Left", "Collapse directory", key_style, desc_style),
        shortcut_line("gg", "Jump to first item", key_style, desc_style),
        shortcut_line("G", "Jump to last item", key_style, desc_style),
        shortcut_line("Tab", "Switch panel", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled("── Viewer ──", dim_style)),
        shortcut_line("j / k", "Scroll line by line", key_style, desc_style),
        shortcut_line("d / u", "Half-page scroll", key_style, desc_style),
        shortcut_line("PageDn / PageUp", "Full-page scroll", key_style, desc_style),
        shortcut_line("gg / G", "Top / bottom", key_style, desc_style),
        shortcut_line("Enter", "Expand table under cursor", key_style, desc_style),
        shortcut_line("Ctrl+f", "Find in document", key_style, desc_style),
        shortcut_line("n / N", "Next / prev match", key_style, desc_style),
        shortcut_line("f", "Open anchor link picker", key_style, desc_style),
        shortcut_line(":", "Go to line", key_style, desc_style),
        shortcut_line(
            "yy",
            "Copy current line to clipboard",
            key_style,
            desc_style,
        ),
        shortcut_line(
            "v",
            "Enter visual (character) selection",
            key_style,
            desc_style,
        ),
        shortcut_line("V", "Enter visual-line selection", key_style, desc_style),
        shortcut_line("h / l", "Move cursor left / right", key_style, desc_style),
        shortcut_line(
            "y (visual)",
            "Copy selection (char or line mode)",
            key_style,
            desc_style,
        ),
        shortcut_line(
            "Esc / v / V",
            "Cancel visual selection",
            key_style,
            desc_style,
        ),
        Line::from(""),
        Line::from(Span::styled("── Table modal ──", dim_style)),
        shortcut_line("h / l", "Pan left / right", key_style, desc_style),
        shortcut_line("H / L", "Pan left / right 10 cols", key_style, desc_style),
        shortcut_line("j / k", "Scroll down / up", key_style, desc_style),
        shortcut_line("0 / $", "Pan to start / end", key_style, desc_style),
        shortcut_line("gg / G", "Top / bottom of table", key_style, desc_style),
        shortcut_line("q / Esc / Enter", "Close table", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled("── Tabs ──", dim_style)),
        shortcut_line("t (tree)", "Open file in new tab", key_style, desc_style),
        shortcut_line("gt / gT", "Next / previous tab", key_style, desc_style),
        shortcut_line("1-9 / 0", "Jump to tab N / last tab", key_style, desc_style),
        shortcut_line("`", "Jump to previous tab", key_style, desc_style),
        shortcut_line("x", "Close current tab", key_style, desc_style),
        shortcut_line("T", "Open tab picker", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled("── Panels ──", dim_style)),
        shortcut_line("[", "Shrink file tree", key_style, desc_style),
        shortcut_line("]", "Grow file tree", key_style, desc_style),
        shortcut_line("H", "Toggle file tree", key_style, desc_style),
        shortcut_line("y", "Copy path/filename", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled("── Search ──", dim_style)),
        shortcut_line("/", "Open search", key_style, desc_style),
        shortcut_line("Tab", "Toggle file / content mode", key_style, desc_style),
        shortcut_line(
            "Aa / aA",
            "Smartcase: uppercase = case-sensitive",
            key_style,
            desc_style,
        ),
        shortcut_line("Enter", "Open selected result", key_style, desc_style),
        shortcut_line("Esc", "Close search", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled("── Settings ──", dim_style)),
        shortcut_line(
            "c",
            "Open settings (theme, line numbers, panels)",
            key_style,
            desc_style,
        ),
        shortcut_line("Esc / c", "Close settings", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled("── General ──", dim_style)),
        shortcut_line("?", "Toggle this help", key_style, desc_style),
        shortcut_line("q", "Quit", key_style, desc_style),
        Line::from(""),
        Line::from(Span::styled("Press any key to close", dim_style)),
    ];

    let height = crate::cast::u16_sat(lines.len()) + 2;
    let width = 54;

    let area = centered_rect(width, height, f.area());

    let block = Block::default()
        .title(" Help ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.border_focused))
        .style(Style::default().bg(p.help_bg));

    let paragraph = Paragraph::new(lines).block(block);

    f.render_widget(Clear, area);
    f.render_widget(paragraph, area);
}

fn shortcut_line<'a>(key: &'a str, desc: &'a str, key_style: Style, desc_style: Style) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("  {key:<20}"), key_style),
        Span::styled(desc, desc_style),
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
