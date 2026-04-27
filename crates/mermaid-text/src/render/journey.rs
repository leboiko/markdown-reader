//! Renderer for [`JourneyDiagram`]. Produces a section/task tree with
//! filled-star score indicators in Unicode.
//!
//! Each section heading is printed flush-left; its tasks are indented below
//! it with tree-branch connectors (`├─` / `└─`). The score is visualised as
//! a five-cell star band (`★` filled, `☆` empty) so the satisfaction level
//! reads at a glance without consulting the numeric label. The numeric form
//! `(n/5)` follows for screen-reader and copy-paste legibility. Actors are
//! listed after an em-dash separator.
//!
//! Example output:
//!
//! ```text
//! Journey: My working day
//!
//!   Go to work
//!    ├─ Make tea         [★★★★★] (5/5) — Me
//!    ├─ Go upstairs      [★★★☆☆] (3/5) — Me
//!    └─ Do work          [★☆☆☆☆] (1/5) — Me, Cat
//!
//!   Go home
//!    ├─ Go downstairs    [★★★★★] (5/5) — Me
//!    └─ Sit down         [★★★☆☆] (3/5) — Me
//! ```
//!
//! The `max_width` parameter is accepted for API consistency with the other
//! renderers but is not currently used to reflow content — task title columns
//! align to the widest title in each section, keeping the output compact.

use unicode_width::UnicodeWidthStr;

use crate::journey::JourneyDiagram;

/// Render a [`JourneyDiagram`] to a Unicode string.
///
/// Task titles are right-padded within each section to align the score
/// columns. Sections are separated by a blank line. The optional diagram
/// title appears on the first line prefixed with `Journey: `.
///
/// `max_width` is accepted for API parity with the other diagram renderers
/// but does not currently trigger any reflow; the layout is always sized to
/// the natural content width.
pub fn render(diag: &JourneyDiagram, _max_width: Option<usize>) -> String {
    let mut out = String::new();

    if let Some(title) = diag.title.as_deref() {
        out.push_str("Journey: ");
        out.push_str(title);
        out.push('\n');
    }

    for section in &diag.sections {
        out.push('\n');

        // Section heading — indented by two spaces; unnamed sections are
        // rendered without a heading row so tasks flow without visual noise.
        if let Some(name) = section.name.as_deref() {
            out.push_str("  ");
            out.push_str(name);
            out.push('\n');
        }

        // Determine the widest task title in this section for column alignment.
        let title_w = section
            .tasks
            .iter()
            .map(|t| UnicodeWidthStr::width(t.title.as_str()))
            .max()
            .unwrap_or(0);

        let last = section.tasks.len().saturating_sub(1);
        for (i, task) in section.tasks.iter().enumerate() {
            // Tree connector: `└─` for the last task, `├─` for all others.
            let connector = if i == last { "└─" } else { "├─" };

            // Title padded to align subsequent columns.
            let tw = UnicodeWidthStr::width(task.title.as_str());
            let pad = title_w.saturating_sub(tw);

            let score_bar = star_bar(task.score);
            let actors = task.actors.join(", ");

            out.push_str("   ");
            out.push_str(connector);
            out.push(' ');
            out.push_str(&task.title);
            // Trailing spaces between the title and the score column.
            for _ in 0..pad + 4 {
                out.push(' ');
            }
            out.push('[');
            out.push_str(&score_bar);
            out.push_str("] (");
            out.push_str(&task.score.to_string());
            out.push_str("/5) \u{2014} "); // U+2014 EM DASH
            out.push_str(&actors);
            out.push('\n');
        }
    }

    // Trim the trailing newline to match other renderers.
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Build the five-cell star bar for a score in 1–5.
///
/// Filled cells use `★` (U+2605 BLACK STAR); empty cells use `☆`
/// (U+2606 WHITE STAR). Five cells total so the score difference is
/// visible at a glance: score 1 → one filled + four empty; score 5 →
/// five filled.
fn star_bar(score: u8) -> String {
    // Clamp defensively — the parser already validates 1–5, but the
    // renderer should not panic on a directly-constructed Task.
    let filled = (score as usize).min(5);
    let mut s = String::with_capacity(5 * 3); // each star is 3 UTF-8 bytes
    for _ in 0..filled {
        s.push('\u{2605}'); // BLACK STAR
    }
    for _ in filled..5 {
        s.push('\u{2606}'); // WHITE STAR
    }
    s
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::journey::parse;

    #[test]
    fn renders_title_when_present() {
        let diag = parse("journey\ntitle My Day\nsection Work\nStep: 3: Me").unwrap();
        let out = render(&diag, None);
        assert!(out.starts_with("Journey: My Day"), "got: {out:?}");
    }

    #[test]
    fn renders_score_visually_distinguishable_at_5_vs_1() {
        let diag = parse(
            "journey\n\
             section S\n\
               Full: 5: Me\n\
               Minimal: 1: Me",
        )
        .unwrap();
        let out = render(&diag, None);
        // Score 5 → five filled stars; score 1 → one filled star.
        // The rendered bar for score 5 contains five BLACK STARs.
        let five_bar = "\u{2605}\u{2605}\u{2605}\u{2605}\u{2605}";
        let one_bar = "\u{2605}\u{2606}\u{2606}\u{2606}\u{2606}";
        assert!(out.contains(five_bar), "5-star bar not found in:\n{out}");
        assert!(out.contains(one_bar), "1-star bar not found in:\n{out}");
    }

    #[test]
    fn renders_multi_actor_task() {
        let diag = parse("journey\nGroup task: 4: Alice, Bob, Carol").unwrap();
        let out = render(&diag, None);
        assert!(out.contains("Alice, Bob, Carol"), "got: {out:?}");
    }

    #[test]
    fn renders_unnamed_section_without_heading() {
        // Tasks before any `section` keyword produce an implicit unnamed
        // section. The renderer must not emit a blank section-name row.
        let diag = parse("journey\nHidden: 2: Me").unwrap();
        let out = render(&diag, None);
        // The task must appear but no section-name line should precede it.
        assert!(out.contains("Hidden"));
        // Section name row would look like "  <text>\n" — there must be
        // none with a non-empty label here.
        let section_name_row = out
            .lines()
            .any(|l| l.starts_with("  ") && !l.starts_with("   "));
        assert!(!section_name_row, "unexpected section heading in: {out:?}");
    }

    #[test]
    fn star_bar_bounds() {
        // Filled count must equal the score exactly.
        for score in 1u8..=5 {
            let bar = star_bar(score);
            let filled = bar.chars().filter(|&c| c == '\u{2605}').count();
            let empty = bar.chars().filter(|&c| c == '\u{2606}').count();
            assert_eq!(filled, score as usize);
            assert_eq!(empty, 5 - score as usize);
            assert_eq!(filled + empty, 5);
        }
    }
}
