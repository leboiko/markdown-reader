//! Renderer for [`Timeline`]. Produces a vertical-flow timeline string using
//! Unicode line-drawing characters.
//!
//! **Layout** — a bullet-on-a-wire style where each time period hangs off a
//! vertical spine with a filled circle (`●`) marker:
//!
//! ```text
//! Timeline: History of Social Media
//!
//! ── 2002-2004 ────────────────────────────────────
//!   2002 ●── LinkedIn
//!   2003 ●── MySpace launched
//!   2004 ●── Facebook
//!            └── Google goes public
//!
//! ── 2005-2008 ────────────────────────────────────
//!   2005 ●── YouTube
//!   2006 ●── Twitter
//!   2007 ●── iPhone
//!            └── Tumblr
//! ```
//!
//! Section headers are rendered as horizontal rules. Periods are
//! right-padded within each section so the bullet column aligns. Additional
//! events beyond the first hang below the period row with a `└──` connector.
//!
//! **max_width**: event text that would push a line past the column budget is
//! truncated with `…` (U+2026 HORIZONTAL ELLIPSIS). Period labels are never
//! truncated (they are opaque keys). Lines longer than `max_width` due to a
//! very long period label are emitted as-is; the caller may scroll or clip.

use unicode_width::UnicodeWidthStr;

use crate::timeline::{Timeline, TimelineSection};

// The indent before a period label (two spaces).
const INDENT: &str = "  ";
// Connector from period to first event.
const BULLET_CONN: &str = " \u{25cf}\u{2500}\u{2500} "; // " ●── "
// Connector for subsequent events below the first.
const CONT_CONN: &str = " \u{2514}\u{2500}\u{2500} "; // " └── "
// Minimum section-rule length (dashes on the right of the section name).
const SECTION_RULE_MIN: usize = 4;
// Total target width for the section rule line (before max_width clipping).
const SECTION_RULE_TARGET: usize = 50;

/// Render a [`Timeline`] to a Unicode string.
///
/// # Arguments
///
/// * `diag`      — the parsed diagram
/// * `max_width` — optional column budget; event text is truncated with `…`
///   when a rendered line would exceed this many terminal cells
///
/// # Returns
///
/// A multi-line string ready for printing. Sections are separated by blank
/// lines; the optional title appears first prefixed with `Timeline: `.
pub fn render(diag: &Timeline, max_width: Option<usize>) -> String {
    let mut out = String::new();

    if let Some(title) = diag.title.as_deref() {
        out.push_str("Timeline: ");
        out.push_str(title);
        out.push('\n');
    }

    for section in &diag.sections {
        out.push('\n');
        render_section(&mut out, section, max_width);
    }

    // Trim the trailing newline to match other renderers.
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Render one section (header + entries) into `out`.
fn render_section(out: &mut String, section: &TimelineSection, max_width: Option<usize>) {
    // Section header — a horizontal rule with the name embedded.
    if let Some(name) = section.name.as_deref() {
        out.push_str("\u{2500}\u{2500} "); // "── "
        out.push_str(name);
        out.push(' ');
        // Fill remaining columns with dashes up to SECTION_RULE_TARGET.
        let used = 3 + UnicodeWidthStr::width(name) + 1;
        let dashes = SECTION_RULE_TARGET
            .saturating_sub(used)
            .max(SECTION_RULE_MIN);
        for _ in 0..dashes {
            out.push('\u{2500}'); // '─'
        }
        out.push('\n');
    }

    // Compute the display width of the widest period label in this section so
    // the bullet column (`●`) aligns across all rows.
    let period_col = section
        .entries
        .iter()
        .map(|e| UnicodeWidthStr::width(e.period.as_str()))
        .max()
        .unwrap_or(0);

    // Width of the fixed prefix before event text on a period row:
    //   INDENT + period (padded) + BULLET_CONN
    // BULLET_CONN is " ●── " — 5 display cells (space + 3-byte ● + 3×─ + space).
    // We measure it via width() for correctness across builds.
    let bullet_conn_w = UnicodeWidthStr::width(BULLET_CONN);
    let prefix_w = INDENT.len() + period_col + bullet_conn_w;

    // Width of the continuation connector prefix (aligns events under BULLET_CONN).
    let cont_conn_w = UnicodeWidthStr::width(CONT_CONN);
    // Pad the continuation line so the event text starts at the same column as
    // the first event: INDENT + period_col + cont_conn_w == prefix_w in most
    // fonts (CONT_CONN and BULLET_CONN are intentionally the same display width).
    let cont_pad_w = (INDENT.len() + period_col).saturating_sub(0);

    for entry in &section.entries {
        let period_w = UnicodeWidthStr::width(entry.period.as_str());
        let pad = period_col.saturating_sub(period_w);

        // First event (or placeholder when the entry has no events).
        let first_event = entry.events.first().map(String::as_str).unwrap_or("");
        let first_truncated = maybe_truncate(first_event, max_width, prefix_w);

        out.push_str(INDENT);
        out.push_str(&entry.period);
        for _ in 0..pad {
            out.push(' ');
        }
        out.push_str(BULLET_CONN);
        out.push_str(&first_truncated);
        out.push('\n');

        // Subsequent events hang below the period row.
        for event in entry.events.iter().skip(1) {
            let continuation_prefix_w = cont_pad_w + cont_conn_w;
            let event_truncated = maybe_truncate(event, max_width, continuation_prefix_w);

            // Pad so the connector sits directly beneath BULLET_CONN.
            for _ in 0..INDENT.len() + period_col {
                out.push(' ');
            }
            out.push_str(CONT_CONN);
            out.push_str(&event_truncated);
            out.push('\n');
        }
    }
}

/// Truncate `text` with `…` if emitting it after a prefix of `prefix_cols`
/// display cells would exceed `max_width`. Returns the (possibly truncated)
/// string. When `max_width` is `None` or the text fits, returns `text` as-is
/// (no allocation needed — callers receive a `&str` slice).
fn maybe_truncate(text: &str, max_width: Option<usize>, prefix_cols: usize) -> String {
    let Some(budget) = max_width else {
        return text.to_string();
    };
    let available = budget.saturating_sub(prefix_cols);
    let text_w = UnicodeWidthStr::width(text);
    if text_w <= available {
        return text.to_string();
    }
    // Reserve 1 cell for the `…` character.
    let target = available.saturating_sub(1);
    let mut result = String::with_capacity(target * 3 + 3);
    let mut used = 0;
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
        if used + w > target {
            break;
        }
        result.push(ch);
        used += w;
    }
    result.push('\u{2026}'); // HORIZONTAL ELLIPSIS
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::timeline::parse;

    #[test]
    fn renders_title_when_present() {
        let diag = parse(
            "timeline\n\
             title History of Social Media\n\
             2002 : LinkedIn",
        )
        .unwrap();
        let out = render(&diag, None);
        assert!(
            out.starts_with("Timeline: History of Social Media"),
            "got: {out:?}"
        );
    }

    #[test]
    fn renders_multi_event_period_with_all_events() {
        let diag = parse(
            "timeline\n\
             2004 : Facebook : Google goes public",
        )
        .unwrap();
        let out = render(&diag, None);
        assert!(out.contains("Facebook"), "got: {out:?}");
        assert!(out.contains("Google goes public"), "got: {out:?}");
        // The continuation connector must appear for the second event.
        assert!(out.contains('\u{2514}'), "└ connector missing in:\n{out}");
    }

    #[test]
    fn multiple_sections_are_visually_separated() {
        let diag = parse(
            "timeline\n\
             section 2002-2004\n\
               2002 : LinkedIn\n\
             section 2005-2008\n\
               2005 : YouTube",
        )
        .unwrap();
        let out = render(&diag, None);
        // Both section headers should appear as horizontal rules.
        assert!(out.contains("2002-2004"), "first section header missing");
        assert!(out.contains("2005-2008"), "second section header missing");
        // Sections are separated by a blank line — there must be a `\n\n`.
        assert!(out.contains("\n\n"), "no blank line between sections");
    }

    #[test]
    fn max_width_truncates_long_event_text() {
        let long_event = "A".repeat(80);
        let src = format!("timeline\n2002 : {long_event}");
        let diag = parse(&src).unwrap();
        let out = render(&diag, Some(40));
        for line in out.lines() {
            let w = UnicodeWidthStr::width(line);
            assert!(w <= 40, "line exceeds max_width=40 ({w} cells): {line:?}");
        }
        assert!(out.contains('\u{2026}'), "ellipsis not inserted");
    }

    #[test]
    fn renders_unnamed_section_without_header_rule() {
        // Entries before any `section` keyword produce an implicit unnamed
        // section — no header rule should be emitted.
        let diag = parse("timeline\n2002 : LinkedIn").unwrap();
        let out = render(&diag, None);
        // A named-section rule line starts with "── " at the beginning of the
        // line (no indent). None of the output lines should start with the
        // section-rule prefix when the only section is the implicit unnamed one.
        let has_rule_line = out.lines().any(|l| l.starts_with("\u{2500}\u{2500} "));
        assert!(!has_rule_line, "unexpected section rule in:\n{out}");
        assert!(out.contains("2002"));
        assert!(out.contains("LinkedIn"));
    }
}
