#[cfg(test)]
mod unit {
    use super::super::highlight::{
        extract_line_text_range, highlight_columns, patch_cursor_highlight,
    };
    use super::super::state::{MarkdownViewState, VisualMode, VisualRange};
    use crate::markdown::{DocBlock, HeadingAnchor, LinkInfo};
    use ratatui::text::{Line, Span, Text};

    // ── VisualRange ──────────────────────────────────────────────────────────

    /// Helper to build a line-mode `VisualRange` for tests that only care about
    /// line containment (no column logic).
    fn line_range(anchor: u32, cursor: u32) -> VisualRange {
        VisualRange {
            mode: VisualMode::Line,
            anchor_line: anchor,
            anchor_col: 0,
            cursor_line: cursor,
            cursor_col: 0,
        }
    }

    /// A selection anchored at 3 with cursor at 5 should contain 3, 4, 5 and
    /// exclude lines outside the range.
    #[test]
    fn visual_range_contains_inclusive() {
        let r = line_range(3, 5);
        assert!(r.contains(3), "should contain anchor");
        assert!(r.contains(4), "should contain middle");
        assert!(r.contains(5), "should contain cursor");
        assert!(!r.contains(2), "should not contain below anchor");
        assert!(!r.contains(6), "should not contain above cursor");
    }

    /// A reversed selection (anchor > cursor) should behave identically because
    /// `top_line()`/`bottom_line()` normalise the direction.
    #[test]
    fn visual_range_contains_reversed() {
        let r = line_range(5, 3);
        assert!(r.contains(3));
        assert!(r.contains(4));
        assert!(r.contains(5));
        assert!(!r.contains(2));
        assert!(!r.contains(6));
    }

    // ── load clears visual_mode ──────────────────────────────────────────────

    #[test]
    fn load_clears_visual_mode() {
        use crate::theme::{Palette, Theme};
        let palette = Palette::from_theme(Theme::Default);
        let mut view = MarkdownViewState {
            visual_mode: Some(line_range(2, 4)),
            ..Default::default()
        };
        view.load(
            std::path::PathBuf::from("/fake/test.md"),
            "test.md".to_string(),
            "hello\nworld\n".to_string(),
            &palette,
            Theme::Default,
        );
        assert_eq!(view.visual_mode, None, "load() must clear visual_mode");
    }

    // ── cursor_down / cursor_up extend visual range ─────────────────────────

    #[test]
    fn cursor_down_in_visual_mode_extends_range() {
        let mut v = MarkdownViewState {
            total_lines: 10,
            cursor_line: 3,
            visual_mode: Some(line_range(3, 3)),
            ..Default::default()
        };
        v.cursor_down(2);
        let range = v.visual_mode.unwrap();
        assert_eq!(range.anchor_line, 3, "anchor must stay fixed");
        assert_eq!(range.cursor_line, 5, "cursor must extend down");
    }

    #[test]
    fn cursor_up_in_visual_mode_extends_range() {
        let mut v = MarkdownViewState {
            total_lines: 10,
            cursor_line: 5,
            visual_mode: Some(line_range(5, 5)),
            ..Default::default()
        };
        v.cursor_up(3);
        let range = v.visual_mode.unwrap();
        assert_eq!(range.anchor_line, 5, "anchor must stay fixed");
        assert_eq!(range.cursor_line, 2, "cursor must move up");
    }

    // ── highlight_columns ────────────────────────────────────────────────────

    /// Full-line highlight: start=0, end=width → every span gets bg patched.
    #[test]
    fn highlight_columns_full_line() {
        use ratatui::style::Color;
        let bg = Color::Rgb(100, 0, 0);
        let line = Line::from(vec![Span::raw("hello"), Span::raw(" world")]);
        let result = highlight_columns(&line, 0, 11, bg);
        for span in &result.spans {
            assert_eq!(span.style.bg, Some(bg), "all spans must carry bg");
        }
    }

    /// Partial single-span selection should produce (before, highlighted, after).
    #[test]
    fn highlight_columns_partial_single_span() {
        use ratatui::style::Color;
        let bg = Color::Rgb(0, 100, 0);
        // "hello" is 5 cells wide; select cols 1..=3 → "ell"
        let line = Line::from(Span::raw("hello"));
        let result = highlight_columns(&line, 1, 4, bg);
        // Expect: "h" (no bg), "ell" (bg), "o" (no bg)
        assert_eq!(result.spans.len(), 3, "must split into 3 spans");
        assert_eq!(result.spans[0].content.as_ref(), "h");
        assert_eq!(result.spans[0].style.bg, None);
        assert_eq!(result.spans[1].content.as_ref(), "ell");
        assert_eq!(result.spans[1].style.bg, Some(bg));
        assert_eq!(result.spans[2].content.as_ref(), "o");
        assert_eq!(result.spans[2].style.bg, None);
    }

    /// Selection across two spans: each span's base style must be preserved on
    /// the non-selected portion and patched with bg on the selected portion.
    #[test]
    fn highlight_columns_across_spans() {
        use ratatui::style::{Color, Style};
        let bg = Color::Rgb(0, 0, 200);
        let s1 = Style::default().fg(Color::Red);
        let s2 = Style::default().fg(Color::Green);
        // "abc" (3) + "def" (3) = 6 cols total; select cols 1..5 → "bcde"
        let line = Line::from(vec![Span::styled("abc", s1), Span::styled("def", s2)]);
        let result = highlight_columns(&line, 1, 5, bg);
        // Expect: "a"(s1), "bc"(s1+bg), "de"(s2+bg), "f"(s2)
        assert_eq!(result.spans.len(), 4);
        assert_eq!(result.spans[0].content.as_ref(), "a");
        assert_eq!(result.spans[0].style.fg, Some(Color::Red));
        assert_eq!(result.spans[0].style.bg, None);
        assert_eq!(result.spans[1].content.as_ref(), "bc");
        assert_eq!(result.spans[1].style.bg, Some(bg));
        assert_eq!(result.spans[2].content.as_ref(), "de");
        assert_eq!(result.spans[2].style.bg, Some(bg));
        assert_eq!(result.spans[3].content.as_ref(), "f");
        assert_eq!(result.spans[3].style.fg, Some(Color::Green));
        assert_eq!(result.spans[3].style.bg, None);
    }

    /// Empty line: `highlight_columns` must return an empty line without panicking.
    #[test]
    fn highlight_columns_empty_line() {
        use ratatui::style::Color;
        let bg = Color::Rgb(1, 2, 3);
        let line = Line::from(vec![]);
        let result = highlight_columns(&line, 0, 5, bg);
        assert!(result.spans.is_empty(), "empty line stays empty");
    }

    // ── VisualRange::char_range_on_line ──────────────────────────────────────

    /// Same-line char selection returns [`start_col`, `end_col`+1).
    #[test]
    fn visual_range_char_same_line() {
        let r = VisualRange {
            mode: VisualMode::Char,
            anchor_line: 2,
            anchor_col: 3,
            cursor_line: 2,
            cursor_col: 7,
        };
        assert_eq!(r.char_range_on_line(2, 20), Some((3, 8)));
        assert_eq!(r.char_range_on_line(1, 20), None);
        assert_eq!(r.char_range_on_line(3, 20), None);
    }

    /// Multi-line char selection: first line partial, middle full, last partial.
    #[test]
    fn visual_range_char_multi_line() {
        let r = VisualRange {
            mode: VisualMode::Char,
            anchor_line: 1,
            anchor_col: 4,
            cursor_line: 3,
            cursor_col: 2,
        };
        // Line 0: outside selection.
        assert_eq!(r.char_range_on_line(0, 10), None);
        // Line 1 (start line): from anchor_col to end of line.
        assert_eq!(r.char_range_on_line(1, 10), Some((4, 10)));
        // Line 2 (middle): full line.
        assert_eq!(r.char_range_on_line(2, 8), Some((0, 8)));
        // Line 3 (end line): from 0 to cursor_col+1.
        assert_eq!(r.char_range_on_line(3, 10), Some((0, 3)));
        // Line 4: outside.
        assert_eq!(r.char_range_on_line(4, 10), None);
    }

    /// Line mode always returns `(0, line_width)` regardless of column fields.
    #[test]
    fn visual_range_line_mode_ignores_columns() {
        let r = VisualRange {
            mode: VisualMode::Line,
            anchor_line: 2,
            anchor_col: 5,
            cursor_line: 4,
            cursor_col: 9,
        };
        assert_eq!(r.char_range_on_line(2, 15), Some((0, 15)));
        assert_eq!(r.char_range_on_line(3, 12), Some((0, 12)));
        assert_eq!(r.char_range_on_line(4, 7), Some((0, 7)));
        assert_eq!(r.char_range_on_line(1, 10), None);
    }

    // ── extract_line_text_range ──────────────────────────────────────────────

    /// Basic substring extraction from a single-span line.
    #[test]
    fn extract_line_text_range_basic() {
        let line = Line::from(Span::raw("hello world"));
        // Extract "ello" (cols 1..5)
        let s = extract_line_text_range(&line, 1, 5);
        assert_eq!(s, "ello");
    }

    /// Extract across two spans.
    #[test]
    fn extract_line_text_range_across_spans() {
        let line = Line::from(vec![Span::raw("abc"), Span::raw("def")]);
        // "bcd" = cols 1..4
        let s = extract_line_text_range(&line, 1, 4);
        assert_eq!(s, "bcd");
    }

    // ── clamp_cursor_col ─────────────────────────────────────────────────────

    /// Moving to a shorter line must clamp `cursor_col` to `line_width`-1.
    #[test]
    fn clamp_cursor_col_on_short_line() {
        // Build a view with one 10-char line followed by one 3-char line.
        let block = DocBlock::Text {
            text: Text::from(vec![
                Line::from(Span::raw("0123456789")), // line 0: width=10
                Line::from(Span::raw("abc")),        // line 1: width=3
            ]),
            links: vec![],
            heading_anchors: vec![],
            source_lines: vec![0, 1],
        };
        let mut v = MarkdownViewState {
            total_lines: 2,
            cursor_line: 0,
            cursor_col: 9, // at end of line 0
            rendered: vec![block],
            ..Default::default()
        };
        // Move down to line 1 — cursor_col must clamp from 9 to 2.
        v.cursor_down(1);
        assert_eq!(v.cursor_line, 1);
        assert_eq!(v.cursor_col, 2, "cursor_col must clamp to width-1=2");
    }

    /// Helper: build a `MarkdownViewState` with a given `total_lines` and
    /// default scroll/cursor at 0.
    fn view_with_lines(total: u32) -> MarkdownViewState {
        MarkdownViewState {
            total_lines: total,
            ..Default::default()
        }
    }

    // ── cursor_down / cursor_up ──────────────────────────────────────────────

    /// Moving down then back up the same amount must return to line 0.
    #[test]
    fn cursor_down_then_up_returns_home() {
        let mut v = view_with_lines(5);
        v.cursor_down(3);
        assert_eq!(v.cursor_line, 3);
        v.cursor_up(3);
        assert_eq!(v.cursor_line, 0);
    }

    /// Moving down more lines than the document has must clamp to the last line.
    #[test]
    fn cursor_down_clamps_to_last_line() {
        let mut v = view_with_lines(3);
        v.cursor_down(100);
        // Last valid line index = total_lines - 1 = 2.
        assert_eq!(v.cursor_line, 2);
    }

    // ── scroll_to_cursor ────────────────────────────────────────────────────

    /// When the cursor is below the viewport, `scroll_to_cursor` scrolls just
    /// enough to bring it to the bottom row of the viewport.
    ///
    /// Document: 10 lines.  `view_height`: 5.  `scroll_offset`: 0.  cursor: 7.
    /// Expected: `scroll_offset` = 7 - (5 - 1) = 3.
    #[test]
    fn cursor_scroll_follows_when_off_screen() {
        let mut v = view_with_lines(10);
        v.scroll_offset = 0;
        v.cursor_line = 7;
        v.scroll_to_cursor(5);
        assert_eq!(v.scroll_offset, 3);
    }

    /// When the cursor is already inside the viewport, `scroll_to_cursor` must
    /// not change `scroll_offset`.
    #[test]
    fn cursor_scroll_unchanged_when_already_visible() {
        let mut v = view_with_lines(20);
        v.scroll_offset = 5;
        v.cursor_line = 7;
        v.scroll_to_cursor(10);
        // cursor (7) is in [5, 15) — no adjustment needed.
        assert_eq!(v.scroll_offset, 5);
    }

    // ── source_line_at ───────────────────────────────────────────────────────

    fn make_text_block_with_sources(source_lines: Vec<u32>) -> DocBlock {
        let n = source_lines.len();
        let text_lines: Vec<Line<'static>> = (0..n)
            .map(|i| Line::from(Span::raw(format!("line {i}"))))
            .collect();
        DocBlock::Text {
            text: Text::from(text_lines),
            links: Vec::<LinkInfo>::new(),
            heading_anchors: Vec::<HeadingAnchor>::new(),
            source_lines,
        }
    }

    /// Querying each logical line in a Text block returns the expected source line.
    #[test]
    fn source_line_at_text_block_exact() {
        use crate::markdown::source_line_at;
        let block = make_text_block_with_sources(vec![0, 1, 2]);
        let blocks = vec![block];
        assert_eq!(source_line_at(&blocks, 0), 0);
        assert_eq!(source_line_at(&blocks, 1), 1);
        assert_eq!(source_line_at(&blocks, 2), 2);
    }

    /// Every logical line within a Table block must return the table's source line.
    /// A table with no `row_source_lines` data (empty) falls back to
    /// `source_line` for every row position (defensive stub path).
    #[test]
    fn source_line_at_table_block_returns_table_start() {
        use crate::markdown::source_line_at;
        use crate::markdown::{TableBlock, TableBlockId};
        let block = DocBlock::Table(TableBlock {
            id: TableBlockId(0),
            headers: vec![],
            rows: vec![],
            alignments: vec![],
            natural_widths: vec![],
            rendered_height: 4,
            source_line: 5,
            row_source_lines: vec![],
        });
        let blocks = vec![block];
        // With no row_source_lines, all positions fall back to source_line = 5.
        assert_eq!(source_line_at(&blocks, 0), 5);
        assert_eq!(source_line_at(&blocks, 3), 5);
    }

    // ── patch_cursor_highlight ───────────────────────────────────────────────

    /// Build a slice of simple `Line`s for highlight tests.
    fn make_lines(count: usize) -> Vec<Line<'static>> {
        (0..count)
            .map(|i| Line::from(Span::raw(format!("line {i}"))))
            .collect()
    }

    /// Patching a middle line must set `bg` on all its spans and leave other
    /// lines unchanged.
    #[test]
    fn patch_cursor_highlight_patches_given_line() {
        use ratatui::style::Color;
        let bg = Color::Rgb(30, 30, 100);
        let mut lines = make_lines(3);
        patch_cursor_highlight(&mut lines, 1, bg);

        // Line 1 spans must carry the bg color.
        for span in &lines[1].spans {
            assert_eq!(span.style.bg, Some(bg), "line 1 span must have bg color");
        }
        // Lines 0 and 2 must be untouched.
        for span in &lines[0].spans {
            assert_eq!(span.style.bg, None, "line 0 must be untouched");
        }
        for span in &lines[2].spans {
            assert_eq!(span.style.bg, None, "line 2 must be untouched");
        }
    }

    /// An empty line at the target index must be replaced with a space span
    /// carrying the bg color so the highlight row is visible.
    #[test]
    fn patch_cursor_highlight_fills_empty_line() {
        use ratatui::style::Color;
        let bg = Color::Rgb(50, 50, 150);
        let mut lines = vec![
            Line::from(Span::raw("before")),
            Line::from(vec![]), // empty — no spans
            Line::from(Span::raw("after")),
        ];
        patch_cursor_highlight(&mut lines, 1, bg);
        assert_eq!(
            lines[1].spans.len(),
            1,
            "empty line must have a filler span injected"
        );
        assert_eq!(
            lines[1].spans[0].content.as_ref(),
            " ",
            "filler span must be a single space"
        );
        assert_eq!(lines[1].spans[0].style.bg, Some(bg));
    }

    /// An out-of-bounds `idx` must not panic or mutate anything.
    #[test]
    fn patch_cursor_highlight_out_of_bounds_noop() {
        use ratatui::style::Color;
        let bg = Color::Rgb(10, 10, 10);
        let mut lines = make_lines(2);
        // idx == 2 is one past the end.
        patch_cursor_highlight(&mut lines, 2, bg);
        // Both lines must be unchanged.
        for line in &lines {
            for span in &line.spans {
                assert_eq!(span.style.bg, None);
            }
        }
    }

    // ── source_line_at — Table with row_source_lines ─────────────────────────

    /// Build a `TableBlock` with explicit `row_source_lines` and verify that
    /// `source_line_at` maps each rendered row to the correct source line.
    ///
    /// Layout (2 body rows):
    ///   0: top border  → header source (5)
    ///   1: header row  → 5
    ///   2: separator   → 5
    ///   3: body[0]     → 7
    ///   4: body[1]     → 8
    ///   5: bottom border → last body (8)
    #[test]
    fn source_line_at_table_block_per_row() {
        use crate::markdown::{TableBlock, TableBlockId, source_line_at};
        let block = DocBlock::Table(TableBlock {
            id: TableBlockId(0),
            headers: vec![vec![Span::raw("H")]],
            rows: vec![vec![vec![Span::raw("a")]], vec![vec![Span::raw("b")]]],
            alignments: vec![pulldown_cmark::Alignment::None],
            natural_widths: vec![1],
            rendered_height: 6,
            source_line: 5,
            row_source_lines: vec![5, 7, 8],
        });
        let blocks = vec![block];
        // top border → header fallback
        assert_eq!(source_line_at(&blocks, 0), 5, "top border -> header");
        // header row
        assert_eq!(source_line_at(&blocks, 1), 5, "header row");
        // separator
        assert_eq!(source_line_at(&blocks, 2), 5, "separator -> header");
        // body[0]
        assert_eq!(source_line_at(&blocks, 3), 7, "body[0]");
        // body[1]
        assert_eq!(source_line_at(&blocks, 4), 8, "body[1]");
        // bottom border → last body fallback
        assert_eq!(source_line_at(&blocks, 5), 8, "bottom border -> last body");
    }

    /// Edge cases: table with only a header (no body rows).
    #[test]
    fn table_row_source_line_helper_boundary_cases() {
        use crate::markdown::{TableBlock, TableBlockId, source_line_at};

        // Header-only: rendered_height = 3 (top border, header, bottom border).
        let header_only = DocBlock::Table(TableBlock {
            id: TableBlockId(1),
            headers: vec![vec![Span::raw("H")]],
            rows: vec![],
            alignments: vec![pulldown_cmark::Alignment::None],
            natural_widths: vec![1],
            rendered_height: 3,
            source_line: 10,
            row_source_lines: vec![10],
        });
        let blocks = vec![header_only];
        // Row 0 = top border → header (10)
        assert_eq!(source_line_at(&blocks, 0), 10);
        // Row 1 = header → 10
        assert_eq!(source_line_at(&blocks, 1), 10);
        // Row 2 = bottom border → last (10)
        assert_eq!(source_line_at(&blocks, 2), 10);

        // Empty row_source_lines: must not panic, must fall back to source_line.
        let empty_rsl = DocBlock::Table(TableBlock {
            id: TableBlockId(2),
            headers: vec![vec![Span::raw("H")]],
            rows: vec![vec![vec![Span::raw("a")]]],
            alignments: vec![pulldown_cmark::Alignment::None],
            natural_widths: vec![1],
            rendered_height: 4,
            source_line: 99,
            row_source_lines: vec![],
        });
        let blocks2 = vec![empty_rsl];
        // All positions must fall back to source_line without panicking.
        for i in 0..4 {
            assert_eq!(source_line_at(&blocks2, i), 99, "empty rsl row {i}");
        }
    }
}
