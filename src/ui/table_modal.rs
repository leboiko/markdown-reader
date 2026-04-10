use crate::app::{App, TableModalState};
use crate::theme::Palette;
use pulldown_cmark::Alignment;
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

/// Render the table modal overlay.
pub fn draw(f: &mut Frame, app: &App) {
    let state = match &app.table_modal {
        Some(s) => s,
        None => return,
    };
    let p = &app.palette;

    let area = f.area();
    let popup = centered_pct(90, 90, area);
    f.render_widget(Clear, popup);

    let num_cols = state.natural_widths.len();

    let title = format!(
        " Table  {} col{}  h/l pan  H/L pan 10  q/Esc close ",
        num_cols,
        if num_cols == 1 { "" } else { "s" },
    );

    let block = Block::default()
        .title(title)
        .title_style(p.title_style())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.border_focused))
        .style(Style::default().bg(p.help_bg));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Reserve 1 line for the footer.
    let content_height = inner.height.saturating_sub(1) as usize;

    // Render the table at natural widths, computing wrapped row heights.
    let rendered = render_modal_table(state, p);

    // Apply v_scroll: skip rows of rendered content.
    let v_scroll = state.v_scroll as usize;
    let visible_lines: Vec<&Line<'static>> = rendered
        .lines
        .iter()
        .skip(v_scroll)
        .take(content_height)
        .collect();

    // Apply h_scroll: slice each line at pixel offset h_scroll.
    let h_scroll = state.h_scroll as usize;
    let visible_width = inner.width as usize;

    let sliced_lines: Vec<Line<'static>> = visible_lines
        .iter()
        .map(|line| slice_line_at(line, h_scroll, visible_width))
        .collect();

    let content_rect = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: content_height as u16,
    };

    f.render_widget(Paragraph::new(Text::from(sliced_lines)), content_rect);

    // Footer with scroll info.
    let total_rendered = rendered.lines.len();
    let footer_text = format!(
        " row {}/{} \u{2502} col {}/{} \u{2502} j/k scroll  d/u half-page  g/G top/bot  0/$ h-pan ",
        v_scroll.saturating_add(1).min(total_rendered),
        total_rendered,
        h_scroll,
        state.natural_widths.iter().sum::<usize>(),
    );
    let footer_rect = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(1),
        width: inner.width,
        height: 1,
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(footer_text, p.dim_style()))),
        footer_rect,
    );
}

/// Render the table at natural widths without truncation, returning a `Text`
/// whose lines can be sliced for the modal viewport.
fn render_modal_table(state: &TableModalState, p: &Palette) -> Text<'static> {
    let border_style = Style::default().fg(p.table_border);
    let header_style = Style::default()
        .fg(p.table_header)
        .add_modifier(Modifier::BOLD);
    let cell_style = Style::default().fg(p.foreground);

    let col_widths = &state.natural_widths;
    let num_cols = col_widths.len();

    let mut lines: Vec<Line<'static>> = Vec::new();

    // Top border
    lines.push(modal_border_line('┌', '─', '┬', '┐', col_widths, border_style));

    // Header
    lines.push(modal_cell_line(
        &state.headers,
        col_widths,
        &state.alignments,
        border_style,
        header_style,
        num_cols,
    ));

    // Separator
    lines.push(modal_border_line('├', '─', '┼', '┤', col_widths, border_style));

    // Data rows
    for row in &state.rows {
        lines.push(modal_cell_line(
            row,
            col_widths,
            &state.alignments,
            border_style,
            cell_style,
            num_cols,
        ));
    }

    // Bottom border
    lines.push(modal_border_line('└', '─', '┴', '┘', col_widths, border_style));

    Text::from(lines)
}

fn modal_border_line(
    left: char,
    fill: char,
    mid: char,
    right: char,
    col_widths: &[usize],
    style: Style,
) -> Line<'static> {
    let mut s = String::new();
    s.push(left);
    for (i, &w) in col_widths.iter().enumerate() {
        for _ in 0..(w + 2) {
            s.push(fill);
        }
        if i + 1 < col_widths.len() {
            s.push(mid);
        }
    }
    s.push(right);
    Line::from(Span::styled(s, style))
}

fn modal_cell_line(
    cells: &[String],
    col_widths: &[usize],
    alignments: &[Alignment],
    border_style: Style,
    cell_style: Style,
    num_cols: usize,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(num_cols * 3 + 1);
    spans.push(Span::styled("│".to_string(), border_style));
    for (i, &w) in col_widths.iter().enumerate().take(num_cols) {
        let raw = cells.get(i).map(|s| s.as_str()).unwrap_or("");
        let aligned = modal_pad_cell(raw, w, alignments.get(i).copied().unwrap_or(Alignment::None));
        spans.push(Span::styled(format!(" {aligned} "), cell_style));
        spans.push(Span::styled("│".to_string(), border_style));
    }
    Line::from(spans)
}

/// Pad a cell to exactly `width` display columns (no truncation in the modal).
fn modal_pad_cell(text: &str, width: usize, alignment: Alignment) -> String {
    let display_width = UnicodeWidthStr::width(text);
    let padding = width.saturating_sub(display_width);
    match alignment {
        Alignment::Right => format!("{}{}", " ".repeat(padding), text),
        Alignment::Center => {
            let left = padding / 2;
            let right = padding - left;
            format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
        }
        Alignment::Left | Alignment::None => format!("{}{}", text, " ".repeat(padding)),
    }
}

/// Extract the visible horizontal slice `[h_scroll, h_scroll + visible_width)` from a line.
///
/// Handles multi-byte and double-width characters correctly by walking display
/// columns. Partially-visible double-width characters at the left edge are
/// replaced with a space.
pub fn slice_line_at(line: &Line<'static>, h_scroll: usize, visible_width: usize) -> Line<'static> {
    if visible_width == 0 {
        return Line::from("");
    }

    // Flatten the line to a single string for column-based slicing.
    let full: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let sliced = slice_row(&full, h_scroll, visible_width);

    // Preserve the style of the first span that has content (good enough for modal).
    let style = line.spans.first().map(|s| s.style).unwrap_or_default();
    Line::from(Span::styled(sliced, style))
}

/// Extract a visible horizontal slice from a rendered row string.
///
/// `h_scroll` is the number of display columns to skip from the left.
/// `visible_width` is the maximum number of display columns to return.
/// Double-width characters that straddle the left edge are replaced with a space.
pub fn slice_row(row: &str, h_scroll: usize, visible_width: usize) -> String {
    if visible_width == 0 {
        return String::new();
    }

    // Walk through the row accumulating display columns.
    let mut col = 0usize;
    let mut result = String::with_capacity(visible_width + 4);
    let mut skipped_half = false; // replaced a cut double-width char with space

    for ch in row.chars() {
        let ch_w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);

        if col + ch_w <= h_scroll {
            col += ch_w;
            continue;
        }

        if col < h_scroll {
            // Double-width char straddles the left edge — emit a replacement space.
            result.push(' ');
            col = h_scroll + 1;
            skipped_half = true;
            continue;
        }

        let used = result
            .chars()
            .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
            .sum::<usize>();

        if used + ch_w > visible_width {
            break;
        }

        result.push(ch);
        col += ch_w;
        let _ = skipped_half;
    }

    result
}

fn centered_pct(w_pct: u16, h_pct: u16, area: Rect) -> Rect {
    let w = (area.width * w_pct / 100).max(10);
    let h = (area.height * h_pct / 100).max(5);
    let vertical = Layout::vertical([Constraint::Length(h)])
        .flex(Flex::Center)
        .split(area);
    Layout::horizontal([Constraint::Length(w)])
        .flex(Flex::Center)
        .split(vertical[0])[0]
}

/// Compute the maximum visible horizontal extent of the rendered table.
pub fn max_h_scroll(state: &TableModalState, visible_width: u16) -> u16 {
    let total_table_width: usize = state.natural_widths.iter().sum::<usize>()
        + state.natural_widths.len() * 3  // padding + borders per col
        + 1; // right border
    (total_table_width.saturating_sub(visible_width as usize)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_row_ascii_mid_column() {
        let row = "abcdefghij";
        assert_eq!(slice_row(row, 3, 4), "defg");
    }

    #[test]
    fn slice_row_start_of_row() {
        let row = "hello world";
        assert_eq!(slice_row(row, 0, 5), "hello");
    }

    #[test]
    fn slice_row_past_end() {
        let row = "short";
        assert_eq!(slice_row(row, 10, 5), "");
    }

    #[test]
    fn slice_row_double_width_straddle() {
        // "AB" + Japanese double-width "ア" (2 cols) + "CD"
        // Positions: A=0, B=1, ア=2-3, C=4, D=5
        let row = "AB\u{30A2}CD";
        // h_scroll=3 cuts ア in half — expect a replacement space then "CD"
        let result = slice_row(row, 3, 5);
        assert!(
            result.starts_with(' '),
            "cut double-width char should be replaced with space: {result:?}"
        );
        assert!(result.contains('C'), "C should be visible: {result:?}");
    }

    #[test]
    fn slice_row_exact_visible_width() {
        let row = "12345678";
        assert_eq!(slice_row(row, 2, 4), "3456");
    }
}
