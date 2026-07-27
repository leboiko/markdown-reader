//! Drawing for [`crate::markdown::DocBlock::Math`] blocks.
//!
//! Mirrors [`super::mermaid_draw`] but is much smaller, because a formula has
//! only three states to render — a typeset image, a Unicode fallback box, and a
//! "still rendering" placeholder — and no full-screen modal or overflow mode.

use super::mermaid_draw::{padded_rect, render_mermaid_placeholder};
use super::state::VisualRange;
use crate::app::App;
use crate::math_image::MathEntry;
use crate::theme::{Palette, Tokens};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph},
};

/// Everything needed to draw one math block.
///
/// Bundled into a struct for the same reason as [`super::mermaid_draw::MermaidDrawParams`]:
/// to stay under clippy's argument limit without collapsing the call site into
/// a wall of positional booleans.
pub struct MathDrawParams<'a> {
    /// Whether the block is visible enough to draw the image without the
    /// widget re-fitting to a shrinking rect and visibly jittering.
    pub fully_visible: bool,
    /// Cache key for this formula.
    pub id: crate::markdown::MathBlockId,
    /// Raw LaTeX, shown in the footer of the fallback box so the user can see
    /// what they wrote when the Unicode approximation is ambiguous.
    pub source: &'a str,
    /// Whether the viewer panel has keyboard focus.
    pub focused: bool,
    /// Absolute logical-line index of the cursor.
    pub cursor_line: u32,
    /// Inclusive start of the block in absolute logical lines.
    pub block_start: u32,
    /// Exclusive end of the block in absolute logical lines.
    pub block_end: u32,
    /// Active visual-line selection, or `None` in normal mode.
    pub visual_mode: Option<VisualRange>,
}

/// Draw a math block at `rect`, dispatching on its cache state.
pub fn draw_math_block(
    f: &mut Frame,
    app: &mut App,
    rect: Rect,
    p: &Palette,
    params: &MathDrawParams,
) {
    let cursor_in_block = params.focused
        && params.cursor_line >= params.block_start
        && params.cursor_line < params.block_end;
    let selection_bg = app.tokens.state.selection_bg;
    let tokens = app.tokens;

    match app.math_cache.get_mut(params.id) {
        // No entry yet — `ensure_queued` runs on the same frame, so this is a
        // single-frame state in practice.
        None => render_mermaid_placeholder(f, rect, "math", p),
        Some(MathEntry::Pending) => render_mermaid_placeholder(f, rect, "typesetting\u{2026}", p),
        Some(MathEntry::Ready { protocol, .. }) => {
            if params.fully_visible {
                use ratatui_image::{Resize, StatefulImage};

                f.render_widget(
                    Block::default().style(Style::default().bg(p.background)),
                    rect,
                );
                // Selection/cursor bars go down first so the image composites
                // over them, leaving a coloured strip in the padding — the same
                // trick the mermaid path uses to keep visual mode legible on a
                // block that has no text rows of its own.
                for row_offset in highlighted_rows(params) {
                    let row_offset = crate::cast::u16_from_u32(row_offset);
                    if row_offset < rect.height {
                        let bar = Rect {
                            x: rect.x,
                            y: rect.y + row_offset,
                            width: rect.width,
                            height: 1,
                        };
                        f.render_widget(
                            Block::default().style(Style::default().bg(selection_bg)),
                            bar,
                        );
                    }
                }

                // Narrower horizontal padding than mermaid (2 vs 4): a formula
                // is typically much smaller than a diagram, and `Resize::Fit`
                // preserves aspect ratio, so wide padding would shrink it
                // needlessly.
                let image = StatefulImage::new().resize(Resize::Fit(None));
                f.render_stateful_widget(image, padded_rect(rect, 2, 0), protocol.as_mut());
            } else {
                render_mermaid_placeholder(f, rect, "scroll to view formula", p);
            }
        }
        Some(MathEntry::Unicode { text, reason }) => {
            let body = build_unicode_text(text, params.source, reason, &tokens, p);
            render_unicode_block(f, rect, body, p, params, selection_bg, cursor_in_block);
        }
    }
}

/// Rows of the block (block-relative) that should carry a selection bar.
fn highlighted_rows(params: &MathDrawParams) -> Vec<u32> {
    match params.visual_mode {
        Some(range) => (0..params.block_end.saturating_sub(params.block_start))
            .filter(|&offset| range.contains(params.block_start + offset))
            .collect(),
        None if params.focused
            && params.cursor_line >= params.block_start
            && params.cursor_line < params.block_end =>
        {
            vec![params.cursor_line - params.block_start]
        }
        None => vec![],
    }
}

/// Build the styled body of the Unicode fallback box.
///
/// The footer carries the reason and the raw LaTeX, so a user whose terminal
/// cannot show images still learns *why* and can read the original source when
/// the Unicode approximation is ambiguous (`((a)/(b))/(c)` being the motivating
/// example).
fn build_unicode_text(
    unicode: &str,
    latex: &str,
    reason: &str,
    tokens: &Tokens,
    p: &Palette,
) -> Text<'static> {
    let math_style = Style::default()
        .fg(tokens.syntax.code_fg)
        .bg(tokens.surface.raised)
        .add_modifier(Modifier::ITALIC);

    let mut lines: Vec<Line<'static>> = unicode
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), math_style)))
        .collect();
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(String::new(), math_style)));
    }
    lines.push(Line::from(Span::styled(
        format!("[math \u{2014} {reason}] {}", truncate(latex.trim(), 60)),
        p.dim_style(),
    )));
    Text::from(lines)
}

/// Render the Unicode fallback as a bordered box, applying the cursor or
/// visual-selection highlight when the viewer is focused.
fn render_unicode_block(
    f: &mut Frame,
    rect: Rect,
    mut text: Text<'static>,
    p: &Palette,
    params: &MathDrawParams,
    selection_bg: ratatui::style::Color,
    cursor_in_block: bool,
) {
    if params.focused && (cursor_in_block || params.visual_mode.is_some()) {
        super::highlight::apply_block_highlight(
            &mut text.lines,
            params.visual_mode,
            params.cursor_line,
            params.block_start,
            params.block_end,
            0,
            selection_bg,
        );
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(p.border_style())
        .style(Style::default().bg(p.background));
    // No `Wrap`: a formula that overflows the pane is clipped rather than
    // reflowed, because wrapping a formula mid-expression is less readable than
    // truncating it, and the block's reserved height assumes no wrap rows.
    f.render_widget(Paragraph::new(text).block(block), rect);
}

/// Truncate `s` to `max` characters, appending `…` when it was cut.
///
/// Operates on chars, not bytes, so a multi-byte LaTeX command cannot be split
/// mid-character.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('\u{2026}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn tokens() -> Tokens {
        Tokens::from_theme(Theme::Default)
    }

    fn palette() -> Palette {
        Palette::from_theme(Theme::Default)
    }

    /// The fallback box must show the Unicode approximation *and* a footer that
    /// names the reason and echoes the LaTeX. Each is checked separately so a
    /// regression that drops one is not masked by the others.
    #[test]
    fn unicode_fallback_shows_formula_reason_and_source() {
        let text = build_unicode_text("a/b", r"\frac{a}{b}", "tmux", &tokens(), &palette());
        let rendered: Vec<String> = text
            .lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();

        assert!(
            rendered.iter().any(|l| l.contains("a/b")),
            "Unicode approximation missing: {rendered:?}",
        );
        let footer = rendered.last().expect("footer line");
        assert!(
            footer.contains("tmux"),
            "reason missing from footer: {footer}"
        );
        assert!(
            footer.contains(r"\frac{a}{b}"),
            "raw LaTeX missing from footer: {footer}",
        );
    }

    /// A multi-line Unicode approximation must produce one styled line per row,
    /// plus exactly one footer — otherwise the block's reserved height (line
    /// count + 2 borders) would not match what is drawn.
    #[test]
    fn unicode_fallback_line_count_matches_reserved_height() {
        let text = build_unicode_text(
            "line1\nline2\nline3",
            "x",
            "text mode",
            &tokens(),
            &palette(),
        );
        assert_eq!(
            text.lines.len(),
            4,
            "expected 3 content lines + 1 footer, got {}",
            text.lines.len(),
        );
    }

    /// An empty approximation must still yield a drawable body — a zero-line
    /// `Text` inside a bordered block renders as a broken frame.
    #[test]
    fn empty_unicode_still_produces_a_line() {
        let text = build_unicode_text("", "x", "text mode", &tokens(), &palette());
        assert!(
            text.lines.len() >= 2,
            "expected at least a content line and a footer, got {}",
            text.lines.len(),
        );
    }

    /// `truncate` must cut on character boundaries, never bytes — a LaTeX
    /// string containing multi-byte characters must not panic or split one.
    #[test]
    fn truncate_is_char_safe_and_marks_elision() {
        assert_eq!(truncate("abc", 10), "abc");
        assert_eq!(truncate("abcdef", 3), "abc\u{2026}");

        // 6 multi-byte chars (each 2 bytes) truncated to 3 chars.
        let cyrillic = "абвгде";
        let out = truncate(cyrillic, 3);
        assert_eq!(out, "абв\u{2026}");
        assert_eq!(out.chars().count(), 4);
    }

    /// In visual mode every selected row of the block gets a bar; in normal
    /// mode only the cursor's row does; unfocused gets none.
    #[test]
    fn highlighted_rows_tracks_focus_cursor_and_selection() {
        let base = MathDrawParams {
            fully_visible: true,
            id: crate::markdown::MathBlockId(0),
            source: "x",
            focused: true,
            cursor_line: 11,
            block_start: 10,
            block_end: 14,
            visual_mode: None,
        };

        assert_eq!(
            highlighted_rows(&base),
            vec![1],
            "focused cursor on line 11 of a block starting at 10 → row offset 1",
        );

        let unfocused = MathDrawParams {
            focused: false,
            ..base
        };
        assert!(
            highlighted_rows(&unfocused).is_empty(),
            "an unfocused viewer must not draw a cursor bar",
        );

        let selected = MathDrawParams {
            visual_mode: Some(VisualRange {
                mode: super::super::state::VisualMode::Line,
                anchor_line: 10,
                anchor_col: 0,
                cursor_line: 12,
                cursor_col: 0,
            }),
            ..base
        };
        assert_eq!(
            highlighted_rows(&selected),
            vec![0, 1, 2],
            "a visual selection covering lines 10..=12 must bar three rows",
        );
    }
}
