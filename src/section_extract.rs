//! Section extraction: find a named heading and return its content.
//!
//! Used by the `--section` CLI flag to print a single heading section
//! (heading line + body until the next same-or-higher-level heading) to
//! stdout without launching the TUI.

/// Parse the ATX heading level of a line (e.g. `## Foo` → `Some(2)`).
///
/// Returns `None` when the line is not a heading.
fn heading_level(line: &str) -> Option<usize> {
    // ATX headings start with 1–6 `#` characters followed by a space or end of
    // line.  Setext-style headings (underline with `===`/`---`) are intentionally
    // not supported here; they are uncommon and the ATX form is sufficient for
    // all practical `--section` use cases.
    let trimmed = line.trim_start_matches('#');
    let hashes = line.len() - trimmed.len();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    // The character immediately after the `#` run must be a space (or the line
    // must end there, which covers `#` on its own — unlikely but valid).
    match trimmed.chars().next() {
        None | Some(' ') => Some(hashes),
        _ => None,
    }
}

/// Extract the heading text from an ATX heading line, stripped of the leading
/// `#` markers and surrounding whitespace.
///
/// Returns an empty string for non-heading lines (should not happen in normal
/// usage since callers guard with [`heading_level`]).
fn heading_text(line: &str) -> &str {
    let without_hashes = line.trim_start_matches('#');
    without_hashes.trim()
}

/// Find the first heading whose text contains `name` (case-insensitive
/// substring match) and return the heading + its body as a `String`.
///
/// # Matching rule
///
/// A heading matches when `heading_text.to_lowercase().contains(name_lower)`.
/// This is a **case-insensitive substring match**: `--section "install"` will
/// match `## Installation`, `### Install guide`, etc.  The first match in
/// document order is used; subsequent matches are ignored.
///
/// # Body extent
///
/// The body runs from the line immediately after the matched heading up to
/// (but not including) the next heading whose level is ≤ the matched heading's
/// level (i.e. same level or a higher-level heading ends the section).
/// If the section extends to the end of the file, all remaining lines are
/// included.
///
/// # Arguments
///
/// * `source` – full markdown source string.
/// * `name`   – heading text to search for (compared case-insensitively).
///
/// # Returns
///
/// `Some(section)` if a matching heading was found; `None` otherwise.
pub fn extract_section(source: &str, name: &str) -> Option<String> {
    let name_lower = name.to_lowercase();

    // Collect lines without consuming so we can index efficiently.
    let lines: Vec<&str> = source.lines().collect();
    let n = lines.len();

    // Find the first matching heading.
    let mut start_idx: Option<usize> = None;
    let mut section_level: usize = 0;

    for (i, &line) in lines.iter().enumerate() {
        if let Some(level) = heading_level(line) {
            let text_lower = heading_text(line).to_lowercase();
            if text_lower.contains(&name_lower) {
                start_idx = Some(i);
                section_level = level;
                break;
            }
        }
    }

    let start = start_idx?;

    // Find the end: the first heading of equal or higher level after `start`.
    // Higher level = lower H number (`#` outranks `##`).
    let end = lines[start + 1..]
        .iter()
        .position(|&line| {
            heading_level(line).is_some_and(|lvl| lvl <= section_level)
        })
        .map(|rel| start + 1 + rel)
        .unwrap_or(n);

    // Build the section string from the matched heading through `end - 1`.
    // Preserve the original line endings by joining with `\n` and appending a
    // final `\n` only when the source had content after the last line
    // (i.e. the source had a trailing newline).
    let section_lines = &lines[start..end];
    let mut out = section_lines.join("\n");

    // Mirror the trailing newline behaviour of the original source: if the
    // source ends with `\n`, every section should also end with `\n` (it's
    // the norm in markdown files).  We only omit it when the source itself
    // has no trailing newline and the section hits EOF.
    let source_has_trailing_newline = source.ends_with('\n');
    if source_has_trailing_newline || end < n {
        out.push('\n');
    }

    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A simple two-section document used across several tests.
    fn two_section_doc() -> &'static str {
        "# Title\n\n## Foo\nbody1\n\n## Bar\nbody2\n"
    }

    // ── heading_level ──────────────────────────────────────────────────────────

    #[test]
    fn heading_level_detects_h1_through_h6() {
        assert_eq!(heading_level("# H1"), Some(1));
        assert_eq!(heading_level("## H2"), Some(2));
        assert_eq!(heading_level("###### H6"), Some(6));
    }

    #[test]
    fn heading_level_rejects_non_headings() {
        assert_eq!(heading_level("regular text"), None);
        assert_eq!(heading_level("##no-space"), None); // `##` not followed by space
        assert_eq!(heading_level("####### too many"), None); // 7 hashes
        assert_eq!(heading_level(""), None);
    }

    // ── extract_section ────────────────────────────────────────────────────────

    #[test]
    fn extracts_first_matching_section() {
        let doc = two_section_doc();

        // "Foo" → second heading, body is "body1".
        let foo = extract_section(doc, "Foo").expect("should find Foo");
        assert_eq!(foo, "## Foo\nbody1\n\n");

        // "Bar" → third heading, body is "body2".
        let bar = extract_section(doc, "Bar").expect("should find Bar");
        assert_eq!(bar, "## Bar\nbody2\n");
    }

    #[test]
    fn case_insensitive_substring_match() {
        // Lower-case query matches mixed-case heading.
        // The blank line between heading and body is preserved in the output.
        let doc = "# Features Overview\n\ncontent\n";
        let result = extract_section(doc, "features").expect("should match");
        assert_eq!(result, "# Features Overview\n\ncontent\n");

        // Substring match: "foo" matches "## Foobar".
        let doc2 = "## Foobar\nbody\n";
        let result2 = extract_section(doc2, "foo").expect("should match");
        assert_eq!(result2, "## Foobar\nbody\n");
    }

    #[test]
    fn stops_at_same_or_higher_level_heading() {
        // `# Other` (level 1) terminates `## Sub` (level 2).
        let doc = "# Top\n\n## Sub\nbody\n# Other\n";
        let result = extract_section(doc, "Sub").expect("should find Sub");
        assert_eq!(result, "## Sub\nbody\n");
    }

    #[test]
    fn last_section_extends_to_eof() {
        // "Appendix" is the last heading — everything to EOF is included.
        let doc = "# Introduction\n\nbody\n\n# Appendix\n\nappendix body\n";
        let result = extract_section(doc, "Appendix").expect("should find Appendix");
        assert_eq!(result, "# Appendix\n\nappendix body\n");
    }

    #[test]
    fn no_match_returns_none() {
        let doc = two_section_doc();
        assert!(extract_section(doc, "DoesNotExist").is_none());
    }

    #[test]
    fn first_match_wins_when_multiple_headings_match() {
        // Both "## FooA" and "## FooB" contain "foo".  The first wins.
        let doc = "## FooA\nfirst\n\n## FooB\nsecond\n";
        let result = extract_section(doc, "foo").expect("should match");
        assert_eq!(result, "## FooA\nfirst\n\n");
    }

    #[test]
    fn section_body_may_contain_lower_level_headings() {
        // `### Sub-sub` (level 3) is below the matched `## Section` (level 2),
        // so it stays inside the extracted body.
        let doc = "## Section\n\n### Sub-sub\n\nbody\n\n## Next\n";
        let result = extract_section(doc, "Section").expect("should find Section");
        assert_eq!(result, "## Section\n\n### Sub-sub\n\nbody\n\n");
    }
}
