//! Hand-rolled parser for Mermaid `graph`/`flowchart` syntax.
//!
//! The parser works statement-by-statement. A "statement" is one logical
//! declaration separated by a newline or semicolon. Each statement is
//! classified as either:
//!
//! - A **node definition**: `A[Label]`, `A{Label}`, `A((Label))`, `A(Label)`, or bare `A`
//! - An **edge chain**: `A --> B --> C`, potentially with inline labels
//! - A **subgraph block**: `subgraph ID [Label]` … `end`
//! - A **header line**: `graph LR` / `flowchart TD` (handled before entering this module)
//! - A blank / comment line — silently ignored
//!
//! Edge style (`-->`, `---`, `-.->`, `==>`, `<-->`, `--o`, `--x`) is parsed
//! and stored on each [`Edge`] for the renderer to use.

use crate::{
    Error,
    types::{Direction, Edge, EdgeEndpoint, EdgeStyle, Graph, Node, NodeShape, Subgraph},
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
/// # Arguments
///
/// * `input` — the complete Mermaid source string
///
/// # Returns
///
/// A [`Graph`] containing all parsed nodes, edges, and subgraphs.
///
/// # Errors
///
/// Returns [`crate::Error::ParseError`] if the header statement is missing or
/// the direction keyword is unrecognised.
///
/// # Examples
///
/// ```
/// use mermaid_text::parser::parse;
/// use mermaid_text::{Direction, NodeShape};
///
/// let graph = parse("graph LR; A[Start] --> B[End]").unwrap();
/// assert_eq!(graph.direction, Direction::LeftToRight);
/// assert_eq!(graph.node("A").unwrap().label, "Start");
/// assert_eq!(graph.node("B").unwrap().shape, NodeShape::Rectangle);
/// assert_eq!(graph.edges.len(), 1);
/// ```
pub fn parse(input: &str) -> Result<Graph, Error> {
    // Normalise: replace newlines with semicolons, then split on ';'.
    // This means both `graph LR; A-->B` and multi-line input are handled
    // identically — the first non-blank, non-comment statement is the header.
    let normalised = input.replace('\n', ";").replace('\r', "");

    let statements: Vec<&str> = normalised
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with("%%"))
        .collect();

    let mut iter = statements.iter().copied();

    // ---- Find and parse the header statement ----------------------------
    let direction = parse_header_stmt(&mut iter)?;
    let mut graph = Graph::new(direction);

    // ---- Parse each remaining statement ---------------------------------
    // We collect remaining statements into a Vec so we can do a stateful
    // multi-statement parse (subgraph blocks span multiple statements).
    let remaining: Vec<&str> = iter.collect();
    parse_statements(&remaining, &mut graph, &mut None);

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

/// Parse a slice of statements into `graph`.
///
/// `current_subgraph_id` is `Some(id)` when we are inside a subgraph block
/// (used to register node membership). This function is called recursively
/// for nested subgraphs — the inner call consumes statements up through `end`
/// and the outer call continues from there.
///
/// Returns the index of the statement **after** the `end` that closed the
/// innermost subgraph block, or `stmts.len()` if there was no `end`.
fn parse_statements(
    stmts: &[&str],
    graph: &mut Graph,
    current_subgraph_id: &mut Option<String>,
) -> usize {
    let mut i = 0;
    while i < stmts.len() {
        let stmt = stmts[i];
        let first_word = stmt.split_whitespace().next().unwrap_or("");

        match first_word {
            "subgraph" => {
                // Parse the subgraph header: `subgraph ID` or `subgraph ID Label`
                let (sg_id, sg_label) = parse_subgraph_header(stmt);

                // Register this subgraph in the parent (or at the top level).
                if let Some(ref parent_id) = current_subgraph_id.clone() {
                    // Nested: link child into parent.
                    if let Some(parent) = graph.subgraphs.iter_mut().find(|s| &s.id == parent_id) {
                        parent.subgraph_ids.push(sg_id.clone());
                    }
                }
                graph.subgraphs.push(Subgraph::new(sg_id.clone(), sg_label));

                // Recurse into the subgraph body. The recursive call consumes
                // statements until it hits the matching `end` and returns the
                // index of the statement after `end`.
                let mut inner_sg = Some(sg_id);
                i += 1;
                let consumed = parse_statements(&stmts[i..], graph, &mut inner_sg);
                i += consumed;
            }
            "end" => {
                // Close the current subgraph block. Tell the caller we consumed
                // through (and including) this `end`.
                return i + 1;
            }
            "direction" => {
                // `direction TB` inside a subgraph: store it on the model.
                if let Some(ref sg_id) = current_subgraph_id.clone() {
                    let dir_word = stmt.split_whitespace().nth(1).unwrap_or("");
                    if let Some(dir) = Direction::parse(dir_word)
                        && let Some(sg) = graph.subgraphs.iter_mut().find(|s| s.id == *sg_id)
                    {
                        sg.direction = Some(dir);
                    }
                }
                i += 1;
            }
            // Style / class directives — silently skip.
            "style" | "classDef" | "class" | "click" | "linkStyle" | "accTitle" | "accDescr" => {
                i += 1;
            }
            _ => {
                // Regular node definition or edge chain.
                parse_statement(stmt, graph, current_subgraph_id);
                i += 1;
            }
        }
    }
    // Consumed all statements without seeing `end` (top-level or unclosed block).
    stmts.len()
}

/// Parse a single statement (already trimmed, no leading/trailing whitespace).
///
/// A statement is either a standalone node definition or an edge chain that
/// may include inline node definitions.
///
/// Any nodes referenced in edges are auto-created if they have not been
/// explicitly defined yet. If `current_subgraph_id` is `Some`, newly seen
/// node IDs are recorded as members of that subgraph.
fn parse_statement(stmt: &str, graph: &mut Graph, current_subgraph_id: &mut Option<String>) {
    // Try to parse as an edge chain first (contains an arrow token).
    if looks_like_edge_chain(stmt) {
        parse_edge_chain(stmt, graph, current_subgraph_id);
    } else {
        // Pure node definition: A[label], A{label}, A((label)), A(label), A
        if let Some(node) = parse_node_definition(stmt) {
            let node_id = node.id.clone();
            graph.upsert_node(node);
            register_node_in_subgraph(graph, &node_id, current_subgraph_id);
        }
    }
}

/// Register `node_id` as a direct member of the current subgraph (if any).
///
/// Only registers if the node is not already a member (avoids duplicates from
/// multiple references to the same node within one subgraph body).
fn register_node_in_subgraph(
    graph: &mut Graph,
    node_id: &str,
    current_subgraph_id: &Option<String>,
) {
    if let Some(sg_id) = current_subgraph_id
        && let Some(sg) = graph.subgraphs.iter_mut().find(|s| s.id == *sg_id)
        && !sg.node_ids.contains(&node_id.to_string())
    {
        sg.node_ids.push(node_id.to_string());
    }
}

/// Parse the `subgraph` header statement and extract `(id, label)`.
///
/// Mermaid supports these forms:
/// - `subgraph ID`           — label defaults to ID
/// - `subgraph ID[Label]`    — label in square brackets (no space before `[`)
/// - `subgraph ID [Label]`   — same with a space
/// - `subgraph "Label"`      — quoted label used as both id and label
fn parse_subgraph_header(stmt: &str) -> (String, String) {
    // Strip the "subgraph" keyword.
    let rest = stmt.trim_start_matches("subgraph").trim();

    if rest.is_empty() {
        // Bare `subgraph` with no identifier — use a placeholder.
        return ("__sg__".to_string(), "".to_string());
    }

    // Check for bracket-style label: `ID[Label]` or `ID [Label]`.
    if let Some(bracket_pos) = rest.find('[') {
        let id = rest[..bracket_pos].trim().to_string();
        let rest_after = &rest[bracket_pos + 1..];
        let label = if let Some(close) = rest_after.find(']') {
            rest_after[..close].trim().to_string()
        } else {
            rest_after.trim().to_string()
        };
        let id = if id.is_empty() { label.clone() } else { id };
        return (id, label);
    }

    // No bracket: the entire rest is the ID, and label == ID.
    let id = rest.to_string();
    (id.clone(), id)
}

/// Return `true` if the statement appears to contain at least one edge arrow.
fn looks_like_edge_chain(s: &str) -> bool {
    // Quick scan: any known arrow token
    s.contains("-->")
        || s.contains("---")
        || s.contains("-.->")
        || s.contains("==>")
        || s.contains("<-->")
        || s.contains("--o")
        || s.contains("--x")
        || s.contains("-- ") // "-- label -->" form
        || s.contains("--") // catch-all for remaining "--" forms
}

// ---------------------------------------------------------------------------
// Edge chain parsing
// ---------------------------------------------------------------------------

/// Parse an edge chain statement and push nodes + edges into `graph`.
///
/// The chain is tokenised by splitting on edge markers while preserving
/// edge-label content between `|...|` delimiters.
fn parse_edge_chain(stmt: &str, graph: &mut Graph, current_subgraph_id: &mut Option<String>) {
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

    // Pending edge metadata carried forward between node tokens.
    let mut pending_edge_label: Option<String> = None;
    let mut pending_edge_style = EdgeStyle::Solid;
    let mut pending_edge_start = EdgeEndpoint::None;
    let mut pending_edge_end = EdgeEndpoint::Arrow;

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
            register_node_in_subgraph(graph, &node_id, current_subgraph_id);

            if let Some(ref from) = prev_id {
                let edge = Edge::new_styled(
                    from.clone(),
                    node_id.clone(),
                    pending_edge_label.take(),
                    pending_edge_style,
                    pending_edge_start,
                    pending_edge_end,
                );
                graph.edges.push(edge);
                // Reset per-edge state for the next edge in the chain.
                pending_edge_style = EdgeStyle::Solid;
                pending_edge_start = EdgeEndpoint::None;
                pending_edge_end = EdgeEndpoint::Arrow;
            }
            prev_id = Some(node_id);
        } else {
            // Arrow token — extract style and optional label.
            let (style, start, end) = classify_arrow(tok);
            pending_edge_style = style;
            pending_edge_start = start;
            pending_edge_end = end;
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
        // Arrows: -->, ---, -.->, ==>, <-->, --o, --x, -- label -->, -->|label|
        // We look for `-`, `=`, or `<` (for bidirectional) not inside a node bracket.
        let ch = chars[i];

        let is_potential_arrow_start =
            (ch == '-' || ch == '=' || ch == '<') && !current.trim().is_empty();

        if is_potential_arrow_start && is_arrow_start(&chars, i) {
            // Push the current node token
            tokens.push(current.trim().to_string());
            current = String::new();

            // Consume the full arrow (including optional |label|)
            let (arrow_tok, consumed) = consume_arrow(&chars, i);
            tokens.push(arrow_tok);
            i += consumed;
            continue;
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
        || remaining.starts_with("<-->")
        || remaining.starts_with("--o")
        || remaining.starts_with("--x")
        || remaining.starts_with("-- ") // "-- label -->"
        || remaining.starts_with("--")
}

/// Classify an arrow token into `(style, start_endpoint, end_endpoint)`.
///
/// The classification mirrors the Mermaid specification:
/// - `<-->` → bidirectional solid
/// - `==>` → thick with arrow
/// - `-.->` / `-..->` → dotted with arrow
/// - `-->` → solid with arrow (default)
/// - `---` → solid, no arrow
/// - `--o` → solid, circle endpoint
/// - `--x` → solid, cross endpoint
fn classify_arrow(arrow: &str) -> (EdgeStyle, EdgeEndpoint, EdgeEndpoint) {
    // Strip any |label| portion before classifying.
    let base = if let Some(pipe) = arrow.find('|') {
        &arrow[..pipe]
    } else {
        arrow
    }
    .trim();

    // Bidirectional: <-->
    if base.starts_with('<') && base.ends_with('>') {
        return (EdgeStyle::Solid, EdgeEndpoint::Arrow, EdgeEndpoint::Arrow);
    }
    // Circle endpoint: --o
    if base.ends_with('o') && base.starts_with('-') {
        return (EdgeStyle::Solid, EdgeEndpoint::None, EdgeEndpoint::Circle);
    }
    // Cross endpoint: --x
    if base.ends_with('x') && base.starts_with('-') {
        return (EdgeStyle::Solid, EdgeEndpoint::None, EdgeEndpoint::Cross);
    }
    // Thick with arrow: ==>
    if base.starts_with('=') {
        let has_arrow = base.ends_with('>');
        let end = if has_arrow {
            EdgeEndpoint::Arrow
        } else {
            EdgeEndpoint::None
        };
        return (EdgeStyle::Thick, EdgeEndpoint::None, end);
    }
    // Dotted: -.- or -.->
    if base.contains(".-") || base.contains("-.") {
        let has_arrow = base.ends_with('>');
        let end = if has_arrow {
            EdgeEndpoint::Arrow
        } else {
            EdgeEndpoint::None
        };
        return (EdgeStyle::Dotted, EdgeEndpoint::None, end);
    }
    // Solid no-arrow: ---  or "-- label --" (no trailing >)
    if base.starts_with('-') && !base.ends_with('>') && !base.ends_with('o') && !base.ends_with('x')
    {
        return (EdgeStyle::Solid, EdgeEndpoint::None, EdgeEndpoint::None);
    }
    // Default: solid arrow -->
    (EdgeStyle::Solid, EdgeEndpoint::None, EdgeEndpoint::Arrow)
}

/// Consume an arrow starting at position `i`, returning `(arrow_token, chars_consumed)`.
///
/// Handles these forms:
/// - `-->` / `-->|label|`
/// - `---`
/// - `-.->` / `-.->|label|`
/// - `==>`
/// - `<-->`
/// - `--o` / `--x`
/// - `-- label -->`
fn consume_arrow(chars: &[char], start: usize) -> (String, usize) {
    let remaining: String = chars[start..].iter().collect();

    // "<-->" bidirectional (must check before "--" forms that start with '<')
    if let Some(rest) = remaining.strip_prefix("<-->") {
        let (label_part, extra) = try_consume_pipe_label(rest);
        let tok = format!("<-->{label_part}");
        return (tok, 4 + extra);
    }

    // "-- label -->" form  (must check before plain "--")
    if let Some(arrow) = try_consume_labeled_dash_arrow(&remaining) {
        let len = arrow.chars().count();
        return (arrow, len);
    }

    // "-.->"|label|? (also handles "-..->")
    if remaining.starts_with("-.-") {
        let base = if remaining.starts_with("-.->") { 4 } else { 3 };
        let (label_part, extra) = try_consume_pipe_label(&remaining[base..]);
        let tok = format!("{}{label_part}", &remaining[..base]);
        return (tok, base + extra);
    }

    // "==>" (also "===", "===>", etc.)
    if remaining.starts_with("==") {
        // Consume all '=' chars then optional '>'
        let mut len = 0;
        for ch in remaining.chars() {
            if ch == '=' {
                len += 1;
            } else {
                break;
            }
        }
        let has_arrow = remaining[len..].starts_with('>');
        if has_arrow {
            len += 1;
        }
        let (label_part, extra) = try_consume_pipe_label(&remaining[len..]);
        let tok = format!("{}{label_part}", &remaining[..len]);
        return (tok, len + extra);
    }

    // "--o" and "--x" endpoint markers
    if remaining.starts_with("--o") {
        return ("--o".to_string(), 3);
    }
    if remaining.starts_with("--x") {
        return ("--x".to_string(), 3);
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
/// `C((name))`, `D(rounded)`, `E([Stadium])`, `F[[Sub]]`, etc., or bare `E`.
///
/// Shape patterns are matched **most-specific-first** to handle multi-char
/// delimiters like `(((`, `((`, `{{`, `[[`, `([`, `[(` before single chars.
///
/// Returns `None` if the token is empty or unparseable.
pub(crate) fn parse_node_definition(token: &str) -> Option<Node> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }

    // Asymmetric `>label]` — starts with `>`, no bracket for ID split.
    if token.starts_with('>') && token.ends_with(']') {
        // Mermaid: `A>label]` where `A` is the id extracted from the caller
        // context. Since we see the whole token here (id prepended), we must
        // find where the `>` starts.
        if let Some(pos) = token.find('>') {
            let id = token[..pos].trim().to_string();
            if !id.is_empty() {
                let inner = token[pos + 1..token.len() - 1].trim().to_string();
                let label = normalize_label(&inner);
                return Some(Node::new(id, label, NodeShape::Asymmetric));
            }
        }
    }

    // Find the first bracket/brace/paren/angle character to split id from shape.
    let shape_start = token.find(['[', '{', '(', '>']);

    let (id, label, shape) = if let Some(pos) = shape_start {
        let id = token[..pos].trim().to_string();
        let rest = &token[pos..];

        // --- Most specific first ---

        // Triple paren: A(((text))) → DoubleCircle
        if rest.starts_with("(((") && rest.ends_with(")))") {
            let inner = rest[3..rest.len() - 3].trim().to_string();
            (id, inner, NodeShape::DoubleCircle)
        }
        // Stadium: A([text])
        else if rest.starts_with("([") && rest.ends_with("])") {
            let inner = rest[2..rest.len() - 2].trim().to_string();
            (id, inner, NodeShape::Stadium)
        }
        // Cylinder: A[(text)]
        else if rest.starts_with("[(") && rest.ends_with(")]") {
            let inner = rest[2..rest.len() - 2].trim().to_string();
            (id, inner, NodeShape::Cylinder)
        }
        // Subroutine: A[[text]]
        else if rest.starts_with("[[") && rest.ends_with("]]") {
            let inner = rest[2..rest.len() - 2].trim().to_string();
            (id, inner, NodeShape::Subroutine)
        }
        // Parallelogram: A[/text/]
        else if rest.starts_with("[/") && rest.ends_with("/]") {
            let inner = rest[2..rest.len() - 2].trim().to_string();
            (id, inner, NodeShape::Parallelogram)
        }
        // Trapezoid: A[/text\]
        else if rest.starts_with("[/") && rest.ends_with("\\]") {
            let inner = rest[2..rest.len() - 2].trim().to_string();
            (id, inner, NodeShape::Trapezoid)
        }
        // Hexagon: A{{text}}
        else if rest.starts_with("{{") && rest.ends_with("}}") {
            let inner = rest[2..rest.len() - 2].trim().to_string();
            (id, inner, NodeShape::Hexagon)
        }
        // Double paren: A((text)) → Circle
        else if rest.starts_with("((") && rest.ends_with("))") {
            let inner = rest[2..rest.len() - 2].trim().to_string();
            (id, inner, NodeShape::Circle)
        }
        // Diamond: A{text}
        else if rest.starts_with('{') && rest.ends_with('}') {
            let inner = rest[1..rest.len() - 1].trim().to_string();
            (id, inner, NodeShape::Diamond)
        }
        // Rectangle: A[text]
        else if rest.starts_with('[') && rest.ends_with(']') {
            let inner = rest[1..rest.len() - 1].trim().to_string();
            (id, inner, NodeShape::Rectangle)
        }
        // Rounded: A(text)
        else if rest.starts_with('(') && rest.ends_with(')') {
            let inner = rest[1..rest.len() - 1].trim().to_string();
            (id, inner, NodeShape::Rounded)
        }
        // Asymmetric: A>text]
        else if rest.starts_with('>') && rest.ends_with(']') {
            let inner = rest[1..rest.len() - 1].trim().to_string();
            (id, inner, NodeShape::Asymmetric)
        } else {
            // Unrecognised bracket pattern — treat entire token as bare ID.
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

    let label = normalize_label(&label);
    Some(Node::new(id, label, shape))
}

/// Soft-wrap threshold for a single label line. Lines longer than this get
/// wrapped at the nearest comma or space before the threshold, producing
/// additional line breaks. Lines without any break point remain intact so
/// a long identifier (`a_very_long_ident_without_separators`) isn't mangled.
const LABEL_WRAP_THRESHOLD: usize = 40;

/// Normalise a label for multi-row rendering.
///
/// Two transformations are applied, in order:
///
/// 1. HTML line-break tags (`<br/>`, `<br>`, `<br />`, case-insensitive on the
///    tag name) are replaced with `\n`. Mermaid uses these as explicit line
///    breaks inside node labels and we honor them.
/// 2. Any resulting line wider than [`LABEL_WRAP_THRESHOLD`] terminal cells is
///    soft-wrapped at the last comma or space at or before the threshold.
///    Words without any wrap-friendly break stay on a single line.
///
/// The renderer interprets `\n` as a line-break and draws each segment on its
/// own row inside the node box, widening the box vertically instead of
/// horizontally.
fn normalize_label(s: &str) -> String {
    // Step 1: replace HTML <br> variants with `\n`. Lower-case first; the
    // upper-case variants are the only other common spellings on the wild.
    let with_breaks = s
        .replace("<br/>", "\n")
        .replace("<br>", "\n")
        .replace("<br />", "\n")
        .replace("<BR/>", "\n")
        .replace("<BR>", "\n")
        .replace("<BR />", "\n");

    // Step 2: soft-wrap each resulting line.
    let mut out = String::with_capacity(with_breaks.len());
    let mut first = true;
    for line in with_breaks.lines() {
        if !first {
            out.push('\n');
        }
        first = false;
        soft_wrap_into(line, &mut out);
    }
    out
}

/// Append `line` to `out`, inserting `\n` breaks at word boundaries so that
/// no resulting row exceeds [`LABEL_WRAP_THRESHOLD`] columns.
///
/// The break character (comma or space) stays on the head side of the split —
/// a trailing space gets trimmed, a trailing comma is preserved so the user's
/// list formatting is kept.
fn soft_wrap_into(line: &str, out: &mut String) {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    if UnicodeWidthStr::width(line) <= LABEL_WRAP_THRESHOLD {
        out.push_str(line);
        return;
    }

    // Walk chars, tracking cumulative width and the byte index of the last
    // break-friendly char (comma or space) seen within the budget.
    let mut cum_w = 0usize;
    let mut last_break: Option<usize> = None;
    for (i, ch) in line.char_indices() {
        cum_w += UnicodeWidthChar::width(ch).unwrap_or(0);
        if cum_w > LABEL_WRAP_THRESHOLD {
            break;
        }
        if ch == ',' || ch == ' ' {
            last_break = Some(i);
        }
    }

    let Some(break_at) = last_break else {
        // No break point within the budget — emit the line as-is rather than
        // mangling a single long word.
        out.push_str(line);
        return;
    };

    // `split_at(break_at + 1)`: `break_at` is the byte index of the break
    // character; `+ 1` includes the break char (all break chars are ASCII,
    // so their UTF-8 length is 1) in the head.
    let (head, tail) = line.split_at(break_at + 1);
    let head = head.trim_end();
    let tail = tail.trim_start();
    out.push_str(head);
    out.push('\n');
    soft_wrap_into(tail, out);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EdgeEndpoint, EdgeStyle, NodeShape};

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

    #[test]
    fn parse_subgraph_basic() {
        let src = "graph LR\nsubgraph Supervisor\nF[Factory] --> W[Worker]\nend";
        let g = parse(src).unwrap();
        assert!(g.has_node("F"), "missing F");
        assert!(g.has_node("W"), "missing W");
        assert_eq!(g.subgraphs.len(), 1);
        assert_eq!(g.subgraphs[0].id, "Supervisor");
        assert_eq!(g.subgraphs[0].label, "Supervisor");
        // Both nodes should be members of the Supervisor subgraph.
        assert!(g.subgraphs[0].node_ids.contains(&"F".to_string()));
        assert!(g.subgraphs[0].node_ids.contains(&"W".to_string()));
    }

    #[test]
    fn parse_subgraph_with_direction() {
        let src = "graph LR\nsubgraph S\ndirection TB\nA-->B\nend";
        let g = parse(src).unwrap();
        assert_eq!(g.subgraphs[0].direction, Some(Direction::TopToBottom));
    }

    #[test]
    fn parse_nested_subgraphs() {
        let src = "graph TD\nsubgraph Outer\nsubgraph Inner\nA[A]\nend\nB[B]\nend";
        let g = parse(src).unwrap();
        // Both subgraphs should be registered.
        assert!(g.find_subgraph("Outer").is_some());
        assert!(g.find_subgraph("Inner").is_some());
        // Inner should be a child of Outer.
        let outer = g.find_subgraph("Outer").unwrap();
        assert!(outer.subgraph_ids.contains(&"Inner".to_string()));
        // A is in Inner, B is in Outer.
        let inner = g.find_subgraph("Inner").unwrap();
        assert!(inner.node_ids.contains(&"A".to_string()));
        assert!(outer.node_ids.contains(&"B".to_string()));
    }

    #[test]
    fn parse_subgraph_edge_crossing_boundary() {
        let src = "graph LR\nsubgraph S\nF[Factory] --> W[Worker]\nend\nW --> HB[Heartbeat]";
        let g = parse(src).unwrap();
        assert!(g.has_node("F"));
        assert!(g.has_node("W"));
        assert!(g.has_node("HB"));
        // W → HB edge should exist (crosses boundary).
        assert!(g.edges.iter().any(|e| e.from == "W" && e.to == "HB"));
        // HB should NOT be in subgraph S.
        let s = g.find_subgraph("S").unwrap();
        assert!(!s.node_ids.contains(&"HB".to_string()));
    }

    #[test]
    fn node_to_subgraph_map() {
        let src = "graph LR\nsubgraph S\nA-->B\nend\nC-->D";
        let g = parse(src).unwrap();
        let map = g.node_to_subgraph();
        assert_eq!(map.get("A").map(String::as_str), Some("S"));
        assert_eq!(map.get("B").map(String::as_str), Some("S"));
        assert!(!map.contains_key("C"));
        assert!(!map.contains_key("D"));
    }

    // ---- New node shape parser tests ------------------------------------

    #[test]
    fn parse_stadium_node() {
        let g = parse("graph LR\nA([Stadium])").unwrap();
        assert_eq!(g.node("A").unwrap().shape, NodeShape::Stadium);
        assert_eq!(g.node("A").unwrap().label, "Stadium");
    }

    #[test]
    fn parse_subroutine_node() {
        let g = parse("graph LR\nA[[Sub]]").unwrap();
        assert_eq!(g.node("A").unwrap().shape, NodeShape::Subroutine);
        assert_eq!(g.node("A").unwrap().label, "Sub");
    }

    #[test]
    fn parse_cylinder_node() {
        let g = parse("graph LR\nA[(DB)]").unwrap();
        assert_eq!(g.node("A").unwrap().shape, NodeShape::Cylinder);
        assert_eq!(g.node("A").unwrap().label, "DB");
    }

    #[test]
    fn parse_hexagon_node() {
        let g = parse("graph LR\nA{{Hex}}").unwrap();
        assert_eq!(g.node("A").unwrap().shape, NodeShape::Hexagon);
        assert_eq!(g.node("A").unwrap().label, "Hex");
    }

    #[test]
    fn parse_asymmetric_node() {
        let g = parse("graph LR\nA>Flag]").unwrap();
        assert_eq!(g.node("A").unwrap().shape, NodeShape::Asymmetric);
        assert_eq!(g.node("A").unwrap().label, "Flag");
    }

    #[test]
    fn parse_parallelogram_node() {
        let g = parse("graph LR\nA[/Lean/]").unwrap();
        assert_eq!(g.node("A").unwrap().shape, NodeShape::Parallelogram);
        assert_eq!(g.node("A").unwrap().label, "Lean");
    }

    #[test]
    fn parse_trapezoid_node() {
        let g = parse("graph LR\nA[/Trap\\]").unwrap();
        assert_eq!(g.node("A").unwrap().shape, NodeShape::Trapezoid);
        assert_eq!(g.node("A").unwrap().label, "Trap");
    }

    #[test]
    fn parse_double_circle_node() {
        let g = parse("graph LR\nA(((Dbl)))").unwrap();
        assert_eq!(g.node("A").unwrap().shape, NodeShape::DoubleCircle);
        assert_eq!(g.node("A").unwrap().label, "Dbl");
    }

    // Disambiguation: (( before ((( — triple paren wins.
    #[test]
    fn triple_paren_beats_double_paren() {
        let g = parse("graph LR\nA(((X)))").unwrap();
        assert_eq!(g.node("A").unwrap().shape, NodeShape::DoubleCircle);
    }

    // Disambiguation: [[ before [ — double bracket wins.
    #[test]
    fn double_bracket_beats_single_bracket() {
        let g = parse("graph LR\nA[[Y]]").unwrap();
        assert_eq!(g.node("A").unwrap().shape, NodeShape::Subroutine);
    }

    // ---- Edge style parser tests ----------------------------------------

    #[test]
    fn parse_dotted_edge_style() {
        let g = parse("graph LR\nA-.->B").unwrap();
        assert_eq!(g.edges[0].style, EdgeStyle::Dotted);
        assert_eq!(g.edges[0].end, EdgeEndpoint::Arrow);
    }

    #[test]
    fn parse_thick_edge_style() {
        let g = parse("graph LR\nA==>B").unwrap();
        assert_eq!(g.edges[0].style, EdgeStyle::Thick);
        assert_eq!(g.edges[0].end, EdgeEndpoint::Arrow);
    }

    #[test]
    fn parse_plain_line_no_arrow() {
        let g = parse("graph LR\nA---B").unwrap();
        assert_eq!(g.edges[0].style, EdgeStyle::Solid);
        assert_eq!(g.edges[0].end, EdgeEndpoint::None);
        assert_eq!(g.edges[0].start, EdgeEndpoint::None);
    }

    #[test]
    fn parse_bidirectional_edge() {
        let g = parse("graph LR\nA<-->B").unwrap();
        assert_eq!(g.edges[0].style, EdgeStyle::Solid);
        assert_eq!(g.edges[0].start, EdgeEndpoint::Arrow);
        assert_eq!(g.edges[0].end, EdgeEndpoint::Arrow);
    }

    #[test]
    fn parse_circle_endpoint() {
        let g = parse("graph LR\nA--oB").unwrap();
        assert_eq!(g.edges[0].end, EdgeEndpoint::Circle);
    }

    #[test]
    fn parse_cross_endpoint() {
        let g = parse("graph LR\nA--xB").unwrap();
        assert_eq!(g.edges[0].end, EdgeEndpoint::Cross);
    }
}
