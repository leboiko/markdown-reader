//! Parser for Mermaid `erDiagram` syntax.
//!
//! Recognises three line shapes after the `erDiagram` header:
//!
//! 1. **Relationship** — `ENTITY1 ||--o{ ENTITY2 : "label"`. The
//!    label is optional; the colon must be present if any label is.
//! 2. **Entity-block open** — `ENTITY {`. Lines until the matching
//!    `}` carry attribute rows.
//! 3. **Attribute row** (only inside an entity block) — `<type>
//!    <name> [keys ...] ["comment"]`.
//!
//! `%%` line comments and blank lines are silently skipped. Lines
//! that don't match any of the above produce a clear parse error
//! pointing at the offending source.
//!
//! # Examples
//!
//! ```
//! use mermaid_text::parser::er::parse;
//!
//! let src = "erDiagram\n\
//!     A ||--o{ B : has\n\
//!     A {\n\
//!         string id PK\n\
//!     }";
//! let diag = parse(src).unwrap();
//! assert_eq!(diag.entities.len(), 2);
//! assert_eq!(diag.relationships.len(), 1);
//! assert_eq!(diag.entities[0].attributes.len(), 1);
//! ```

use crate::Error;
use crate::er::{
    Attribute, AttributeKey, Cardinality, ErDiagram, LineStyle, Relationship,
};
use crate::parser::common::strip_inline_comment;

pub fn parse(src: &str) -> Result<ErDiagram, Error> {
    let mut diag = ErDiagram::default();
    let mut header_seen = false;
    let mut current_entity: Option<usize> = None;

    for raw in src.lines() {
        let line = strip_inline_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        if !header_seen {
            // First non-blank line must be the header (case-insensitive).
            if !line.eq_ignore_ascii_case("erdiagram") {
                return Err(Error::ParseError(format!(
                    "expected `erDiagram` header, got {line:?}"
                )));
            }
            header_seen = true;
            continue;
        }

        // Inside an entity block: handle the closing brace and
        // attribute rows.
        if let Some(entity_idx) = current_entity {
            if line == "}" {
                current_entity = None;
                continue;
            }
            let attribute = parse_attribute_row(line)?;
            diag.entities[entity_idx].attributes.push(attribute);
            continue;
        }

        // Outside any entity block: expect either a relationship line,
        // an entity-block opener, or `}` (which would be unexpected).
        if line == "}" {
            return Err(Error::ParseError(
                "stray `}` outside any entity block".to_string(),
            ));
        }

        // Entity-block opener: `<NAME> {` on a single line, or just
        // `<NAME> {` followed by attributes on subsequent lines.
        if let Some(name_part) = line.strip_suffix('{') {
            let name = name_part.trim();
            if name.is_empty() {
                return Err(Error::ParseError(
                    "entity block opener missing entity name".to_string(),
                ));
            }
            let idx = diag.ensure_entity(name);
            current_entity = Some(idx);
            continue;
        }

        // Otherwise the line must be a relationship — `A <card>--<card> B [: label]`.
        let rel = parse_relationship_line(line)?;
        diag.ensure_entity(&rel.from);
        diag.ensure_entity(&rel.to);
        diag.relationships.push(rel);
    }

    if !header_seen {
        return Err(Error::ParseError(
            "missing `erDiagram` header line".to_string(),
        ));
    }
    if let Some(idx) = current_entity {
        return Err(Error::ParseError(format!(
            "unclosed entity block for `{}` (missing `}}`)",
            diag.entities[idx].name
        )));
    }
    Ok(diag)
}

/// Parse one attribute row inside an entity block.
///
/// Format: `<type> <name> [KEY ...] ["comment"]`. Keys are
/// whitespace-separated tokens drawn from `PK`, `FK`, `UK` (any
/// other token is rejected). The trailing comment is a single
/// double-quoted string.
fn parse_attribute_row(line: &str) -> Result<Attribute, Error> {
    // Split off the trailing quoted comment, if any. The comment is
    // the LAST `"…"` segment on the line — we scan from the right so
    // a comment containing other quoted material doesn't trip us.
    let (head, comment) = match line.rfind('"') {
        Some(close) if close > 0 => match line[..close].rfind('"') {
            Some(open) => (
                line[..open].trim_end(),
                Some(line[open + 1..close].to_string()),
            ),
            None => (line, None),
        },
        _ => (line, None),
    };

    let mut tokens = head.split_whitespace();
    let type_name = tokens.next().ok_or_else(|| {
        Error::ParseError(format!("attribute row missing type: {line:?}"))
    })?;
    let name = tokens.next().ok_or_else(|| {
        Error::ParseError(format!("attribute row missing name: {line:?}"))
    })?;

    let mut keys = Vec::new();
    for tok in tokens {
        // Mermaid permits comma-separated key lists like `PK,UK`.
        for piece in tok.split(',') {
            let piece = piece.trim();
            if piece.is_empty() {
                continue;
            }
            keys.push(parse_attribute_key(piece, line)?);
        }
    }

    Ok(Attribute {
        type_name: type_name.to_string(),
        name: name.to_string(),
        keys,
        comment,
    })
}

fn parse_attribute_key(token: &str, line: &str) -> Result<AttributeKey, Error> {
    match token {
        "PK" => Ok(AttributeKey::PrimaryKey),
        "FK" => Ok(AttributeKey::ForeignKey),
        "UK" => Ok(AttributeKey::UniqueKey),
        other => Err(Error::ParseError(format!(
            "unknown attribute key {other:?} (expected PK / FK / UK) in {line:?}"
        ))),
    }
}

/// Parse a relationship line of the form `A <card>(--|..)<card> B [: label]`.
///
/// The cardinality halves are 1- or 2-character tokens drawn from
/// the table in [`parse_cardinality_pair`]. Whitespace between the
/// entity names and the cardinality block is required for
/// disambiguation; whitespace between the two cardinality halves is
/// not (Mermaid concatenates `||--o{`).
fn parse_relationship_line(line: &str) -> Result<Relationship, Error> {
    // Split off the optional `: label` suffix first so the connector
    // scan below doesn't accidentally land in the label text.
    let (head, label) = match line.split_once(':') {
        Some((h, t)) => (h.trim_end(), Some(t.trim().trim_matches('"').to_string())),
        None => (line, None),
    };

    // Find the connector (`--` or `..`) — the only two-character
    // sequence that isn't a cardinality glyph.
    let (connector_pos, line_style) = find_connector(head).ok_or_else(|| {
        Error::ParseError(format!(
            "relationship line missing `--` or `..` connector: {line:?}"
        ))
    })?;

    let left_block = head[..connector_pos].trim_end();
    let right_block = head[connector_pos + 2..].trim_start();

    // Left block: `<from-entity> <left-cardinality>` (whitespace-separated).
    let (from_name, left_card_str) = split_last_token(left_block).ok_or_else(|| {
        Error::ParseError(format!(
            "left side missing entity name + cardinality: {line:?}"
        ))
    })?;
    let from_cardinality = parse_left_cardinality(left_card_str, line)?;

    // Right block: `<right-cardinality> <to-entity>`.
    let (right_card_str, to_name) = split_first_token(right_block).ok_or_else(|| {
        Error::ParseError(format!(
            "right side missing cardinality + entity name: {line:?}"
        ))
    })?;
    let to_cardinality = parse_right_cardinality(right_card_str, line)?;

    Ok(Relationship {
        from: from_name.to_string(),
        to: to_name.to_string(),
        from_cardinality,
        to_cardinality,
        line_style,
        label,
    })
}

/// Find the connector inside a relationship's pre-label text. Returns
/// `(byte_offset, LineStyle)` of the first `--` or `..` occurrence.
/// Both characters of the connector come from a tiny alphabet; no
/// quoting concerns inside the relationship line.
fn find_connector(s: &str) -> Option<(usize, LineStyle)> {
    let bytes = s.as_bytes();
    for i in 0..bytes.len().saturating_sub(1) {
        match (bytes[i], bytes[i + 1]) {
            (b'-', b'-') => return Some((i, LineStyle::Identifying)),
            (b'.', b'.') => return Some((i, LineStyle::NonIdentifying)),
            _ => {}
        }
    }
    None
}

/// Mermaid's left-side cardinality halves (read source-to-connector):
/// the "many" notations point INTO the connector, so `}|`/`}o` mean
/// many at the source. The `|` and `o` halves are the singular forms.
fn parse_left_cardinality(token: &str, line: &str) -> Result<Cardinality, Error> {
    match token {
        "||" => Ok(Cardinality::ExactlyOne),
        "|o" => Ok(Cardinality::ZeroOrOne),
        "}|" => Ok(Cardinality::OneOrMany),
        "}o" => Ok(Cardinality::ZeroOrMany),
        other => Err(Error::ParseError(format!(
            "invalid left-side cardinality {other:?} (expected ||, |o, }}|, or }}o) in {line:?}"
        ))),
    }
}

/// Right-side cardinality halves (read connector-to-target): the
/// "many" notations point OUT of the connector, so `|{`/`o{` mean
/// many at the target.
fn parse_right_cardinality(token: &str, line: &str) -> Result<Cardinality, Error> {
    match token {
        "||" => Ok(Cardinality::ExactlyOne),
        "o|" => Ok(Cardinality::ZeroOrOne),
        "|{" => Ok(Cardinality::OneOrMany),
        "o{" => Ok(Cardinality::ZeroOrMany),
        other => Err(Error::ParseError(format!(
            "invalid right-side cardinality {other:?} (expected ||, o|, |{{, or o{{) in {line:?}"
        ))),
    }
}

/// Split off the last whitespace-delimited token from `s`. Returns
/// `(everything_before_last_token, last_token)` with whitespace
/// trimmed. Returns `None` if `s` has no tokens or only one (caller
/// needs both halves).
fn split_last_token(s: &str) -> Option<(&str, &str)> {
    let trimmed = s.trim_end();
    let last_space = trimmed.rfind(char::is_whitespace)?;
    let head = trimmed[..last_space].trim_end();
    let tail = trimmed[last_space + 1..].trim_start();
    if head.is_empty() || tail.is_empty() {
        return None;
    }
    Some((head, tail))
}

/// Split off the first whitespace-delimited token from `s`. Mirror of
/// [`split_last_token`].
fn split_first_token(s: &str) -> Option<(&str, &str)> {
    let trimmed = s.trim_start();
    let first_space = trimmed.find(char::is_whitespace)?;
    let head = trimmed[..first_space].trim_end();
    let tail = trimmed[first_space + 1..].trim_start();
    if head.is_empty() || tail.is_empty() {
        return None;
    }
    Some((head, tail))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_with_header_only_errors() {
        // Empty body is fine — header alone produces an empty diagram.
        let diag = parse("erDiagram").unwrap();
        assert!(diag.entities.is_empty());
        assert!(diag.relationships.is_empty());
    }

    #[test]
    fn parse_missing_header_errors() {
        let err = parse("CUSTOMER ||--o{ ORDER").unwrap_err();
        assert!(err.to_string().contains("erDiagram"));
    }

    #[test]
    fn parse_one_relationship_creates_two_entities() {
        let diag = parse("erDiagram\nCUSTOMER ||--o{ ORDER : places").unwrap();
        assert_eq!(diag.entities.len(), 2);
        assert_eq!(diag.entities[0].name, "CUSTOMER");
        assert_eq!(diag.entities[1].name, "ORDER");
        assert_eq!(diag.relationships.len(), 1);
        let r = &diag.relationships[0];
        assert_eq!(r.from, "CUSTOMER");
        assert_eq!(r.to, "ORDER");
        assert_eq!(r.from_cardinality, Cardinality::ExactlyOne);
        assert_eq!(r.to_cardinality, Cardinality::ZeroOrMany);
        assert_eq!(r.line_style, LineStyle::Identifying);
        assert_eq!(r.label.as_deref(), Some("places"));
    }

    #[test]
    fn parse_all_cardinality_codes_round_trip() {
        let diag = parse(
            "erDiagram\n\
             A ||--|| B : exact\n\
             A |o--o| B : optional\n\
             A }|--|{ B : many\n\
             A }o--o{ B : optionalMany",
        )
        .unwrap();
        assert_eq!(diag.relationships[0].from_cardinality, Cardinality::ExactlyOne);
        assert_eq!(diag.relationships[0].to_cardinality, Cardinality::ExactlyOne);
        assert_eq!(diag.relationships[1].from_cardinality, Cardinality::ZeroOrOne);
        assert_eq!(diag.relationships[1].to_cardinality, Cardinality::ZeroOrOne);
        assert_eq!(diag.relationships[2].from_cardinality, Cardinality::OneOrMany);
        assert_eq!(diag.relationships[2].to_cardinality, Cardinality::OneOrMany);
        assert_eq!(diag.relationships[3].from_cardinality, Cardinality::ZeroOrMany);
        assert_eq!(diag.relationships[3].to_cardinality, Cardinality::ZeroOrMany);
    }

    #[test]
    fn parse_non_identifying_line_style() {
        let diag = parse("erDiagram\nA ||..o{ B").unwrap();
        assert_eq!(
            diag.relationships[0].line_style,
            LineStyle::NonIdentifying
        );
    }

    #[test]
    fn parse_relationship_without_label() {
        let diag = parse("erDiagram\nA ||--o{ B").unwrap();
        assert!(diag.relationships[0].label.is_none());
    }

    #[test]
    fn parse_quoted_label_strips_quotes() {
        let diag = parse(
            "erDiagram\nCUSTOMER ||--o{ ORDER : \"places multiple\"",
        )
        .unwrap();
        assert_eq!(
            diag.relationships[0].label.as_deref(),
            Some("places multiple")
        );
    }

    #[test]
    fn parse_entity_block_with_attributes() {
        let diag = parse(
            "erDiagram\n\
             CUSTOMER {\n\
               string name\n\
               string email PK\n\
               int age FK,UK\n\
             }",
        )
        .unwrap();
        assert_eq!(diag.entities.len(), 1);
        let e = &diag.entities[0];
        assert_eq!(e.name, "CUSTOMER");
        assert_eq!(e.attributes.len(), 3);
        assert_eq!(e.attributes[0].type_name, "string");
        assert_eq!(e.attributes[0].name, "name");
        assert!(e.attributes[0].keys.is_empty());
        assert_eq!(e.attributes[1].keys, vec![AttributeKey::PrimaryKey]);
        assert_eq!(
            e.attributes[2].keys,
            vec![AttributeKey::ForeignKey, AttributeKey::UniqueKey]
        );
    }

    #[test]
    fn parse_attribute_with_comment() {
        let diag = parse(
            "erDiagram\nA {\n  string id PK \"the unique identifier\"\n}",
        )
        .unwrap();
        let a = &diag.entities[0].attributes[0];
        assert_eq!(a.comment.as_deref(), Some("the unique identifier"));
        assert_eq!(a.keys, vec![AttributeKey::PrimaryKey]);
    }

    #[test]
    fn parse_unknown_attribute_key_errors() {
        let err = parse("erDiagram\nA {\n  string foo XX\n}").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("XX") && msg.contains("PK"));
    }

    #[test]
    fn parse_unclosed_entity_block_errors() {
        let err = parse("erDiagram\nA {\n  string name").unwrap_err();
        assert!(err.to_string().contains("unclosed"));
    }

    #[test]
    fn parse_stray_close_brace_errors() {
        let err = parse("erDiagram\n}").unwrap_err();
        assert!(err.to_string().contains("stray"));
    }

    #[test]
    fn parse_missing_connector_errors() {
        let err = parse("erDiagram\nA || o{ B").unwrap_err();
        assert!(err.to_string().contains("connector"));
    }

    #[test]
    fn parse_invalid_left_cardinality_errors() {
        let err = parse("erDiagram\nA xy--o{ B").unwrap_err();
        assert!(err.to_string().contains("left-side"));
    }

    #[test]
    fn parse_skips_comments_and_blanks() {
        let diag = parse(
            "%% header comment\n\
             erDiagram\n\
             \n\
             %% middle comment\n\
             A ||--|| B",
        )
        .unwrap();
        assert_eq!(diag.relationships.len(), 1);
    }

    #[test]
    fn parse_entity_referenced_in_relationship_then_declared_keeps_attributes() {
        // Forward reference: relationship mentions ORDER before its
        // body. The entity should be created bare, then later
        // populated when the body appears.
        let diag = parse(
            "erDiagram\n\
             CUSTOMER ||--o{ ORDER : places\n\
             ORDER {\n  int orderNumber PK\n}",
        )
        .unwrap();
        let order_idx = diag.entity_index("ORDER").unwrap();
        assert_eq!(diag.entities[order_idx].attributes.len(), 1);
    }
}
