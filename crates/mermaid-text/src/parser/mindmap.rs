//! Parser for Mermaid `mindmap` diagrams.
//!
//! Accepted syntax:
//!
//! ```text
//! mindmap
//!   root((mindmap))
//!     Origins
//!       Long history
//!       ::icon(fa fa-book)
//!       Popularisation
//!         British popular psychology author Tony Buzan
//!     Research
//!       On effectiveness<br/>and features
//!     Tools
//!       Pen and paper
//!       Mermaid
//! ```
//!
//! Rules:
//! - `mindmap` keyword is required as the first non-blank, non-comment line.
//! - Each subsequent non-blank, non-comment, non-icon line is a node.
//! - **Indentation** determines the tree structure. A node indented more than the
//!   previous node becomes its child; equal or less indentation makes it a
//!   sibling or an ancestor's sibling. Tabs are normalised to 4 spaces before
//!   measuring indent.
//! - The first node after the `mindmap` keyword is the **root**.
//! - Node shape brackets are stripped to inner text (Phase 1 limitation):
//!   `((text))` → `text`, `(text)` → `text`, `{{text}}` → `text`,
//!   `))text((` → `text`, `)text(` → `text`.
//! - `::icon(...)` lines are silently ignored.
//! - `%%` comment lines and blank lines are silently skipped.
//! - `accTitle` / `accDescr` accessibility metadata lines are silently ignored.
//!
//! # Examples
//!
//! ```
//! use mermaid_text::parser::mindmap::parse;
//!
//! let diag = parse("mindmap\n  root\n    child").unwrap();
//! assert_eq!(diag.root.text, "root");
//! assert_eq!(diag.root.children[0].text, "child");
//! ```

use crate::Error;
use crate::mindmap::{Mindmap, MindmapNode};
use crate::parser::common::strip_inline_comment;

/// Parse a `mindmap` source string into a [`Mindmap`].
///
/// # Errors
///
/// - [`Error::ParseError`] — missing `mindmap` header, or the source contains
///   no root node after the header.
pub fn parse(src: &str) -> Result<Mindmap, Error> {
    let mut header_seen = false;
    // Collect (indent, text) pairs for all content lines.
    let mut node_lines: Vec<(usize, String)> = Vec::new();

    for raw in src.lines() {
        let stripped = strip_inline_comment(raw);

        if !header_seen {
            let trimmed = stripped.trim();
            if trimmed.is_empty() || trimmed.starts_with("%%") {
                continue;
            }
            if !trimmed.eq_ignore_ascii_case("mindmap") {
                return Err(Error::ParseError(format!(
                    "expected `mindmap` header, got {trimmed:?}"
                )));
            }
            header_seen = true;
            continue;
        }

        let trimmed = stripped.trim();

        // Skip blank and comment lines.
        if trimmed.is_empty() || trimmed.starts_with("%%") {
            continue;
        }

        // Silently skip accessibility metadata.
        if trimmed.starts_with("accTitle") || trimmed.starts_with("accDescr") {
            continue;
        }

        // Silently skip icon directives (e.g. `::icon(fa fa-book)`).
        if trimmed.starts_with("::icon(") {
            continue;
        }

        // Measure indent: tabs are 4 spaces each.
        let indent = measure_indent(raw);

        // Strip any node-shape bracket syntax to get the display text.
        let text = strip_node_shape(trimmed);

        node_lines.push((indent, text));
    }

    if !header_seen {
        return Err(Error::ParseError(
            "missing `mindmap` header line".to_string(),
        ));
    }

    if node_lines.is_empty() {
        return Err(Error::ParseError(
            "mindmap has no nodes (at least a root node is required)".to_string(),
        ));
    }

    let root = build_tree(&node_lines);
    Ok(Mindmap { root })
}

/// Measure the indentation of a raw source line.
///
/// Tabs are treated as 4 spaces each so that a single tab aligns with
/// four spaces of indentation (the most common convention in Mermaid
/// examples). The count stops at the first non-whitespace character.
fn measure_indent(line: &str) -> usize {
    let mut count = 0;
    for ch in line.chars() {
        match ch {
            ' ' => count += 1,
            '\t' => count += 4,
            _ => break,
        }
    }
    count
}

/// Strip node-shape bracket syntax and return just the inner text.
///
/// Phase 1: all 6 shape variants are normalised to plain text; the shape
/// itself is not recorded. The stripping is greedy — we try the most
/// specific patterns first (e.g. `((text))` before `(text)`).
fn strip_node_shape(s: &str) -> String {
    // Remove an optional leading id prefix before the bracket (e.g. `id[text]`
    // or `id((text))`). In Phase 1 we ignore the id and keep only the inner
    // text. The id is defined as the contiguous non-bracket prefix.
    let body = if let Some(bracket_start) = s.find(['[', '(', '{', ')']) {
        // Check whether the part before the bracket is plausibly an id (no
        // whitespace). If it contains whitespace the whole token is plain text.
        let prefix = &s[..bracket_start];
        if prefix.chars().all(|c: char| !c.is_whitespace()) && !prefix.is_empty() {
            &s[bracket_start..]
        } else {
            s
        }
    } else {
        s
    };

    // Double-parenthesis circle: `((text))`
    if let Some(inner) = body.strip_prefix("((").and_then(|t| t.strip_suffix("))")) {
        return inner.trim().to_string();
    }
    // Bang shape: `))text((`
    if let Some(inner) = body.strip_prefix("))").and_then(|t| t.strip_suffix("((")) {
        return inner.trim().to_string();
    }
    // Cloud: `)text(`
    if let Some(inner) = body.strip_prefix(')').and_then(|t| t.strip_suffix('(')) {
        return inner.trim().to_string();
    }
    // Single-parenthesis rounded: `(text)`
    if let Some(inner) = body.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
        return inner.trim().to_string();
    }
    // Double-brace hexagon: `{{text}}`
    if let Some(inner) = body.strip_prefix("{{").and_then(|t| t.strip_suffix("}}")) {
        return inner.trim().to_string();
    }
    // Square bracket: `[text]`
    if let Some(inner) = body.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        return inner.trim().to_string();
    }

    // No shape brackets — use the body directly (which may be the original `s`
    // if the prefix check didn't find a bracket-start we could use).
    body.to_string()
}

/// Build a tree from a flat list of `(indent, text)` pairs.
///
/// The algorithm maintains a stack of `(indent, index_into_arena)` entries.
/// The arena is a flat `Vec<MindmapNode>` where children are accumulated and
/// then moved into their parent when the parent is popped off the stack.
///
/// The indentation-stack invariant:
/// - The stack always ends with the deepest node that is still a potential
///   parent of the next line.
/// - When a new line's indent is ≤ any stack entry's indent, all deeper entries
///   are popped first, transplanting children upward as they go.
///
/// This is the natural tree-building algorithm for indent-delimited formats
/// (Python, YAML, Mermaid mindmap). We track indices into a flat Vec rather
/// than building nested structures directly because Rust's ownership rules make
/// it awkward to hold multiple `&mut MindmapNode` references simultaneously.
fn build_tree(lines: &[(usize, String)]) -> MindmapNode {
    // Flat arena: each slot is (node, children_indices).
    let mut nodes: Vec<MindmapNode> = Vec::with_capacity(lines.len());

    // Stack entries: (indent_of_node, arena_index).
    let mut stack: Vec<(usize, usize)> = Vec::new();

    // `children_map[parent_idx]` holds arena indices of its children in order.
    let mut children_map: Vec<Vec<usize>> = Vec::with_capacity(lines.len());

    for (indent, text) in lines {
        let new_idx = nodes.len();
        nodes.push(MindmapNode::new(text));
        children_map.push(Vec::new());

        // Pop any stack entries whose indent is >= the new node's indent;
        // those are no longer potential parents.
        while let Some(&(stack_indent, _)) = stack.last() {
            if stack_indent >= *indent {
                stack.pop();
            } else {
                break;
            }
        }

        // The top of the stack (if any) is now the parent.
        if let Some(&(_, parent_idx)) = stack.last() {
            children_map[parent_idx].push(new_idx);
        }

        stack.push((*indent, new_idx));
    }

    // Reconstruct the tree bottom-up: work backwards through the arena,
    // moving children into each node.
    for parent_idx in (0..nodes.len()).rev() {
        let child_indices: Vec<usize> = children_map[parent_idx].clone();
        // We need to pull children out of `nodes`. Because we go in reverse
        // parent order we must collect first, then move. Use a temporary
        // placeholder to avoid indexing aliasing issues.
        let children: Vec<MindmapNode> = child_indices
            .into_iter()
            .map(|ci| {
                // Replace the child slot with a placeholder; we'll use the
                // real node returned here.
                std::mem::replace(&mut nodes[ci], MindmapNode::new(""))
            })
            .collect();
        nodes[parent_idx].children = children;
    }

    // The root is always the first node (index 0).
    std::mem::replace(&mut nodes[0], MindmapNode::new(""))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_root_only() {
        let diag = parse("mindmap\n  root").unwrap();
        assert_eq!(diag.root.text, "root");
        assert!(diag.root.children.is_empty());
    }

    #[test]
    fn parses_one_level_children() {
        let diag = parse("mindmap\n  root\n    A\n    B\n    C").unwrap();
        assert_eq!(diag.root.text, "root");
        assert_eq!(diag.root.children.len(), 3);
        assert_eq!(diag.root.children[0].text, "A");
        assert_eq!(diag.root.children[1].text, "B");
        assert_eq!(diag.root.children[2].text, "C");
    }

    #[test]
    fn parses_nested_two_levels() {
        let src = "mindmap\n  root\n    Parent\n      Child1\n      Child2\n    Sibling";
        let diag = parse(src).unwrap();
        assert_eq!(diag.root.children.len(), 2);
        let parent = &diag.root.children[0];
        assert_eq!(parent.text, "Parent");
        assert_eq!(parent.children.len(), 2);
        assert_eq!(parent.children[0].text, "Child1");
        assert_eq!(parent.children[1].text, "Child2");
        let sibling = &diag.root.children[1];
        assert_eq!(sibling.text, "Sibling");
        assert!(sibling.children.is_empty());
    }

    #[test]
    fn parses_node_shapes_strips_brackets() {
        let src = "mindmap\n  root((circle))\n    rounded(text)\n    hex{{hexa}}\n    plain text";
        let diag = parse(src).unwrap();
        assert_eq!(diag.root.text, "circle");
        assert_eq!(diag.root.children[0].text, "text");
        assert_eq!(diag.root.children[1].text, "hexa");
        assert_eq!(diag.root.children[2].text, "plain text");
    }

    #[test]
    fn ignores_icon_directive() {
        let src = "mindmap\n  root\n    Origins\n      ::icon(fa fa-book)\n      Long history";
        let diag = parse(src).unwrap();
        let origins = &diag.root.children[0];
        assert_eq!(origins.text, "Origins");
        // The icon line must be gone; only "Long history" remains as child.
        assert_eq!(origins.children.len(), 1);
        assert_eq!(origins.children[0].text, "Long history");
    }

    #[test]
    fn comment_lines_skipped() {
        let src = "%% preamble\nmindmap\n  %% inner comment\n  root\n    child %% trailing";
        let diag = parse(src).unwrap();
        assert_eq!(diag.root.text, "root");
        assert_eq!(diag.root.children.len(), 1);
        assert_eq!(diag.root.children[0].text, "child");
    }

    #[test]
    fn tabs_count_as_four_spaces() {
        // One tab == 4 spaces, so a tab-indented child is at depth 4.
        let src = "mindmap\n\troot\n\t\tchild";
        let diag = parse(src).unwrap();
        assert_eq!(diag.root.text, "root");
        assert_eq!(diag.root.children.len(), 1);
        assert_eq!(diag.root.children[0].text, "child");
    }

    #[test]
    fn dedent_attaches_sibling_to_correct_parent() {
        // A -> A1 -> A1a, then dedent back to A's level for B.
        let src = "mindmap\n  root\n    A\n      A1\n        A1a\n    B";
        let diag = parse(src).unwrap();
        assert_eq!(diag.root.children.len(), 2);
        let a = &diag.root.children[0];
        assert_eq!(a.text, "A");
        assert_eq!(a.children.len(), 1);
        assert_eq!(a.children[0].text, "A1");
        assert_eq!(a.children[0].children[0].text, "A1a");
        let b = &diag.root.children[1];
        assert_eq!(b.text, "B");
        assert!(b.children.is_empty());
    }

    #[test]
    fn missing_header_returns_error() {
        let err = parse("root\n  child").unwrap_err();
        assert!(
            err.to_string().contains("mindmap"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn accessibility_metadata_is_silently_ignored() {
        let src =
            "mindmap\n  accTitle: My title\n  accDescr: A description\n  root\n    child";
        let diag = parse(src).unwrap();
        // The accTitle/accDescr lines must not appear as nodes.
        assert_eq!(diag.root.text, "root");
        assert_eq!(diag.root.children.len(), 1);
        assert_eq!(diag.root.children[0].text, "child");
    }
}
