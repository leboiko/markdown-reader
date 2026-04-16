use crate::app::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr as _;

/// Maximum display width for a tab's filename before truncation (cells).
const MAX_NAME_WIDTH: usize = 20;
/// Minimum display width for a truncated filename (cells).
const MIN_NAME_WIDTH: usize = 6;
/// Display width (cells) of the close button span " × ".
const CLOSE_WIDTH: u16 = 3;

/// Truncate `name` to at most `MAX_NAME_WIDTH` display cells, preserving the extension.
///
/// A file named `very-long-filename.md` becomes `very-lon…md` so the
/// extension stays readable. Names shorter than the limit pass through unchanged.
///
/// This function measures widths using `unicode_width` so that multi-byte characters
/// (e.g. CJK) count as their rendered cell width rather than their byte length.
fn truncate_name(name: &str) -> String {
    // Build the truncated string by accumulating characters until we hit the limit.
    // This avoids byte-indexing into a UTF-8 string, which would panic on multi-byte chars.
    let display_width = name.width();
    if display_width <= MAX_NAME_WIDTH {
        return name.to_string();
    }

    // Split on the last `.` to isolate the extension.
    if let Some(dot) = name.rfind('.') {
        let ext = &name[dot..]; // e.g. ".md"
        let stem = &name[..dot];
        let ext_w = ext.width();
        // We need stem_cells + 1 (for "…") + ext_cells <= MAX_NAME_WIDTH.
        let available_stem = MAX_NAME_WIDTH.saturating_sub(1 + ext_w).max(1);
        if available_stem < MIN_NAME_WIDTH.saturating_sub(ext_w) {
            // Extension too long — truncate the whole name instead.
            return collect_cells(name, MAX_NAME_WIDTH.saturating_sub(1)) + "…";
        }
        let stem_part = collect_cells(stem, available_stem);
        format!("{stem_part}…{ext}")
    } else {
        collect_cells(name, MAX_NAME_WIDTH.saturating_sub(1)) + "…"
    }
}

/// Return a prefix of `s` whose total display-cell width is at most `limit`.
///
/// Characters are accumulated left-to-right; the first character that would
/// push the total past `limit` is excluded.
fn collect_cells(s: &str, limit: usize) -> String {
    use unicode_width::UnicodeWidthChar as _;
    let mut out = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > limit {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

/// Compute the display-cell width of a label string produced by `format!(" {num}: {name} ")`.
///
/// Uses `unicode_width` so that multi-byte or wide characters count correctly.
fn label_display_width(label: &str) -> u16 {
    label.width() as u16
}

/// Render the tab bar into `area`.
///
/// Writes per-tab rects into `app.tab_bar_rects` for mouse hit-testing.
/// Does nothing if there are no open tabs.
pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    app.tab_bar_rects.clear();

    if app.tabs.is_empty() {
        return;
    }

    let p = &app.palette;
    let n = app.tabs.len();
    let active_idx = app.tabs.active_index().unwrap_or(0);

    // Build label strings for each tab.  Each renders as ` N: name ` where N
    // is the 1-based index (hidden past 9 — shown as space).
    let labels: Vec<String> = app
        .tabs
        .tabs
        .iter()
        .enumerate()
        .map(|(i, tab)| {
            let num = if i < 9 {
                format!("{}", i + 1)
            } else {
                " ".to_string()
            };
            let name = truncate_name(&tab.view.file_name);
            format!(" {num}: {name} ")
        })
        .collect();

    // Determine the window of tabs that fits in `area.width`, keeping the
    // active tab visible.
    //
    // Use unicode_width (cell count) not byte length for label widths so that
    // filenames with multi-byte characters (e.g. CJK, …) are measured correctly.
    let label_widths: Vec<u16> = labels.iter().map(|l| label_display_width(l)).collect();
    // CLOSE_WIDTH cells per tab for the " × " close button.
    let widths: Vec<u16> = label_widths.iter().map(|w| w + CLOSE_WIDTH).collect();
    // `+K` overflow indicator occupies at most 5 cells " +32 ".
    const OVERFLOW_MAX: u16 = 5;

    let (start, end) = visible_window(&widths, active_idx, area.width, OVERFLOW_MAX);

    let hidden_before = start;
    let hidden_after = n.saturating_sub(end);

    // When a " +N " overflow indicator is prepended, it occupies cells to the
    // left of the first tab.  We must account for its width in x_cursor so that
    // the hit-test rects stay aligned with what ratatui actually renders.
    let overflow_before_w: u16 = if hidden_before > 0 {
        format!(" +{hidden_before} ").width() as u16
    } else {
        0
    };

    let mut spans: Vec<Span> = Vec::new();
    // x_cursor tracks the next free column in terminal coordinates.
    let mut x_cursor = area.x + overflow_before_w;

    // Render each visible tab as a styled span.
    for i in start..end {
        let tab_id = app.tabs.tabs[i].id;
        let label = &labels[i];
        let lw = label_widths[i];

        let style = if i == active_idx {
            Style::default()
                .fg(p.on_accent_fg)
                .bg(p.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.dim).bg(p.status_bar_bg)
        };

        let close_style = if i == active_idx {
            Style::default().fg(p.dim).bg(p.accent)
        } else {
            Style::default().fg(p.dim).bg(p.status_bar_bg)
        };

        // The close button is " × " (space, multiplication sign, space) so that
        // the click target is CLOSE_WIDTH (3) cells wide instead of just 1.
        // A 1-cell target is nearly impossible to hit with a mouse cursor.
        let close_label = " × ";

        spans.push(Span::styled(label.clone(), style));
        spans.push(Span::styled(close_label, close_style));

        // Record the tab rect (label only) for activate click.
        app.tab_bar_rects.push((
            tab_id,
            Rect {
                x: x_cursor,
                y: area.y,
                width: lw,
                height: 1,
            },
        ));
        // Record the close button rect (" × ") for close click.
        // The rect starts immediately after the label and is CLOSE_WIDTH cells wide.
        app.tab_close_rects.push((
            tab_id,
            Rect {
                x: x_cursor + lw,
                y: area.y,
                width: CLOSE_WIDTH,
                height: 1,
            },
        ));
        x_cursor += lw + CLOSE_WIDTH;
    }

    // Overflow indicators — built as separate spans so spans are ordered left-to-right.
    if hidden_before > 0 {
        // Prepend the " +N " indicator.  x_cursor was already offset by
        // overflow_before_w above, so the tab rects recorded in the loop above
        // are already at the correct screen positions.
        spans.insert(
            0,
            Span::styled(
                format!(" +{hidden_before} "),
                Style::default().fg(p.accent_alt).bg(p.status_bar_bg),
            ),
        );
    }
    if hidden_after > 0 {
        spans.push(Span::styled(
            format!(" +{hidden_after} "),
            Style::default().fg(p.accent_alt).bg(p.status_bar_bg),
        ));
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line).style(Style::default().bg(p.status_bar_bg));
    f.render_widget(paragraph, area);
}

/// Compute the `[start, end)` index range of tabs that fits within `available_width`,
/// guaranteeing the active tab is always included.
///
/// The algorithm greedily expands left then right from `active_idx`, reserving
/// `overflow_reserve` cells for any `+N` indicator that must be shown on the
/// side(s) where tabs are hidden.
fn visible_window(
    widths: &[u16],
    active_idx: usize,
    available_width: u16,
    overflow_reserve: u16,
) -> (usize, usize) {
    let n = widths.len();
    if n == 0 {
        return (0, 0);
    }

    let mut start = active_idx;
    let mut end = active_idx + 1;
    let mut used: u16 = widths[active_idx];

    // Alternately try to expand left and right until nothing more fits.
    loop {
        let mut expanded = false;

        // Try left.
        if start > 0 {
            let extra = widths[start - 1];
            let reserve_l = if start > 1 { overflow_reserve } else { 0 };
            let reserve_r = if end < n { overflow_reserve } else { 0 };
            if used
                .saturating_add(extra)
                .saturating_add(reserve_l)
                .saturating_add(reserve_r)
                <= available_width
            {
                start -= 1;
                used = used.saturating_add(extra);
                expanded = true;
            }
        }

        // Try right.
        if end < n {
            let extra = widths[end];
            let reserve_l = if start > 0 { overflow_reserve } else { 0 };
            let reserve_r = if end + 1 < n { overflow_reserve } else { 0 };
            if used
                .saturating_add(extra)
                .saturating_add(reserve_l)
                .saturating_add(reserve_r)
                <= available_width
            {
                end += 1;
                used = used.saturating_add(extra);
                expanded = true;
            }
        }

        if !expanded {
            break;
        }
    }

    (start, end)
}
