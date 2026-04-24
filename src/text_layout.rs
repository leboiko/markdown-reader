//! Width-aware span wrapping and measurement.
//!
//! This module is the **single source of truth** for display-width calculation
//! over ratatui span lists. All other code in the crate that needs to know how
//! wide a sequence of styled spans is, or how many visual rows it occupies at a
//! given terminal width, should call [`measure`] or [`wrap_spans`] rather than
//! recomputing widths inline.
//!
//! # Algorithm
//!
//! [`wrap_spans`] implements greedy word-wrap ported verbatim from
//! `src/ui/table_modal::wrap_cell_spans`, which is battle-tested against the
//! full range of ratatui `Span` styles. The steps are:
//!
//! 1. Flatten the span list to a sequence of `(char, display_width, Style)`
//!    triples, preserving per-char style.
//! 2. Split on hard `'\n'` characters to produce *hard lines*.
//! 3. Within each hard line, split on whitespace to produce *words*.
//! 4. Greedily pack words onto output rows; start a new row when a word won't
//!    fit. Words wider than `max_width` are hard-split character-by-character
//!    so every output row is guaranteed `≤ max_width` columns. Combining marks
//!    (zero-width chars) stay attached to their base char for free since a
//!    wrap boundary only fires when the next non-zero-width char would push
//!    the row past `max_width`.
//! 5. Adjacent same-style characters on the same output row are merged into a
//!    single [`WrappedSpan`].
//!
//! # Deviation from ratatui's `Wrap { trim: false }`
//!
//! Ratatui preserves leading/trailing whitespace on wrapped rows when
//! `trim: false`. This module's [`wrap_spans`] **consumes** inter-word
//! whitespace at wrap points (i.e. the space between the last word of a row
//! and the first word of the next row is dropped). This matches the behaviour
//! of `wrap_cell_spans` in the table modal and is intentional: it keeps
//! column-width accounting exact and avoids phantom indentation on continuation
//! rows. Callers that need ratatui-compatible `trim: false` behaviour should
//! continue to use `Paragraph::wrap` directly.

use unicode_width::UnicodeWidthChar;

// ── Public types ─────────────────────────────────────────────────────────────

/// A single styled chunk of text on one visual row, with its display width
/// pre-computed and cached.
///
/// `content` is owned (`String`) because [`wrap_spans`] always synthesises new
/// content from the input span character stream — there is no zero-copy path
/// to preserve `&'static str` borrows from the source `Span`s. Keeping the
/// type owned makes the allocation explicit at the call site.
///
/// `width` is a saturating cast from `usize` to `u16`; values that would
/// exceed `u16::MAX` (≈65 535 terminal columns) are clamped. In practice no
/// terminal is that wide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedSpan {
    /// The text content of this span chunk.
    pub content: String,
    /// The ratatui style applied to every character in `content`.
    pub style: ratatui::style::Style,
    /// Pre-computed display width in terminal columns.
    pub width: u16,
}

/// A single visual output row produced by [`wrap_spans`].
///
/// Each `WrappedLine` is one terminal row: no embedded newlines, guaranteed
/// `≤ max_width` columns. The cached `width` equals
/// `spans.iter().map(|s| s.width).sum()`, which avoids re-scanning the
/// content for width on every layout pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedLine {
    /// Ordered sequence of styled span chunks on this row.
    pub spans: Vec<WrappedSpan>,
    /// Total display width of the row: `spans.iter().map(|s| s.width).sum()`.
    pub width: u16,
}

impl WrappedLine {
    /// Convert to a `ratatui::text::Line<'static>` ready to feed into a
    /// `Paragraph` or `Text`. Each [`WrappedSpan`] becomes a
    /// `Span::styled(content.clone(), style)`.
    ///
    /// Used by every consumer of the wrap output (the viewer's draw path,
    /// the table renderer, char-mode yank). Centralising the conversion
    /// here means the shape of `WrappedSpan` can evolve without forcing
    /// every call site to know how to map it.
    pub fn to_ratatui_line(&self) -> ratatui::text::Line<'static> {
        ratatui::text::Line::from(
            self.spans
                .iter()
                .map(|ws| ratatui::text::Span::styled(ws.content.clone(), ws.style))
                .collect::<Vec<_>>(),
        )
    }
}

// ── Public functions ──────────────────────────────────────────────────────────

/// Greedy word-wrap of `spans` to fit within `max_width` display columns.
///
/// # Behaviour
///
/// - Always returns **at least one** element. Empty input → one empty row.
/// - Hard `'\n'` characters inside any span force a row boundary; the `'\n'`
///   itself is consumed (not present on either side of the split).
/// - Inter-word whitespace at wrap points is **consumed** (see module-level
///   docs for the deviation from ratatui's `trim: false`).
/// - Words wider than `max_width` are hard-split character-by-character. Every
///   output row is guaranteed `width ≤ max_width`. Combining marks
///   (zero-width chars) stay attached to their base char.
/// - `max_width == 0` short-circuits to one output row per hard line, each
///   with an empty `spans` vec and `width == 0`. This matches the existing
///   `line_visual_rows` short-circuit so adapters in `visual_rows.rs` stay
///   correct.
///
/// # Arguments
///
/// * `spans`     – input styled spans; borrows are ok, widths are recomputed.
/// * `max_width` – maximum display columns per output row.
pub fn wrap_spans(spans: &[ratatui::text::Span<'_>], max_width: u16) -> Vec<WrappedLine> {
    // Short-circuit: width 0 means "no wrapping info available yet"; emit one
    // empty row per hard line so callers can count logical lines.
    if max_width == 0 {
        let n = hard_line_count(spans).max(1);
        return (0..n)
            .map(|_| WrappedLine {
                spans: vec![],
                width: 0,
            })
            .collect();
    }

    // Flatten all spans to (char, char_width, Style) triples, preserving style.
    // This matches the `StyledChar` flatten in `wrap_cell_spans`.
    let styled: Vec<(char, usize, ratatui::style::Style)> = spans
        .iter()
        .flat_map(|span| {
            let style = span.style;
            span.content.chars().map(move |ch| {
                let w = UnicodeWidthChar::width(ch).unwrap_or(0);
                (ch, w, style)
            })
        })
        .collect();

    if styled.is_empty() {
        return vec![WrappedLine {
            spans: vec![],
            width: 0,
        }];
    }

    let max = max_width as usize;
    let mut result: Vec<WrappedLine> = Vec::new();

    // Iterate over hard lines delimited by '\n'.
    let mut line_start = 0usize;
    loop {
        let line_end = styled[line_start..]
            .iter()
            .position(|(ch, _, _)| *ch == '\n')
            .map_or(styled.len(), |p| line_start + p);

        emit_wrapped_hard_line(&styled[line_start..line_end], max, &mut result);

        if line_end >= styled.len() {
            break;
        }
        line_start = line_end + 1;
    }

    if result.is_empty() {
        result.push(WrappedLine {
            spans: vec![],
            width: 0,
        });
    }
    result
}

/// Total display width (in terminal columns) of a span list.
///
/// Equivalent to calling [`wrap_spans`] on a single-line input and reading
/// `result[0].width`, but faster — O(n) without any allocation.
///
/// Returns `0` for an empty span list. The result is a saturating cast to
/// `u16`; values exceeding 65 535 columns are clamped.
///
/// Use this for column-position arithmetic (cursor placement, gutter offsets,
/// highlight range boundaries). Use [`wrap_spans`] when you also need the
/// wrapped row decomposition.
pub fn measure(spans: &[ratatui::text::Span<'_>]) -> u16 {
    let total: usize = spans
        .iter()
        .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    crate::cast::u16_sat(total)
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Count the number of hard lines (newline-delimited segments) in `spans`.
///
/// Used by the `max_width == 0` short-circuit to produce the correct number
/// of empty output rows — one per hard line, or one if there are no newlines.
fn hard_line_count(spans: &[ratatui::text::Span<'_>]) -> usize {
    spans
        .iter()
        .flat_map(|s| s.content.chars())
        .filter(|&ch| ch == '\n')
        .count()
        + 1
}

/// Wrap a single hard line (guaranteed no embedded '\n') and push output rows
/// to `out`.
///
/// This is the core greedy-packing routine, ported from `emit_wrapped_hard_line`
/// in `src/ui/table_modal.rs`. The algorithm:
///
/// 1. Collect whitespace-separated word slices from the flat char list.
/// 2. For each word:
///    - If it fits on the current row (with space separator if needed), append it.
///    - If it doesn't fit, flush the current row and start a new one.
///    - If the word itself is wider than `max_width`, hard-split it
///      character-by-character. Combining marks (zero-width chars) stay
///      attached to their base char for free: a wrap boundary only fires
///      when `chunk_w + cw > max_width`, and that's never true for `cw == 0`.
fn emit_wrapped_hard_line(
    chars: &[(char, usize, ratatui::style::Style)],
    max_width: usize,
    out: &mut Vec<WrappedLine>,
) {
    // Split into whitespace-separated words (slices of the input triple vec).
    let mut words: Vec<&[(char, usize, ratatui::style::Style)]> = Vec::new();
    let mut word_start: Option<usize> = None;
    for (i, (ch, _, _)) in chars.iter().enumerate() {
        if ch.is_whitespace() {
            if let Some(start) = word_start.take() {
                words.push(&chars[start..i]);
            }
        } else if word_start.is_none() {
            word_start = Some(i);
        }
    }
    if let Some(start) = word_start {
        words.push(&chars[start..]);
    }

    if words.is_empty() {
        out.push(WrappedLine {
            spans: vec![],
            width: 0,
        });
        return;
    }

    // Accumulator for the current output row: (char, Style) pairs.
    // Using owned pairs avoids borrow complexity with the mutable accumulator.
    let mut row_buf: Vec<(char, ratatui::style::Style)> = Vec::new();
    let mut row_w = 0usize;

    for word in &words {
        let word_w: usize = word.iter().map(|(_, w, _)| w).sum();

        if word_w <= max_width {
            // Word fits on one row.
            if row_w > 0 && row_w + 1 + word_w > max_width {
                // Flush current row; this word starts the next one.
                out.push(pack_row(&row_buf));
                row_buf.clear();
                row_w = 0;
            }
            if row_w > 0 {
                // Insert inter-word space using the word's first char style.
                let space_style = word.first().map(|(_, _, s)| *s).unwrap_or_default();
                row_buf.push((' ', space_style));
                row_w += 1;
            }
            for &(ch, cw, style) in *word {
                row_buf.push((ch, style));
                row_w += cw;
            }
        } else {
            // Word is wider than max_width — hard-split at char boundaries.
            // Verbatim port of the inner loop in `wrap_cell_spans`. This keeps
            // combining marks (which have width 0) attached to their base char
            // for free: a wrap boundary only fires when `chunk_w + cw >
            // max_width`, and that's never true for `cw == 0`.
            if row_w > 0 {
                out.push(pack_row(&row_buf));
                row_buf.clear();
            }
            let mut chunk_w: usize = 0;
            for &(ch, cw, style) in *word {
                if chunk_w + cw > max_width {
                    out.push(pack_row(&row_buf));
                    row_buf.clear();
                    chunk_w = 0;
                }
                row_buf.push((ch, style));
                chunk_w += cw;
            }
            row_w = chunk_w;
        }
    }

    if !row_buf.is_empty() {
        out.push(pack_row(&row_buf));
    }
}

/// Pack a `(char, Style)` buffer into a [`WrappedLine`], merging adjacent
/// same-style chars into single [`WrappedSpan`] values.
///
/// Equivalent to `merge_char_style_pairs` + `WrappedLine` construction in one pass.
fn pack_row(pairs: &[(char, ratatui::style::Style)]) -> WrappedLine {
    let mut spans: Vec<WrappedSpan> = Vec::new();
    let mut row_width: u16 = 0;

    for &(ch, style) in pairs {
        let ch_w = crate::cast::u16_sat(UnicodeWidthChar::width(ch).unwrap_or(0));
        row_width = row_width.saturating_add(ch_w);

        if let Some(last) = spans.last_mut()
            && last.style == style
        {
            // Extend the existing span in-place — adjacent same-style chars
            // become one span.
            last.content.push(ch);
            last.width = last.width.saturating_add(ch_w);
        } else {
            spans.push(WrappedSpan {
                content: ch.to_string(),
                style,
                width: ch_w,
            });
        }
    }

    WrappedLine {
        spans,
        width: row_width,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        style::{Modifier, Style},
        text::Span,
    };

    // ── helpers ───────────────────────────────────────────────────────────────

    fn raw(s: impl Into<String>) -> Span<'static> {
        Span::raw(s.into())
    }

    fn styled(s: impl Into<String>, style: Style) -> Span<'static> {
        Span::styled(s.into(), style)
    }

    fn bold() -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    fn italic() -> Style {
        Style::default().add_modifier(Modifier::ITALIC)
    }

    /// Assert every row in `rows` has `row.width ≤ max_width` and that
    /// `row.width` matches the recomputed sum of span widths.
    fn assert_invariants(rows: &[WrappedLine], max_width: u16) {
        for (i, row) in rows.iter().enumerate() {
            let recomputed: u16 = row.spans.iter().map(|s| s.width).sum();
            assert_eq!(
                recomputed,
                row.width,
                "row {i}: cached width {w} != recomputed {recomputed}",
                w = row.width
            );
            if max_width > 0 {
                assert!(
                    row.width <= max_width,
                    "row {i} width {w} exceeds max_width {max_width}",
                    w = row.width
                );
            }
        }
    }

    /// Run `f` over several widths and assert the basic invariants each time.
    fn at_widths<F>(widths: &[u16], mut f: F)
    where
        F: FnMut(u16) -> Vec<WrappedLine>,
    {
        for &w in widths {
            let rows = f(w);
            assert!(!rows.is_empty(), "wrap_spans must never return empty vec");
            assert_invariants(&rows, w);
        }
    }

    /// Flatten soft-wrapped rows back to a single-line span list.
    ///
    /// Joins rows with a single space — the inter-word whitespace
    /// `wrap_spans` consumes at soft-wrap boundaries. **Only valid for
    /// inputs without hard newlines**: a `'\n'` is consumed at hard
    /// breaks too, but the boundary is semantically a line break, not a
    /// space, and re-wrapping the flattened form would land row breaks
    /// at different points. Used solely by `soft_wrap_idempotence`,
    /// which guards against passing hard-newline inputs by construction.
    fn flatten_soft_wrapped(rows: &[WrappedLine]) -> Vec<Span<'static>> {
        let mut out: Vec<Span<'static>> = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            if i > 0 {
                out.push(raw(" "));
            }
            for ws in &row.spans {
                out.push(Span::styled(ws.content.clone(), ws.style));
            }
        }
        out
    }

    // ── case 1: empty input ───────────────────────────────────────────────────

    #[test]
    fn empty_input_yields_one_empty_row() {
        let rows = wrap_spans(&[], 80);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].spans, vec![]);
        assert_eq!(rows[0].width, 0);
    }

    // ── case 2: single short word ─────────────────────────────────────────────

    #[test]
    fn single_short_word_one_row() {
        let rows = wrap_spans(&[raw("hello")], 80);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].width, 5);
    }

    // ── case 3: oversize word — hard-split at grapheme boundaries ─────────────

    #[test]
    fn oversize_word_hard_split() {
        let widths = [20u16, 40, 60, 80, 120, 200];
        at_widths(&widths, |w| {
            let word = "a".repeat(300);
            wrap_spans(&[raw(&word)], w)
        });
        // Also verify the split at narrow width specifically.
        let rows = wrap_spans(&[raw("abcdefghij")], 4);
        assert!(rows.len() >= 2, "long word must hard-split: {rows:?}");
        for row in &rows {
            assert!(row.width <= 4, "row too wide: {}", row.width);
        }
        // No content lost.
        let total: String = rows
            .iter()
            .flat_map(|r| r.spans.iter())
            .map(|s| s.content.as_str())
            .collect();
        assert_eq!(total, "abcdefghij");
    }

    // ── case 4: two words that fit exactly ───────────────────────────────────

    #[test]
    fn two_words_fit_exactly_one_row() {
        // "ab cd" = 5 cols; max_width = 5 → must fit on one row.
        let rows = wrap_spans(&[raw("ab cd")], 5);
        assert_eq!(rows.len(), 1, "should be one row: {rows:?}");
        assert_eq!(rows[0].width, 5);
    }

    // ── case 5: second word forces wrap ──────────────────────────────────────

    #[test]
    fn second_word_forces_wrap() {
        let widths = [20u16, 40, 60, 80, 120, 200];
        at_widths(&widths, |w| {
            // First word fills the row completely; second word must go to row 2.
            let first = "a".repeat(w as usize);
            let second = "b".repeat(w as usize);
            let input = format!("{first} {second}");
            wrap_spans(&[raw(&input)], w)
        });

        // Concrete narrow case.
        let rows = wrap_spans(&[raw("hello world")], 8);
        assert_eq!(rows.len(), 2, "second word must wrap: {rows:?}");
        // Inter-word space consumed, not carried to the next row.
        let second_row_text: String = rows[1].spans.iter().map(|s| s.content.as_str()).collect();
        assert!(
            !second_row_text.starts_with(' '),
            "leading space must be consumed"
        );
    }

    // ── case 6: hard newline mid-span ────────────────────────────────────────

    #[test]
    fn hard_newline_mid_span_forces_break() {
        let b = bold();
        let spans = [styled("first\nsecond", b)];
        let rows = wrap_spans(&spans, 80);
        assert_eq!(rows.len(), 2, "\\n must split into two rows: {rows:?}");
        // Both halves keep the bold style.
        for row in &rows {
            for ws in &row.spans {
                assert_eq!(ws.style, b, "style must be preserved across newline");
            }
        }
        // Content is intact on each side.
        let first_text: String = rows[0].spans.iter().map(|s| s.content.as_str()).collect();
        let second_text: String = rows[1].spans.iter().map(|s| s.content.as_str()).collect();
        assert_eq!(first_text, "first");
        assert_eq!(second_text, "second");
    }

    // ── case 7: mixed styles within a word ───────────────────────────────────

    #[test]
    fn mixed_styles_within_word_preserved_on_split() {
        let widths = [20u16, 40, 60, 80, 120, 200];
        at_widths(&widths, |w| {
            // A "word" composed of two adjacent styled spans (no space between).
            let spans = [styled("AAAA", bold()), styled("BBBB", italic())];
            // With a small width, the word is split across rows; both styles survive.
            wrap_spans(&spans, w)
        });

        // Narrow case: 3-col width forces a hard split inside the word.
        let spans = [styled("AAA", bold()), styled("BBB", italic())];
        let rows = wrap_spans(&spans, 3);
        // Every row must contain only bold or italic content.
        for row in &rows {
            for ws in &row.spans {
                assert!(
                    ws.style == bold() || ws.style == italic(),
                    "unexpected style on row: {:?}",
                    ws.style
                );
            }
        }
    }

    // ── case 8: wide CJK characters ──────────────────────────────────────────

    #[test]
    fn wide_cjk_no_row_exceeds_width() {
        let widths = [20u16, 40, 60, 80, 120, 200];
        at_widths(&widths, |w| {
            // Each CJK char is 2 columns wide.
            let s: String = "你好世界".repeat(20);
            wrap_spans(&[raw(&s)], w)
        });
    }

    // ── case 9: combining marks stay glued to their base char ─────────────────

    #[test]
    fn combining_marks_stay_glued() {
        // 'e' + U+0301 COMBINING ACUTE ACCENT = "é" as two code points.
        // The grapheme cluster must not be split across rows.
        let composed = "e\u{0301}"; // decomposed é
        // Repeat to force wrapping at a narrow width.
        let input: String = composed.repeat(50);
        let rows = wrap_spans(&[raw(&input)], 10);
        // Every character in every row must be followed by its combining mark
        // (no split). We detect a split by checking: no row ends in a char with
        // width == 0 that was meant to combine with the first char of the next row.
        for row in &rows {
            let text: String = row.spans.iter().map(|s| s.content.as_str()).collect();
            // Combining char should not appear at position 0 of any row text
            // (that would mean it was separated from its base).
            let first_char = text.chars().next().unwrap_or('x');
            let first_w = UnicodeWidthChar::width(first_char).unwrap_or(1);
            assert!(
                first_w > 0,
                "row must not start with a combining mark: {:?}",
                text
            );
        }
    }

    // ── case 10: max_width == 0 ───────────────────────────────────────────────

    #[test]
    fn max_width_zero_one_row_per_hard_line_width_zero() {
        // Two hard lines → two output rows, each with width 0.
        let rows = wrap_spans(&[raw("hello\nworld")], 0);
        assert_eq!(rows.len(), 2, "one row per hard line: {rows:?}");
        for row in &rows {
            assert_eq!(row.width, 0);
            assert!(row.spans.is_empty());
        }

        // No newlines → one row.
        let rows = wrap_spans(&[raw("hello")], 0);
        assert_eq!(rows.len(), 1);
    }

    // ── case 11: idempotence (soft-wrap only) ────────────────────────────────

    /// `wrap(flatten(wrap(input, w)), w) == wrap(input, w)` for soft-wrap-only
    /// inputs. Inputs containing hard `'\n'` characters do **not** satisfy
    /// this property because `flatten_soft_wrapped` cannot recover whether a
    /// row boundary was a soft wrap or a hard break — the input here is
    /// deliberately newline-free to keep the property well-defined.
    #[test]
    fn soft_wrap_idempotence() {
        let input = [raw("the quick brown fox jumps over the lazy dog")];
        debug_assert!(
            !input.iter().any(|s| s.content.as_ref().contains('\n')),
            "soft_wrap_idempotence requires newline-free input",
        );
        for &w in &[20u16, 40, 80] {
            let first = wrap_spans(&input, w);
            let flat = flatten_soft_wrapped(&first);
            let second = wrap_spans(&flat, w);
            assert_eq!(
                first.len(),
                second.len(),
                "width {w}: row count changed after re-wrap (first={}, second={})",
                first.len(),
                second.len()
            );
            for (i, (r1, r2)) in first.iter().zip(second.iter()).enumerate() {
                assert_eq!(
                    r1.width, r2.width,
                    "width {w} row {i}: width changed (first={}, second={})",
                    r1.width, r2.width
                );
            }
        }
    }

    /// Hard `'\n'` characters are **consumed** at row boundaries: they
    /// never appear in any output `WrappedSpan::content`. Companion to
    /// `soft_wrap_idempotence` — together they pin down the
    /// boundary-character semantics that `flatten_soft_wrapped` cannot
    /// round-trip on its own.
    #[test]
    fn hard_newline_consumed_never_appears_in_output() {
        let rows = wrap_spans(&[raw("alpha\nbeta\ngamma")], 80);
        assert_eq!(rows.len(), 3);
        for row in &rows {
            for ws in &row.spans {
                assert!(
                    !ws.content.contains('\n'),
                    "hard newline leaked into output: {ws:?}",
                );
            }
        }
    }

    // ── case 12: measure round-trip ───────────────────────────────────────────

    #[test]
    fn measure_round_trip() {
        let spans = [raw("hello world")];
        let m = measure(&spans);
        let rows = wrap_spans(&spans, u16::MAX);
        assert_eq!(
            rows.len(),
            1,
            "single-line input at max width should produce exactly one row"
        );
        assert_eq!(
            m, rows[0].width,
            "measure() must equal wrap_spans(..., u16::MAX)[0].width"
        );
    }

    // ── width-sweep invariant for case 3 ─────────────────────────────────────
    //
    // The `at_widths` calls within cases 3, 5, 7, 8 already cover the sweep;
    // this extra explicit test demonstrates the helper in isolation.
    #[test]
    fn width_sweep_long_word() {
        at_widths(&[20, 40, 60, 80, 120, 200], |w| {
            let s = "superlongwordwithnobreaks".repeat(8);
            wrap_spans(&[raw(&s)], w)
        });
    }
}
