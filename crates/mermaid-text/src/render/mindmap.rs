//! Renderer for [`Mindmap`]. Produces a Unicode tree string using standard
//! line-drawing characters.
//!
//! **Layout** — a vertical tree with the root displayed in a rounded box at
//! the top, then a trunk line down to the first level. Children branch off with
//! standard tree-drawing connectors:
//!
//! ```text
//! ╭──────────╮
//! │ mindmap  │
//! ╰────┬─────╯
//!      ├── Origins
//!      │   ├── Long history
//!      │   └── Popularisation
//!      │       └── British popular psychology...
//!      ├── Research
//!      │   └── On effectiveness and features
//!      └── Tools
//!          ├── Pen and paper
//!          └── Mermaid
//! ```
//!
//! **Glyph alphabet** (geometric line-drawing characters — not emoji):
//!
//! | Glyph | Meaning                          |
//! |-------|----------------------------------|
//! | `╭`   | Top-left box corner              |
//! | `╰`   | Bottom-left box corner           |
//! | `╮`   | Top-right box corner             |
//! | `╯`   | Bottom-right box corner          |
//! | `─`   | Horizontal box border / branch   |
//! | `│`   | Vertical box border / trunk      |
//! | `┬`   | T-junction (trunk exits box)     |
//! | `├`   | Branch junction (non-last child) |
//! | `└`   | Branch junction (last child)     |
//!
//! **max_width** — when `max_width` is `Some(n)`, node text that would push a
//! line past the column budget is truncated with `…` (U+2026). The root box
//! and all connector prefix columns are counted in the budget.

use unicode_width::UnicodeWidthStr;

use crate::mindmap::{Mindmap, MindmapNode};

// Connector for a non-last child.
const BRANCH: &str = "\u{251C}\u{2500}\u{2500} "; // "├── "
// Connector for the last child.
const LAST_BRANCH: &str = "\u{2514}\u{2500}\u{2500} "; // "└── "
// Continuation pipe (under a non-last child's branch).
const PIPE: &str = "\u{2502}   "; // "│   "
// Blank continuation (under the last child — no more siblings).
const BLANK: &str = "    "; // "    "

/// Render a [`Mindmap`] to a Unicode string.
///
/// # Arguments
///
/// * `diag`      — the parsed diagram
/// * `max_width` — optional column budget; node text is truncated with `…`
///   when a rendered line would exceed this many terminal cells
///
/// # Returns
///
/// A multi-line string ready for printing. The root appears as a small rounded
/// box at the top; children branch below it using standard tree-drawing glyphs.
pub fn render(diag: &Mindmap, max_width: Option<usize>) -> String {
    let mut out = String::new();

    render_root_box(&mut out, &diag.root.text, max_width);

    // Render all children of the root under the trunk.
    for (i, child) in diag.root.children.iter().enumerate() {
        let is_last = i == diag.root.children.len() - 1;
        render_node(&mut out, child, "", is_last, max_width);
    }

    // Trim trailing newline to match other renderers.
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Render the root as a rounded box with a trunk connector at the bottom.
///
/// The box is sized to the root text; a `┬` glyph appears in the bottom border
/// centred under the trunk of the first-child connector column.
fn render_root_box(out: &mut String, text: &str, max_width: Option<usize>) {
    // Determine the display width of the text, truncating if needed.
    // The box adds 4 cells of overhead: "│ " + " │" = 2+2 = 4.
    // With corners: "╭─" + "─╮" = 4 border chars plus the content.
    let box_overhead = 4usize; // "│ " + " │" inner padding
    let corner_overhead = 2usize; // "╭" + "╮" on top/bottom lines
    let total_fixed = box_overhead + corner_overhead; // 6 cells total frame width
    let _ = total_fixed; // used below in available calc

    // Available width for the text (inside box): max_width - 4 (for "│ " + " │")
    let text_w = UnicodeWidthStr::width(text);
    let (display_text, content_w) = if let Some(budget) = max_width {
        // "╭─…─╮\n│ … │\n╰─…─╯" — box content width: budget - 4 for "│ " + " │"
        let available = budget.saturating_sub(4);
        if text_w <= available {
            (text.to_string(), text_w)
        } else {
            let truncated = truncate_text(text, available.saturating_sub(1));
            let tw = UnicodeWidthStr::width(truncated.as_str());
            (truncated, tw)
        }
    } else {
        (text.to_string(), text_w)
    };

    // The branch connector column is at position: 4 + content_w / 2.
    // "╭─" is 2 cells, "─╮" is 2 cells, middle cells = content_w.
    // Trunk position (0-indexed from line start): 2 + content_w / 2.
    // We use this to place `┬` in the bottom border.
    let trunk_col = 1 + content_w / 2; // 1 for "╰", then trunk_col dashes before ┬

    // Top border: ╭─────────╮
    out.push('\u{256D}'); // ╭
    for _ in 0..content_w + 2 {
        out.push('\u{2500}'); // ─
    }
    out.push('\u{256E}'); // ╮
    out.push('\n');

    // Content row: │ text │
    out.push('\u{2502}'); // │
    out.push(' ');
    out.push_str(&display_text);
    out.push(' ');
    out.push('\u{2502}'); // │
    out.push('\n');

    // Bottom border: ╰──┬──╯  (trunk position marks where children attach)
    out.push('\u{2570}'); // ╰
    for i in 0..content_w + 2 {
        if i == trunk_col {
            out.push('\u{252C}'); // ┬
        } else {
            out.push('\u{2500}'); // ─
        }
    }
    out.push('\u{256F}'); // ╯
    out.push('\n');

    // Trunk line: "      │" — the vertical connector from box to first child.
    // The trunk is at column: 1 (for ╰) + trunk_col.
    // We need to pad `trunk_col` spaces then `│`.
    if !display_text.is_empty() {
        for _ in 0..=trunk_col {
            out.push(' ');
        }
        out.push('\u{2502}'); // │
        out.push('\n');
    }
}

/// Recursively render a node and its children.
///
/// `prefix` is the string of continuation-pipe / blank-indent characters that
/// must be prepended before this node's connector glyph. Each call appends its
/// own connector (`├──` or `└──`) then recurses with an extended prefix.
fn render_node(
    out: &mut String,
    node: &MindmapNode,
    prefix: &str,
    is_last: bool,
    max_width: Option<usize>,
) {
    let connector = if is_last { LAST_BRANCH } else { BRANCH };

    let prefix_w = UnicodeWidthStr::width(prefix) + UnicodeWidthStr::width(connector);
    let text = maybe_truncate(&node.text, max_width, prefix_w);

    out.push_str(prefix);
    out.push_str(connector);
    out.push_str(&text);
    out.push('\n');

    // Build the child prefix: extend by either "│   " or "    " depending on
    // whether this node has more siblings (i.e. is not the last child).
    let child_prefix = if is_last {
        format!("{prefix}{BLANK}")
    } else {
        format!("{prefix}{PIPE}")
    };

    for (i, child) in node.children.iter().enumerate() {
        let child_is_last = i == node.children.len() - 1;
        render_node(out, child, &child_prefix, child_is_last, max_width);
    }
}

/// Truncate `text` to fit within `available` display cells, appending `…`.
fn truncate_text(text: &str, available: usize) -> String {
    let mut result = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
        if used + w > available {
            break;
        }
        result.push(ch);
        used += w;
    }
    result.push('\u{2026}'); // HORIZONTAL ELLIPSIS
    result
}

/// Truncate `text` with `…` if emitting it after `prefix_cols` cells would
/// exceed `max_width`. Returns the (possibly truncated) string.
fn maybe_truncate(text: &str, max_width: Option<usize>, prefix_cols: usize) -> String {
    let Some(budget) = max_width else {
        return text.to_string();
    };
    let available = budget.saturating_sub(prefix_cols);
    let text_w = UnicodeWidthStr::width(text);
    if text_w <= available {
        return text.to_string();
    }
    truncate_text(text, available.saturating_sub(1))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::mindmap::parse;

    #[test]
    fn single_root_renders_just_the_box() {
        let diag = parse("mindmap\n  root").unwrap();
        let out = render(&diag, None);
        // The root text must appear in the output.
        assert!(out.contains("root"), "got: {out:?}");
        // The box corners must be present.
        assert!(out.contains('\u{256D}'), "top-left corner missing");
        assert!(out.contains('\u{256E}'), "top-right corner missing");
        assert!(out.contains('\u{2570}'), "bottom-left corner missing");
        assert!(out.contains('\u{256F}'), "bottom-right corner missing");
        // No branch glyphs when there are no children.
        assert!(!out.contains('\u{251C}'), "unexpected branch glyph");
        assert!(!out.contains('\u{2514}'), "unexpected last-branch glyph");
    }

    #[test]
    fn tree_uses_branch_glyphs() {
        let src = "mindmap\n  root\n    A\n    B";
        let diag = parse(src).unwrap();
        let out = render(&diag, None);
        assert!(out.contains("A"), "node A missing");
        assert!(out.contains("B"), "node B missing");
        // Non-last child uses ├──; last child uses └──.
        assert!(out.contains('\u{251C}'), "├ branch glyph missing");
        assert!(out.contains('\u{2514}'), "└ last-branch glyph missing");
    }

    #[test]
    fn nested_levels_indent_progressively() {
        let src = "mindmap\n  root\n    Parent\n      Child";
        let diag = parse(src).unwrap();
        let out = render(&diag, None);
        // "Child" must appear with greater indentation than "Parent".
        let parent_line = out.lines().find(|l| l.contains("Parent")).unwrap();
        let child_line = out.lines().find(|l| l.contains("Child")).unwrap();
        let parent_indent = parent_line
            .chars()
            .take_while(|c| !c.is_alphanumeric() && *c != '\u{251C}' && *c != '\u{2514}')
            .count();
        let child_indent = child_line
            .chars()
            .take_while(|c| !c.is_alphanumeric() && *c != '\u{251C}' && *c != '\u{2514}')
            .count();
        assert!(
            child_indent > parent_indent,
            "child ({child_indent}) must be indented more than parent ({parent_indent})"
        );
    }

    #[test]
    fn max_width_truncates_long_node_text() {
        let long_text = "A".repeat(80);
        let src = format!("mindmap\n  root\n    {long_text}");
        let diag = parse(&src).unwrap();
        let out = render(&diag, Some(40));
        for line in out.lines() {
            let w = UnicodeWidthStr::width(line);
            assert!(w <= 40, "line exceeds max_width=40 ({w} cells): {line:?}");
        }
        assert!(out.contains('\u{2026}'), "ellipsis must appear on truncated text");
    }
}
