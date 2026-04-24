use pulldown_cmark::Alignment;
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span, Text},
};

use crate::markdown::{CellSpans, TableBlock};
use crate::text_layout::{WrappedLine, wrap_spans};
use crate::theme::Palette;

// ── Private layout types ──────────────────────────────────────────────────────

/// Per markdown row, the wrapped output for each column plus the row's
/// physical height (max wrap-line count across columns).
///
/// # Invariants
///
/// - `cells.len() == num_cols` — one inner `Vec<WrappedLine>` per column.
/// - `height == cells.iter().map(|c| c.len()).max().unwrap_or(1)` — the
///   number of physical terminal rows this markdown row occupies after
///   wrapping. A row with all empty cells still occupies exactly one
///   physical row.
/// - Every `WrappedLine` inside `cells[c]` satisfies
///   `line.width <= col_widths[c]`, guaranteed by [`wrap_spans`].
pub(super) struct WrappedRow {
    /// Outer Vec length == num_cols. Each inner Vec is the wrapped output
    /// for that cell at its column width. Empty cells produce a single empty
    /// `WrappedLine` so `height` is always `>= 1`.
    pub(super) cells: Vec<Vec<WrappedLine>>,
    /// `max(cells[c].len())` — the number of physical terminal rows this
    /// markdown row occupies after wrapping.
    pub(super) height: usize,
}

/// Wrap every cell of every row (headers + body) to its column width.
///
/// Returns one `WrappedRow` per logical markdown row in the sequence
/// `[headers, body[0], body[1], ...]`.
///
/// # Arguments
///
/// * `headers`    – header cell spans.
/// * `body`       – body rows, each a `Vec<CellSpans>`.
/// * `col_widths` – allotted display-column width per column.
pub(super) fn wrap_table_rows(
    headers: &[CellSpans],
    body: &[Vec<CellSpans>],
    col_widths: &[usize],
) -> Vec<WrappedRow> {
    let num_cols = col_widths.len();

    // Helper: wrap one markdown row's cells to their column widths.
    let wrap_row = |cells: &[CellSpans]| -> WrappedRow {
        let wrapped_cells: Vec<Vec<WrappedLine>> = (0..num_cols)
            .map(|c| {
                let cell: &[Span<'static>] = cells.get(c).map_or(&[], |s| s.as_slice());
                let w = crate::cast::u16_sat(col_widths.get(c).copied().unwrap_or(1).max(1));
                wrap_spans(cell, w)
            })
            .collect();
        // Every column produces at least one WrappedLine (wrap_spans
        // guarantees a non-empty result), so max is always Some.
        let height = wrapped_cells.iter().map(Vec::len).max().unwrap_or(1);
        WrappedRow {
            cells: wrapped_cells,
            height,
        }
    };

    let mut rows = Vec::with_capacity(1 + body.len());
    rows.push(wrap_row(headers));
    for row in body {
        rows.push(wrap_row(row));
    }
    rows
}

/// Emit the rendered ratatui `Line`s for one `WrappedRow` (`row.height` lines).
///
/// Top-aligns short cells: sub-rows beyond a cell's `cells[c].len()` are
/// padded with `col_widths[c]` spaces. Vertical bars are emitted on every
/// sub-row so column boundaries stay aligned.
///
/// `cell_style` is used for padding spans only; actual cell content retains
/// whatever style was set by the markdown renderer.
///
/// # Arguments
///
/// * `row`          – pre-wrapped row produced by [`wrap_table_rows`].
/// * `col_widths`   – same slice used when wrapping (widths in display columns).
/// * `alignments`   – per-column alignment from pulldown-cmark.
/// * `border_style` – style for `│` characters.
/// * `cell_style`   – style for padding / blank sub-row spans.
pub(super) fn emit_row_lines(
    row: &WrappedRow,
    col_widths: &[usize],
    alignments: &[Alignment],
    border_style: Style,
    cell_style: Style,
) -> Vec<Line<'static>> {
    let num_cols = col_widths.len();
    let mut out = Vec::with_capacity(row.height);

    for sub in 0..row.height {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(num_cols * 4 + 1);
        spans.push(Span::styled("│".to_string(), border_style));

        for (c, &w) in col_widths.iter().enumerate().take(num_cols) {
            let alignment = alignments.get(c).copied().unwrap_or(Alignment::None);
            let cell_line: &[crate::text_layout::WrappedSpan] = row
                .cells
                .get(c)
                .and_then(|lines| lines.get(sub))
                .map_or(&[], |l| l.spans.as_slice());

            // Display width of this sub-row's content.
            let cell_w: usize = cell_line.iter().map(|s| s.width as usize).sum();
            let padding = w.saturating_sub(cell_w);

            // Convert WrappedSpan → ratatui Span (owned content, same style).
            // This is a small allocation; `WrappedSpan` content is already owned.
            let content_spans: Vec<Span<'static>> = cell_line
                .iter()
                .map(|ws| Span::styled(ws.content.clone(), ws.style))
                .collect();

            // Emit: leading space + alignment padding + content + trailing space + border.
            match alignment {
                Alignment::Right => {
                    let pad_str = format!(" {}", " ".repeat(padding));
                    spans.push(Span::styled(pad_str, cell_style));
                    spans.extend(content_spans);
                    spans.push(Span::styled(" │".to_string(), border_style));
                }
                Alignment::Center => {
                    let left = padding / 2;
                    let right = padding - left;
                    let pad_str = format!(" {}", " ".repeat(left));
                    spans.push(Span::styled(pad_str, cell_style));
                    spans.extend(content_spans);
                    let trail = format!("{} │", " ".repeat(right));
                    spans.push(Span::styled(trail, border_style));
                }
                Alignment::Left | Alignment::None => {
                    spans.push(Span::styled(" ".to_string(), cell_style));
                    spans.extend(content_spans);
                    let trail = format!("{} │", " ".repeat(padding));
                    spans.push(Span::styled(trail, border_style));
                }
            }
        }

        out.push(Line::from(spans));
    }

    out
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Lay out a table for the given `inner_width` and render it to a `Text`.
///
/// Returns `(rendered_text, height_in_lines, physical_to_source)`.
///
/// `physical_to_source[i]` is the 0-indexed source line for physical sub-row `i`
/// of the rendered table (counting from the top border at index 0). Length equals
/// the total rendered line count. Each markdown row's source line is repeated for
/// every of its physical sub-rows. Border rows (top/mid/bottom) inherit the
/// nearest content row's source line.
///
/// When the table is too narrow to render (`inner_width < min_width`), returns a
/// single-line placeholder with an empty `physical_to_source`.
pub fn layout_table(
    table: &TableBlock,
    inner_width: u16,
    palette: &Palette,
) -> (Text<'static>, u32, Vec<u32>) {
    let num_cols = table
        .headers
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));

    if num_cols == 0 {
        return (Text::from(""), 0, vec![]);
    }

    let border_style = Style::default().fg(palette.table_border);
    let header_style = Style::default()
        .fg(palette.table_header)
        .add_modifier(Modifier::BOLD);
    let cell_style = Style::default().fg(palette.foreground);
    let dim_style = Style::default().fg(palette.dim);

    // Too-narrow check: need at least 1 char per cell + 2 padding + borders.
    let min_width = crate::cast::u16_sat(num_cols) * 3 + crate::cast::u16_sat(num_cols) + 1;
    if inner_width < min_width {
        let placeholder = Line::from(Span::styled(
            "[ table \u{2014} too narrow, press \u{23ce} to expand ]".to_string(),
            dim_style,
        ));
        // Too-narrow uses the table's source_line as fallback for the single line.
        return (Text::from(vec![placeholder]), 1, vec![table.source_line]);
    }

    // Available content width after removing all borders (num_cols+1) and padding (2*num_cols).
    let target = (inner_width as usize)
        .saturating_sub(num_cols + 1)
        .saturating_sub(2 * num_cols);

    let col_widths = fair_share_widths(&table.natural_widths, num_cols, target);

    // Wrap all rows (index 0 = headers, 1..=rows.len() = body).
    let wrapped = wrap_table_rows(&table.headers, &table.rows, &col_widths);

    let mut lines: Vec<Line<'static>> = Vec::new();
    // `physical_to_source[i]` = source line for the i-th rendered line.
    let mut physical_to_source: Vec<u32> = Vec::new();

    // Source lines: index 0 = header, 1..=rows.len() = body rows.
    let header_source = table
        .row_source_lines
        .first()
        .copied()
        .unwrap_or(table.source_line);
    let last_source = table
        .row_source_lines
        .last()
        .copied()
        .unwrap_or(table.source_line);

    // Top border: ┌──┬──┐  → inherits header's source line.
    lines.push(border_line('┌', '─', '┬', '┐', &col_widths, border_style));
    physical_to_source.push(header_source);

    // Header row(s).
    let header_row = &wrapped[0];
    for sub_line in emit_row_lines(
        header_row,
        &col_widths,
        &table.alignments,
        border_style,
        header_style,
    ) {
        physical_to_source.push(header_source);
        lines.push(sub_line);
    }

    // Header separator: ├──┼──┤  → inherits header source line.
    lines.push(border_line('├', '─', '┼', '┤', &col_widths, border_style));
    physical_to_source.push(header_source);

    // Body rows.
    for (row_idx, body_row) in wrapped[1..].iter().enumerate() {
        let row_source = table
            .row_source_lines
            .get(1 + row_idx)
            .copied()
            .unwrap_or(table.source_line);
        for sub_line in emit_row_lines(
            body_row,
            &col_widths,
            &table.alignments,
            border_style,
            cell_style,
        ) {
            physical_to_source.push(row_source);
            lines.push(sub_line);
        }
    }

    // Bottom border: └──┴──┘  → inherits last row's source line.
    lines.push(border_line('└', '─', '┴', '┘', &col_widths, border_style));
    physical_to_source.push(last_source);

    let height = crate::cast::u32_sat(lines.len());
    debug_assert_eq!(
        lines.len(),
        physical_to_source.len(),
        "physical_to_source length must equal rendered line count"
    );
    (Text::from(lines), height, physical_to_source)
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
    for (i, (&natural, &min)) in naturals.iter().zip(&mins).enumerate() {
        let excess = natural.saturating_sub(min);
        if let Some(extra) = (excess * remaining).checked_div(total_excess) {
            widths[i] = (min + extra).min(natural);
        }
    }
    widths
}

/// Render a horizontal border line (top, separator, or bottom).
///
/// Visible to `pub(super)` so the expanded-table modal in
/// `super::table_modal` can share this exact function — Phase 2's
/// "modal and inline use one pipeline" guarantee. The four corner
/// characters parameterise the three border kinds: `(┌, ─, ┬, ┐)` top,
/// `(├, ─, ┼, ┤)` separator, `(└, ─, ┴, ┘)` bottom.
pub(super) fn border_line(
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{CellSpans, TableBlock, TableBlockId};
    use crate::text_layout::measure;
    use crate::theme::{Palette, Theme};
    use insta::assert_snapshot;
    use ratatui::style::Modifier;

    fn palette() -> Palette {
        Palette::from_theme(Theme::Default)
    }

    fn str_cell(s: &str) -> CellSpans {
        vec![Span::raw(s.to_string())]
    }

    fn styled_cell(parts: &[(&str, Style)]) -> CellSpans {
        parts
            .iter()
            .map(|(s, style)| Span::styled(s.to_string(), *style))
            .collect()
    }

    fn make_table(
        headers: &[&str],
        rows: &[&[&str]],
        alignments: &[Alignment],
        source_line: u32,
        row_source_lines: &[u32],
    ) -> TableBlock {
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
            natural_widths[i] = natural_widths[i].max(measure(cell) as usize);
        }
        for row in &r {
            for (i, cell) in row.iter().enumerate() {
                if i < headers.len() {
                    natural_widths[i] = natural_widths[i].max(measure(cell) as usize);
                }
            }
        }
        for w in &mut natural_widths {
            *w = (*w).max(1);
        }
        let rsl: Vec<u32> = if row_source_lines.is_empty() {
            std::iter::once(source_line)
                .chain((source_line + 2..).take(rows.len()))
                .collect()
        } else {
            row_source_lines.to_vec()
        };
        TableBlock {
            id: TableBlockId(0),
            headers: h,
            rows: r,
            alignments: aligns,
            natural_widths,
            rendered_height: 3,
            source_line,
            row_source_lines: rsl,
        }
    }

    // ── Helpers to render table to a string ─────────────────────────────────

    /// Render a table and flatten all lines to a `Vec<String>`.
    fn render_lines(table: &TableBlock, width: u16) -> Vec<String> {
        let (text, _, _) = layout_table(table, width, &palette());
        text.lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    // ── table_fits_naturally ─────────────────────────────────────────────────

    #[test]
    fn table_fits_naturally() {
        let table = make_table(
            &["A", "B", "C"],
            &[&["a1", "b1", "c1"], &["a2", "b2", "c2"]],
            &[],
            0,
            &[],
        );
        let (text, _height, p2s) = layout_table(&table, 80, &palette());
        // No cell exceeds its column width — the table wraps nothing.
        let top = &text.lines[0].spans[0].content;
        assert!(top.contains('┌'), "Top border missing");
        assert_eq!(
            text.lines.len(),
            p2s.len(),
            "physical_to_source length mismatch"
        );
    }

    // ── table_wraps_long_cell ────────────────────────────────────────────────

    #[test]
    fn table_wraps_long_cell() {
        let long_cell = "x".repeat(200);
        let table = make_table(
            &["Short", "Very Long Column Header"],
            &[&["val", long_cell.as_str()]],
            &[],
            0,
            &[],
        );
        let (text, _height, _p2s) = layout_table(&table, 60, &palette());

        // Content must be fully present across wrapped rows — no ellipsis.
        let all_text: String = text
            .lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            !all_text.contains('\u{2026}'),
            "Wrapped table must not contain ellipsis"
        );
        // All 200 x's must appear somewhere.
        let x_count = all_text.chars().filter(|&c| c == 'x').count();
        assert_eq!(
            x_count, 200,
            "All content characters must be present after wrapping"
        );
        // More than 5 lines (the wrapped row occupies multiple physical rows).
        assert!(
            text.lines.len() > 5,
            "Wrapped table should have more lines than a simple 1-row table"
        );
    }

    // ── too_narrow_fallback ──────────────────────────────────────────────────

    #[test]
    fn too_narrow_fallback() {
        let table = make_table(&["A", "B", "C"], &[&["x", "y", "z"]], &[], 0, &[]);
        // 3 cols need min 3*3+3+1 = 13 cols; use 5.
        let (text, height, p2s) = layout_table(&table, 5, &palette());
        assert_eq!(height, 1, "Too-narrow returns exactly 1 line");
        assert_eq!(p2s.len(), 1, "Too-narrow physical_to_source has 1 entry");
        let line: String = text.lines[0]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(line.contains("too narrow"), "Placeholder text missing");
    }

    // ── unicode_width_cells ──────────────────────────────────────────────────

    #[test]
    fn unicode_width_cells() {
        // Japanese characters are double-width.
        let wide_cell = "\u{30A2}\u{30A4}\u{30A6}"; // ア イ ウ — 6 display cols
        let table = make_table(&["JP"], &[&[wide_cell]], &[], 0, &[]);
        // Natural width should be 6.
        assert_eq!(table.natural_widths[0], 6);
        // Render in a terminal wide enough to fit.
        let (text, _h, _p2s) = layout_table(&table, 20, &palette());
        // All wide chars must be present — no ellipsis.
        let all_text: String = text
            .lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            !all_text.contains('\u{2026}'),
            "Wide char table must not contain ellipsis"
        );
        // Render with a very narrow terminal — cells wrap rather than truncate.
        let (text2, _h2, p2s2) = layout_table(&table, 8, &palette());
        let all_text2: String = text2
            .lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(
            !all_text2.contains('\u{2026}'),
            "Narrow wide-char table must wrap, not truncate with ellipsis"
        );
        assert_eq!(
            text2.lines.len(),
            p2s2.len(),
            "physical_to_source length mismatch"
        );
    }

    // ── alignment_respected ──────────────────────────────────────────────────

    #[test]
    fn alignment_respected() {
        let table = make_table(&["Num"], &[&["42"]], &[Alignment::Right], 0, &[]);
        // Natural width of "42" is 2; render wide enough.
        let (text, _h, _trunc) = layout_table(&table, 20, &palette());
        // Data row is text.lines[2+header_height] (top, header sub-rows, sep, data sub-rows, bottom).
        // For a single-line header the header is at index 1, sep at 2, data at 3.
        let data_row: String = text.lines[3]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        let inner = data_row.trim_matches('│');
        let trimmed = inner.trim();
        assert!(
            inner.starts_with(' '),
            "Right-aligned cell must start with space padding: {inner:?}"
        );
        assert!(trimmed == "42", "Cell content must be '42'");
    }

    // ── wrap_table_rows_width_sweep ──────────────────────────────────────────

    #[test]
    fn wrap_table_rows_width_sweep() {
        // 3-col table with one very long cell.
        let long_content = "the quick brown fox jumps over the lazy dog ".repeat(7); // ~300 chars
        let table = make_table(
            &["Col A", "Col B", "Col C"],
            &[&["short", long_content.as_str(), "also short"]],
            &[],
            0,
            &[],
        );

        for width in [40u16, 60, 80, 120, 200] {
            let (text, height, p2s) = layout_table(&table, width, &palette());

            // Every rendered line must be within terminal width.
            for (i, line) in text.lines.iter().enumerate() {
                let line_w: usize = line
                    .spans
                    .iter()
                    .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
                    .sum();
                assert!(
                    line_w <= width as usize,
                    "width {width}: line {i} has width {line_w} > {width}"
                );
            }

            // physical_to_source length must match line count.
            assert_eq!(
                text.lines.len(),
                p2s.len(),
                "width {width}: physical_to_source length mismatch"
            );

            // height return value must equal actual line count.
            assert_eq!(
                height as usize,
                text.lines.len(),
                "width {width}: height mismatch"
            );

            // The long content words must all appear across the table's lines.
            let all_text: String = text
                .lines
                .iter()
                .flat_map(|l| l.spans.iter())
                .map(|s| s.content.as_ref())
                .collect::<String>()
                .to_lowercase();
            assert!(all_text.contains("quick"), "width {width}: 'quick' missing");
            assert!(all_text.contains("lazy"), "width {width}: 'lazy' missing");
            assert!(
                !all_text.contains('\u{2026}'),
                "width {width}: ellipsis must not appear"
            );
        }
    }

    // ── mixed_height_rows_top_aligned ────────────────────────────────────────

    #[test]
    fn mixed_height_rows_top_aligned() {
        // 2-col table: col 0 has a very short cell, col 1 has a long cell that wraps.
        let long = "word ".repeat(20); // ~100 chars
        let table = make_table(
            &["Short", "Long Header"],
            &[&["A", long.trim()]],
            &[],
            0,
            &[],
        );
        let (text, _h, _p2s) = layout_table(&table, 40, &palette());

        // Find the body row sub-lines (after top border + header + separator).
        // We need to check that every sub-row has a `│` in the expected column position.
        // The table border at col 0 is always `│`.
        let body_start = {
            // Skip top border + header sub-rows + separator.
            // Just check that all rendered body sub-rows start with `│`.
            let mut found = false;
            let mut idx = 0;
            for (i, line) in text.lines.iter().enumerate() {
                let s: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                if !found && s.starts_with('├') {
                    idx = i + 1;
                    found = true;
                }
            }
            idx
        };

        // All body sub-rows must start with `│`.
        for line in &text.lines[body_start..text.lines.len().saturating_sub(1)] {
            let s: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            if s.starts_with('└') {
                break;
            }
            assert!(
                s.starts_with('│'),
                "Body sub-row must start with vertical bar: {s:?}"
            );
        }
    }

    // ── header_separator_after_last_header_subrow ────────────────────────────

    #[test]
    fn header_separator_after_last_header_subrow() {
        // Multi-line header: force it to wrap.
        let long_header = "very long column header name that wraps".to_string();
        let table = make_table(&[long_header.as_str(), "B"], &[&["x", "y"]], &[], 0, &[]);
        let (text, _h, _p2s) = layout_table(&table, 30, &palette());

        let lines: Vec<String> = text
            .lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();

        // The top border is first.
        assert!(lines[0].starts_with('┌'), "first line must be top border");
        // Find the separator ├─┼─┤.
        let sep_idx = lines
            .iter()
            .position(|l| l.starts_with('├'))
            .expect("separator must exist");
        // The line immediately before the separator must be a content row (│), not the top border.
        let before_sep: &str = &lines[sep_idx - 1];
        assert!(
            before_sep.starts_with('│'),
            "line before separator must be last header sub-row: {before_sep:?}"
        );
        // No additional ├ after the first one (no inter-body separators).
        let sep_count = lines.iter().filter(|l| l.starts_with('├')).count();
        assert_eq!(sep_count, 1, "only one header separator expected");
    }

    // ── no_inter_body_separators ─────────────────────────────────────────────

    #[test]
    fn no_inter_body_separators() {
        // Three multi-row body rows.
        let long = "alpha beta gamma delta epsilon zeta eta theta";
        let table = make_table(&["Col"], &[&[long], &[long], &[long]], &[], 0, &[]);
        let (text, _h, _p2s) = layout_table(&table, 20, &palette());
        let lines: Vec<String> = text
            .lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();

        // Only one ├─┤ line (header separator) and one └─┘ (bottom border).
        let mid_count = lines.iter().filter(|l| l.starts_with('├')).count();
        let bot_count = lines.iter().filter(|l| l.starts_with('└')).count();
        assert_eq!(
            mid_count, 1,
            "only header separator, no inter-body separators"
        );
        assert_eq!(bot_count, 1, "exactly one bottom border");
    }

    // ── physical_to_source_maps_to_md_row ────────────────────────────────────

    #[test]
    fn physical_to_source_maps_to_md_row() {
        // Three body rows starting at source lines 7, 10, 13.
        // row_source_lines = [header=5, body[0]=7, body[1]=10, body[2]=13]
        let long = "word ".repeat(10);
        let table = make_table(
            &["Col"],
            &[&[long.trim()], &[long.trim()], &[long.trim()]],
            &[],
            5,
            &[5, 7, 10, 13],
        );
        let (text, _h, p2s) = layout_table(&table, 20, &palette());
        let lines: Vec<String> = text
            .lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();

        assert_eq!(
            text.lines.len(),
            p2s.len(),
            "physical_to_source length must equal line count"
        );

        // Top border inherits header source (5).
        assert_eq!(p2s[0], 5, "top border must map to header source line");

        // Find where each section starts.
        let sep_idx = lines.iter().position(|l| l.starts_with('├')).unwrap();
        let bot_idx = lines.iter().position(|l| l.starts_with('└')).unwrap();

        // All header sub-rows map to source 5.
        for (i, &src) in p2s.iter().enumerate().take(sep_idx).skip(1) {
            assert_eq!(src, 5, "header sub-row {i} must map to source 5");
        }
        // Separator maps to header source (5).
        assert_eq!(p2s[sep_idx], 5, "separator must map to header source 5");

        // Find body row boundaries by scanning p2s after sep_idx.
        // body[0] → source 7, body[1] → source 10, body[2] → source 13.
        let body_sources = [7u32, 10, 13];
        let mut cur_source = p2s[sep_idx + 1];
        let mut source_idx = 0usize;
        for (i, &src) in p2s.iter().enumerate().take(bot_idx).skip(sep_idx + 1) {
            if src != cur_source {
                source_idx += 1;
                cur_source = src;
            }
            assert_eq!(
                src, body_sources[source_idx],
                "body physical row {i} maps to wrong source"
            );
        }
        // Bottom border inherits last source (13).
        assert_eq!(
            p2s[bot_idx], 13,
            "bottom border must map to last source line"
        );
    }

    // ── Snapshot tests ────────────────────────────────────────────────────────

    /// Flatten rendered table lines to a single displayable string.
    fn table_to_snapshot_str(table: &TableBlock, width: u16) -> String {
        render_lines(table, width).join("\n")
    }

    #[test]
    fn tbl_2col_short() {
        let table = make_table(
            &["Name", "Value"],
            &[&["Alice", "100"], &["Bob", "200"]],
            &[],
            0,
            &[],
        );
        assert_snapshot!(table_to_snapshot_str(&table, 40));
    }

    #[test]
    fn tbl_2col_long_cell() {
        let long = "The quick brown fox jumps over the lazy dog and then some more words here";
        let table = make_table(&["Label", "Description"], &[&["Item", long]], &[], 0, &[]);
        assert_snapshot!(table_to_snapshot_str(&table, 50));
    }

    #[test]
    fn tbl_5col_mixed() {
        let long1 = "alpha beta gamma delta epsilon zeta eta";
        let long2 = "one two three four five six seven eight nine";
        let table = make_table(
            &["A", "B", "C", "D", "E"],
            &[&["short", long1, "x", long2, "y"]],
            &[],
            0,
            &[],
        );
        assert_snapshot!(table_to_snapshot_str(&table, 60));
    }

    #[test]
    fn tbl_styled_wrap() {
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let italic = Style::default().add_modifier(Modifier::ITALIC);
        let cell = styled_cell(&[
            ("Bold start ", bold),
            ("italic continuation here and more", italic),
        ]);

        let headers = vec![str_cell("Header"), str_cell("Styled")];
        let rows = vec![vec![str_cell("plain"), cell]];
        let mut natural_widths = vec![
            measure(&headers[0]) as usize,
            // Natural width: sum of the styled parts
            "Bold start italic continuation here and more".len(),
        ];
        for w in &mut natural_widths {
            *w = (*w).max(1);
        }
        let table = TableBlock {
            id: TableBlockId(1),
            headers,
            rows,
            alignments: vec![Alignment::None, Alignment::None],
            natural_widths,
            rendered_height: 3,
            source_line: 0,
            row_source_lines: vec![0, 2],
        };
        assert_snapshot!(table_to_snapshot_str(&table, 40));
    }
}
