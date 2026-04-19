//! Parser for Mermaid `sequenceDiagram` syntax.
//!
//! Accepts the MVP subset of the syntax:
//! - `participant ID` / `participant ID as Alias`
//! - `actor ID` / `actor ID as Alias` (treated identically to `participant`)
//! - Message arrows: `->>`, `-->>`, `->`, `-->`
//! - Comments (`%% …`) and blank lines (silently skipped)
//!
//! # Examples
//!
//! ```
//! use mermaid_text::parser::sequence::parse;
//!
//! let src = "sequenceDiagram\nA->>B: hello";
//! let diag = parse(src).unwrap();
//! assert_eq!(diag.participants.len(), 2);
//! assert_eq!(diag.messages.len(), 1);
//! ```

use crate::Error;
use crate::sequence::{Message, MessageStyle, Participant, SequenceDiagram};

// ---------------------------------------------------------------------------
// Arrow token table — ordered longest-first so the greediest match wins.
// Each entry is (token, MessageStyle).
// ---------------------------------------------------------------------------
const ARROWS: &[(&str, MessageStyle)] = &[
    // dashed with arrowhead must come before dashed-open and solid-arrow
    ("-->>", MessageStyle::DashedArrow),
    ("-->", MessageStyle::DashedLine),
    ("->>", MessageStyle::SolidArrow),
    ("->", MessageStyle::SolidLine),
];

/// Parse a `sequenceDiagram` source string into a [`SequenceDiagram`].
///
/// The `sequenceDiagram` header line is required (the caller may pass the
/// full source including that line).  Lines beginning with `%%` and blank
/// lines are silently skipped.
///
/// # Errors
///
/// Returns [`Error::ParseError`] if a non-blank, non-comment, non-header line
/// cannot be recognised.
///
/// # Examples
///
/// ```
/// use mermaid_text::parser::sequence::parse;
///
/// let src = "sequenceDiagram\n    participant A as Alice\n    A->>A: self";
/// let diag = parse(src).unwrap();
/// assert_eq!(diag.participants[0].label, "Alice");
/// assert_eq!(diag.messages[0].from, "A");
/// assert_eq!(diag.messages[0].to, "A");
/// ```
pub fn parse(src: &str) -> Result<SequenceDiagram, Error> {
    let mut diag = SequenceDiagram::default();

    for raw in src.lines() {
        let line = raw.trim();

        // Skip blank lines and comments.
        if line.is_empty() || line.starts_with("%%") {
            continue;
        }

        // Skip the header line.
        if line.to_lowercase().starts_with("sequencediagram") {
            continue;
        }

        // TODO: `autonumber` directive — not implemented in MVP.
        if line.eq_ignore_ascii_case("autonumber") {
            continue;
        }

        // TODO: Note over/left of/right of — not implemented in MVP.
        if line.to_lowercase().starts_with("note ") {
            continue;
        }

        // TODO: activate/deactivate — not implemented in MVP.
        if line.to_lowercase().starts_with("activate ")
            || line.to_lowercase().starts_with("deactivate ")
        {
            continue;
        }

        // TODO: block statements (loop, alt, opt, par, critical, break, rect, end)
        // — not implemented in MVP.
        let lower = line.to_lowercase();
        if matches!(
            lower.split_whitespace().next().unwrap_or(""),
            "loop" | "alt" | "opt" | "par" | "critical" | "break" | "rect" | "end"
        ) {
            continue;
        }

        // `participant ID` or `participant ID as Alias`
        // `actor ID` or `actor ID as Alias` (treated identically)
        if let Some(rest) = strip_keyword_prefix(line, "participant")
            .or_else(|| strip_keyword_prefix(line, "actor"))
        {
            let p = parse_participant_decl(rest)?;
            // If already present (e.g. auto-created by a message), update label.
            if let Some(idx) = diag.participant_index(&p.id) {
                diag.participants[idx].label = p.label;
            } else {
                diag.participants.push(p);
            }
            continue;
        }

        // Message arrow lines: `From<arrow>To: text`
        if let Some(msg) = try_parse_message(line) {
            diag.ensure_participant(&msg.from.clone());
            diag.ensure_participant(&msg.to.clone());
            diag.messages.push(msg);
            continue;
        }

        // Unrecognised non-blank, non-comment line — surface as a parse error
        // so callers can distinguish "I don't understand this" from silent skips.
        return Err(Error::ParseError(format!(
            "unrecognised sequence diagram line: {line:?}"
        )));
    }

    Ok(diag)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Strip a case-insensitive keyword prefix followed by at least one space.
/// Returns the trimmed remainder, or `None` if the prefix does not match.
fn strip_keyword_prefix<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let len = keyword.len();
    if line.len() > len
        && line[..len].eq_ignore_ascii_case(keyword)
        && line.as_bytes()[len].is_ascii_whitespace()
    {
        Some(line[len..].trim())
    } else {
        None
    }
}

/// Parse the part of a participant/actor declaration that follows the keyword.
///
/// Formats:
/// - `ID` → label defaults to ID
/// - `ID as Alias` → label is Alias (may contain spaces)
fn parse_participant_decl(rest: &str) -> Result<Participant, Error> {
    // Look for ` as ` separator (case-insensitive, surrounded by whitespace).
    // We split on the first occurrence.
    let lower = rest.to_lowercase();

    // Find " as " with surrounding whitespace.
    if let Some(as_idx) = lower.find(" as ") {
        let id = rest[..as_idx].trim().to_string();
        let label = rest[as_idx + 4..].trim().to_string();
        if id.is_empty() {
            return Err(Error::ParseError(
                "participant declaration has an empty ID".to_string(),
            ));
        }
        Ok(Participant::with_label(id, label))
    } else {
        let id = rest.trim().to_string();
        if id.is_empty() {
            return Err(Error::ParseError(
                "participant declaration has an empty ID".to_string(),
            ));
        }
        Ok(Participant::new(id))
    }
}

/// Attempt to parse a message arrow line of the form `From<arrow>To: text`.
///
/// Returns `None` when no known arrow token is found in the line.
fn try_parse_message(line: &str) -> Option<Message> {
    for &(arrow, style) in ARROWS {
        if let Some((from, rest)) = line.split_once(arrow) {
            let from = from.trim().to_string();
            // Remaining text: `To: message text` or just `To`
            let (to, text) = if let Some((to_part, msg_part)) = rest.split_once(':') {
                (to_part.trim().to_string(), msg_part.trim().to_string())
            } else {
                (rest.trim().to_string(), String::new())
            };

            if from.is_empty() || to.is_empty() {
                continue;
            }

            return Some(Message {
                from,
                to,
                text,
                style,
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sequence::MessageStyle;

    #[test]
    fn parse_minimal_sequence() {
        let src = "sequenceDiagram\nA->>B: hi";
        let diag = parse(src).unwrap();
        assert_eq!(diag.participants.len(), 2, "expected 2 participants");
        assert_eq!(diag.messages.len(), 1, "expected 1 message");
        assert_eq!(diag.messages[0].from, "A");
        assert_eq!(diag.messages[0].to, "B");
        assert_eq!(diag.messages[0].text, "hi");
        assert_eq!(diag.messages[0].style, MessageStyle::SolidArrow);
    }

    #[test]
    fn parse_explicit_participants_with_aliases() {
        let src = "sequenceDiagram\nparticipant W as Worker\nparticipant S as Server";
        let diag = parse(src).unwrap();
        assert_eq!(diag.participants[0].id, "W");
        assert_eq!(diag.participants[0].label, "Worker");
        assert_eq!(diag.participants[1].id, "S");
        assert_eq!(diag.participants[1].label, "Server");
    }

    #[test]
    fn parse_actor_treated_like_participant() {
        let src = "sequenceDiagram\nactor U as User\nU->>S: hello\nS-->>U: world";
        let diag = parse(src).unwrap();
        assert_eq!(diag.participants[0].label, "User");
        assert_eq!(diag.messages[1].style, MessageStyle::DashedArrow);
    }

    #[test]
    fn parse_all_arrow_styles() {
        let src = "sequenceDiagram\nA->>B: solid arrow\nA-->>B: dashed arrow\nA->B: solid line\nA-->B: dashed line";
        let diag = parse(src).unwrap();
        assert_eq!(diag.messages[0].style, MessageStyle::SolidArrow);
        assert_eq!(diag.messages[1].style, MessageStyle::DashedArrow);
        assert_eq!(diag.messages[2].style, MessageStyle::SolidLine);
        assert_eq!(diag.messages[3].style, MessageStyle::DashedLine);
    }

    #[test]
    fn parse_comment_and_blank_lines_ignored() {
        let src = "sequenceDiagram\n%% This is a comment\n\nA->>B: ok";
        let diag = parse(src).unwrap();
        assert_eq!(diag.messages.len(), 1);
    }

    #[test]
    fn parse_participant_auto_created_from_message() {
        // No explicit participant declarations — both should be auto-created.
        let src = "sequenceDiagram\nAlice->>Bob: hello";
        let diag = parse(src).unwrap();
        assert_eq!(diag.participants.len(), 2);
        assert_eq!(diag.participants[0].id, "Alice");
        assert_eq!(diag.participants[1].id, "Bob");
    }

    #[test]
    fn parse_self_message() {
        let src = "sequenceDiagram\nA->>A: self";
        let diag = parse(src).unwrap();
        assert_eq!(diag.participants.len(), 1);
        assert_eq!(diag.messages[0].from, "A");
        assert_eq!(diag.messages[0].to, "A");
    }
}
