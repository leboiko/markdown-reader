use crate::ui::layout::centered_rect;
use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::ConfigPopupState;
use crate::config::{Config, MermaidMode, MermaidTextBackend, SearchPreview, TreePosition};
use crate::theme::{Palette, Theme};

/// Parameters for [`render_config_popup`].
///
/// Grouping these avoids the `clippy::too_many_arguments` lint.
pub struct ConfigPopupParams<'a> {
    /// Currently highlighted row in the flat option list.
    pub state: &'a ConfigPopupState,
    /// Currently active theme.
    pub theme: Theme,
    /// Whether line numbers are shown in the viewer.
    pub show_line_numbers: bool,
    /// Which side the file-tree panel is on.
    pub tree_position: TreePosition,
    /// Active search-result preview mode.
    pub search_preview: SearchPreview,
    /// Active mermaid rendering mode.
    pub mermaid_mode: MermaidMode,
    /// Active layered-layout backend for text-mode flowchart and state diagrams.
    pub mermaid_text_backend: MermaidTextBackend,
    /// Active colour palette.
    pub palette: &'a Palette,
}

const ACTIVE_BULLET: &str = "●";
const INACTIVE_BULLET: &str = "○";

/// Render the settings popup centered over the full terminal area.
///
/// # Arguments
///
/// * `f`      - Ratatui frame to render into.
/// * `params` - All display parameters (theme, flags, palette, etc.).
pub fn render_config_popup(f: &mut Frame, params: &ConfigPopupParams<'_>) {
    // content-sized: 46 cols fits the longest config label; 30 rows = original 22 + 1 blank
    //   + 1 "Mermaid" section header + 3 mode options + 3 text-backend options
    let area = centered_rect(46, 30, f.area());
    f.render_widget(Clear, area);

    let lines = build_lines(params);

    let block = Block::default()
        .title(" Settings ")
        .title_style(params.palette.title_style())
        .borders(Borders::ALL)
        .border_style(Style::new().fg(params.palette.border_focused))
        .style(Style::default().bg(params.palette.help_bg));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

#[allow(clippy::too_many_lines)]
fn build_lines<'a>(params: &ConfigPopupParams<'_>) -> Vec<Line<'a>> {
    let ConfigPopupParams {
        state,
        theme,
        show_line_numbers,
        tree_position,
        search_preview,
        mermaid_mode,
        mermaid_text_backend,
        palette,
    } = params;
    let theme = *theme;
    let show_line_numbers = *show_line_numbers;
    let tree_position = *tree_position;
    let search_preview = *search_preview;
    let mermaid_mode = *mermaid_mode;
    let mermaid_text_backend = *mermaid_text_backend;

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
    row += 1;

    lines.push(Line::from(""));

    // --- Search section ---
    lines.push(Line::from(vec![
        Span::styled("  ", text_style),
        Span::styled(ConfigPopupState::SECTIONS[3].0, section_style),
    ]));
    lines.push(option_line(
        row == state.cursor,
        search_preview == SearchPreview::FullLine,
        "Full line preview",
        cursor_style,
        active_style,
        text_style,
        dim_style,
    ));
    row += 1;
    lines.push(option_line(
        row == state.cursor,
        search_preview == SearchPreview::Snippet,
        "Snippet preview (~80 chars)",
        cursor_style,
        active_style,
        text_style,
        dim_style,
    ));
    row += 1;

    lines.push(Line::from(""));

    // --- Mermaid section ---
    lines.push(Line::from(vec![
        Span::styled("  ", text_style),
        Span::styled(ConfigPopupState::SECTIONS[4].0, section_style),
    ]));
    lines.push(option_line(
        row == state.cursor,
        mermaid_mode == MermaidMode::Auto,
        Config::mermaid_mode_label(MermaidMode::Auto),
        cursor_style,
        active_style,
        text_style,
        dim_style,
    ));
    row += 1;
    lines.push(option_line(
        row == state.cursor,
        mermaid_mode == MermaidMode::Text,
        Config::mermaid_mode_label(MermaidMode::Text),
        cursor_style,
        active_style,
        text_style,
        dim_style,
    ));
    row += 1;
    lines.push(option_line(
        row == state.cursor,
        mermaid_mode == MermaidMode::Image,
        Config::mermaid_mode_label(MermaidMode::Image),
        cursor_style,
        active_style,
        text_style,
        dim_style,
    ));
    row += 1;
    // Three text-backend options live inside the Mermaid section (no
    // separate header) — they only affect text-mode flowchart and state
    // diagrams, so they read as a sub-choice of the mode rows above.
    // Auto comes first so it's the visible recommendation as we move
    // toward making it the default in a follow-up release.
    lines.push(option_line(
        row == state.cursor,
        mermaid_text_backend == MermaidTextBackend::Auto,
        Config::mermaid_text_backend_label(MermaidTextBackend::Auto),
        cursor_style,
        active_style,
        text_style,
        dim_style,
    ));
    row += 1;
    lines.push(option_line(
        row == state.cursor,
        mermaid_text_backend == MermaidTextBackend::Sugiyama,
        Config::mermaid_text_backend_label(MermaidTextBackend::Sugiyama),
        cursor_style,
        active_style,
        text_style,
        dim_style,
    ));
    row += 1;
    lines.push(option_line(
        row == state.cursor,
        mermaid_text_backend == MermaidTextBackend::Native,
        Config::mermaid_text_backend_label(MermaidTextBackend::Native),
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

    // Suppress unused variable warning: `row` is incremented to maintain the
    // flat-index logic for future sections but is not read after the last option.
    let _ = row;

    lines
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
