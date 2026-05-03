//! Renderer for [`XyChart`]. Produces a Unicode bar/line chart with a
//! labeled y-axis and categorical or numeric x-axis tick marks.
//!
//! ## Layout (example with bar + line)
//!
//! ```text
//! Sales Revenue
//!
//! Revenue (in $)
//! 11000 ┤         ██
//! 10000 ┤      ██ ██ ██
//!  9000 ┤   ██ ██ ██ ██ ██
//!  8000 ┤   ██ ██ ██ ██ ██
//!  7000 ┤██ ██ ██ ██ ██ ██ ██
//!  6000 ┤██ ██ ██ ██ ██ ██ ██ ██
//!  5000 ┤██ ██ ██ ██ ██ ██ ██ ██ ██
//!  4000 ┤
//!        └─┬───┬───┬───┬───┬───┬─
//!          jan feb mar apr may jun
//! ```
//!
//! Bar series uses `█` block columns. Line series plots points connected
//! with `╭─╯╰` curve glyphs on top of (or instead of) bar columns.
//! Bars are drawn first; if both series are present the line is overlaid.
//!
//! ## max_width
//!
//! When `max_width` is `Some(n)`, the column width is clamped to that budget.
//! The y-axis label column is sized to the widest tick label.
//!
//! ## Phase 1 limitations
//!
//! - Horizontal orientation is not rendered (always rendered vertically).
//! - Series length mismatches (caught by the parser) are not checked again
//!   here — the parser is the single point of validation.
//! - For numeric x-axes the tick marks show the range endpoints only.

use unicode_width::UnicodeWidthStr;

use crate::xy_chart::{XAxis, XyChart};

/// Default canvas width (columns) when no `max_width` is given.
const DEFAULT_WIDTH: usize = 72;

/// Minimum canvas width; below this the chart is unreadable.
const MIN_WIDTH: usize = 30;

/// Number of rows in the chart body (y-axis rows, not counting title / labels).
const DEFAULT_CHART_ROWS: usize = 20;

/// The bar column glyph (full block).
const BAR_GLYPH: char = '\u{2588}'; // █

/// Render an [`XyChart`] to a Unicode string.
///
/// # Arguments
///
/// * `diag`      — the parsed diagram
/// * `max_width` — optional column budget; the canvas is sized to fit within
///   this budget (minimum [`MIN_WIDTH`] columns)
///
/// # Returns
///
/// A multi-line string ready for printing.
pub fn render(diag: &XyChart, max_width: Option<usize>) -> String {
    let canvas_width = max_width.map(|w| w.max(MIN_WIDTH)).unwrap_or(DEFAULT_WIDTH);

    let mut out = String::new();

    // ---- Title -------------------------------------------------------------
    if let Some(title) = &diag.title {
        out.push_str(title);
        out.push('\n');
        out.push('\n');
    }

    // ---- Y-axis label (above chart body) -----------------------------------
    if let Some(label) = &diag.y_axis.label {
        out.push_str(label);
        out.push('\n');
    }

    // ---- Compute layout dimensions -----------------------------------------

    // Build tick labels for y-axis. We generate up to DEFAULT_CHART_ROWS
    // evenly-spaced ticks from max down to min (top-to-bottom).
    let y_min = diag.y_axis.min;
    let y_max = diag.y_axis.max;
    let chart_rows = DEFAULT_CHART_ROWS;

    // Width of the widest y-axis tick label (used for alignment).
    let y_label_width = compute_y_label_width(y_min, y_max, chart_rows);

    // Each data column occupies `col_width` characters (must be at least 2
    // to fit `██`). We compute how many data columns we can show.
    let n_data = count_data_points(diag);

    // Usable width = canvas_width - y_label_width - 2 (for `┤ ` prefix).
    // Each column takes col_width chars.
    let axis_prefix_width = y_label_width + 2; // "9999 ┤"
    let usable_width = canvas_width.saturating_sub(axis_prefix_width);

    // Column width: try to fit all data, minimum 2.
    let col_width = if n_data == 0 || usable_width == 0 {
        3
    } else {
        (usable_width / n_data).max(2)
    };

    // ---- Build the chart grid ----------------------------------------------

    // We render the chart into a Vec<String> of rows (top = row 0 = y_max).
    // Each row corresponds to a y-value range bucket.
    // We use a 2-D character canvas: canvas[row][col].
    let total_cols = usable_width.min(n_data * col_width);
    let mut canvas: Vec<Vec<char>> = vec![vec![' '; total_cols + 1]; chart_rows + 1];

    // Draw bar series first.
    if !diag.bar_series.is_empty() {
        draw_bars(
            &mut canvas,
            &diag.bar_series,
            y_min,
            y_max,
            col_width,
            chart_rows,
        );
    }

    // Draw line series on top.
    if !diag.line_series.is_empty() {
        draw_line(
            &mut canvas,
            &diag.line_series,
            y_min,
            y_max,
            col_width,
            chart_rows,
        );
    }

    // ---- Emit chart rows with y-axis ticks ---------------------------------
    for (row_idx, row_chars) in canvas.iter().enumerate().take(chart_rows + 1) {
        // y value at this row's top edge (row 0 = top = y_max, row chart_rows = y_min).
        let y_val = y_max - (row_idx as f64 / chart_rows as f64) * (y_max - y_min);

        // Only label rows that correspond to round tick values.
        let tick_label = if row_idx == 0 {
            format_tick(y_max)
        } else if row_idx == chart_rows {
            format_tick(y_min)
        } else {
            // Label every (chart_rows / 4) or so rows; suppress others.
            let step = chart_rows / 4;
            if step > 0 && row_idx % step == 0 {
                format_tick(y_val)
            } else {
                String::new()
            }
        };

        // Right-align the tick label within y_label_width.
        let label_str = if tick_label.is_empty() {
            " ".repeat(y_label_width)
        } else {
            let w = tick_label.len();
            format!(
                "{}{}",
                " ".repeat(y_label_width.saturating_sub(w)),
                tick_label
            )
        };

        // All rows in the chart body use `┤`.
        let axis_glyph = '\u{2524}'; // ┤

        let row_str: String = row_chars.iter().collect();
        let row_trimmed = row_str.trim_end();

        out.push_str(&label_str);
        out.push(' ');
        out.push(axis_glyph);
        out.push_str(row_trimmed);
        out.push('\n');
    }

    // ---- X-axis bottom bar -------------------------------------------------
    // Format:  "<pad>└─┬───┬─..."
    let pad = " ".repeat(y_label_width + 1); // align with ┤
    out.push_str(&pad);
    out.push('\u{2514}'); // └

    if n_data > 0 {
        for col in 0..n_data {
            // Each column: "─┬" with (col_width - 2) dashes between tick marks.
            let dashes = "\u{2500}".repeat(col_width.saturating_sub(1));
            out.push_str(&dashes);
            if col + 1 < n_data {
                out.push('\u{252C}'); // ┬
            }
        }
        out.push('\u{2500}'); // trailing dash
    }
    out.push('\n');

    // ---- X-axis tick labels ------------------------------------------------
    let x_labels = build_x_labels(diag, n_data);
    if !x_labels.is_empty() {
        let label_pad = " ".repeat(y_label_width + 2); // align under axis body
        out.push_str(&label_pad);
        for (i, label) in x_labels.iter().enumerate() {
            let lw = UnicodeWidthStr::width(label.as_str());
            let left_pad = col_width.saturating_sub(lw) / 2;
            let right_pad = col_width.saturating_sub(lw).saturating_sub(left_pad);
            out.push_str(&" ".repeat(left_pad));
            out.push_str(label);
            if i + 1 < x_labels.len() {
                out.push_str(&" ".repeat(right_pad));
            }
        }
        out.push('\n');
    }

    // Trim trailing newlines.
    while out.ends_with('\n') {
        out.pop();
    }

    out
}

/// Compute the width of the widest y-axis tick label.
fn compute_y_label_width(y_min: f64, y_max: f64, chart_rows: usize) -> usize {
    let mut max_w = 0;
    let candidates = [
        format_tick(y_min),
        format_tick(y_max),
        format_tick((y_min + y_max) / 2.0),
    ];
    for c in &candidates {
        max_w = max_w.max(c.len());
    }
    // Also check the row-step ticks.
    let step = chart_rows / 4;
    if step > 0 {
        for i in (0..=chart_rows).step_by(step) {
            let y = y_max - (i as f64 / chart_rows as f64) * (y_max - y_min);
            let w = format_tick(y).len();
            max_w = max_w.max(w);
        }
    }
    max_w
}

/// Format a y-axis tick value.
///
/// Uses integer formatting when the value is a whole number, otherwise
/// one decimal place.
fn format_tick(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{:.1}", v)
    }
}

/// Count the number of data points in the chart (x-axis category count or
/// series length, whichever is non-zero).
fn count_data_points(diag: &XyChart) -> usize {
    if !diag.bar_series.is_empty() {
        return diag.bar_series.len();
    }
    if !diag.line_series.is_empty() {
        return diag.line_series.len();
    }
    match &diag.x_axis {
        XAxis::Categorical { labels } => labels.len(),
        XAxis::Numeric { .. } => 0,
    }
}

/// Map a data value to a canvas row index (0 = top = y_max, chart_rows = y_min).
///
/// Values outside [y_min, y_max] are clamped.
fn value_to_row(value: f64, y_min: f64, y_max: f64, chart_rows: usize) -> usize {
    if y_max <= y_min {
        return chart_rows;
    }
    let frac = (y_max - value.clamp(y_min, y_max)) / (y_max - y_min);
    (frac * chart_rows as f64).round() as usize
}

/// Draw bar columns into the canvas.
///
/// Each bar occupies `col_width` characters. A bar for value `v` fills all
/// rows from the row corresponding to `v` down to the baseline row (y_min).
fn draw_bars(
    canvas: &mut [Vec<char>],
    values: &[f64],
    y_min: f64,
    y_max: f64,
    col_width: usize,
    chart_rows: usize,
) {
    let baseline_row = chart_rows; // row index at y_min
    let canvas_cols = canvas[0].len();

    for (i, &val) in values.iter().enumerate() {
        let top_row = value_to_row(val, y_min, y_max, chart_rows);
        let start_col = i * col_width;

        // Fill rows from top_row..baseline_row with BAR_GLYPH.
        // Leave one character gap between columns (right edge is a space).
        let bar_chars = col_width.saturating_sub(1); // leave 1 space gap

        for row in top_row..baseline_row {
            for c in 0..bar_chars {
                let col = start_col + c;
                if col < canvas_cols && row < canvas.len() {
                    canvas[row][col] = BAR_GLYPH;
                }
            }
        }
    }
}

/// Draw a line series into the canvas using curve glyphs.
///
/// The line is drawn as a series of connected segments. At each column the
/// data point occupies one cell; the transition between adjacent points uses
/// `╭─╯` (rise) or `╰─╮` (fall) style glyphs.
fn draw_line(
    canvas: &mut [Vec<char>],
    values: &[f64],
    y_min: f64,
    y_max: f64,
    col_width: usize,
    chart_rows: usize,
) {
    let canvas_cols = canvas[0].len();
    let n = values.len();

    // Compute the row for each data point.
    let rows: Vec<usize> = values
        .iter()
        .map(|&v| value_to_row(v, y_min, y_max, chart_rows))
        .collect();

    // Two-pass: connectors first, markers last. The previous order
    // (marker-then-segment) was visually correct for descending lines
    // (the segment's bottom corner landed at `(r1, c1)` which the next
    // iteration's marker overwrote anyway) but BROKE rising lines: the
    // segment's bottom corner `╯` lands at `(r0, c0)` — exactly where
    // the just-drawn marker sat — so every ascending peak lost its
    // `●`. Drawing all connectors first, then overlaying markers on top,
    // guarantees the `●` is preserved at every data point regardless of
    // which side of the peak it's on. See the regression guard
    // `xy_chart_line_has_marker_per_data_point` in tests/snapshots.rs.
    for i in 0..n.saturating_sub(1) {
        let row = rows[i];
        let center_col = i * col_width + col_width / 2;
        let next_row = rows[i + 1];
        let next_center_col = (i + 1) * col_width + col_width / 2;
        draw_segment(
            canvas,
            row,
            center_col,
            next_row,
            next_center_col,
            chart_rows,
        );
    }

    for (i, &row) in rows.iter().enumerate() {
        let center_col = i * col_width + col_width / 2;
        if row < canvas.len() && center_col < canvas_cols {
            canvas[row][center_col] = '\u{25CF}'; // ●
        }
    }
}

/// Draw a connecting segment from (r0, c0) to (r1, c1) using box-drawing glyphs.
///
/// Horizontal segments use `─`; the rise/fall transitions use:
/// - Going up: `╭` at the bend from horizontal to vertical, `╯` at destination.
/// - Going down: `╰` at start, `╮` at destination, with `│` in between.
/// - Same row: straight `─` through.
fn draw_segment(
    canvas: &mut [Vec<char>],
    r0: usize,
    c0: usize,
    r1: usize,
    c1: usize,
    chart_rows: usize,
) {
    let canvas_rows = canvas.len();
    let canvas_cols = if canvas_rows > 0 { canvas[0].len() } else { 0 };

    // Draw horizontal dashes between the two column centres.
    let dash_start = c0 + 1;
    let dash_end = c1.saturating_sub(1);
    let mid_row = if r0 <= r1 { r0 } else { r1 }; // draw dashes at the higher row

    if mid_row < canvas_rows {
        let dash_end_clamped = dash_end.min(canvas_cols.saturating_sub(1));
        if dash_start <= dash_end_clamped {
            for cell in &mut canvas[mid_row][dash_start..=dash_end_clamped] {
                if *cell == ' ' {
                    *cell = '\u{2500}'; // ─
                }
            }
        }
    }

    // Draw vertical connectors when the rows differ.
    if r0 != r1 {
        let (top_row, bottom_row) = if r0 < r1 { (r0, r1) } else { (r1, r0) };
        let vert_col = if r0 < r1 { c1 } else { c0 };

        // Corner glyphs.
        let (top_corner, bot_corner) = if r0 < r1 {
            // Going down from left to right: ╮ at top-right, ╰ at bottom-left.
            ('\u{256E}', '\u{2570}') // ╮ ╰
        } else {
            // Going up from left to right: ╯ at destination (r1, c1), ╭ at (r0, c0).
            ('\u{256D}', '\u{256F}') // ╭ ╯
        };

        // Place corner at top.
        if top_row < canvas_rows && vert_col < canvas_cols {
            canvas[top_row][vert_col] = top_corner;
        }

        // Vertical bar between rows.
        for row in canvas.iter_mut().take(bottom_row).skip(top_row + 1) {
            if vert_col < row.len() && row[vert_col] == ' ' {
                row[vert_col] = '\u{2502}'; // │
            }
        }

        // Place corner at bottom.
        if bottom_row < canvas_rows && vert_col < canvas_cols {
            canvas[bottom_row][vert_col] = bot_corner;
        }

        let _ = (top_corner, bot_corner, chart_rows);
    }
}

/// Build the list of x-axis labels to display.
///
/// For categorical axes, returns the category labels.
/// For numeric axes, returns min and max only.
fn build_x_labels(diag: &XyChart, n_data: usize) -> Vec<String> {
    match &diag.x_axis {
        XAxis::Categorical { labels } => labels.clone(),
        XAxis::Numeric { min, max, .. } => {
            if n_data == 0 {
                return Vec::new();
            }
            let mut out = vec![String::new(); n_data];
            out[0] = format_tick(*min);
            if n_data > 1 {
                out[n_data - 1] = format_tick(*max);
            }
            out
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::xy_chart::parse;

    fn canonical_src() -> &'static str {
        "xychart-beta
    title \"Sales Revenue\"
    x-axis [jan, feb, mar, apr, may, jun, jul, aug, sep, oct, nov, dec]
    y-axis \"Revenue (in $)\" 4000 --> 11000
    bar [5000, 6000, 7500, 8200, 9500, 10500, 11000, 10200, 9200, 8500, 7000, 6000]
    line [5000, 6000, 7500, 8200, 9500, 10500, 11000, 10200, 9200, 8500, 7000, 6000]"
    }

    #[test]
    fn title_appears_in_output() {
        let chart = parse(canonical_src()).unwrap();
        let out = render(&chart, None);
        assert!(
            out.contains("Sales Revenue"),
            "title missing from output:\n{out}"
        );
    }

    #[test]
    fn y_axis_label_appears_in_output() {
        let chart = parse(canonical_src()).unwrap();
        let out = render(&chart, None);
        assert!(
            out.contains("Revenue (in $)"),
            "y-axis label missing:\n{out}"
        );
    }

    #[test]
    fn x_axis_labels_appear_in_output() {
        let chart = parse(canonical_src()).unwrap();
        let out = render(&chart, None);
        assert!(out.contains("jan"), "jan label missing:\n{out}");
        assert!(out.contains("dec"), "dec label missing:\n{out}");
    }

    #[test]
    fn bar_glyphs_present_when_bar_series_set() {
        let chart = parse(canonical_src()).unwrap();
        let out = render(&chart, None);
        assert!(
            out.contains(BAR_GLYPH),
            "bar glyph missing from output:\n{out}"
        );
    }

    #[test]
    fn empty_chart_renders_without_panic() {
        let src = "xychart-beta\n    y-axis 0 --> 100";
        let chart = parse(src).unwrap();
        let out = render(&chart, Some(80));
        // Must produce a non-empty string with axis glyphs.
        assert!(!out.is_empty());
        assert!(out.contains('\u{2524}') || out.contains('\u{2514}'));
    }

    #[test]
    fn max_width_is_honoured() {
        let chart = parse(canonical_src()).unwrap();
        let out = render(&chart, Some(60));
        // No single line should exceed 60 chars (approximately — labels may
        // slightly overflow, but the chart body is constrained).
        let longest = out.lines().map(|l| l.chars().count()).max().unwrap_or(0);
        assert!(
            longest <= 80, // some tolerance for label overflow
            "line too long ({longest} chars) with max_width=60:\n{out}"
        );
    }

    #[test]
    fn line_only_chart_renders_without_bar_glyphs() {
        let src = "xychart-beta\n    x-axis [a, b, c]\n    y-axis 0 --> 10\n    line [3, 7, 5]";
        let chart = parse(src).unwrap();
        let out = render(&chart, None);
        assert!(
            !out.contains(BAR_GLYPH),
            "bar glyph should not appear when only line series is set:\n{out}"
        );
    }

    #[test]
    fn y_axis_tick_values_appear_in_output() {
        let src = "xychart-beta\n    y-axis 0 --> 100\n    bar [50, 80, 30]";
        let chart = parse(src).unwrap();
        let out = render(&chart, None);
        assert!(out.contains("100"), "y_max tick missing:\n{out}");
        assert!(out.contains("0"), "y_min tick missing:\n{out}");
    }

    #[test]
    fn x_axis_same_width_labels_have_uniform_spacing() {
        // Regression: same-width labels (e.g. 3-letter month names) used to drift
        // left as you moved right because the loop only emitted left_pad on i==0.
        for &(n, label_len) in &[(3usize, 3), (5, 3), (8, 3), (12, 3), (15, 1)] {
            let labels: Vec<String> = (0..n)
                .map(|i| {
                    let base = (b'a' + (i % 26) as u8) as char;
                    std::iter::repeat_n(base, label_len).collect()
                })
                .collect();
            let series: Vec<String> = (0..n).map(|i| ((i + 1) * 10).to_string()).collect();
            let src = format!(
                "xychart-beta\n    x-axis [{}]\n    y-axis 0 --> 200\n    bar [{}]",
                labels.join(", "),
                series.join(", "),
            );
            let chart = parse(&src).unwrap();
            let out = render(&chart, None);
            let label_line = out.lines().last().expect("last line should be labels");

            let positions: Vec<usize> = labels
                .iter()
                .scan(0usize, |start, label| {
                    let pos = label_line[*start..].find(label.as_str())? + *start;
                    *start = pos + label.len();
                    Some(pos)
                })
                .collect();
            assert_eq!(
                positions.len(),
                n,
                "missing labels for n={n} label_len={label_len}:\n{out}"
            );

            if positions.len() >= 3 {
                let gaps: Vec<usize> = positions.windows(2).map(|w| w[1] - w[0]).collect();
                let first = gaps[0];
                assert!(
                    gaps.iter().all(|g| *g == first),
                    "label gaps drift for n={n} label_len={label_len}: {gaps:?}\n{out}"
                );
            }
        }
    }
}
