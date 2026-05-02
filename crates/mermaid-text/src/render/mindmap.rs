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

    let trunk_col = render_root_box(&mut out, &diag.root.text, max_width);

    // Indent every level-1 child so its branch glyph (`├` / `└`) sits in
    // the same column as the trunk pipe (`│`) that drops from the root
    // box. Without this prefix the children render at column 0 and the
    // trunk visibly terminates in empty space.
    let root_prefix: String = " ".repeat(trunk_col);

    for (i, child) in diag.root.children.iter().enumerate() {
        let is_last = i == diag.root.children.len() - 1;
        render_node(&mut out, child, &root_prefix, is_last, max_width);
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
///
/// Returns the **byte column** of the trunk `│` glyph emitted on the
/// trunk line below the box. The caller uses this to indent every level-1
/// child so its branch glyph (`├` / `└`) aligns with the trunk; the
/// returned value already accounts for the multi-byte width of the trunk
/// glyph and can be used directly as `" ".repeat(N)` for an ASCII-space
/// prefix.
fn render_root_box(out: &mut String, text: &str, max_width: Option<usize>) -> usize {
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
    // We need to pad `trunk_col + 1` spaces then `│`.
    if !display_text.is_empty() {
        for _ in 0..=trunk_col {
            out.push(' ');
        }
        out.push('\u{2502}'); // │
        out.push('\n');
    }

    // Return the byte column of the trunk `│` so the caller can indent
    // level-1 children to match.
    trunk_col + 1
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

    /// Helper used by alignment tests: return the byte position of `│` in
    /// the trunk line — the first line whose content is exactly N ASCII
    /// spaces (N ≥ 1) followed by a single `│` and nothing else. Skips the
    /// inside-box content row (which starts with `│`, not a space) and the
    /// bottom-border line (which contains `╰`/`┬`/`╯` non-pipe glyphs).
    fn find_trunk_pipe_col(out: &str) -> Option<usize> {
        out.lines().find_map(|l| {
            let mut chars = l.chars();
            if chars.next() != Some(' ') {
                return None;
            }
            let mut saw_pipe = false;
            for c in chars {
                match c {
                    ' ' => continue,
                    '\u{2502}' if !saw_pipe => saw_pipe = true,
                    _ => return None,
                }
            }
            if saw_pipe { l.find('\u{2502}') } else { None }
        })
    }

    /// The first level-1 child's branch glyph (`├`/`└`) MUST sit in the
    /// same column as the trunk pipe (`│`) that drops from the root box.
    /// Otherwise the trunk drops from the root, ends mid-air, and the
    /// children appear visually disconnected at the left margin.
    ///
    /// We anchor against the trunk-line `│` (not the bottom-border `┬`)
    /// because the trunk line contains only ASCII spaces before its box
    /// glyph, so byte position equals visual column — making the
    /// comparison robust against the multi-byte nature of `┬` / `╰`.
    ///
    /// A trivially-broken implementation that emits children at col 0
    /// (the pre-fix state) cannot satisfy the equality: the trunk `│`
    /// will be at byte ≥3 (one ASCII space + one box-drawing char minimum)
    /// while the child `├` would be at byte 0.
    #[test]
    fn first_child_branch_aligns_with_root_trunk_column() {
        let src = "mindmap\n  mindmap\n    Origins\n    Research\n    Tools";
        let diag = parse(src).unwrap();
        let out = render(&diag, None);

        let trunk_pipe_col = find_trunk_pipe_col(&out)
            .unwrap_or_else(|| panic!("expected trunk-pipe line in:\n{out}"));

        let first_child_branch_col = out
            .lines()
            .find_map(|l| l.find(['\u{251C}', '\u{2514}']))
            .expect("at least one level-1 child must emit a branch glyph");

        assert_eq!(
            first_child_branch_col, trunk_pipe_col,
            "first child's branch glyph (byte col {first_child_branch_col}) \
             must sit in the same column as the trunk pipe │ that drops \
             from the root box (byte col {trunk_pipe_col}); otherwise the \
             trunk drops into empty space and children appear disconnected \
             at the left margin.\n\nFull output:\n{out}"
        );
    }

    /// Continuation pipes (`│`) UNDER a non-last level-1 child must also
    /// sit in the trunk column, so the visual spine reads as one
    /// continuous line from the root box down through every level-1
    /// child's branch glyph. Catches the half-fix where the first child
    /// is aligned but later children's continuation pipes drift.
    #[test]
    fn level1_continuation_pipes_align_with_trunk() {
        let src = "mindmap\n  mindmap\n    Origins\n      Long history\n    Research";
        let diag = parse(src).unwrap();
        let out = render(&diag, None);

        // The trunk line is the line whose entire content is one or more
        // ASCII spaces followed by a single `│` and nothing else. The
        // inside-box content row `│ mindmap │` superficially also contains
        // `│`, but it has alphanumerics and a leading `│`, so it is filtered
        // out by the strict shape check.
        let trunk_pipe_col = find_trunk_pipe_col(&out)
            .unwrap_or_else(|| panic!("expected trunk-pipe line in:\n{out}"));

        // The "Long history" line is a level-2 child rendered UNDER a
        // non-last level-1 child (Origins), so its prefix MUST start with
        // a continuation pipe `│` at the trunk column.
        let long_history_line = out
            .lines()
            .find(|l| l.contains("Long history"))
            .expect("Long history must appear in output");
        let pipe_col = long_history_line
            .find('\u{2502}')
            .expect("Long history's prefix must contain a continuation │");
        assert_eq!(
            pipe_col, trunk_pipe_col,
            "level-2 continuation │ (byte col {pipe_col}) must align with \
             the root trunk pipe (byte col {trunk_pipe_col}). Full \
             output:\n{out}"
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
        assert!(
            out.contains('\u{2026}'),
            "ellipsis must appear on truncated text"
        );
    }
}
