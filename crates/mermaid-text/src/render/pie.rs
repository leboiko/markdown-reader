//! Renderer for [`PieChart`]. Produces a horizontal bar chart in Unicode.
//!
//! Real Mermaid renders pie charts as circular slices; in monospace text
//! a horizontal bar chart per slice is far more legible than any ASCII
//! pie attempt. Each slice gets its own row:
//!
//! ```text
//! Pet Counts
//!
//! Dogs   ████████████████████████████░░░░░░░░░░░░░░░░░░  79.3%  (386)
//! Cats   ██████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  17.5%  (85)
//! Rats   █░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░   3.1%  (15)
//! ```
//!
//! - Labels are left-padded to the longest label's display width.
//! - Bar width auto-scales to `max_width` (default 80 columns).
//! - Filled cells use `█` (U+2588 FULL BLOCK); unfilled use `░`
//!   (U+2591 LIGHT SHADE) so the row's right edge stays anchored.
//! - Percentages format as `{:>5.1}%` so the column aligns regardless of
//!   value (`100.0%` and `  3.1%` both fit five cells).
//! - When `show_data` is set the raw value follows in parentheses.

use unicode_width::UnicodeWidthStr;

use crate::pie::PieChart;

/// Default canvas width when the caller doesn't provide one. Matches the
/// `render_with_options` default for the other diagram types and reads
/// well in most terminals.
const DEFAULT_WIDTH: usize = 80;
/// Minimum bar column width — even on very narrow terminals we render
/// something rather than collapsing the bar to zero cells.
const MIN_BAR_WIDTH: usize = 10;
/// Gap (in spaces) between adjacent text columns: label↔bar, bar↔pct,
/// pct↔value. Three gaps × 2 spaces = 6 chrome cells.
const GAP: usize = 2;

/// Render a [`PieChart`] as a horizontal bar chart.
///
/// `max_width` caps the total line width; bar columns scale to fit the
/// remaining budget after the label / percentage / value columns. When
/// `None`, defaults to [`DEFAULT_WIDTH`].
pub fn render(chart: &PieChart, max_width: Option<usize>) -> String {
    let budget = max_width.unwrap_or(DEFAULT_WIDTH);
    let total = chart.total();

    // Column widths.
    let label_w = chart
        .slices
        .iter()
        .map(|s| UnicodeWidthStr::width(s.label.as_str()))
        .max()
        .unwrap_or(0);
    let pct_w = 6; // "100.0%"
    // Value column width when show_data is on: `(<value>)` for the largest
    // value (others left-pad to match).
    let value_strs: Vec<String> = if chart.show_data {
        chart
            .slices
            .iter()
            .map(|s| format!("({})", format_value(s.value)))
            .collect()
    } else {
        Vec::new()
    };
    let val_w = value_strs.iter().map(|s| s.len()).max().unwrap_or(0);

    let chrome = label_w + pct_w + GAP * 2 + if val_w > 0 { val_w + GAP } else { 0 };
    let bar_w = budget.saturating_sub(chrome).max(MIN_BAR_WIDTH);
    let row_w = chrome + bar_w;

    let mut out = String::new();

    // Title row (centred over the full row width) followed by a blank.
    if let Some(title) = chart.title.as_deref() {
        let tw = UnicodeWidthStr::width(title);
        let pad = row_w.saturating_sub(tw) / 2;
        out.push_str(&" ".repeat(pad));
        out.push_str(title);
        out.push('\n');
        out.push('\n');
    }

    for (i, slice) in chart.slices.iter().enumerate() {
        let share = if total > 0.0 { slice.value / total } else { 0.0 };
        let filled = (share * bar_w as f64).round() as usize;
        let filled = filled.min(bar_w);
        let unfilled = bar_w - filled;

        // Label, left-padded to label_w.
        let lw = UnicodeWidthStr::width(slice.label.as_str());
        out.push_str(&slice.label);
        out.push_str(&" ".repeat(label_w.saturating_sub(lw)));
        out.push_str(&" ".repeat(GAP));

        // Bar.
        out.push_str(&"█".repeat(filled));
        out.push_str(&"░".repeat(unfilled));
        out.push_str(&" ".repeat(GAP));

        // Percentage (right-aligned in 6 cells).
        out.push_str(&format!("{:>5.1}%", share * 100.0));

        // Value (only when show_data is on).
        if chart.show_data {
            out.push_str(&" ".repeat(GAP));
            let v = &value_strs[i];
            // Right-align values so the closing `)` lines up.
            out.push_str(&" ".repeat(val_w.saturating_sub(v.len())));
            out.push_str(v);
        }

        out.push('\n');
    }

    // Trim the trailing newline so the output matches the convention
    // of other renderers (which don't end with a blank line).
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Format a numeric slice value: integers stay integer-formatted (no
/// `.0`); decimals retain enough precision to be readable. Avoids the
/// awkward `386.0` for a clearly-integer input like `"Dogs" : 386`.
fn format_value(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        // Strip trailing zeros from a 6-decimal format, but keep at
        // least one digit after the decimal point.
        let mut s = format!("{v:.6}");
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.push('0');
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::pie::parse;

    #[test]
    fn renders_minimal() {
        let c = parse("pie\n\"A\" : 1\n\"B\" : 1").unwrap();
        let out = render(&c, Some(60));
        assert!(out.contains('█'));
        assert!(out.contains("50.0%"));
    }

    #[test]
    fn renders_title_centred() {
        let c = parse("pie title Pets\n\"A\" : 1").unwrap();
        let out = render(&c, Some(60));
        assert!(out.contains("Pets"));
    }

    #[test]
    fn show_data_appends_raw_value() {
        let c = parse("pie showData\n\"A\" : 386").unwrap();
        let out = render(&c, Some(80));
        assert!(out.contains("(386)"));
    }

    #[test]
    fn show_data_off_omits_raw_value() {
        let c = parse("pie\n\"A\" : 386").unwrap();
        let out = render(&c, Some(80));
        assert!(!out.contains("(386)"));
    }

    #[test]
    fn format_value_integers_drop_decimal() {
        assert_eq!(format_value(386.0), "386");
        assert_eq!(format_value(0.5), "0.5");
        assert_eq!(format_value(1.25), "1.25");
    }

    #[test]
    fn narrow_terminal_clamps_to_min_bar_width() {
        let c = parse("pie\n\"A\" : 1\n\"B\" : 1").unwrap();
        // Budget of 20 is impossibly tight; expect MIN_BAR_WIDTH bar.
        let out = render(&c, Some(20));
        let bar_count = out.chars().filter(|&c| c == '█' || c == '░').count();
        assert!(bar_count >= MIN_BAR_WIDTH * c.slices.len());
    }
}
