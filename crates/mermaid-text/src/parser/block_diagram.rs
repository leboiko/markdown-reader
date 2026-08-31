//! Parser for Mermaid `block-beta` (and `block`) diagrams.
//!
//! Accepted syntax (subset — Phase 1):
//!
//! ```text
//! block-beta
//!     columns 3
//!     a["A label"] b:2 c
//!     d e f
//!     g["spans across"]:3
//!     A --> B
//!     B --> C
//! ```
//!
//! Rules:
//! - `block-beta` or `block` keyword is required as the first non-blank,
//!   non-comment line.
//! - `columns N` — sets the grid column count for subsequent block rows.
//!   When absent, the column count defaults to 1.
//! - Block specs are space-separated tokens on a line. Each token may be:
//!   - `id` — bare identifier, default rectangle, col_span = 1.
//!   - `id["text"]` — block with display text, col_span = 1.
//!   - `id:N` — bare identifier, col_span = N.
//!   - `id["text"]:N` — display text with col_span = N.
//! - `<source> --> <target>` — a directed edge. The source and target are
//!   whitespace-trimmed identifiers. An optional `|label|` suffix is captured.
//! - `%%` comment lines, blank lines, and `accTitle`/`accDescr` lines are
//!   silently skipped.
//! - Unknown lines (e.g. nested `block … end` blocks, style directives) are
//!   silently ignored for forward compatibility.
//!
//! # Examples
//!
//! ```
//! use mermaid_text::parser::block_diagram::parse;
//!
//! let diag = parse("block-beta\n    columns 2\n    A B\n    A --> B").unwrap();
//! assert_eq!(diag.columns, 2);
//! assert_eq!(diag.blocks.len(), 2);
//! assert_eq!(diag.edges.len(), 1);
//! assert_eq!(diag.edges[0].source, "A");
//! assert_eq!(diag.edges[0].target, "B");
//! ```

use crate::Error;
use crate::block_diagram::{Block, BlockDiagram, BlockEdge};
use crate::parser::common::strip_inline_comment;

/// Parse a `block-beta` source string into a [`BlockDiagram`].
///
/// # Errors
///
/// - [`Error::ParseError`] — missing `block-beta`/`block` header, invalid
///   `columns N` directive, or malformed edge syntax.
pub fn parse(src: &str) -> Result<BlockDiagram, Error> {
    let mut header_seen = false;
    let mut diag = BlockDiagram {
        columns: 1,
        ..Default::default()
    };
    // Track whether `columns N` was explicitly set.
    let mut columns_set = false;
    // Track nesting depth so we can skip nested block declarations.
    let mut nest_depth: usize = 0;

    for raw in src.lines() {
        let stripped = strip_inline_comment(raw);
        let trimmed = stripped.trim();

        if !header_seen {
            if trimmed.is_empty() || trimmed.starts_with("%%") {
                continue;
            }
            let keyword = trimmed.split_whitespace().next().unwrap_or("");
            if keyword.eq_ignore_ascii_case("block-beta") || keyword.eq_ignore_ascii_case("block") {
                header_seen = true;
                continue;
            }
            return Err(Error::ParseError(format!(
                "expected `block-beta` or `block` header, got {trimmed:?}"
            )));
        }

        // Skip blank and comment lines.
        if trimmed.is_empty() || trimmed.starts_with("%%") {
            continue;
        }

        // Silently skip accessibility metadata.
        if trimmed.starts_with("accTitle") || trimmed.starts_with("accDescr") {
            continue;
        }

        // Track nested block … end blocks (skip their content).
        if trimmed.eq_ignore_ascii_case("end") {
            nest_depth = nest_depth.saturating_sub(1);
            continue;
        }

        // `columns N` directive (only at the top level).
        if let Some(rest) = trimmed
            .strip_prefix("columns ")
            .or_else(|| trimmed.strip_prefix("columns\t"))
        {
            if nest_depth == 0 {
                let n_str = rest.trim();
                let n = n_str.parse::<usize>().map_err(|_| {
                    Error::ParseError(format!(
                        "invalid `columns` value {n_str:?}: must be a positive integer"
                    ))
                })?;
                if n == 0 {
                    return Err(Error::ParseError(
                        "`columns 0` is invalid — column count must be ≥ 1".to_string(),
                    ));
                }
                diag.columns = n;
                columns_set = true;
            }
            continue;
        }

        // Detect start of a nested block declaration — skip content until `end`.
        // Nested blocks look like `block <id>` or just `block` on a line.
        {
            let first_token = trimmed.split_whitespace().next().unwrap_or("");
            if first_token.eq_ignore_ascii_case("block") {
                nest_depth += 1;
                continue;
            }
        }

        // Skip lines inside nested blocks.
        if nest_depth > 0 {
            continue;
        }

        // Detect a directed edge: `<source> --> <target>` (optionally `|label|`).
        if let Some(edge) = try_parse_edge(trimmed)? {
            diag.edges.push(edge);
            continue;
        }

        // Otherwise treat the line as a row of block specs.
        let blocks = parse_block_row(trimmed)?;
        diag.blocks.extend(blocks);
    }

    if !header_seen {
        return Err(Error::ParseError(
            "missing `block-beta` or `block` header line".to_string(),
        ));
    }

    // If no `columns` directive was given and there are blocks, default to 1.
    if !columns_set && !diag.blocks.is_empty() {
        diag.columns = 1;
    }

    Ok(diag)
}

/// Try to parse a directed edge from a line of the form `A --> B` or `A -->|label| B`.
///
/// Returns `Ok(Some(edge))` on match, `Ok(None)` when the line is not an edge,
/// and `Err(...)` when the line structurally looks like an edge but is malformed.
fn try_parse_edge(line: &str) -> Result<Option<BlockEdge>, Error> {
    // Quick check: the line must contain `-->`.
    let Some(arrow_pos) = line.find("-->") else {
        return Ok(None);
    };

    let source = line[..arrow_pos].trim().to_string();
    if source.is_empty() {
        return Err(Error::ParseError(format!(
            "edge has empty source in {line:?}"
        )));
    }

    // Everything after `-->`.
    let after_arrow = line[arrow_pos + 3..].trim();

    // Check for optional `|label| target` form.
    let (label, target_str) = if let Some(rest) = after_arrow.strip_prefix('|') {
        // This grammar has one edge per line, so the final pipe closes the
        // label and any earlier pipes are label content.
        if let Some(close) = rest.rfind('|') {
            let lbl = rest[..close].trim().to_string();
            let target_part = rest[close + 1..].trim().to_string();
            (Some(lbl), target_part)
        } else {
            // Malformed label — treat entire remainder as target, no label.
            (None, after_arrow.to_string())
        }
    } else {
        (None, after_arrow.to_string())
    };

    let target = target_str.trim().to_string();
    if target.is_empty() {
        return Err(Error::ParseError(format!(
            "edge has empty target in {line:?}"
        )));
    }

    Ok(Some(BlockEdge {
        source,
        target,
        label,
    }))
}

/// Parse a single row of space-separated block specifications.
///
/// Each token in the row is one block spec in one of these forms:
/// - `id`
/// - `id["text"]`
/// - `id:N`
/// - `id["text"]:N`
///
/// Tokens are split on ASCII whitespace; quoted text inside `["…"]` is
/// respected so that labels with spaces are captured correctly. The parse
/// scans character-by-character to handle embedded quotes and colons.
fn parse_block_row(line: &str) -> Result<Vec<Block>, Error> {
    let mut blocks = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut pos = 0;

    while pos < len {
        // Skip leading whitespace between tokens.
        while pos < len && chars[pos].is_whitespace() {
            pos += 1;
        }
        if pos >= len {
            break;
        }

        // Read the identifier (stops at `[`, `:`, or whitespace).
        let id_start = pos;
        while pos < len && chars[pos] != '[' && chars[pos] != ':' && !chars[pos].is_whitespace() {
            pos += 1;
        }
        let id: String = chars[id_start..pos].iter().collect();
        if id.is_empty() {
            // Shouldn't happen after skipping whitespace, but be safe.
            break;
        }

        // Check for optional `["text"]` label.
        let text = if pos < len && chars[pos] == '[' {
            // Consume `[`.
            pos += 1;
            if pos < len && chars[pos] == '"' {
                // Consume `"`.
                pos += 1;
                let text_start = pos;
                // Read until the closing `"`.
                while pos < len && chars[pos] != '"' {
                    pos += 1;
                }
                let t: String = chars[text_start..pos].iter().collect();
                // Consume `"` if present.
                if pos < len {
                    pos += 1;
                }
                // Consume `]` if present.
                if pos < len && chars[pos] == ']' {
                    pos += 1;
                }
                t
            } else {
                // Bare bracket without quote — consume until `]`.
                let text_start = pos;
                while pos < len && chars[pos] != ']' {
                    pos += 1;
                }
                let t: String = chars[text_start..pos].iter().collect();
                if pos < len {
                    pos += 1; // consume `]`
                }
                t
            }
        } else {
            String::new()
        };

        // Check for optional `:N` column span.
        let col_span = if pos < len && chars[pos] == ':' {
            pos += 1; // consume `:`
            let span_start = pos;
            while pos < len && chars[pos].is_ascii_digit() {
                pos += 1;
            }
            let span_str: String = chars[span_start..pos].iter().collect();
            if span_str.is_empty() {
                1
            } else {
                span_str.parse::<usize>().unwrap_or(1).max(1)
            }
        } else {
            1
        };

        blocks.push(Block { id, text, col_span });
    }

    Ok(blocks)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "block-beta\n";

    // --- header detection ---------------------------------------------------

    #[test]
    fn parses_block_beta_header() {
        let src = "block-beta\n    A";
        let diag = parse(src).unwrap();
        assert_eq!(diag.blocks.len(), 1);
    }

    #[test]
    fn parses_block_header_alias() {
        // `block` (without `-beta`) is also accepted.
        let src = "block\n    A";
        let diag = parse(src).unwrap();
        assert_eq!(diag.blocks.len(), 1);
    }

    #[test]
    fn missing_header_returns_error() {
        assert!(parse("A B C").is_err(), "no header should fail");
        assert!(parse("").is_err(), "empty input should fail");
    }

    // --- columns directive --------------------------------------------------

    #[test]
    fn parses_columns_directive() {
        let src = format!("{HEADER}    columns 3\n    A B C");
        let diag = parse(&src).unwrap();
        assert_eq!(diag.columns, 3);
    }

    #[test]
    fn columns_zero_returns_error() {
        let src = format!("{HEADER}    columns 0");
        assert!(parse(&src).is_err(), "columns 0 should fail");
    }

    #[test]
    fn columns_non_integer_returns_error() {
        let src = format!("{HEADER}    columns abc");
        assert!(parse(&src).is_err(), "columns abc should fail");
    }

    #[test]
    fn default_columns_is_one_when_absent() {
        let src = format!("{HEADER}    A\n    B");
        let diag = parse(&src).unwrap();
        assert_eq!(diag.columns, 1);
    }

    // --- block parsing -------------------------------------------------------

    #[test]
    fn parses_bare_block_ids() {
        let src = format!("{HEADER}    columns 3\n    A B C");
        let diag = parse(&src).unwrap();
        assert_eq!(diag.blocks.len(), 3);
        assert_eq!(diag.blocks[0].id, "A");
        assert_eq!(diag.blocks[1].id, "B");
        assert_eq!(diag.blocks[2].id, "C");
        // All have empty text (bare id) and col_span 1.
        for b in &diag.blocks {
            assert_eq!(b.col_span, 1);
            assert!(b.text.is_empty());
        }
    }

    #[test]
    fn parses_block_with_quoted_text() {
        let src = format!("{HEADER}    a[\"A label\"]");
        let diag = parse(&src).unwrap();
        assert_eq!(diag.blocks.len(), 1);
        assert_eq!(diag.blocks[0].id, "a");
        assert_eq!(diag.blocks[0].text, "A label");
        assert_eq!(diag.blocks[0].col_span, 1);
    }

    #[test]
    fn parses_block_with_column_span() {
        let src = format!("{HEADER}    b:2");
        let diag = parse(&src).unwrap();
        assert_eq!(diag.blocks.len(), 1);
        assert_eq!(diag.blocks[0].col_span, 2);
    }

    #[test]
    fn parses_block_with_text_and_span() {
        let src = format!("{HEADER}    g[\"spans across\"]:3");
        let diag = parse(&src).unwrap();
        assert_eq!(diag.blocks.len(), 1);
        assert_eq!(diag.blocks[0].id, "g");
        assert_eq!(diag.blocks[0].text, "spans across");
        assert_eq!(diag.blocks[0].col_span, 3);
    }

    #[test]
    fn parses_multiple_rows() {
        let src = format!("{HEADER}    columns 3\n    a b c\n    d e f");
        let diag = parse(&src).unwrap();
        assert_eq!(diag.blocks.len(), 6);
        assert_eq!(diag.blocks[3].id, "d");
        assert_eq!(diag.blocks[5].id, "f");
    }

    // --- edge parsing --------------------------------------------------------

    #[test]
    fn parses_directed_edge() {
        let src = format!("{HEADER}    A\n    B\n    A --> B");
        let diag = parse(&src).unwrap();
        assert_eq!(diag.edges.len(), 1);
        assert_eq!(diag.edges[0].source, "A");
        assert_eq!(diag.edges[0].target, "B");
        assert!(diag.edges[0].label.is_none());
    }

    #[test]
    fn parses_multiple_edges() {
        let src = format!("{HEADER}    A\n    B\n    C\n    A --> B\n    B --> C");
        let diag = parse(&src).unwrap();
        assert_eq!(diag.edges.len(), 2);
        assert_eq!(diag.edges[0].source, "A");
        assert_eq!(diag.edges[0].target, "B");
        assert_eq!(diag.edges[1].source, "B");
        assert_eq!(diag.edges[1].target, "C");
    }

    #[test]
    fn parses_edge_with_label() {
        let src = format!("{HEADER}    A B\n    A -->|calls| B");
        let diag = parse(&src).unwrap();
        assert_eq!(diag.edges.len(), 1);
        assert_eq!(diag.edges[0].label, Some("calls".to_string()));
    }

    #[test]
    fn parses_pipe_inside_edge_label() {
        let src = format!("{HEADER}    A B\n    A -->|one|two| B");
        let diag = parse(&src).unwrap();
        assert_eq!(diag.edges.len(), 1);
        assert_eq!(diag.edges[0].source, "A");
        assert_eq!(diag.edges[0].target, "B");
        assert_eq!(diag.edges[0].label.as_deref(), Some("one|two"));
    }

    // --- comment and blank line handling -------------------------------------

    #[test]
    fn skips_comment_and_blank_lines() {
        let src = "%% preamble\nblock-beta\n%% inner\n\n    A B";
        let diag = parse(src).unwrap();
        assert_eq!(diag.blocks.len(), 2);
    }

    // --- full example --------------------------------------------------------

    #[test]
    fn parses_canonical_example() {
        let src = "block-beta
    columns 3
    a[\"A label\"] b:2 c
    d e f
    g[\"spans across\"]:3";
        let diag = parse(src).unwrap();
        assert_eq!(diag.columns, 3);
        assert_eq!(diag.blocks.len(), 7);
        // Block `b` spans 2 columns.
        assert_eq!(diag.blocks[1].id, "b");
        assert_eq!(diag.blocks[1].col_span, 2);
        // Block `g` spans 3 columns.
        assert_eq!(diag.blocks[6].id, "g");
        assert_eq!(diag.blocks[6].text, "spans across");
        assert_eq!(diag.blocks[6].col_span, 3);
    }
}
