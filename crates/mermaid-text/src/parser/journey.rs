//! Parser for Mermaid `journey` (user-journey) diagrams.
//!
//! Accepted syntax:
//!
//! ```text
//! journey
//!     title My working day
//!     section Go to work
//!       Make tea: 5: Me
//!       Go upstairs: 3: Me
//!       Do work: 1: Me, Cat
//!     section Go home
//!       Go downstairs: 5: Me
//!       Sit down: 3: Me
//! ```
//!
//! Rules:
//! - `journey` keyword is required as the first non-blank, non-comment line.
//! - `title <text>` is optional; sets the diagram title.
//! - `section <text>` opens a new group; subsequent task lines belong to it.
//! - Task lines have the form `<title>: <score>: <actor1>[, <actor2>...]`.
//!   Leading whitespace is stripped before classification.
//! - Tasks before any `section` keyword are grouped under `Section { name: None }`.
//! - Score must be an integer in the range 1–5 (inclusive); values outside this
//!   range produce [`crate::Error::ParseError`].
//! - `%%` comment lines and blank lines are silently skipped.
//!
//! # Examples
//!
//! ```
//! use mermaid_text::parser::journey::parse;
//!
//! let diag = parse("journey\ntitle Day\nsection Work\nMake tea: 5: Me").unwrap();
//! assert_eq!(diag.title.as_deref(), Some("Day"));
//! assert_eq!(diag.sections.len(), 1);
//! assert_eq!(diag.sections[0].tasks[0].score, 5);
//! ```

use crate::Error;
use crate::journey::{JourneyDiagram, Section, Task};
use crate::parser::common::strip_inline_comment;

/// Parse a `journey` source string into a [`JourneyDiagram`].
///
/// # Errors
///
/// - [`Error::ParseError`] — missing `journey` header, malformed task line,
///   or score outside 1–5.
pub fn parse(src: &str) -> Result<JourneyDiagram, Error> {
    let mut diag = JourneyDiagram::default();
    let mut header_seen = false;

    // Index of the section currently being populated. `None` until either the
    // first task (creates an unnamed implicit section) or the first `section`
    // keyword appears.
    let mut current_section: Option<usize> = None;

    for raw in src.lines() {
        let line = strip_inline_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        if !header_seen {
            if !line.eq_ignore_ascii_case("journey") {
                return Err(Error::ParseError(format!(
                    "expected `journey` header, got {line:?}"
                )));
            }
            header_seen = true;
            continue;
        }

        // Optional `title <text>` line — must appear after the header and
        // before any section/task (though Mermaid itself tolerates any order;
        // we follow the spec for simplicity: last `title` wins if repeated).
        if let Some(rest) = strip_keyword_ci(line, "title") {
            diag.title = Some(rest.to_string());
            continue;
        }

        // `section <text>` — open a new named group.
        if let Some(rest) = strip_keyword_ci(line, "section") {
            diag.sections.push(Section {
                name: Some(rest.to_string()),
                tasks: Vec::new(),
            });
            current_section = Some(diag.sections.len() - 1);
            continue;
        }

        // Everything else is treated as a task line: `<title>: <score>: <actors>`.
        let task = parse_task_line(line)?;

        // If no section has been opened yet, create the implicit unnamed one.
        let idx = match current_section {
            Some(i) => i,
            None => {
                diag.sections.push(Section {
                    name: None,
                    tasks: Vec::new(),
                });
                let i = diag.sections.len() - 1;
                current_section = Some(i);
                i
            }
        };
        diag.sections[idx].tasks.push(task);
    }

    if !header_seen {
        return Err(Error::ParseError(
            "missing `journey` header line".to_string(),
        ));
    }

    Ok(diag)
}

/// Parse a task line of the form `<title>: <score>: <actor1>[, <actor2>...]`.
///
/// Returns [`Error::ParseError`] for any malformed input, including a score
/// that is not an integer or falls outside 1–5.
fn parse_task_line(line: &str) -> Result<Task, Error> {
    // Split on `:` at most 3 parts — title, score, actors. The task title
    // may itself contain colons in theory (Mermaid doesn't allow it, but
    // our spec says "any non-colon chars"), so `splitn(3, ':')` is correct.
    let mut parts = line.splitn(3, ':');

    let title = parts
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::ParseError(format!("task line missing title: {line:?}")))?;

    let score_str = parts
        .next()
        .map(str::trim)
        .ok_or_else(|| Error::ParseError(format!("task line missing score: {line:?}")))?;

    let score_raw: i32 = score_str.parse().map_err(|_| {
        Error::ParseError(format!(
            "task score is not an integer: {score_str:?} in {line:?}"
        ))
    })?;

    if !(1..=5).contains(&score_raw) {
        return Err(Error::ParseError(format!(
            "task score must be between 1 and 5, got {score_raw} in {line:?}"
        )));
    }

    let actors_str = parts
        .next()
        .map(str::trim)
        .ok_or_else(|| Error::ParseError(format!("task line missing actors: {line:?}")))?;

    let actors: Vec<String> = actors_str
        .split(',')
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty())
        .collect();

    if actors.is_empty() {
        return Err(Error::ParseError(format!(
            "task line has no actors: {line:?}"
        )));
    }

    Ok(Task {
        title: title.to_string(),
        score: score_raw as u8,
        actors,
    })
}

/// Strip a case-insensitive keyword prefix followed by at least one space.
/// Returns `Some(trimmed_remainder)` on match, `None` otherwise.
fn strip_keyword_ci<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let klen = keyword.len();
    if line.len() > klen
        && line[..klen].eq_ignore_ascii_case(keyword)
        && line.as_bytes()[klen].is_ascii_whitespace()
    {
        Some(line[klen..].trim())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_journey() {
        let diag = parse("journey\nStep one: 3: Me").unwrap();
        assert_eq!(diag.title, None);
        assert_eq!(diag.sections.len(), 1);
        assert_eq!(diag.sections[0].name, None); // implicit unnamed section
        assert_eq!(diag.sections[0].tasks.len(), 1);
        let task = &diag.sections[0].tasks[0];
        assert_eq!(task.title, "Step one");
        assert_eq!(task.score, 3);
        assert_eq!(task.actors, vec!["Me"]);
    }

    #[test]
    fn parses_title_and_section() {
        let diag = parse(
            "journey\n\
             title My Day\n\
             section Morning\n\
               Make tea: 5: Alice",
        )
        .unwrap();
        assert_eq!(diag.title.as_deref(), Some("My Day"));
        assert_eq!(diag.sections.len(), 1);
        assert_eq!(diag.sections[0].name.as_deref(), Some("Morning"));
        assert_eq!(diag.sections[0].tasks[0].title, "Make tea");
        assert_eq!(diag.sections[0].tasks[0].score, 5);
    }

    #[test]
    fn parses_multiple_actors_per_task() {
        let diag = parse("journey\nTask: 5: Alice, Bob").unwrap();
        let actors = &diag.sections[0].tasks[0].actors;
        assert_eq!(actors, &["Alice", "Bob"]);
    }

    #[test]
    fn tasks_before_section_go_to_unnamed_section() {
        let diag = parse(
            "journey\n\
             Implicit: 4: Me\n\
             section Named\n\
               Explicit: 2: Me",
        )
        .unwrap();
        assert_eq!(diag.sections.len(), 2);
        assert_eq!(diag.sections[0].name, None);
        assert_eq!(diag.sections[0].tasks[0].title, "Implicit");
        assert_eq!(diag.sections[1].name.as_deref(), Some("Named"));
        assert_eq!(diag.sections[1].tasks[0].title, "Explicit");
    }

    #[test]
    fn comment_and_blank_lines_are_ignored() {
        let diag = parse(
            "%% leading comment\n\
             journey\n\
             \n\
             %% mid comment\n\
             Step: 2: Me",
        )
        .unwrap();
        assert_eq!(diag.sections[0].tasks.len(), 1);
    }

    #[test]
    fn score_outside_1_to_5_is_an_error() {
        let err = parse("journey\nTask: 6: Me").unwrap_err();
        assert!(
            err.to_string().contains("1 and 5"),
            "unexpected error: {err}"
        );
        let err2 = parse("journey\nTask: 0: Me").unwrap_err();
        assert!(err2.to_string().contains("1 and 5"));
    }

    #[test]
    fn missing_journey_header_returns_error() {
        // When the parser is invoked directly on input without the `journey`
        // keyword, it returns a parse error (the `detect` pass is bypassed).
        let err = parse("section Work\nMake tea: 5: Me").unwrap_err();
        assert!(err.to_string().contains("journey"));
    }

    #[test]
    fn non_integer_score_is_an_error() {
        let err = parse("journey\nTask: abc: Me").unwrap_err();
        assert!(err.to_string().contains("not an integer"));
    }

    #[test]
    fn multiple_sections_tasks_grouped_correctly() {
        let diag = parse(
            "journey\n\
             section A\n\
               T1: 1: Me\n\
               T2: 2: Me\n\
             section B\n\
               T3: 3: Me",
        )
        .unwrap();
        assert_eq!(diag.sections.len(), 2);
        assert_eq!(diag.sections[0].tasks.len(), 2);
        assert_eq!(diag.sections[1].tasks.len(), 1);
        assert_eq!(diag.total_tasks(), 3);
    }
}
