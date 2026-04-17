use pulldown_cmark::Alignment;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span, Text},
};
use unicode_width::UnicodeWidthStr;

use crate::markdown::{CellSpans, TableBlock, cell_display_width};
use crate::theme::Palette;

/// Lay out a table for the given `inner_width` and render it to a `Text`.
///
/// Returns `(rendered_text, height_in_lines, was_truncated)`.
///
/// The `was_truncated` flag is true if any column's allotted width is less
/// than its natural width. When true, a dim hint line is appended below the
/// bottom border, and the height includes that line.
pub fn layout_table(
    table: &TableBlock,
    inner_width: u16,
    palette: &Palette,
) -> (Text<'static>, u32, bool) {
    let num_cols = table
        .headers
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));

    if num_cols == 0 {
        return (Text::from(""), 0, false);
    }

    let border_style = Style::default().fg(palette.table_border);
    let header_style = Style::default()
        .fg(palette.table_header)
        .add_modifier(Modifier::BOLD);
    let cell_style = Style::default().fg(palette.foreground);
    let dim_style = Style::default().fg(palette.dim);

    // Too-narrow check: need at least 1 char per cell + 2 padding + borders.
    // Minimum layout: num_cols + 1 borders + 2*num_cols padding + num_cols*1 content
    // = num_cols*3 + num_cols + 1
    let min_width = crate::cast::u16_sat(num_cols) * 3 + crate::cast::u16_sat(num_cols) + 1;
    if inner_width < min_width {
        let placeholder = Line::from(Span::styled(
            "[ table \u{2014} too narrow, press \u{23ce} to expand ]".to_string(),
            dim_style,
        ));
        return (Text::from(vec![placeholder]), 1, true);
    }

    // Available content width after removing all borders (num_cols+1) and padding (2*num_cols).
    let target = (inner_width as usize)
        .saturating_sub(num_cols + 1)
        .saturating_sub(2 * num_cols);

    let col_widths = fair_share_widths(&table.natural_widths, num_cols, target);
    let was_truncated = col_widths
        .iter()
        .zip(&table.natural_widths)
        .any(|(allotted, &natural)| *allotted < natural);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(table.rows.len() + 4);

    // Top border: ┌──┬──┐
    lines.push(border_line('┌', '─', '┬', '┐', &col_widths, border_style));

    // Header row
    lines.push(span_cell_line(
        &table.headers,
        &col_widths,
        &table.alignments,
        border_style,
        header_style,
        num_cols,
        palette,
    ));

    // Header separator: ├──┼──┤
    lines.push(border_line('├', '─', '┼', '┤', &col_widths, border_style));

    // Data rows
    for row in &table.rows {
        lines.push(span_cell_line(
            row,
            &col_widths,
            &table.alignments,
            border_style,
            cell_style,
            num_cols,
            palette,
        ));
    }

    // Bottom border: └──┴──┘
    lines.push(border_line('└', '─', '┴', '┘', &col_widths, border_style));

    if was_truncated {
        lines.push(Line::from(Span::styled(
            "  [press \u{23ce} to expand full table]".to_string(),
            dim_style,
        )));
    }

    let height = crate::cast::u32_sat(lines.len());
    (Text::from(lines), height, was_truncated)
}

/// Compute column widths using a proportional fair-share algorithm.
///
/// If all naturals fit within `target`, returns natural widths (clamped to >= 1).
/// Otherwise, each column gets a minimum of `min(6, natural_width)`, and remaining
/// space is distributed proportionally to each column's excess over its minimum.
fn fair_share_widths(natural_widths: &[usize], num_cols: usize, target: usize) -> Vec<usize> {
    let naturals: Vec<usize> = (0..num_cols)
        .map(|i| natural_widths.get(i).copied().unwrap_or(1).max(1))
        .collect();

    let total_natural: usize = naturals.iter().sum();
    if total_natural <= target {
        return naturals;
    }

    let mins: Vec<usize> = naturals.iter().map(|&n| n.clamp(1, 6)).collect();
    let total_min: usize = mins.iter().sum();

    if total_min >= target {
        // Even minimums don't fit; distribute target evenly (each col gets at least 1).
        let per_col = (target / num_cols).max(1);
        return mins.iter().map(|&m| m.min(per_col).max(1)).collect();
    }

    let remaining = target - total_min;
    let total_excess: usize = naturals
        .iter()
        .zip(&mins)
        .map(|(&n, &m)| n.saturating_sub(m))
        .sum();

    let mut widths = mins.clone();
    if total_excess > 0 {
        for (i, (&natural, &min)) in naturals.iter().zip(&mins).enumerate() {
            let excess = natural.saturating_sub(min);
            let extra = (excess * remaining) / total_excess;
            widths[i] = (min + extra).min(natural);
        }
    }
    widths
}

/// Render a horizontal border line (top, separator, or bottom).
fn border_line(
    left: char,
    fill: char,
    mid: char,
    right: char,
    col_widths: &[usize],
    style: Style,
) -> Line<'static> {
    let mut s = String::with_capacity(col_widths.iter().sum::<usize>() + col_widths.len() * 4);
    s.push(left);
    for (i, &w) in col_widths.iter().enumerate() {
        // +2 for the single-space padding on each side
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

/// Render one data or header row, preserving each cell's inline span styling.
///
/// `cell_style` is applied only as a fallback for padding spans; actual cell
/// content retains whatever style was set by the markdown renderer.
fn span_cell_line(
    cells: &[CellSpans],
    col_widths: &[usize],
    alignments: &[Alignment],
    border_style: Style,
    cell_style: Style,
    num_cols: usize,
    palette: &Palette,
) -> Line<'static> {
    let empty: CellSpans = Vec::new();
    let mut out: Vec<Span<'static>> = Vec::with_capacity(num_cols * 4 + 1);
    out.push(Span::styled("│".to_string(), border_style));

    for (i, &w) in col_widths.iter().enumerate().take(num_cols) {
        let cell = cells.get(i).unwrap_or(&empty);
        let cell_w = cell_display_width(cell);
        let alignment = alignments.get(i).copied().unwrap_or(Alignment::None);

        out.push(Span::styled(" ".to_string(), cell_style));

        if cell_w <= w {
            let padding = w - cell_w;
            let (left_pad, right_pad) = match alignment {
                Alignment::Right => (padding, 0),
                Alignment::Center => (padding / 2, padding - padding / 2),
                Alignment::Left | Alignment::None => (0, padding),
            };
            if left_pad > 0 {
                out.push(Span::styled(" ".repeat(left_pad), cell_style));
            }
            out.extend(cell.iter().cloned());
            if right_pad > 0 {
                out.push(Span::styled(" ".repeat(right_pad), cell_style));
            }
        } else {
            out.extend(truncate_spans(cell, w, palette));
        }

        out.push(Span::styled(" │".to_string(), border_style));
    }

    Line::from(out)
}

/// Truncate a sequence of spans to fit within `max_width` display columns.
///
/// Walks spans in order, accumulating displayed width. When adding the next span
/// would exceed the budget, the current span is cut at the last char boundary
/// that fits within `max_width - 1` columns, and a `…` span with a dim style is
/// appended. The truncated span retains its original style up to the cut point.
pub fn truncate_spans(
    spans: &[Span<'static>],
    max_width: usize,
    palette: &Palette,
) -> Vec<Span<'static>> {
    if max_width == 0 {
        return Vec::new();
    }

    let dim_style = palette.dim_style();
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let budget = max_width.saturating_sub(1); // reserve 1 col for '…'

    'outer: for span in spans {
        let text = span.content.as_ref();
        let span_w = UnicodeWidthStr::width(text);

        if used + span_w <= budget {
            out.push(span.clone());
            used += span_w;
            continue;
        }

        // This span overflows — cut inside it.
        let mut byte_end = 0usize;
        let mut accumulated = 0usize;
        for ch in text.chars() {
            let ch_w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            if used + accumulated + ch_w > budget {
                break;
            }
            accumulated += ch_w;
            byte_end += ch.len_utf8();
        }

        if byte_end > 0 {
            out.push(Span::styled(text[..byte_end].to_string(), span.style));
        }
        // Pad to fill the gap left by double-width chars if needed.
        let pad = budget.saturating_sub(used + accumulated);
        if pad > 0 {
            out.push(Span::styled(" ".repeat(pad), span.style));
        }
        out.push(Span::styled("\u{2026}".to_string(), dim_style));
        break 'outer;
    }

    // If all spans fit without hitting the budget (used == cell_w <= max_width),
    // no truncation occurred — return as-is without the ellipsis.
    if used
        == spans
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum::<usize>()
    {
        return out;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{CellSpans, TableBlock, TableBlockId, cell_display_width};
    use crate::theme::{Palette, Theme};

    fn palette() -> Palette {
        Palette::from_theme(Theme::Default)
    }

    fn str_cell(s: &str) -> CellSpans {
        vec![Span::raw(s.to_string())]
    }

    fn make_table(headers: &[&str], rows: &[&[&str]], alignments: &[Alignment]) -> TableBlock {
        let h: Vec<CellSpans> = headers.iter().map(|s| str_cell(s)).collect();
        let r: Vec<Vec<CellSpans>> = rows
            .iter()
            .map(|row| row.iter().map(|s| str_cell(s)).collect())
            .collect();
        let aligns: Vec<Alignment> = if alignments.is_empty() {
            vec![Alignment::None; headers.len()]
        } else {
            alignments.to_vec()
        };
        let mut natural_widths = vec![0usize; headers.len()];
        for (i, cell) in h.iter().enumerate() {
            natural_widths[i] = natural_widths[i].max(cell_display_width(cell));
        }
        for row in &r {
            for (i, cell) in row.iter().enumerate() {
                if i < headers.len() {
                    natural_widths[i] = natural_widths[i].max(cell_display_width(cell));
                }
            }
        }
        for w in &mut natural_widths {
            *w = (*w).max(1);
        }
        // Stub row_source_lines: header at line 0, body rows at 2, 3, ...
        // Exact values don't matter for layout tests.
        let row_source_lines: Vec<u32> = std::iter::once(0)
            .chain((2u32..).take(rows.len()))
            .collect();
        TableBlock {
            id: TableBlockId(0),
            headers: h,
            rows: r,
            alignments: aligns,
            natural_widths,
            rendered_height: 3,
            source_line: 0,
            row_source_lines,
        }
    }

    #[test]
    fn table_fits_naturally() {
        let table = make_table(
            &["A", "B", "C"],
            &[&["a1", "b1", "c1"], &["a2", "b2", "c2"]],
            &[],
        );
        // Natural widths: A=1, B=1, C=1 (all single char). Wide terminal.
        let (text, _height, was_truncated) = layout_table(&table, 80, &palette());
        assert!(!was_truncated, "Short cells should fit naturally");
        // Border line: ┌───┬───┬───┐ for 1-char cols (+2 padding each)
        let top = &text.lines[0].spans[0].content;
        assert!(top.contains('┌'), "Top border missing");
        assert!(!top.contains('\u{2026}'), "No ellipsis in borders");
    }

    #[test]
    fn table_needs_truncation() {
        let long_cell = "x".repeat(200);
        let table = make_table(
            &["Short", "Very Long Column Header"],
            &[&["val", long_cell.as_str()]],
            &[],
        );
        let (text, _height, was_truncated) = layout_table(&table, 60, &palette());
        assert!(was_truncated, "200-char cell must trigger truncation");
        // Last line should be the hint
        let last = text.lines.last().unwrap();
        let hint: String = last.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            hint.contains('\u{23ce}'),
            "Hint line must contain the enter symbol"
        );
    }

    #[test]
    fn too_narrow_fallback() {
        let table = make_table(&["A", "B", "C"], &[&["x", "y", "z"]], &[]);
        // 3 cols need min 3*3+3+1 = 13 cols; use 5.
        let (text, height, was_truncated) = layout_table(&table, 5, &palette());
        assert!(was_truncated);
        assert_eq!(height, 1, "Too-narrow returns exactly 1 line");
        let line: String = text.lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(line.contains("too narrow"), "Placeholder text missing");
    }

    #[test]
    fn unicode_width_cells() {
        // Japanese characters are double-width.
        let wide_cell = "\u{30A2}\u{30A4}\u{30A6}"; // ア イ ウ — 6 display cols
        let table = make_table(&["JP"], &[&[wide_cell]], &[]);
        // Natural width should be 6.
        assert_eq!(table.natural_widths[0], 6);
        // Render in a terminal wide enough to fit.
        let (text, _h, was_truncated) = layout_table(&table, 20, &palette());
        assert!(!was_truncated, "Wide chars fit in 20 cols");
        // Render with a very narrow terminal to trigger truncation.
        let (text2, _h2, was_truncated2) = layout_table(&table, 8, &palette());
        assert!(
            was_truncated2,
            "Wide chars must trigger truncation in 8 cols"
        );
        // Confirm the ellipsis is present in a data row.
        let row_line: String = text2.lines[3]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            row_line.contains('\u{2026}'),
            "Truncated wide cell must end with ellipsis"
        );
        let _ = text;
    }

    #[test]
    fn alignment_respected() {
        let table = make_table(&["Num"], &[&["42"]], &[Alignment::Right]);
        // Natural width of "42" is 2; render wide enough.
        let (text, _h, _trunc) = layout_table(&table, 20, &palette());
        // Data row is text.lines[3] (top, header, sep, data, bottom).
        let data_row: String = text.lines[3]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        // In right-aligned col, cell content is padded on the left.
        // "│ " + spaces + "42" + " │"  -- the spaces come before "42".
        let inner = data_row.trim_matches('│');
        let trimmed = inner.trim();
        // Content "42" should be at the right edge of the inner padding.
        assert!(
            inner.starts_with(' '),
            "Right-aligned cell must start with space padding: {inner:?}"
        );
        assert!(trimmed == "42", "Cell content must be '42'");
    }

    #[test]
    fn truncate_spans_cuts_inside_plain_span() {
        use ratatui::style::{Color, Style};
        let p = palette();
        let bold = Style::default().add_modifier(ratatui::style::Modifier::BOLD);
        let plain = Style::default().fg(Color::White);
        let code = Style::default().fg(Color::Yellow);

        let spans: Vec<Span<'static>> = vec![
            Span::styled("bold".to_string(), bold),
            Span::styled(" plain text here".to_string(), plain),
            Span::styled("`code`".to_string(), code),
        ];
        // Width=10 means budget=9. "bold" (4) + " plain" (6) = 10 > 9.
        // So we cut inside " plain text here" at " plain" (6 chars, fits in budget=9-4=5 remaining).
        let result = truncate_spans(&spans, 10, &p);

        // First span (bold) must survive unchanged.
        assert_eq!(result[0].content.as_ref(), "bold");
        assert_eq!(result[0].style, bold);

        // The plain span must be partially present (truncated).
        let plain_part = &result[1];
        assert_eq!(plain_part.style, plain);
        let plain_text = plain_part.content.as_ref();
        assert!(
            UnicodeWidthStr::width(plain_text) <= 5,
            "plain truncated part too wide: {plain_text:?}"
        );

        // Last output span must be the ellipsis.
        let last = result.last().unwrap();
        assert_eq!(last.content.as_ref(), "\u{2026}", "must end with ellipsis");

        // Code span must not appear.
        assert!(
            !result.iter().any(|s| s.content.as_ref().contains("`code`")),
            "code span must not appear after truncation"
        );

        // Total displayed width must be <= 10.
        let total_w: usize = result
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum();
        assert!(total_w <= 10, "total width {total_w} exceeds limit 10");
    }

    #[test]
    fn truncate_spans_short_cell_no_truncation() {
        let p = palette();
        let spans = vec![Span::raw("hi".to_string())];
        let result = truncate_spans(&spans, 10, &p);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content.as_ref(), "hi");
    }
}
