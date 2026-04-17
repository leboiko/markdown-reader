//! Hand-rolled parser for Mermaid `graph`/`flowchart` syntax.
//!
//! The parser works statement-by-statement. A "statement" is one logical
//! declaration separated by a newline or semicolon. Each statement is
//! classified as either:
//!
//! - A **node definition**: `A[Label]`, `A{Label}`, `A((Label))`, `A(Label)`, or bare `A`
//! - An **edge chain**: `A --> B --> C`, potentially with inline labels
//! - A **header line**: `graph LR` / `flowchart TD` (handled before entering this module)
//! - A blank / comment line — silently ignored
//!
//! All edge types (`-->`, `---`, `-.->`, `==>`) are treated identically in
//! Phase 1 (rendered as solid arrows).

use crate::{
    Error,
    types::{Direction, Edge, Graph, Node, NodeShape},
};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Parse a Mermaid `graph`/`flowchart` source string into a [`Graph`].
///
/// The function expects the *full* input including the header line
/// (`graph LR`, `flowchart TD`, etc.). Both newlines and semicolons are
/// treated as statement separators, so `graph LR; A-->B` is valid.
///
/// # Errors
///
/// Returns [`Error::ParseError`] if the header statement is missing or the
/// direction keyword is unrecognised.
pub fn parse(input: &str) -> Result<Graph, Error> {
    // Normalise: replace newlines with semicolons, then split on ';'.
    // This means both `graph LR; A-->B` and multi-line input are handled
    // identically — the first non-blank, non-comment statement is the header.
    let normalised = input.replace('\n', ";").replace('\r', "");

    let mut statements = normalised
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with("%%"));

    // ---- Find and parse the header statement ----------------------------
    let direction = parse_header_stmt(&mut statements)?;
    let mut graph = Graph::new(direction);

    // ---- Parse each remaining statement ---------------------------------
    for stmt in statements {
        parse_statement(stmt, &mut graph);
    }

    Ok(graph)
}

// ---------------------------------------------------------------------------
// Header parsing
// ---------------------------------------------------------------------------

/// Consume the first statement from `stmts` and parse it as a
/// `graph`/`flowchart` header, returning the [`Direction`].
///
/// The direction is the first whitespace-delimited token after the keyword.
fn parse_header_stmt<'a>(stmts: &mut impl Iterator<Item = &'a str>) -> Result<Direction, Error> {
    let stmt = stmts
        .next()
        .ok_or_else(|| Error::ParseError("no 'graph'/'flowchart' header found".to_string()))?;

    // e.g. "graph LR" or "flowchart TD"
    let mut parts = stmt.splitn(3, |c: char| c.is_whitespace());
    let keyword = parts.next().unwrap_or("").to_lowercase();

    if keyword != "graph" && keyword != "flowchart" {
        return Err(Error::ParseError(format!(
            "expected 'graph' or 'flowchart', got '{keyword}'"
        )));
    }

    // The direction is the next whitespace-separated token (just the first
    // word — we ignore any trailing content on the header line since we
    // already split on semicolons above).
    let dir_str = parts
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("TD"); // default to top-down if omitted

    Direction::parse(dir_str)
        .ok_or_else(|| Error::ParseError(format!("unknown direction '{dir_str}'")))
}

// ---------------------------------------------------------------------------
// Statement parsing
// ---------------------------------------------------------------------------

/// Parse a single statement (already trimmed, no leading/trailing whitespace).
///
/// A statement is either a standalone node definition or an edge chain that
/// may include inline node definitions.
///
/// Any nodes referenced in edges are auto-created if they have not been
/// explicitly defined yet.
fn parse_statement(stmt: &str, graph: &mut Graph) {
    // Skip mermaid keywords that are not node definitions or edge chains.
    // These appear inside subgraph blocks, style directives, etc.
    let first_word = stmt.split_whitespace().next().unwrap_or("");
    if matches!(
        first_word,
        "subgraph" | "end" | "direction" | "style" | "classDef" | "class"
            | "click" | "linkStyle" | "accTitle" | "accDescr"
    ) {
        return;
    }

    // Try to parse as an edge chain first (contains an arrow token).
    // Edge chains look like: A --> B  or  A -->|label| B --> C
    if looks_like_edge_chain(stmt) {
        parse_edge_chain(stmt, graph);
    } else {
        // Pure node definition: A[label], A{label}, A((label)), A(label), A
        if let Some(node) = parse_node_definition(stmt) {
            graph.upsert_node(node);
        }
    }
}

/// Return `true` if the statement appears to contain at least one edge arrow.
fn looks_like_edge_chain(s: &str) -> bool {
    // Quick scan: any of -->, ---, -.->, ==> or their variants
    s.contains("-->")
        || s.contains("---")
        || s.contains("-.->")
        || s.contains("==>")
        || s.contains("-- ") // "-- label -->" form
}

// ---------------------------------------------------------------------------
// Edge chain parsing
// ---------------------------------------------------------------------------

/// Parse an edge chain statement and push nodes + edges into `graph`.
///
/// The chain is tokenised by splitting on edge markers while preserving
/// edge-label content between `|...|` delimiters.
fn parse_edge_chain(stmt: &str, graph: &mut Graph) {
    // We build a list of (node_token, edge_label_or_none) pairs.
    // Strategy: walk char-by-char, extracting alternating node/edge segments.

    let tokens = tokenise_chain(stmt);
    if tokens.is_empty() {
        return;
    }

    // tokens = [node_tok, edge_tok, node_tok, edge_tok, node_tok, ...]
    // Odd indices are node tokens, even indices are edge (arrow+label) tokens.
    // Actually our tokeniser returns: node, arrow, node, arrow, node
    // i.e. length is always odd and ≥ 1.

    // Collect (node_token, Option<edge_label_before_next_node>) pairs.
    // We iterate pairs of (node_tok, Option<arrow_tok>).
    let mut i = 0;
    let mut prev_id: Option<String> = None;
    let mut pending_edge_label: Option<String> = None;

    while i < tokens.len() {
        let tok = tokens[i].trim();

        if i % 2 == 0 {
            // Node token
            if tok.is_empty() {
                i += 1;
                continue;
            }
            let node = parse_node_definition(tok).unwrap_or_else(|| {
                // Treat as bare ID
                Node::new(tok, tok, NodeShape::Rectangle)
            });
            let node_id = node.id.clone();
            graph.upsert_node(node);

            if let Some(ref from) = prev_id {
                let edge = Edge::new(from.clone(), node_id.clone(), pending_edge_label.take());
                graph.edges.push(edge);
            }
            prev_id = Some(node_id);
        } else {
            // Arrow token — extract optional label
            pending_edge_label = extract_arrow_label(tok);
        }

        i += 1;
    }
}

/// Split a chain statement into alternating node/arrow tokens.
///
/// Returns a `Vec<String>` where even indices are node tokens and odd indices
/// are arrow tokens (including any `|label|` portion).
fn tokenise_chain(stmt: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let chars: Vec<char> = stmt.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut current = String::new();

    while i < len {
        // Detect start of an arrow sequence.
        // Arrows: -->, ---, -.->  ==>, -- label -->, -->|label|
        // We look for `-` or `=` not inside a node bracket.
        let ch = chars[i];

        if (ch == '-' || ch == '=') && !current.trim().is_empty() {
            // Peek ahead to see if this is really an arrow
            if is_arrow_start(&chars, i) {
                // Push the current node token
                tokens.push(current.trim().to_string());
                current = String::new();

                // Consume the full arrow (including optional |label|)
                let (arrow_tok, consumed) = consume_arrow(&chars, i);
                tokens.push(arrow_tok);
                i += consumed;
                continue;
            }
        }

        current.push(ch);
        i += 1;
    }

    // Push the last node token
    let last = current.trim().to_string();
    if !last.is_empty() {
        tokens.push(last);
    }

    tokens
}

/// Return `true` if position `i` in `chars` starts an arrow sequence.
fn is_arrow_start(chars: &[char], i: usize) -> bool {
    let remaining: String = chars[i..].iter().collect();
    remaining.starts_with("-->")
        || remaining.starts_with("---")
        || remaining.starts_with("-.->")
        || remaining.starts_with("==>")
        || remaining.starts_with("-- ") // "-- label -->"
        || remaining.starts_with("--")
}

/// Consume an arrow starting at position `i`, returning `(arrow_token, chars_consumed)`.
///
/// Handles these forms:
/// - `-->` / `-->|label|`
/// - `---`
/// - `-.->` / `-.->|label|`
/// - `==>`
/// - `-- label -->`
fn consume_arrow(chars: &[char], start: usize) -> (String, usize) {
    let remaining: String = chars[start..].iter().collect();

    // "-- label -->" form  (must check before plain "--")
    if let Some(arrow) = try_consume_labeled_dash_arrow(&remaining) {
        let len = arrow.chars().count();
        return (arrow, len);
    }

    // "-.->"|label|?
    if remaining.starts_with("-.-") {
        let base = if remaining.starts_with("-.->") { 4 } else { 3 };
        let (label_part, extra) = try_consume_pipe_label(&remaining[base..]);
        let tok = format!("{}{label_part}", &remaining[..base]);
        return (tok, base + extra);
    }

    // "==>"
    if let Some(rest) = remaining.strip_prefix("==>") {
        let (label_part, extra) = try_consume_pipe_label(rest);
        let tok = format!("==>{label_part}");
        return (tok, 3 + extra);
    }

    // "-->" / "---"
    if let Some(rest) = remaining.strip_prefix("-->") {
        let (label_part, extra) = try_consume_pipe_label(rest);
        let tok = format!("-->{label_part}");
        return (tok, 3 + extra);
    }
    if let Some(rest) = remaining.strip_prefix("---") {
        let (label_part, extra) = try_consume_pipe_label(rest);
        let tok = format!("---{label_part}");
        return (tok, 3 + extra);
    }
    // Fallback: consume "--"
    (remaining[..2].to_string(), 2)
}

/// Try to parse `-- label -->` form. Returns the full token string if matched.
fn try_consume_labeled_dash_arrow(s: &str) -> Option<String> {
    // Must start with "-- " (dash dash space)
    if !s.starts_with("-- ") {
        return None;
    }
    // Find closing "-->"
    let rest = &s[3..];
    rest.find("-->").map(|end| {
        let full_len = 3 + end + 3; // "-- " + label + "-->"
        s[..full_len].to_string()
    })
}

/// Try to consume a `|label|` suffix. Returns `(consumed_string, char_count)`.
fn try_consume_pipe_label(s: &str) -> (String, usize) {
    if let Some(inner) = s.strip_prefix('|')
        && let Some(end) = inner.find('|')
    {
        let portion = &s[..end + 2]; // includes both pipes
        return (portion.to_string(), end + 2);
    }
    (String::new(), 0)
}

/// Extract a label string from an arrow token, if present.
///
/// Handles `-->|label|`, `-- label -->`, etc.
fn extract_arrow_label(arrow: &str) -> Option<String> {
    // Pipe-style: -->|label| or -.->|label|
    if let Some(start) = arrow.find('|')
        && let Some(end) = arrow[start + 1..].find('|')
    {
        let label = arrow[start + 1..start + 1 + end].trim().to_string();
        if !label.is_empty() {
            return Some(label);
        }
    }
    // Dash-style: -- label -->
    if arrow.starts_with("-- ")
        && let Some(end) = arrow.rfind("-->")
    {
        let label = arrow[3..end].trim().to_string();
        if !label.is_empty() {
            return Some(label);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Node definition parsing
// ---------------------------------------------------------------------------

/// Parse a single node-definition token such as `A[Label]`, `B{text}`,
/// `C((name))`, `D(rounded)`, or bare `E`.
///
/// Returns `None` if the token is empty or unparseable.
pub fn parse_node_definition(token: &str) -> Option<Node> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }

    // Find the first bracket/brace/paren character to split id from shape.
    let shape_start = token.find(['[', '{', '(']);

    let (id, label, shape) = if let Some(pos) = shape_start {
        let id = token[..pos].trim().to_string();
        let rest = &token[pos..];

        if rest.starts_with("((") && rest.ends_with("))") {
            // Circle: A((text))
            let inner = rest[2..rest.len() - 2].trim().to_string();
            (id, inner, NodeShape::Circle)
        } else if rest.starts_with('{') && rest.ends_with('}') {
            // Diamond: A{text}
            let inner = rest[1..rest.len() - 1].trim().to_string();
            (id, inner, NodeShape::Diamond)
        } else if rest.starts_with('[') && rest.ends_with(']') {
            // Rectangle: A[text]
            let inner = rest[1..rest.len() - 1].trim().to_string();
            (id, inner, NodeShape::Rectangle)
        } else if rest.starts_with('(') && rest.ends_with(')') {
            // Rounded: A(text)
            let inner = rest[1..rest.len() - 1].trim().to_string();
            (id, inner, NodeShape::Rounded)
        } else {
            // Unrecognised bracket pattern — treat entire token as bare ID
            let id = token.to_string();
            (id.clone(), id, NodeShape::Rectangle)
        }
    } else {
        // Bare ID
        (token.to_string(), token.to_string(), NodeShape::Rectangle)
    };

    if id.is_empty() {
        return None;
    }

    // Strip HTML-like line breaks that Mermaid supports in labels.
    let label = label
        .replace("<br/>", " ")
        .replace("<br>", " ")
        .replace("<br />", " ");

    Some(Node::new(id, label, shape))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NodeShape;

    #[test]
    fn parse_simple_lr() {
        let g = parse("graph LR\nA-->B-->C").unwrap();
        assert_eq!(g.direction, Direction::LeftToRight);
        assert!(g.has_node("A"));
        assert!(g.has_node("B"));
        assert!(g.has_node("C"));
        assert_eq!(g.edges.len(), 2);
    }

    #[test]
    fn parse_semicolons() {
        let g = parse("graph LR; A-->B; B-->C").unwrap();
        assert_eq!(g.edges.len(), 2);
    }

    #[test]
    fn parse_labeled_nodes() {
        let g = parse("graph LR\nA[Start] --> B[End]").unwrap();
        assert_eq!(g.node("A").unwrap().label, "Start");
        assert_eq!(g.node("B").unwrap().label, "End");
    }

    #[test]
    fn parse_diamond_node() {
        let g = parse("graph LR\nA{Decision}").unwrap();
        assert_eq!(g.node("A").unwrap().shape, NodeShape::Diamond);
        assert_eq!(g.node("A").unwrap().label, "Decision");
    }

    #[test]
    fn parse_circle_node() {
        let g = parse("graph LR\nA((Circle))").unwrap();
        assert_eq!(g.node("A").unwrap().shape, NodeShape::Circle);
    }

    #[test]
    fn parse_rounded_node() {
        let g = parse("graph LR\nA(Rounded)").unwrap();
        assert_eq!(g.node("A").unwrap().shape, NodeShape::Rounded);
    }

    #[test]
    fn parse_edge_label_pipe() {
        let g = parse("graph LR\nA -->|yes| B").unwrap();
        assert_eq!(g.edges[0].label.as_deref(), Some("yes"));
    }

    #[test]
    fn parse_edge_label_dash() {
        let g = parse("graph LR\nA -- hello --> B").unwrap();
        assert_eq!(g.edges[0].label.as_deref(), Some("hello"));
    }

    #[test]
    fn parse_flowchart_keyword() {
        let g = parse("flowchart TD\nA-->B").unwrap();
        assert_eq!(g.direction, Direction::TopToBottom);
    }

    #[test]
    fn bad_direction_returns_error() {
        assert!(parse("graph XY\nA-->B").is_err());
    }

    #[test]
    fn no_header_returns_error() {
        assert!(parse("A-->B").is_err());
    }
}
