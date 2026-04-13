use crate::app::App;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// Maximum display width for a tab's filename before truncation (cells).
const MAX_NAME_WIDTH: usize = 20;
/// Minimum display width for a truncated filename (cells).
const MIN_NAME_WIDTH: usize = 6;

/// Truncate `name` to at most `MAX_NAME_WIDTH` cells, preserving the extension.
///
/// A file named `very-long-filename.md` becomes `very-lon…md` so the
/// extension stays readable. Names shorter than the limit pass through unchanged.
fn truncate_name(name: &str) -> String {
    if name.len() <= MAX_NAME_WIDTH {
        return name.to_string();
    }

    // Split on the last `.` to isolate the extension.
    if let Some(dot) = name.rfind('.') {
        let ext = &name[dot..]; // e.g. ".md"
        let stem = &name[..dot];
        // We need stem_len + "…" + ext.len() <= MAX_NAME_WIDTH
        // (with a floor of MIN_NAME_WIDTH total).
        let available_stem = MAX_NAME_WIDTH.saturating_sub(1 + ext.len()).max(1);
        if available_stem < MIN_NAME_WIDTH.saturating_sub(ext.len()) {
            // Extension too long — just truncate the whole name.
            let mut s = name[..MAX_NAME_WIDTH.saturating_sub(1)].to_string();
            s.push('…');
            return s;
        }
        let stem_part = &stem[..available_stem.min(stem.len())];
        format!("{stem_part}…{ext}")
    } else {
        let mut s = name[..MAX_NAME_WIDTH.saturating_sub(1)].to_string();
        s.push('…');
        s
    }
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
    let label_widths: Vec<u16> = labels.iter().map(|l| l.len() as u16).collect();
    // +1 per tab for the × close button.
    let widths: Vec<u16> = label_widths.iter().map(|w| w + 1).collect();
    // `+K` overflow indicator occupies at most 5 cells " +32 ".
    const OVERFLOW_MAX: u16 = 5;

    let (start, end) = visible_window(&widths, active_idx, area.width, OVERFLOW_MAX);

    let hidden_before = start;
    let hidden_after = n.saturating_sub(end);

    let mut spans: Vec<Span> = Vec::new();
    let mut x_cursor = area.x;

    // Render each visible tab as a styled span.
    for i in start..end {
        let tab_id = app.tabs.tabs[i].id;
        let label = &labels[i];

        let style = if i == active_idx {
            Style::default()
                .fg(p.selection_fg)
                .bg(p.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(p.dim).bg(p.status_bar_bg)
        };

        spans.push(Span::styled(label.clone(), style));

        let close_style = if i == active_idx {
            Style::default()
                .fg(p.dim)
                .bg(p.accent)
        } else {
            Style::default().fg(p.dim).bg(p.status_bar_bg)
        };
        let close_label = "×";
        let close_w = 1u16;
        spans.push(Span::styled(close_label, close_style));

        let lw = label_widths[i];
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
        // Record the close button rect (× after the label) for close click.
        app.tab_close_rects.push((
            tab_id,
            Rect {
                x: x_cursor + lw,
                y: area.y,
                width: close_w,
                height: 1,
            },
        ));
        x_cursor += lw + close_w;
    }

    // Overflow indicators.
    if hidden_before > 0 {
        // Rare case where active is near the right and window slides; prepend indicator.
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
