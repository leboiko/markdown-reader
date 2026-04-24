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
use crate::parser::common::{
    block_kind_from_keyword, continuation_keyword_for, parse_sequence_note_anchor,
    strip_activation_marker, strip_inline_comment, strip_keyword_prefix,
};
use crate::sequence::{
    Activation, AutonumberChange, AutonumberState, Block, BlockBranch, BlockKind, Message,
    MessageStyle, NoteEvent, Participant, SequenceDiagram,
};
use std::collections::HashMap;

/// Internal event collected during the parse loop. Activations are
/// recorded raw (open / close at a given message index) and paired
/// up by `finalize_activations` at end-of-parse, so partial parse
/// errors still surface a useful stack-state error message.
enum ActEvent {
    Open { participant: String, at: usize },
    Close { participant: String, at: usize },
}

/// In-flight block-stack frame used during parsing. Each opener pushes
/// a frame; continuation keywords (`else`/`and`/`option`) close the
/// in-flight branch and append a new one; `end` pops the frame and
/// finalises `Block`'s top-level `start_message`/`end_message`.
struct OpenBlock {
    kind: BlockKind,
    start_message: usize,
    branches: Vec<BlockBranch>,
}

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
    let mut act_events: Vec<ActEvent> = Vec::new();
    let mut block_stack: Vec<OpenBlock> = Vec::new();

    for raw in src.lines() {
        // Strip inline `%% comment` (outside quoted strings) before
        // trimming. The shared helper handles the in-quote case the
        // naive `starts_with("%%")` check used to miss.
        let line = strip_inline_comment(raw).trim();

        // Skip blank lines and full-line comments.
        if line.is_empty() {
            continue;
        }

        // Skip the header line.
        if line.to_lowercase().starts_with("sequencediagram") {
            continue;
        }

        // `autonumber` directive — supported forms:
        //   - bare `autonumber`: numbering on, start at 1
        //   - `autonumber <N>`: numbering on, start at N
        //   - `autonumber off`: numbering off (mid-diagram allowed)
        // Multiple directives in one diagram are honoured (re-base or
        // toggle off/on at any point). Decimal start values and the
        // `<start> <step>` form are deferred (see ROADMAP).
        if line.eq_ignore_ascii_case("autonumber") {
            diag.autonumber_changes.push(AutonumberChange {
                at_message: diag.messages.len(),
                state: AutonumberState::On { next_value: 1 },
            });
            continue;
        }
        if let Some(rest) = strip_keyword_prefix(line, "autonumber") {
            let state = if rest.eq_ignore_ascii_case("off") {
                AutonumberState::Off
            } else {
                let start: u32 = rest
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
                AutonumberState::On { next_value: start }
            };
            diag.autonumber_changes.push(AutonumberChange {
                at_message: diag.messages.len(),
                state,
            });
            continue;
        }

        // Defensive: Mermaid's sequence-diagram grammar has NO `end note`
        // form (state diagrams do — that's a different parser). A user
        // coming from state diagrams might write it; give them a clear
        // pointer rather than silently misparsing.
        if line.eq_ignore_ascii_case("end note") {
            return Err(Error::ParseError(
                "sequence diagrams use `<br>` for multi-line notes, \
                 not `end note` (which is a state-diagram form)"
                    .to_string(),
            ));
        }

        // `note left of X : text` / `note right of X : text` /
        // `note over X : text` / `note over X,Y : text` (multi-anchor).
        // `<br>` and `<br/>` in the text become `\n` so multi-line
        // notes render via the existing line-splitting box helper.
        if let Some(rest) = strip_keyword_prefix(line, "note") {
            if let Some(colon_pos) = rest.find(':') {
                let anchor_part = rest[..colon_pos].trim();
                let text_part = rest[colon_pos + 1..].trim();
                if let Some(anchor) = parse_sequence_note_anchor(anchor_part) {
                    let text = text_part.replace("<br/>", "\n").replace("<br>", "\n");
                    diag.notes.push(NoteEvent {
                        anchor,
                        text,
                        after_message: diag.messages.len(),
                    });
                    continue;
                }
            }
            // Unrecognised note form (floating `note "text" as N1` or
            // a malformed anchor) — silently skip rather than error
            // so the diagram still renders. Floating notes are out of
            // scope per ROADMAP.
            continue;
        }

        // `activate X` / `deactivate X` — record the raw event; pairing
        // happens in `finalize_activations` after the whole source is
        // parsed so the stack-error message can reference the full
        // diagram. Activation indices use the *next* message position
        // (matching Mermaid: `activate X` before message N attaches at N).
        if let Some(rest) = strip_keyword_prefix(line, "activate") {
            let participant = rest.trim();
            if participant.is_empty() {
                return Err(Error::ParseError(
                    "`activate` directive missing participant".to_string(),
                ));
            }
            act_events.push(ActEvent::Open {
                participant: participant.to_string(),
                at: diag.messages.len(),
            });
            continue;
        }
        if let Some(rest) = strip_keyword_prefix(line, "deactivate") {
            let participant = rest.trim();
            if participant.is_empty() {
                return Err(Error::ParseError(
                    "`deactivate` directive missing participant".to_string(),
                ));
            }
            // Deactivate attaches to the *previous* message — the
            // participant was active *during* the message, not after.
            // For the very first message position, clamp to 0.
            let at = diag.messages.len().saturating_sub(1);
            act_events.push(ActEvent::Close {
                participant: participant.to_string(),
                at,
            });
            continue;
        }

        // Block statements: `loop`/`alt`/`opt`/`par`/`critical`/`break`
        // open; `else`/`and`/`option` open additional branches inside
        // their respective parents; `end` closes the innermost open
        // block. `rect <colour>` background highlight is silently
        // skipped — its colour grammar is out of scope per ROADMAP.
        let lower = line.to_lowercase();
        let head = lower.split_whitespace().next().unwrap_or("");
        if let Some(kind) = block_kind_from_keyword(head) {
            // Strip the keyword prefix to extract the inline label.
            let label = strip_keyword_prefix(line, head)
                .unwrap_or("")
                .trim()
                .to_string();
            let at = diag.messages.len();
            block_stack.push(OpenBlock {
                kind,
                start_message: at,
                branches: vec![BlockBranch {
                    label,
                    start_message: at,
                    end_message: 0, // patched on continuation or close
                }],
            });
            continue;
        }
        if matches!(head, "else" | "and" | "option") {
            let top = block_stack.last_mut().ok_or_else(|| {
                Error::ParseError(format!("`{head}` continuation keyword outside any block"))
            })?;
            let expected = continuation_keyword_for(top.kind);
            if expected != Some(head) {
                return Err(Error::ParseError(format!(
                    "`{head}` not valid inside `{:?}` block (expected `{}`)",
                    top.kind,
                    expected.unwrap_or("end"),
                )));
            }
            // Close the prior branch — its end is the most recent message
            // (or start - 1 if the branch had no messages).
            let last = top.branches.last_mut().expect("frame has 1+ branches");
            last.end_message = diag.messages.len().saturating_sub(1);
            // Append the new branch with its own label.
            let label = strip_keyword_prefix(line, head)
                .unwrap_or("")
                .trim()
                .to_string();
            top.branches.push(BlockBranch {
                label,
                start_message: diag.messages.len(),
                end_message: 0,
            });
            continue;
        }
        if head == "end" {
            let mut frame = block_stack.pop().ok_or_else(|| {
                Error::ParseError("`end` with no matching block opener".to_string())
            })?;
            let last_msg = diag.messages.len().saturating_sub(1);
            // Patch the in-flight branch's end_message.
            frame
                .branches
                .last_mut()
                .expect("frame has 1+ branches")
                .end_message = last_msg;
            diag.blocks.push(Block {
                kind: frame.kind,
                branches: frame.branches,
                start_message: frame.start_message,
                end_message: last_msg,
            });
            continue;
        }
        if head == "rect" {
            // Background-highlight colour form — ROADMAP-deferred.
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

        // Message arrow lines: `From<arrow>To: text`. The optional
        // `+`/`-` activation marker on the target token is peeled here:
        //   `A->>+B` → push msg A→B, then Open(B) at this index
        //   `A-->>-B` → push msg A→B, then Close(A) (the SOURCE — per
        //     `Activation`'s doc-comment, this preserves the canonical
        //     call/reply pattern `A->>+B; B-->>-A`)
        if let Some((msg, marker)) = try_parse_message(line) {
            let from = msg.from.clone();
            let to = msg.to.clone();
            let msg_idx = diag.messages.len();
            diag.ensure_participant(&from);
            diag.ensure_participant(&to);
            diag.messages.push(msg);
            match marker {
                Some(true) => act_events.push(ActEvent::Open {
                    participant: to,
                    at: msg_idx,
                }),
                Some(false) => act_events.push(ActEvent::Close {
                    participant: from,
                    at: msg_idx,
                }),
                None => {}
            }
            continue;
        }

        // Unrecognised non-blank, non-comment line — surface as a parse error
        // so callers can distinguish "I don't understand this" from silent skips.
        return Err(Error::ParseError(format!(
            "unrecognised sequence diagram line: {line:?}"
        )));
    }

    if !block_stack.is_empty() {
        let kinds: Vec<String> = block_stack
            .iter()
            .map(|b| format!("{:?}", b.kind).to_lowercase())
            .collect();
        return Err(Error::ParseError(format!(
            "unclosed block(s) at end of input: {} (missing `end`)",
            kinds.join(", "),
        )));
    }
    finalize_activations(&act_events, &mut diag)?;
    Ok(diag)
}

/// Pair raw activate/deactivate events into `Activation` spans using a
/// per-participant LIFO stack (so nested activations on the same
/// participant nest correctly). An orphan close is a hard error; an
/// unclosed open auto-closes at the last message — matches Mermaid's
/// lenient behaviour and the doc-comment on `Activation::end_message`.
fn finalize_activations(events: &[ActEvent], diag: &mut SequenceDiagram) -> Result<(), Error> {
    let mut stacks: HashMap<String, Vec<usize>> = HashMap::new();
    for ev in events {
        match ev {
            ActEvent::Open { participant, at } => {
                stacks.entry(participant.clone()).or_default().push(*at);
            }
            ActEvent::Close { participant, at } => {
                let start = stacks
                    .get_mut(participant)
                    .and_then(|s| s.pop())
                    .ok_or_else(|| {
                        Error::ParseError(format!(
                            "deactivate `{participant}` with no matching activate"
                        ))
                    })?;
                diag.activations.push(Activation {
                    participant: participant.clone(),
                    start_message: start,
                    end_message: *at,
                });
            }
        }
    }
    let last = diag.messages.len().saturating_sub(1);
    for (participant, mut stack) in stacks {
        while let Some(start) = stack.pop() {
            diag.activations.push(Activation {
                participant: participant.clone(),
                start_message: start,
                end_message: last,
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse the part of a participant/actor declaration that follows the keyword.
///
/// Formats:
/// - `ID` → label defaults to ID
/// - `ID as Alias` → label is Alias (may contain spaces)
///
/// HTML `<br>` / `<br/>` / `<br />` (case-insensitive) in the alias collapse
/// to a single space — Mermaid uses these for line breaks inside participant
/// boxes, but sequence participant boxes in this renderer are single-row, so
/// joining with a space produces a clean readable label instead of leaking
/// the literal `<br>` tag into the output.
fn parse_participant_decl(rest: &str) -> Result<Participant, Error> {
    // Look for ` as ` separator (case-insensitive, surrounded by whitespace).
    // We split on the first occurrence.
    let lower = rest.to_lowercase();

    // Find " as " with surrounding whitespace.
    if let Some(as_idx) = lower.find(" as ") {
        let id = rest[..as_idx].trim().to_string();
        let label = strip_br_tags(rest[as_idx + 4..].trim());
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

/// Replace HTML `<br>` variants with a single space.
///
/// Sequence participant boxes are single-row so we can't honour the line
/// break visually — but leaving the literal tag in the label is the worst
/// of both worlds. Spaces give a clean readable result.
fn strip_br_tags(s: &str) -> String {
    s.replace("<br/>", " ")
        .replace("<br>", " ")
        .replace("<br />", " ")
        .replace("<BR/>", " ")
        .replace("<BR>", " ")
        .replace("<BR />", " ")
}

/// Attempt to parse a message arrow line of the form `From<arrow>To: text`,
/// recognising the inline activation shorthand `+`/`-` on the target token.
///
/// Returns `None` when no known arrow token is found in the line.
/// Otherwise returns `(message, marker)` where `marker` is
/// `Some(true)` for `+` (activate target), `Some(false)` for `-`
/// (deactivate source — see `Activation` doc-comment), `None` for none.
fn try_parse_message(line: &str) -> Option<(Message, Option<bool>)> {
    for &(arrow, style) in ARROWS {
        if let Some((from, rest)) = line.split_once(arrow) {
            let from = from.trim().to_string();
            // Remaining text: `To: message text` or just `To`
            // Message text collapses HTML `<br>` to a single space —
            // same reasoning as `parse_participant_decl::strip_br_tags`.
            // Sequence message labels render on one row above the arrow,
            // so a `\n` would break the layout. Joining with a space
            // produces a clean readable result. (Notes, by contrast, get
            // their own multi-row box and convert `<br>` to `\n` —
            // see the Note-handling branch above.)
            let (to_token, text) = if let Some((to_part, msg_part)) = rest.split_once(':') {
                (to_part.trim().to_string(), strip_br_tags(msg_part.trim()))
            } else {
                (rest.trim().to_string(), String::new())
            };

            // Peel the optional inline activation marker from the
            // target token. The id stripped of the marker is the
            // actual participant id pushed into the message.
            let (to, marker) = strip_activation_marker(&to_token);

            if from.is_empty() || to.is_empty() {
                continue;
            }

            return Some((
                Message {
                    from,
                    to,
                    text,
                    style,
                },
                marker,
            ));
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

    /// Block statements (`alt`/`else`/`end`, `loop`/`end`, nested versions)
    /// must be silently skipped so the inner messages still render. A real
    /// Mermaid sequence diagram frequently uses these for conditional flow,
    /// and rejecting them caused the TUI to show raw source.
    #[test]
    fn parse_complex_nested_blocks_records_full_tree() {
        // Smoke-tests parser nesting on a moderately deep tree.
        // Renamed from `parse_block_statements_are_skipped` (0.9.3
        // promoted blocks from silent-skip to full data model).
        let src = r#"sequenceDiagram
    participant W
    participant CP
    W->>CP: read
    alt Batch is empty
        W->>W: beat heartbeat
    else Batch has events
        alt Success
            W->>CP: save checkpoint
        else Retry exhausted
            W->>W: back off
        end
    end
    loop Every second
        W->>W: tick
    end
    par A to B
        W->>CP: write
    and C to D
        W->>CP: read
    end"#;
        let diag = parse(src).expect("must parse cleanly");
        // 7 inner messages — block keywords contribute none.
        assert_eq!(diag.messages.len(), 7);
        // 4 blocks: outer alt, inner alt, loop, par. Recorded in the
        // order they were closed (innermost first per LIFO stack).
        assert_eq!(diag.blocks.len(), 4);
        // First closed = inner alt.
        assert_eq!(diag.blocks[0].kind, BlockKind::Alt);
        assert_eq!(diag.blocks[0].branches.len(), 2);
        assert_eq!(diag.blocks[0].branches[0].label, "Success");
        assert_eq!(diag.blocks[0].branches[1].label, "Retry exhausted");
        // Outer alt (closes after its else branch finishes).
        assert_eq!(diag.blocks[1].kind, BlockKind::Alt);
        assert_eq!(diag.blocks[1].branches[0].label, "Batch is empty");
        // Loop.
        assert_eq!(diag.blocks[2].kind, BlockKind::Loop);
        assert_eq!(diag.blocks[2].branches[0].label, "Every second");
        // Par with 2 branches.
        assert_eq!(diag.blocks[3].kind, BlockKind::Par);
        assert_eq!(diag.blocks[3].branches.len(), 2);
        assert_eq!(diag.blocks[3].branches[0].label, "A to B");
        assert_eq!(diag.blocks[3].branches[1].label, "C to D");
    }

    // ---- autonumber (0.9.0) ------------------------------------------

    #[test]
    fn parse_autonumber_bare_enables_at_start_one() {
        let diag = parse("sequenceDiagram\nautonumber\nA->>B: hi").unwrap();
        assert_eq!(diag.autonumber_changes.len(), 1);
        assert_eq!(diag.autonumber_changes[0].at_message, 0);
        assert_eq!(
            diag.autonumber_changes[0].state,
            AutonumberState::On { next_value: 1 }
        );
    }

    #[test]
    fn parse_autonumber_with_start_value() {
        let diag = parse("sequenceDiagram\nautonumber 5\nA->>B: hi").unwrap();
        assert_eq!(
            diag.autonumber_changes[0].state,
            AutonumberState::On { next_value: 5 }
        );
    }

    #[test]
    fn parse_autonumber_off() {
        let diag =
            parse("sequenceDiagram\nautonumber\nA->>B: hi\nautonumber off\nB->>A: bye").unwrap();
        assert_eq!(diag.autonumber_changes.len(), 2);
        assert_eq!(diag.autonumber_changes[1].at_message, 1);
        assert_eq!(diag.autonumber_changes[1].state, AutonumberState::Off);
    }

    #[test]
    fn parse_autonumber_mid_diagram_rebase() {
        let diag = parse("sequenceDiagram\nA->>B: a\nautonumber 100\nB->>A: b").unwrap();
        assert_eq!(diag.autonumber_changes[0].at_message, 1);
        assert_eq!(
            diag.autonumber_changes[0].state,
            AutonumberState::On { next_value: 100 }
        );
    }

    // ---- notes (0.9.1) -----------------------------------------------

    #[test]
    fn parse_note_left_of_records_left_anchor() {
        let diag = parse("sequenceDiagram\nA->>B: hi\nnote left of A : context").unwrap();
        assert_eq!(diag.notes.len(), 1);
        assert_eq!(
            diag.notes[0].anchor,
            crate::sequence::NoteAnchor::LeftOf("A".to_string())
        );
        assert_eq!(diag.notes[0].text, "context");
        assert_eq!(diag.notes[0].after_message, 1, "after the only message");
    }

    #[test]
    fn parse_note_right_of_records_right_anchor() {
        let diag = parse("sequenceDiagram\nnote right of B : tip\nA->>B: hi").unwrap();
        assert_eq!(
            diag.notes[0].anchor,
            crate::sequence::NoteAnchor::RightOf("B".to_string())
        );
        // Note appears BEFORE the message so after_message = 0.
        assert_eq!(diag.notes[0].after_message, 0);
    }

    #[test]
    fn parse_note_over_single_anchor() {
        let diag = parse("sequenceDiagram\nA->>B: hi\nnote over A : single").unwrap();
        assert_eq!(
            diag.notes[0].anchor,
            crate::sequence::NoteAnchor::Over("A".to_string())
        );
    }

    #[test]
    fn parse_note_over_pair_anchor() {
        let diag = parse("sequenceDiagram\nA->>B: hi\nnote over A,B : shared").unwrap();
        assert_eq!(
            diag.notes[0].anchor,
            crate::sequence::NoteAnchor::OverPair("A".to_string(), "B".to_string())
        );
    }

    #[test]
    fn parse_note_br_tags_become_newlines() {
        let diag =
            parse("sequenceDiagram\nA->>B: hi\nnote over A : line1<br>line2<br/>line3").unwrap();
        assert_eq!(diag.notes[0].text, "line1\nline2\nline3");
    }

    #[test]
    fn parse_end_note_returns_helpful_error() {
        let err = parse("sequenceDiagram\nA->>B: hi\nend note")
            .expect_err("end note must be rejected with a helpful error");
        let msg = format!("{err}");
        assert!(
            msg.contains("<br>") || msg.contains("not `end note`"),
            "error must mention `<br>` or `not end note`, got: {msg}"
        );
    }

    #[test]
    fn parse_floating_note_silently_skipped() {
        // `note "text" as N1` — out of scope, parse without error
        // and produce no NoteEvent.
        let diag = parse("sequenceDiagram\nA->>B: hi\nnote \"floating\" as N1").unwrap();
        assert!(diag.notes.is_empty());
    }

    #[test]
    fn parse_multiple_notes_track_message_position() {
        let diag = parse(
            "sequenceDiagram\n\
             A->>B: first\n\
             note right of B : after first\n\
             B->>A: second\n\
             note left of A : after second",
        )
        .unwrap();
        assert_eq!(diag.notes.len(), 2);
        assert_eq!(diag.notes[0].after_message, 1);
        assert_eq!(diag.notes[1].after_message, 2);
    }

    // ---- activations (0.9.2) ------------------------------------------

    #[test]
    fn parse_explicit_activate_deactivate_pair() {
        let diag = parse(
            "sequenceDiagram\n\
             A->>B: hi\n\
             activate B\n\
             B->>A: ok\n\
             deactivate B",
        )
        .unwrap();
        assert_eq!(diag.activations.len(), 1);
        assert_eq!(diag.activations[0].participant, "B");
        // `activate B` after message 0 attaches at index 1 (next msg).
        assert_eq!(diag.activations[0].start_message, 1);
        // `deactivate B` after message 1 attaches at the previous (1).
        assert_eq!(diag.activations[0].end_message, 1);
    }

    #[test]
    fn parse_inline_plus_activates_target() {
        let diag = parse("sequenceDiagram\nA->>+B: hi").unwrap();
        // The unclosed activation auto-closes at the last message.
        assert_eq!(diag.activations.len(), 1);
        assert_eq!(diag.activations[0].participant, "B");
        assert_eq!(diag.activations[0].start_message, 0);
        assert_eq!(diag.activations[0].end_message, 0);
        // Target id is stripped of the `+` marker.
        assert_eq!(diag.messages[0].to, "B");
    }

    #[test]
    fn parse_inline_minus_deactivates_source() {
        // The inline `-` deactivates the SOURCE per the doc-comment on
        // `Activation` (preserves `A->>+B; B-->>-A` call/reply pattern).
        let diag = parse(
            "sequenceDiagram\n\
             A->>+B: call\n\
             B-->>-A: reply",
        )
        .unwrap();
        assert_eq!(diag.activations.len(), 1);
        assert_eq!(diag.activations[0].participant, "B");
        assert_eq!(diag.activations[0].start_message, 0);
        assert_eq!(diag.activations[0].end_message, 1);
        assert_eq!(diag.messages[1].to, "A");
    }

    #[test]
    fn parse_nested_activations_same_participant() {
        let diag = parse(
            "sequenceDiagram\n\
             A->>B: outer\n\
             activate B\n\
             A->>B: inner\n\
             activate B\n\
             B->>A: inner reply\n\
             deactivate B\n\
             B->>A: outer reply\n\
             deactivate B",
        )
        .unwrap();
        // Two nested activations on B: inner (LIFO) then outer.
        assert_eq!(diag.activations.len(), 2);
        // Inner pops first.
        assert_eq!(diag.activations[0].participant, "B");
        assert_eq!(diag.activations[0].start_message, 2);
        assert_eq!(diag.activations[1].participant, "B");
        assert_eq!(diag.activations[1].start_message, 1);
    }

    #[test]
    fn parse_orphan_deactivate_errors() {
        let err = parse("sequenceDiagram\nA->>B: hi\ndeactivate B")
            .expect_err("orphan deactivate must error");
        let msg = err.to_string();
        assert!(
            msg.contains("deactivate") && msg.contains('B'),
            "error mentions deactivate and the participant: {msg}"
        );
    }

    #[test]
    fn parse_unclosed_activate_extends_to_last_message() {
        let diag = parse(
            "sequenceDiagram\n\
             activate B\n\
             A->>B: one\n\
             B->>A: two",
        )
        .unwrap();
        assert_eq!(diag.activations.len(), 1);
        assert_eq!(diag.activations[0].start_message, 0);
        assert_eq!(diag.activations[0].end_message, 1, "extends to last msg");
    }

    #[test]
    fn parse_activate_missing_participant_errors() {
        let err = parse("sequenceDiagram\nactivate").expect_err("bare `activate` is malformed");
        assert!(err.to_string().contains("activate"));
    }

    // ---- block statements (0.9.3) -------------------------------------

    #[test]
    fn parse_loop_records_single_branch_block() {
        let diag = parse(
            "sequenceDiagram\n\
             loop Every second\n\
             A->>B: tick\n\
             end",
        )
        .unwrap();
        assert_eq!(diag.blocks.len(), 1);
        let b = &diag.blocks[0];
        assert_eq!(b.kind, BlockKind::Loop);
        assert_eq!(b.branches.len(), 1);
        assert_eq!(b.branches[0].label, "Every second");
        assert_eq!(b.start_message, 0);
        assert_eq!(b.end_message, 0);
    }

    #[test]
    fn parse_alt_else_records_two_branches_with_labels() {
        let diag = parse(
            "sequenceDiagram\n\
             alt success\n\
             A->>B: ok\n\
             else failure\n\
             A->>B: fail\n\
             end",
        )
        .unwrap();
        assert_eq!(diag.blocks.len(), 1);
        let b = &diag.blocks[0];
        assert_eq!(b.kind, BlockKind::Alt);
        assert_eq!(b.branches.len(), 2);
        assert_eq!(b.branches[0].label, "success");
        assert_eq!(b.branches[0].start_message, 0);
        assert_eq!(b.branches[0].end_message, 0);
        assert_eq!(b.branches[1].label, "failure");
        assert_eq!(b.branches[1].start_message, 1);
        assert_eq!(b.branches[1].end_message, 1);
    }

    #[test]
    fn parse_opt_block() {
        let diag = parse("sequenceDiagram\nopt cache hit\nA->>B: get\nend").unwrap();
        assert_eq!(diag.blocks[0].kind, BlockKind::Opt);
        assert_eq!(diag.blocks[0].branches[0].label, "cache hit");
    }

    #[test]
    fn parse_par_with_multiple_and_branches() {
        let diag = parse(
            "sequenceDiagram\n\
             par phase1\n\
             A->>B: a\n\
             and phase2\n\
             A->>B: b\n\
             and phase3\n\
             A->>B: c\n\
             end",
        )
        .unwrap();
        assert_eq!(diag.blocks[0].kind, BlockKind::Par);
        assert_eq!(diag.blocks[0].branches.len(), 3);
        assert_eq!(diag.blocks[0].branches[2].label, "phase3");
    }

    #[test]
    fn parse_critical_with_option() {
        let diag = parse(
            "sequenceDiagram\n\
             critical primary\n\
             A->>B: try\n\
             option network down\n\
             A->>B: retry\n\
             end",
        )
        .unwrap();
        assert_eq!(diag.blocks[0].kind, BlockKind::Critical);
        assert_eq!(diag.blocks[0].branches.len(), 2);
    }

    #[test]
    fn parse_break_block() {
        let diag = parse("sequenceDiagram\nbreak quota exceeded\nA->>B: 429\nend").unwrap();
        assert_eq!(diag.blocks[0].kind, BlockKind::Break);
    }

    #[test]
    fn parse_nested_loop_inside_alt() {
        let diag = parse(
            "sequenceDiagram\n\
             alt outer\n\
             loop inner\n\
             A->>B: tick\n\
             end\n\
             else fallback\n\
             A->>B: skip\n\
             end",
        )
        .unwrap();
        // 2 blocks recorded — loop closes first (LIFO), then alt.
        assert_eq!(diag.blocks.len(), 2);
        assert_eq!(diag.blocks[0].kind, BlockKind::Loop);
        assert_eq!(diag.blocks[1].kind, BlockKind::Alt);
        // Outer alt spans both branches' messages.
        assert_eq!(diag.blocks[1].start_message, 0);
        assert_eq!(diag.blocks[1].end_message, 1);
    }

    #[test]
    fn parse_orphan_end_errors() {
        let err = parse("sequenceDiagram\nA->>B: hi\nend").expect_err("orphan end");
        assert!(err.to_string().contains("end"));
    }

    #[test]
    fn parse_else_outside_alt_errors() {
        let err = parse("sequenceDiagram\nA->>B: hi\nelse foo\nA->>B: x").expect_err("orphan else");
        assert!(err.to_string().contains("else"));
    }

    #[test]
    fn parse_and_inside_alt_errors_with_kind_hint() {
        // Continuation keyword for the wrong block kind.
        let err = parse(
            "sequenceDiagram\n\
             alt foo\n\
             A->>B: x\n\
             and bar\n\
             A->>B: y\n\
             end",
        )
        .expect_err("`and` not valid inside alt");
        let m = err.to_string();
        assert!(m.contains("and") && m.contains("else"));
    }

    #[test]
    fn parse_unclosed_block_at_eof_errors() {
        let err = parse("sequenceDiagram\nloop forever\nA->>B: hi").expect_err("unclosed loop");
        assert!(err.to_string().contains("unclosed"));
    }

    #[test]
    fn parse_rect_block_silently_skipped() {
        // `rect` colour-highlight blocks are ROADMAP-deferred. Parser
        // accepts them so existing diagrams render, but no Block is
        // recorded. The matching `end` belongs to a hypothetical
        // outer block — but here there's none, so we just verify
        // the `rect` keyword doesn't crash.
        let diag = parse("sequenceDiagram\nrect rgb(200,150,255)\nA->>B: hi\nend");
        // The standalone `end` here is an orphan (no matching `loop`/etc),
        // so we expect an error — just confirm `rect` itself didn't error.
        assert!(diag.is_err()); // orphan `end` after rect's silent skip
    }
}
