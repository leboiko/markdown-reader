use crate::app::{App, TableModalState};
use crate::theme::Palette;
use crate::ui::table_render::{border_line, emit_row_lines, wrap_table_rows};
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_width::UnicodeWidthChar;

/// Render the table modal overlay.
///
/// Caches `popup` into `app.table_modal_rect` each frame so the mouse handler
/// can do hit-testing without re-computing the layout.
pub fn draw(f: &mut Frame, app: &mut App) {
    let Some(state) = &app.table_modal else {
        return;
    };
    let p = &app.palette;

    let area = f.area();
    let popup = centered_pct(90, 90, area);
    // Cache for mouse hit-testing (see `handle_table_modal_mouse`).
    app.table_modal_rect = Some(popup);
    f.render_widget(Clear, popup);

    let num_cols = state.natural_widths.len();

    let title = format!(
        " Table  {} col{}  h/l col  H/L \u{00bd}pg  q/Esc close ",
        num_cols,
        if num_cols == 1 { "" } else { "s" },
    );

    // Use the viewer background rather than `help_bg` so the grid border
    // colour (which is tuned for contrast against the main background) stays
    // legible.  The focused-border colour around the modal still signals
    // "this is a modal" visually.
    let block = Block::default()
        .title(title)
        .title_style(p.title_style())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(p.border_focused))
        .style(Style::default().bg(p.background));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Reserve 1 line for the footer.
    let content_height = inner.height.saturating_sub(1) as usize;

    // Render the table at natural widths with span-aware word-wrap.
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
        height: crate::cast::u16_sat(content_height),
    };

    f.render_widget(Paragraph::new(Text::from(sliced_lines)), content_rect);

    // Footer with scroll info.
    let total_rendered = rendered.lines.len();
    let footer_text = format!(
        " row {}/{} \u{2502} col {}/{} \u{2502} j/k scroll  d/u \u{00bd}pg  g/G top/bot  0/$ h-pan  h/l col ",
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

/// Render the table at natural widths using the shared wrap pipeline, returning
/// a `Text` whose lines can be sliced for the modal viewport.
///
/// The modal uses `state.natural_widths` directly — no fair-share needed because
/// the user opened this expanded view to see everything.
fn render_modal_table(state: &TableModalState, p: &Palette) -> Text<'static> {
    let border_style = Style::default().fg(p.table_border);
    let header_style = Style::default()
        .fg(p.table_header)
        .add_modifier(Modifier::BOLD);
    let cell_style = Style::default().fg(p.foreground);

    let col_widths = &state.natural_widths;

    // Wrap all rows with the shared pipeline (same as inline layout_table).
    let wrapped = wrap_table_rows(&state.headers, &state.rows, col_widths);

    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(border_line('┌', '─', '┬', '┐', col_widths, border_style));

    // Header row(s).
    let header_row = &wrapped[0];
    lines.extend(emit_row_lines(
        header_row,
        col_widths,
        &state.alignments,
        border_style,
        header_style,
    ));

    lines.push(border_line('├', '─', '┼', '┤', col_widths, border_style));

    // Body rows.
    for body_row in &wrapped[1..] {
        lines.extend(emit_row_lines(
            body_row,
            col_widths,
            &state.alignments,
            border_style,
            cell_style,
        ));
    }

    lines.push(border_line('└', '─', '┴', '┘', col_widths, border_style));
    Text::from(lines)
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

    // Walk spans preserving each span's style. A naive "flatten to a string
    // and re-span with the first style" approach loses every cell's colour
    // because the first span is always the left border `│`, so the entire
    // row inherits the border's muted grey.
    let mut out: Vec<Span<'static>> = Vec::with_capacity(line.spans.len());
    let mut col = 0usize; // absolute display column across the whole line
    let mut used = 0usize; // display columns already written to `out`

    for span in &line.spans {
        if used >= visible_width {
            break;
        }
        let mut buf = String::new();
        for ch in span.content.chars() {
            let ch_w = UnicodeWidthChar::width(ch).unwrap_or(0);

            // Fully before the left edge — skip without emitting.
            if col + ch_w <= h_scroll {
                col += ch_w;
                continue;
            }

            // Double-width char straddling the left edge — replace with a
            // space so downstream column math stays aligned.
            if col < h_scroll {
                if used + 1 > visible_width {
                    break;
                }
                buf.push(' ');
                used += 1;
                col = h_scroll + 1;
                continue;
            }

            if used + ch_w > visible_width {
                break;
            }
            buf.push(ch);
            used += ch_w;
            col += ch_w;
        }
        if !buf.is_empty() {
            out.push(Span::styled(buf, span.style));
        }
    }

    Line::from(out)
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
    crate::cast::u16_sat(total_table_width.saturating_sub(visible_width as usize))
}

/// Return the column-start offsets (in display columns) for each column.
///
/// Each column occupies `width + 3` display columns: one leading space, the
/// cell content, one trailing space, and one border character. This matches
/// the formula used by [`max_h_scroll`].
///
/// The returned slice contains one entry per column. Offset `0` is the leading
/// outer border, so column `i` starts at `sum(widths[0..i]) + i * 3`.
fn col_boundaries(widths: &[usize]) -> Vec<u16> {
    let mut offsets = Vec::with_capacity(widths.len());
    let mut acc: usize = 0;
    for &w in widths {
        offsets.push(crate::cast::u16_sat(acc));
        // Each column occupies: 1 leading space + w content + 1 trailing space
        // + 1 border = w + 3 display columns.
        acc += w + 3;
    }
    offsets
}

/// Snap `h_scroll` to the start of the previous column.
///
/// Finds the largest column-start offset strictly less than `h_scroll` and
/// returns it. If `h_scroll` is already at or before the first boundary (0),
/// returns 0.
///
/// # Examples
///
/// ```
/// // widths [10, 20, 15] → boundaries [0, 13, 36]
/// // From 17 (inside col 1, which starts at 13), previous boundary is 13.
/// assert_eq!(prev_col_boundary(&[10, 20, 15], 17), 13);
/// // From 13 exactly, previous boundary is 0 (start of col 0).
/// assert_eq!(prev_col_boundary(&[10, 20, 15], 13), 0);
/// // From 0 there is no earlier boundary — stays at 0.
/// assert_eq!(prev_col_boundary(&[10, 20, 15], 0), 0);
/// ```
pub fn prev_col_boundary(widths: &[usize], h_scroll: u16) -> u16 {
    // rfind on the sorted boundary list returns the last boundary strictly less
    // than h_scroll, which is the start of the column we are currently inside.
    col_boundaries(widths)
        .into_iter()
        .rfind(|&b| b < h_scroll)
        .unwrap_or(0)
}

/// Snap `h_scroll` to the start of the next column, clamped to `max`.
///
/// Finds the smallest column-start offset strictly greater than `h_scroll` and
/// returns it, clamped to `max`. If there is no such boundary, returns `max`.
///
/// # Examples
///
/// ```
/// // widths [10, 20, 15] → boundaries [0, 13, 36]
/// assert_eq!(next_col_boundary(&[10, 20, 15], 0, 100), 13);
/// assert_eq!(next_col_boundary(&[10, 20, 15], 36, 100), 100);
/// ```
pub fn next_col_boundary(widths: &[usize], h_scroll: u16, max: u16) -> u16 {
    col_boundaries(widths)
        .into_iter()
        // Strictly greater than current position.
        .find(|&b| b > h_scroll)
        .unwrap_or(max)
        .min(max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TableModalState;
    use crate::markdown::CellSpans;
    use crate::theme::{Palette, Theme};
    use insta::assert_snapshot;
    use pulldown_cmark::Alignment;
    use ratatui::style::Color;

    fn palette() -> Palette {
        Palette::from_theme(Theme::Default)
    }

    fn plain(s: &str) -> CellSpans {
        vec![Span::raw(s.to_string())]
    }

    fn styled(text: &str, fg: ratatui::style::Color) -> Span<'static> {
        Span::styled(text.to_string(), Style::default().fg(fg))
    }

    fn make_modal_state(headers: &[&str], rows: &[&[&str]], widths: Vec<usize>) -> TableModalState {
        TableModalState {
            tab_id: crate::ui::tabs::TabId(0),
            headers: headers.iter().map(|s| plain(s)).collect(),
            rows: rows
                .iter()
                .map(|row| row.iter().map(|s| plain(s)).collect())
                .collect(),
            alignments: vec![Alignment::None; headers.len()],
            natural_widths: widths,
            v_scroll: 0,
            h_scroll: 0,
        }
    }

    // ── slice_line_at tests ──────────────────────────────────────────────────

    #[test]
    fn slice_line_at_preserves_per_span_styles() {
        // Simulate a header row: border + header text + border + header text + border.
        let line = Line::from(vec![
            styled("│", Color::Gray),
            styled(" Name ", Color::Blue),
            styled("│", Color::Gray),
            styled(" Value ", Color::Blue),
            styled("│", Color::Gray),
        ]);
        let sliced = slice_line_at(&line, 0, 100);
        // The header text must retain Blue — not collapse to the border's grey.
        let blue_count = sliced
            .spans
            .iter()
            .filter(|s| s.style.fg == Some(Color::Blue))
            .count();
        assert!(
            blue_count >= 2,
            "expected at least 2 blue header spans, got {blue_count}: {:#?}",
            sliced.spans,
        );
    }

    #[test]
    fn slice_line_at_ascii_mid_column() {
        let line = Line::from(vec![Span::raw("abcdefghij".to_string())]);
        let sliced = slice_line_at(&line, 3, 4);
        let text: String = sliced.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "defg");
    }

    #[test]
    fn slice_line_at_past_end_returns_empty() {
        let line = Line::from(vec![Span::raw("short".to_string())]);
        let sliced = slice_line_at(&line, 10, 5);
        let text: String = sliced.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "");
    }

    #[test]
    fn slice_line_at_double_width_straddle() {
        // AB<wide>CD — ask for columns 3..8. The wide char straddles left edge
        // and must be replaced with a single space.
        let line = Line::from(vec![Span::raw("AB\u{30A2}CD".to_string())]);
        let sliced = slice_line_at(&line, 3, 5);
        let text: String = sliced.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            text.starts_with(' '),
            "cut double-width char should be replaced with space: {text:?}",
        );
        assert!(text.contains('C'), "C should be visible: {text:?}");
    }

    #[test]
    fn slice_line_at_exact_visible_width() {
        let line = Line::from(vec![Span::raw("12345678".to_string())]);
        let sliced = slice_line_at(&line, 2, 4);
        let text: String = sliced.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "3456");
    }

    #[test]
    fn slice_line_at_zero_width_returns_empty_line() {
        let line = Line::from(vec![Span::raw("hello".to_string())]);
        let sliced = slice_line_at(&line, 0, 0);
        let text: String = sliced.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "");
    }

    // ── column boundary helper tests ─────────────────────────────────────────

    /// widths [10, 20, 15] produce boundaries [0, 13, 36]:
    ///   col 0 starts at 0
    ///   col 1 starts at 10 + 3 = 13
    ///   col 2 starts at 13 + 20 + 3 = 36
    #[test]
    fn prev_col_boundary_jumps_to_start_of_current_column() {
        let widths = [10usize, 20, 15];
        // From 17 (inside col 1, which starts at 13), largest boundary < 17 is 13.
        assert_eq!(prev_col_boundary(&widths, 17), 13);
        // From 13 exactly (on a boundary), largest boundary < 13 is 0 (start of col 0).
        assert_eq!(prev_col_boundary(&widths, 13), 0);
        // From 0 there is no earlier boundary — stays at 0.
        assert_eq!(prev_col_boundary(&widths, 0), 0);
    }

    #[test]
    fn next_col_boundary_jumps_past_current_column() {
        let widths = [10usize, 20, 15];
        // From 0, the next boundary is 13 (start of col 1).
        assert_eq!(next_col_boundary(&widths, 0, 200), 13);
        // From 36 (last boundary), no further boundary → clamp to max.
        assert_eq!(next_col_boundary(&widths, 36, 100), 100);
        // From 13, the next boundary is 36 (start of col 2).
        assert_eq!(next_col_boundary(&widths, 13, 200), 36);
    }

    #[test]
    fn boundary_helpers_handle_empty_widths() {
        // No columns → no movement in either direction.
        assert_eq!(prev_col_boundary(&[], 5), 0);
        assert_eq!(next_col_boundary(&[], 5, 50), 50);
    }

    // ── Modal snapshot test ──────────────────────────────────────────────────

    #[test]
    fn tbl_modal_5col_natural() {
        let long = "description with multiple words that wrap";
        let state = make_modal_state(
            &["ID", "Name", "Value", "Description", "Status"],
            &[
                &["1", "Alice", "100", long, "active"],
                &["2", "Bob", "200", "short", "inactive"],
            ],
            vec![2, 5, 5, long.len(), 8],
        );
        let rendered = render_modal_table(&state, &palette());
        let snap: String = rendered
            .lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert_snapshot!(snap);
    }
}
