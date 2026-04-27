#[cfg(test)]
mod unit {
    use super::super::highlight::{
        extract_line_text_range, highlight_columns, patch_cursor_highlight,
    };
    use super::super::state::{MarkdownViewState, VisualMode, VisualRange};
    use crate::markdown::{DocBlock, HeadingAnchor, LinkInfo, TextBlockId};
    use ratatui::text::{Line, Span, Text};
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

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
        let src_lines = vec![0u32, 1];
        let n = src_lines.len();
        let text_lines_clamp = vec![
            Line::from(Span::raw("0123456789")),
            Line::from(Span::raw("abc")),
        ];
        let block_id = {
            let mut h = DefaultHasher::new();
            for line in &text_lines_clamp {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            n.hash(&mut h);
            TextBlockId(h.finish())
        };
        let block = DocBlock::Text {
            id: block_id,
            text: Text::from(text_lines_clamp),
            links: vec![],
            heading_anchors: vec![],
            source_lines: src_lines,
            wrapped_height: std::cell::Cell::new(2),
            source_byte_start: 0,
            source_byte_end: 0,
        };
        // Populate text_layouts so `current_line_width` can read wrapped row widths.
        // At width 80 neither line wraps — row 0 width=10, row 1 width=3.
        let mut text_layouts = std::collections::HashMap::new();
        crate::markdown::update_text_layouts(std::slice::from_ref(&block), &mut text_layouts, 80);
        let mut v = MarkdownViewState {
            total_lines: 2,
            cursor_line: 0,
            cursor_col: 9, // at end of line 0
            rendered: vec![block],
            text_layouts,
            ..Default::default()
        };
        // Move down to line 1 — cursor_col must clamp from 9 to 2 (width 3 - 1).
        v.cursor_down(1);
        assert_eq!(v.cursor_line, 1);
        assert_eq!(v.cursor_col, 2, "cursor_col must clamp to width-1=2");
    }

    /// `current_line_width` must convert the visual-row `cursor_line` into a
    /// logical line index before looking it up in `text.lines`. Otherwise
    /// the cursor sitting on a wrapped paragraph indexes past the end of
    /// `text.lines`, returns 0, and `clamp_cursor_col` resets `cursor_col`
    /// to 0 — breaking horizontal arrow movement (regression caught right
    /// after 1.18.4 shipped).
    #[test]
    fn current_line_width_handles_wrapped_lines() {
        // After Phase 3, `current_line_width` reads from the `text_layouts` cache
        // (pre-wrapped rows), not from `text.lines` directly. Populate the cache
        // with width=20 so the 50-'a' line wraps to 3 physical rows of widths
        // 20, 20, 10. The cursor on physical row 2 (visual row 2 within the block)
        // is the third wrapped row — width 10.
        let long: String = "a".repeat(50);
        let src_lines = vec![0u32, 1, 2];
        let n = src_lines.len();
        let text_lines_wrap = vec![
            Line::from(Span::raw("short")),
            Line::from(Span::raw(long.clone())),
            Line::from(Span::raw("end")),
        ];
        let block_id = {
            let mut h = DefaultHasher::new();
            for line in &text_lines_wrap {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            n.hash(&mut h);
            TextBlockId(h.finish())
        };
        let block = DocBlock::Text {
            id: block_id,
            text: Text::from(text_lines_wrap),
            links: vec![],
            heading_anchors: vec![],
            source_lines: src_lines,
            // Width 20 -> long line wraps: rows are 20+20+10 = 5 total.
            wrapped_height: std::cell::Cell::new(5),
            source_byte_start: 0,
            source_byte_end: 0,
        };
        // Build the text layout cache at width 20.
        let mut text_layouts = std::collections::HashMap::new();
        crate::markdown::update_text_layouts(std::slice::from_ref(&block), &mut text_layouts, 20);
        let v = MarkdownViewState {
            total_lines: 5,
            cursor_line: 2, // physical row 2 = second wrapped row of the long line (width 20)
            cursor_col: 0,
            rendered: vec![block],
            layout_width: 20,
            text_layouts,
            ..Default::default()
        };
        // Physical rows: 0="short"(5), 1=aa...(20), 2=aa...(20), 3=aa...(10), 4="end"(3).
        // Cursor at row 2 → second 20-char chunk of the wrapped long line.
        assert_eq!(v.current_line_width(), 20);
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
        // Hash rendered text content (not source_lines) — stable across line-number shifts.
        let block_id = {
            let mut h = DefaultHasher::new();
            for line in &text_lines {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            n.hash(&mut h);
            TextBlockId(h.finish())
        };
        DocBlock::Text {
            id: block_id,
            text: Text::from(text_lines),
            links: Vec::<LinkInfo>::new(),
            heading_anchors: Vec::<HeadingAnchor>::new(),
            source_lines,
            wrapped_height: std::cell::Cell::new(crate::cast::u32_sat(n)),
            source_byte_start: 0,
            source_byte_end: 0,
        }
    }

    /// Querying each logical line in a Text block returns the expected source line.
    #[test]
    fn source_line_at_text_block_exact() {
        use crate::markdown::{source_line_at, update_text_layouts};
        use std::collections::HashMap;
        let block = make_text_block_with_sources(vec![0, 1, 2]);
        // Populate text_layouts so `source_line_at` can use `physical_to_logical`.
        // Width 80 — all lines fit on one row each (no wrapping); logical == physical.
        let mut tl = HashMap::new();
        update_text_layouts(std::slice::from_ref(&block), &mut tl, 80);
        let blocks = vec![block];
        let bl = HashMap::new();
        assert_eq!(source_line_at(&blocks, 0, &tl, &bl), 0);
        assert_eq!(source_line_at(&blocks, 1, &tl, &bl), 1);
        assert_eq!(source_line_at(&blocks, 2, &tl, &bl), 2);
    }

    /// Every logical line within a Table block must return the table's source line.
    /// A table with no `row_source_lines` data (empty) falls back to
    /// `source_line` for every row position (defensive stub path).
    #[test]
    fn source_line_at_table_block_returns_table_start() {
        use crate::markdown::source_line_at;
        use crate::markdown::{TableBlock, TableBlockId};
        use std::collections::HashMap;
        let block = DocBlock::Table(TableBlock {
            id: TableBlockId(0),
            headers: vec![],
            rows: vec![],
            alignments: vec![],
            natural_widths: vec![],
            rendered_height: 4,
            source_line: 5,
            row_source_lines: vec![],
            source_byte_start: 0,
            source_byte_end: 0,
        });
        let blocks = vec![block];
        let tl = HashMap::new();
        let bl = HashMap::new();
        // With no row_source_lines, all positions fall back to source_line = 5.
        assert_eq!(source_line_at(&blocks, 0, &tl, &bl), 5);
        assert_eq!(source_line_at(&blocks, 3, &tl, &bl), 5);
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
        use std::collections::HashMap;
        let block = DocBlock::Table(TableBlock {
            id: TableBlockId(0),
            headers: vec![vec![Span::raw("H")]],
            rows: vec![vec![vec![Span::raw("a")]], vec![vec![Span::raw("b")]]],
            alignments: vec![pulldown_cmark::Alignment::None],
            natural_widths: vec![1],
            rendered_height: 6,
            source_line: 5,
            row_source_lines: vec![5, 7, 8],
            source_byte_start: 0,
            source_byte_end: 0,
        });
        let blocks = vec![block];
        let tl = HashMap::new();
        let bl = HashMap::new();
        // top border → header fallback
        assert_eq!(
            source_line_at(&blocks, 0, &tl, &bl),
            5,
            "top border -> header"
        );
        // header row
        assert_eq!(source_line_at(&blocks, 1, &tl, &bl), 5, "header row");
        // separator
        assert_eq!(
            source_line_at(&blocks, 2, &tl, &bl),
            5,
            "separator -> header"
        );
        // body[0]
        assert_eq!(source_line_at(&blocks, 3, &tl, &bl), 7, "body[0]");
        // body[1]
        assert_eq!(source_line_at(&blocks, 4, &tl, &bl), 8, "body[1]");
        // bottom border → last body fallback
        assert_eq!(
            source_line_at(&blocks, 5, &tl, &bl),
            8,
            "bottom border -> last body"
        );
    }

    /// Edge cases: table with only a header (no body rows).
    #[test]
    fn table_row_source_line_helper_boundary_cases() {
        use crate::markdown::{TableBlock, TableBlockId, source_line_at};
        use std::collections::HashMap;
        let tl = HashMap::new();
        let bl = HashMap::new();

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
            source_byte_start: 0,
            source_byte_end: 0,
        });
        let blocks = vec![header_only];
        // Row 0 = top border → header (10)
        assert_eq!(source_line_at(&blocks, 0, &tl, &bl), 10);
        // Row 1 = header → 10
        assert_eq!(source_line_at(&blocks, 1, &tl, &bl), 10);
        // Row 2 = bottom border → last (10)
        assert_eq!(source_line_at(&blocks, 2, &tl, &bl), 10);

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
            source_byte_start: 0,
            source_byte_end: 0,
        });
        let blocks2 = vec![empty_rsl];
        // All positions must fall back to source_line without panicking.
        for i in 0..4 {
            assert_eq!(
                source_line_at(&blocks2, i, &tl, &bl),
                99,
                "empty rsl row {i}"
            );
        }
    }

    // ── Phase 3 cache tests ──────────────────────────────────────────────────

    /// After `update_text_layouts` runs, the `text_layouts` HashMap must contain
    /// an entry for the Text block's id. The entry's `wrapped` length must be
    /// at least as large as the number of logical lines (each line is >= 1 row).
    #[test]
    fn text_layout_cache_populated_on_width_change() {
        use crate::markdown::update_text_layouts;
        use std::collections::HashMap;
        let block = make_text_block_with_sources(vec![0, 1, 2]);
        let id = if let DocBlock::Text { id, .. } = &block {
            *id
        } else {
            panic!("expected Text block")
        };
        let mut cache = HashMap::new();
        update_text_layouts(std::slice::from_ref(&block), &mut cache, 80);
        assert!(
            cache.contains_key(&id),
            "cache must have entry for block id"
        );
        let layout = &cache[&id];
        // 3 logical lines at width 80 → 3 physical rows (no wrapping needed).
        assert_eq!(layout.wrapped.len(), 3);
        assert_eq!(layout.physical_to_logical, vec![0, 1, 2]);
    }

    /// The `wrapped_height` cell on the block must equal `layout.wrapped.len()`
    /// after `update_text_layouts` runs. This ensures `DocBlock::height()` returns
    /// the correct visual-row count for scroll math.
    #[test]
    fn text_layout_cache_height_matches_wrapped_len() {
        use crate::markdown::update_text_layouts;
        use std::collections::HashMap;
        // 50 'a' chars at width 20 wraps to 3 physical rows.
        let long: String = "a".repeat(50);
        let src_lines = vec![0u32, 1];
        let n = src_lines.len();
        let text_lines_height = vec![
            Line::from(Span::raw("short")),
            Line::from(Span::raw(long.clone())),
        ];
        let block_id = {
            let mut h = DefaultHasher::new();
            for line in &text_lines_height {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            n.hash(&mut h);
            TextBlockId(h.finish())
        };
        let block = DocBlock::Text {
            id: block_id,
            text: Text::from(text_lines_height),
            links: vec![],
            heading_anchors: vec![],
            source_lines: src_lines,
            wrapped_height: std::cell::Cell::new(2),
            source_byte_start: 0,
            source_byte_end: 0,
        };
        let mut cache = HashMap::new();
        update_text_layouts(std::slice::from_ref(&block), &mut cache, 20);
        let layout = cache.get(&block_id).expect("cache must have entry");
        // 1 + 3 = 4 physical rows total.
        assert_eq!(layout.wrapped.len(), 4);
        // Block's wrapped_height cell must be updated to match.
        if let DocBlock::Text { wrapped_height, .. } = &block {
            assert_eq!(
                wrapped_height.get(),
                layout.wrapped.len() as u32,
                "wrapped_height must equal wrapped.len()"
            );
        }
    }

    /// `current_line_width` must return the display width of the wrapped row
    /// under the cursor at each layout width. A 40-char line at width 20 wraps
    /// into two rows of 20; at width 80 it stays on one row of 40.
    #[test]
    fn current_line_width_at_widths_20_40_80_120() {
        use crate::markdown::update_text_layouts;
        let content: String = "a".repeat(40); // 40 display columns
        let src_lines = vec![0u32];
        let n = src_lines.len();
        let text_line_40 = vec![Line::from(Span::raw(content.clone()))];
        let block_id = {
            let mut h = DefaultHasher::new();
            for line in &text_line_40 {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            n.hash(&mut h);
            TextBlockId(h.finish())
        };

        for &width in &[20u16, 40, 80, 120] {
            let block = DocBlock::Text {
                id: block_id,
                text: Text::from(vec![Line::from(Span::raw(content.clone()))]),
                links: vec![],
                heading_anchors: vec![],
                source_lines: src_lines.clone(),
                wrapped_height: std::cell::Cell::new(1),
                source_byte_start: 0,
                source_byte_end: 0,
            };
            let mut cache = std::collections::HashMap::new();
            update_text_layouts(std::slice::from_ref(&block), &mut cache, width);
            let v = MarkdownViewState {
                total_lines: block.height(),
                cursor_line: 0,
                rendered: vec![block],
                text_layouts: cache,
                ..Default::default()
            };
            // At width 20: row 0 is 20 chars. At width 40+: row 0 is 40 chars.
            let expected = width.min(40);
            assert_eq!(
                v.current_line_width(),
                expected,
                "width={width}: row 0 should be {expected} cols wide"
            );
        }
    }

    /// `current_line_width` on physical row 1 (the tail of a wrapped paragraph)
    /// must return the tail width, not the full logical line width. The 'l' key
    /// handler uses `current_line_width()` to clamp cursor_col, so this ensures
    /// the cursor cannot be placed past the visible text on continuation rows.
    #[test]
    fn cursor_horizontal_movement_on_wrapped_paragraph() {
        use crate::markdown::update_text_layouts;
        // "a" * 30 at width 20 → row 0: 20 cols, row 1: 10 cols.
        let content: String = "a".repeat(30);
        let src_lines = vec![0u32];
        let n = src_lines.len();
        let text_line_30 = vec![Line::from(Span::raw(content.clone()))];
        let block_id = {
            let mut h = DefaultHasher::new();
            for line in &text_line_30 {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            n.hash(&mut h);
            TextBlockId(h.finish())
        };
        let block = DocBlock::Text {
            id: block_id,
            text: Text::from(text_line_30),
            links: vec![],
            heading_anchors: vec![],
            source_lines: src_lines,
            wrapped_height: std::cell::Cell::new(2),
            source_byte_start: 0,
            source_byte_end: 0,
        };
        let mut cache = std::collections::HashMap::new();
        update_text_layouts(std::slice::from_ref(&block), &mut cache, 20);
        let v = MarkdownViewState {
            total_lines: 2,
            cursor_line: 1, // physical row 1 = tail (10 cols)
            cursor_col: 0,
            rendered: vec![block],
            text_layouts: cache,
            ..Default::default()
        };
        // The max valid cursor_col for row 1 is width - 1 = 9.
        let line_width = v.current_line_width();
        assert_eq!(line_width, 10, "tail row width must be 10 (30 mod 20)");
        assert_eq!(
            line_width.saturating_sub(1),
            9,
            "max cursor col on tail row must be 9"
        );
    }

    /// After `load()` re-renders a file, `text_layouts` must be empty (the cache
    /// is cleared). This is the invalidation path: any stale layout entries from
    /// the previous file are dropped, and the draw loop will re-populate on the
    /// next frame.
    #[test]
    fn text_layout_cache_invalidated_on_load() {
        use crate::markdown::update_text_layouts;
        use crate::theme::{Palette, Theme};
        let palette = Palette::from_theme(Theme::Default);
        let block = make_text_block_with_sources(vec![0, 1]);
        let mut cache = std::collections::HashMap::new();
        update_text_layouts(std::slice::from_ref(&block), &mut cache, 80);
        assert!(!cache.is_empty(), "pre-condition: cache must be populated");

        let mut view = MarkdownViewState {
            text_layouts: cache,
            ..Default::default()
        };
        view.load(
            std::path::PathBuf::from("/fake/reload.md"),
            "reload.md".to_string(),
            "hello\nworld\n".to_string(),
            &palette,
            Theme::Default,
        );
        assert!(
            view.text_layouts.is_empty(),
            "load() must clear the text_layouts cache"
        );
    }

    /// `recompute_positions` must use the `text_layouts` cache to map block-relative
    /// logical line indices to absolute visual rows. A link on logical line 1 of a
    /// block that starts at visual row 0 should end up at absolute line 1 (or the
    /// first physical row of logical line 1 when the cache is populated).
    #[test]
    fn recompute_positions_uses_cached_layout() {
        use crate::markdown::{LinkInfo, update_text_layouts};
        let link = LinkInfo {
            line: 1, // logical line 1 within the block
            col_start: 0,
            col_end: 5,
            url: "#section".to_string(),
            text: "sec".to_string(),
        };
        let src_lines = vec![0u32, 1, 2];
        let n = src_lines.len();
        let text_lines_recomp = vec![
            Line::from(Span::raw("line 0")),
            Line::from(Span::raw("line 1")),
            Line::from(Span::raw("line 2")),
        ];
        let block_id = {
            let mut h = DefaultHasher::new();
            for line in &text_lines_recomp {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            n.hash(&mut h);
            TextBlockId(h.finish())
        };
        let block = DocBlock::Text {
            id: block_id,
            text: Text::from(text_lines_recomp),
            links: vec![link],
            heading_anchors: vec![],
            source_lines: src_lines,
            wrapped_height: std::cell::Cell::new(3),
            source_byte_start: 0,
            source_byte_end: 0,
        };
        let mut cache = std::collections::HashMap::new();
        update_text_layouts(std::slice::from_ref(&block), &mut cache, 80);
        let mut v = MarkdownViewState {
            total_lines: 3,
            rendered: vec![block],
            text_layouts: cache,
            ..Default::default()
        };
        v.recompute_positions();
        // Logical line 1 maps to physical row 1 at width 80 (no wrapping).
        // Absolute visual row = block_start (0) + physical_row (1) = 1.
        let link = v.links.first().expect("must have one link");
        assert_eq!(
            link.line, 1,
            "link on logical line 1 must map to visual row 1"
        );
    }

    /// `collect_match_lines` with a populated text_layouts cache must match
    /// against physical (wrapped) rows, not logical lines. A search term that
    /// appears in the second physical row of a wrapped logical line must return
    /// the physical row's absolute index, not the logical line's index.
    #[test]
    fn collect_match_lines_text_block_uses_cache() {
        use crate::app::collect_match_lines;
        use crate::markdown::update_text_layouts;
        use crate::mermaid::MermaidCache;
        use std::collections::HashMap;

        // A line that fits in one row — "hello world" is 11 cols, fits at width 80.
        let block = make_text_block_with_sources(vec![0, 1, 2]);
        let mut text_layouts = HashMap::new();
        update_text_layouts(std::slice::from_ref(&block), &mut text_layouts, 80);
        let blocks = vec![block];
        let table_layouts = HashMap::new();
        let cache = MermaidCache::new();

        // "line 1" appears in logical line 1 = physical row 1.
        let matches = collect_match_lines(&blocks, &text_layouts, &table_layouts, &cache, "line 1");
        assert_eq!(matches, vec![1], "match must land on physical row 1");
    }

    /// When the text_layouts cache is empty (before the first draw), `collect_match_lines`
    /// must fall back to iterating logical lines without panicking. The result may
    /// differ from the post-wrap result but must not be empty for a term that exists.
    #[test]
    fn text_block_with_no_cache_falls_back_safely() {
        use crate::app::collect_match_lines;
        use crate::mermaid::MermaidCache;
        use std::collections::HashMap;

        let block = make_text_block_with_sources(vec![0, 1, 2]);
        let blocks = vec![block];
        let text_layouts = HashMap::new(); // empty — simulates pre-draw state
        let table_layouts = HashMap::new();
        let cache = MermaidCache::new();

        // "line 2" is in logical line 2; fallback iterates `text.lines`.
        let matches = collect_match_lines(&blocks, &text_layouts, &table_layouts, &cache, "line 2");
        assert!(
            !matches.is_empty(),
            "fallback must still find matches in logical lines"
        );
    }

    /// The blank-padding count in the gutter for a wrapped logical line must equal
    /// the number of extra physical rows produced by wrapping (wrap_count - 1).
    /// A 50-char line at width 20 produces 3 physical rows → 2 blank gutter rows.
    #[test]
    fn gutter_blank_padding_count_matches_wrap_count() {
        use crate::markdown::update_text_layouts;
        // "a" * 50 at width 20 → 3 physical rows.
        let long = "a".repeat(50);
        let src_lines = vec![0u32];
        let n = src_lines.len();
        let text_line_gutter = vec![Line::from(Span::raw(long.clone()))];
        let block_id = {
            let mut h = DefaultHasher::new();
            for line in &text_line_gutter {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            n.hash(&mut h);
            TextBlockId(h.finish())
        };
        let block = DocBlock::Text {
            id: block_id,
            text: Text::from(text_line_gutter),
            links: vec![],
            heading_anchors: vec![],
            source_lines: src_lines,
            wrapped_height: std::cell::Cell::new(3),
            source_byte_start: 0,
            source_byte_end: 0,
        };
        let mut cache = std::collections::HashMap::new();
        update_text_layouts(std::slice::from_ref(&block), &mut cache, 20);
        let layout = cache.get(&block_id).unwrap();

        // Count how many physical rows belong to logical line 0.
        let wrap_count = layout
            .physical_to_logical
            .iter()
            .filter(|&&l| l == 0)
            .count();
        // The first row gets a number; wrap_count - 1 rows get blank entries.
        assert_eq!(wrap_count, 3, "50 chars at width 20 must wrap to 3 rows");
        // Blank padding rows = wrap_count - 1 = 2.
        let blank_count = wrap_count - 1;
        assert_eq!(
            blank_count, 2,
            "2 blank gutter rows expected for a 3-row wrap"
        );
    }

    /// When a document has a link on logical line 1 of a Text block and the
    /// `text_layouts` cache is populated, `recompute_positions` must store the
    /// link at the correct visual row. A `try_follow_link_click`-style lookup
    /// must find the link at its physical row position.
    #[test]
    fn link_picker_jumps_to_link_on_wrapped_paragraph() {
        use crate::markdown::{LinkInfo, update_text_layouts};
        // Block: 2 logical lines, each fits on 1 row at width 80.
        // Link is on logical line 1.
        let link = LinkInfo {
            line: 1,
            col_start: 0,
            col_end: 4,
            url: "#target".to_string(),
            text: "link".to_string(),
        };
        let src_lines = vec![0u32, 1];
        let n = src_lines.len();
        let text_lines_link = vec![
            Line::from(Span::raw("first line")),
            Line::from(Span::raw("link text")),
        ];
        let block_id = {
            let mut h = DefaultHasher::new();
            for line in &text_lines_link {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            n.hash(&mut h);
            TextBlockId(h.finish())
        };
        let block = DocBlock::Text {
            id: block_id,
            text: Text::from(text_lines_link),
            links: vec![link],
            heading_anchors: vec![],
            source_lines: src_lines,
            wrapped_height: std::cell::Cell::new(2),
            source_byte_start: 0,
            source_byte_end: 0,
        };
        let mut cache = std::collections::HashMap::new();
        update_text_layouts(std::slice::from_ref(&block), &mut cache, 80);
        let mut v = MarkdownViewState {
            total_lines: 2,
            rendered: vec![block],
            text_layouts: cache,
            ..Default::default()
        };
        v.recompute_positions();
        // Link on logical line 1 → physical row 1 (no wrapping at width 80).
        // Absolute line = 0 (block offset) + 1 (physical row) = 1.
        let l = v.links.first().expect("must have one link");
        assert_eq!(l.line, 1, "link must be at visual row 1");
        assert_eq!(l.url, "#target");
    }

    /// A search term that appears in the second physical row of a wrapped logical
    /// line must return the second row's absolute index (not the logical line's
    /// index). This exercises `collect_match_lines`'s physical-row iteration path.
    ///
    /// Layout at width 20:
    ///   logical line 0: "aaaaaaaaaaaaaaaaaaaa needle" → two words
    ///     physical row 0: "aaaaaaaaaaaaaaaaaaaa" (20 cols)
    ///     physical row 1: "needle"               ( 6 cols)
    ///   logical line 1: "other" → physical row 2
    #[test]
    fn doc_search_match_lands_on_correct_visual_row() {
        use crate::app::collect_match_lines;
        use crate::markdown::update_text_layouts;
        use crate::mermaid::MermaidCache;
        use std::collections::HashMap;

        // Word-wrap: 20 'a' chars + " needle" — the word "needle" is pushed to row 1.
        let line0 = format!("{} needle", "a".repeat(20));
        let src_lines = vec![0u32, 1];
        let n = src_lines.len();
        let text_lines_needle = vec![
            Line::from(Span::raw(line0.clone())),
            Line::from(Span::raw("other")),
        ];
        let block_id = {
            let mut h = DefaultHasher::new();
            for line in &text_lines_needle {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            n.hash(&mut h);
            TextBlockId(h.finish())
        };
        let block = DocBlock::Text {
            id: block_id,
            text: Text::from(text_lines_needle),
            links: vec![],
            heading_anchors: vec![],
            source_lines: src_lines,
            wrapped_height: std::cell::Cell::new(3),
            source_byte_start: 0,
            source_byte_end: 0,
        };
        let mut tl = HashMap::new();
        update_text_layouts(std::slice::from_ref(&block), &mut tl, 20);
        let blocks = vec![block];
        let bl = HashMap::new();
        let cache = MermaidCache::new();
        let matches = collect_match_lines(&blocks, &tl, &bl, &cache, "needle");
        // Word "needle" is on physical row 1.
        assert_eq!(matches, vec![1], "needle must be on physical row 1");
    }

    /// `source_line_at` on a Text block must correctly map physical rows when
    /// the cache is populated. A 2-line text block at width 80 maps physical
    /// row 0 → source 0, physical row 1 → source 1.
    #[test]
    fn source_line_at_with_cache_resolves_physical_rows() {
        use crate::markdown::{source_line_at, update_text_layouts};
        use std::collections::HashMap;
        let block = make_text_block_with_sources(vec![10, 20]);
        let mut tl = HashMap::new();
        update_text_layouts(std::slice::from_ref(&block), &mut tl, 80);
        let blocks = vec![block];
        let bl = HashMap::new();
        // At width 80, no wrapping: physical row 0 → logical 0 → source 10.
        assert_eq!(source_line_at(&blocks, 0, &tl, &bl), 10);
        // Physical row 1 → logical 1 → source 20.
        assert_eq!(source_line_at(&blocks, 1, &tl, &bl), 20);
    }
}
