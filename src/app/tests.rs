/// Unit tests for the `app` module.
///
/// Kept in a dedicated file to keep `mod.rs` focused on production code.
use super::*;
use crate::markdown::{CellSpans, MermaidBlockId, TableBlock, TableBlockId};
use crate::mermaid::{DEFAULT_MERMAID_HEIGHT, MermaidEntry};
use crate::ui::editor::{CommandOutcome, dispatch_command};
use crate::ui::markdown_view::TableLayout;
use std::time::Instant;
// `MouseEvent` is not pulled in by `use super::*`; the others (KeyModifiers,
// MouseButton, MouseEventKind) are already in scope from the parent module.
use crossterm::event::MouseEvent;
use ratatui::text::{Line, Span, Text};
use std::cell::Cell;

fn make_text_block(lines: &[&str]) -> DocBlock {
    let text_lines: Vec<Line<'static>> = lines
        .iter()
        .map(|l| Line::from(Span::raw(l.to_string())))
        .collect();
    let n = text_lines.len();
    DocBlock::Text {
        text: Text::from(text_lines),
        links: Vec::new(),
        heading_anchors: Vec::new(),
        source_lines: (0..crate::cast::u32_sat(n)).collect(),
    }
}

fn str_cell(s: &str) -> CellSpans {
    vec![Span::raw(s.to_string())]
}

fn make_table_block(id: u64, headers: &[&str], rows: &[&[&str]]) -> DocBlock {
    let h: Vec<CellSpans> = headers.iter().map(|s| str_cell(s)).collect();
    let r: Vec<Vec<CellSpans>> = rows
        .iter()
        .map(|row| row.iter().map(|s| str_cell(s)).collect())
        .collect();
    let num_cols = h.len();
    let natural_widths = vec![10usize; num_cols];
    // Stub row_source_lines: header at line 0, body rows at 2, 3, ...
    let row_source_lines: Vec<u32> = std::iter::once(0)
        .chain((2u32..).take(rows.len()))
        .collect();
    DocBlock::Table(TableBlock {
        id: TableBlockId(id),
        headers: h,
        rows: r,
        alignments: vec![pulldown_cmark::Alignment::None; num_cols],
        natural_widths,
        rendered_height: 4,
        source_line: 0,
        row_source_lines,
    })
}

fn make_cached_layout(lines: &[&str]) -> TableLayout {
    let text_lines: Vec<Line<'static>> = lines
        .iter()
        .map(|l| Line::from(Span::raw(l.to_string())))
        .collect();
    TableLayout {
        text: Text::from(text_lines),
    }
}

fn empty_mermaid_cache() -> MermaidCache {
    MermaidCache::new()
}

fn source_only_cache(id: u64) -> MermaidCache {
    let mut cache = MermaidCache::new();
    cache.insert(
        MermaidBlockId(id),
        MermaidEntry::SourceOnly("test".to_string()),
    );
    cache
}

fn ready_cache(id: u64) -> MermaidCache {
    // We can't build a StatefulProtocol in tests, so we use Failed as a
    // stand-in for "showing as image" — which would normally suppress search.
    // For the Ready variant specifically we use Failed to confirm the negative
    // (Failed does show source). Use a separate test for the suppression path.
    let mut cache = MermaidCache::new();
    cache.insert(
        MermaidBlockId(id),
        MermaidEntry::Failed("irrelevant".to_string()),
    );
    cache
}

#[test]
fn collect_matches_text_block() {
    let blocks = vec![make_text_block(&["hello world", "no match", "world again"])];
    let layouts = HashMap::new();
    let cache = empty_mermaid_cache();
    let result = collect_match_lines(&blocks, &layouts, &cache, "world");
    assert_eq!(result, vec![0, 2]);
}

#[test]
fn collect_matches_table_with_layout_cache() {
    let blocks = vec![
        make_text_block(&["intro"]),
        make_table_block(1, &["Header"], &[&["alpha"], &["beta needle"]]),
    ];
    let mut layouts = HashMap::new();
    layouts.insert(
        TableBlockId(1),
        make_cached_layout(&[
            "\u{250c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2510}",
            "\u{2502} Header \u{2502}",
            "\u{251c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2524}",
            "\u{2502} alpha  \u{2502}",
            "\u{2502} beta needle \u{2502}",
            "\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2518}",
        ]),
    );
    let cache = empty_mermaid_cache();
    let result = collect_match_lines(&blocks, &layouts, &cache, "needle");
    // text block has 1 line (offset 0); table starts at offset 1.
    // "beta needle" is at layout index 4, so absolute = 1 + 4 = 5.
    assert_eq!(result, vec![5]);
}

#[test]
fn collect_matches_table_fallback_no_layout() {
    let blocks = vec![make_table_block(2, &["Col"], &[&["findme"], &["nothing"]])];
    let layouts = HashMap::new();
    let cache = empty_mermaid_cache();
    let result = collect_match_lines(&blocks, &layouts, &cache, "findme");
    // Fallback: header row is at row_offset=1, data rows follow.
    // "findme" is the first data row → row_offset = 2 → absolute = 0+2 = 2.
    assert_eq!(result, vec![2]);
}

#[test]
fn collect_matches_mermaid_source_only() {
    let source = "graph LR\n    A --> needle\n    B --> C";
    let mermaid_id = MermaidBlockId(99);
    let blocks = vec![
        make_text_block(&["before"]),
        DocBlock::Mermaid {
            id: mermaid_id,
            source: source.to_string(),
            cell_height: Cell::new(DEFAULT_MERMAID_HEIGHT),
            source_line: 0,
        },
    ];
    let cache = source_only_cache(99);
    let layouts = HashMap::new();
    let result = collect_match_lines(&blocks, &layouts, &cache, "needle");
    // text block: 1 line (offset 0). mermaid starts at offset 1.
    // "A --> needle" is source line index 1, so absolute = 1 + 1 = 2.
    assert_eq!(result, vec![2]);
}

#[test]
fn collect_matches_mermaid_failed_shows_source() {
    let mermaid_id = MermaidBlockId(42);
    let blocks = vec![DocBlock::Mermaid {
        id: mermaid_id,
        source: "graph LR\n    find_this".to_string(),
        cell_height: Cell::new(DEFAULT_MERMAID_HEIGHT),
        source_line: 0,
    }];
    let cache = ready_cache(42);
    let layouts = HashMap::new();
    let result = collect_match_lines(&blocks, &layouts, &cache, "find_this");
    assert_eq!(result, vec![1]);
}

#[test]
fn collect_matches_mermaid_absent_shows_source() {
    let mermaid_id = MermaidBlockId(7);
    let blocks = vec![DocBlock::Mermaid {
        id: mermaid_id,
        source: "sequenceDiagram\n    A ->> match_me: call".to_string(),
        cell_height: Cell::new(DEFAULT_MERMAID_HEIGHT),
        source_line: 0,
    }];
    let layouts = HashMap::new();
    let cache = empty_mermaid_cache();
    let result = collect_match_lines(&blocks, &layouts, &cache, "match_me");
    assert_eq!(result, vec![1]);
}

// ── table modal key / mouse handler tests ───────────────────────────────

/// Build an `App` with an active `TableModalState` using the given column
/// widths and initial scroll positions.  Uses `"."` as the root so it runs
/// without a special directory.
fn make_app_with_modal(natural_widths: Vec<usize>, h_scroll: u16, v_scroll: u16) -> App {
    let mut app = App::new(std::path::PathBuf::from("."), None);
    app.table_modal = Some(TableModalState {
        tab_id: crate::ui::tabs::TabId(0),
        h_scroll,
        v_scroll,
        headers: vec![],
        rows: vec![],
        alignments: vec![],
        natural_widths,
    });
    app.focus = Focus::TableModal;
    app
}

#[test]
fn h_key_snaps_to_prev_column_boundary() {
    // widths [10, 20, 15] → boundaries [0, 13, 36]
    // From 17 (inside col 1 which starts at 13), h snaps back to 13.
    let mut app = make_app_with_modal(vec![10, 20, 15], 17, 0);
    app.handle_table_modal_key(KeyCode::Char('h'));
    assert_eq!(app.table_modal.as_ref().unwrap().h_scroll, 13);
}

#[test]
fn l_key_snaps_to_next_column_boundary() {
    // From 0, next boundary is 13 (start of col 1).
    let mut app = make_app_with_modal(vec![10, 20, 15], 0, 0);
    app.handle_table_modal_key(KeyCode::Char('l'));
    assert_eq!(app.table_modal.as_ref().unwrap().h_scroll, 13);
}

#[test]
fn capital_h_half_page_left() {
    // inner_width = rect.width - 2 = 42 - 2 = 40; half = 20
    // h_scroll 50 - 20 = 30
    let mut app = make_app_with_modal(vec![10, 20, 15], 50, 0);
    app.table_modal_rect = Some(ratatui::layout::Rect {
        x: 0,
        y: 0,
        width: 42,
        height: 20,
    });
    app.handle_table_modal_key(KeyCode::Char('H'));
    assert_eq!(app.table_modal.as_ref().unwrap().h_scroll, 30);
}

#[test]
fn scroll_wheel_in_modal_scrolls_vertically() {
    let mut app = make_app_with_modal(vec![10, 20, 15], 0, 0);
    // Populate the rect so the click registers as "inside".
    app.table_modal_rect = Some(ratatui::layout::Rect {
        x: 5,
        y: 5,
        width: 80,
        height: 30,
    });
    let m = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 10,
        row: 10,
        modifiers: KeyModifiers::empty(),
    };
    app.handle_table_modal_mouse(m);
    assert_eq!(app.table_modal.as_ref().unwrap().v_scroll, 3);
}

#[test]
fn shift_scroll_in_modal_pans_column() {
    // widths [10, 20, 15] → boundaries [0, 13, 36]; Shift+ScrollDown from 0 → 13
    let mut app = make_app_with_modal(vec![10, 20, 15], 0, 0);
    app.table_modal_rect = Some(ratatui::layout::Rect {
        x: 5,
        y: 5,
        width: 80,
        height: 30,
    });
    let m = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: 10,
        row: 10,
        modifiers: KeyModifiers::SHIFT,
    };
    app.handle_table_modal_mouse(m);
    assert_eq!(app.table_modal.as_ref().unwrap().h_scroll, 13);
}

#[test]
fn click_outside_modal_closes_it() {
    let mut app = make_app_with_modal(vec![10, 20, 15], 0, 0);
    app.table_modal_rect = Some(ratatui::layout::Rect {
        x: 10,
        y: 10,
        width: 60,
        height: 20,
    });
    // Click at (5, 5) — outside the rect (which starts at (10, 10)).
    let m = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 5,
        row: 5,
        modifiers: KeyModifiers::empty(),
    };
    app.handle_table_modal_mouse(m);
    assert!(
        app.table_modal.is_none(),
        "modal should close on outside click"
    );
}

#[test]
fn click_inside_modal_does_not_close_it() {
    let mut app = make_app_with_modal(vec![10, 20, 15], 5, 2);
    app.table_modal_rect = Some(ratatui::layout::Rect {
        x: 10,
        y: 10,
        width: 60,
        height: 20,
    });
    // Click at (15, 15) — inside the rect.
    let m = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 15,
        row: 15,
        modifiers: KeyModifiers::empty(),
    };
    app.handle_table_modal_mouse(m);
    assert!(
        app.table_modal.is_some(),
        "modal should stay open on inside click"
    );
    // Scroll must not have changed.
    let s = app.table_modal.as_ref().unwrap();
    assert_eq!(s.h_scroll, 5);
    assert_eq!(s.v_scroll, 2);
}

#[test]
fn collect_matches_absolute_offsets_across_blocks() {
    let blocks = vec![
        make_text_block(&["line0", "line1", "line2"]),
        make_table_block(5, &["H"], &[&["row0"], &["row1 target"]]),
        make_text_block(&["after"]),
    ];
    let mut layouts = HashMap::new();
    layouts.insert(
        TableBlockId(5),
        make_cached_layout(&[
            "\u{250c}\u{2500}\u{2510}",
            "\u{2502}H\u{2502}",
            "\u{251c}\u{2500}\u{2524}",
            "\u{2502}row0\u{2502}",
            "\u{2502}row1 target\u{2502}",
            "\u{2514}\u{2500}\u{2518}",
        ]),
    );
    let cache = empty_mermaid_cache();
    let result = collect_match_lines(&blocks, &layouts, &cache, "target");
    // text block: 3 lines (offsets 0-2). table starts at 3, rendered_height=4.
    // "row1 target" is at layout index 4 → absolute = 3+4 = 7.
    // after block starts at 3+4=7. "after" is at 7+0=7 — no match for "target".
    assert_eq!(result, vec![7]);
}

// ── Editor spike tests ────────────────────────────────────────────────────

/// Open a tab with known content and put the app in a state suitable for
/// editor tests.  Returns the `App` and the path used.
fn make_app_with_tab(content: &str) -> (App, PathBuf) {
    let mut app = App::new(PathBuf::from("."), None);
    let path = PathBuf::from("/fake/test.md");
    // Use open_or_focus to create the tab, then manually set content.
    app.tabs.open_or_focus(&path, true);
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.content = content.to_string();
        tab.view.current_path = Some(path.clone());
        tab.view.file_name = "test.md".to_string();
    }
    app.focus = Focus::Viewer;
    (app, path)
}

#[test]
fn enter_edit_mode_initializes_editor_from_view_content() {
    let (mut app, _path) = make_app_with_tab("# Hello\n\nworld");
    app.enter_edit_mode();
    let tab = app.tabs.active_tab().expect("tab must exist");
    let editor = tab
        .editor
        .as_ref()
        .expect("editor must be Some after enter_edit_mode");
    assert_eq!(editor.baseline, "# Hello\n\nworld");
    assert!(!editor.is_dirty());
    assert_eq!(app.focus, Focus::Editor);
}

#[test]
fn q_with_no_dirty_returns_to_viewer() {
    let (mut app, _path) = make_app_with_tab("clean content");
    app.enter_edit_mode();
    // Dispatch :q — buffer is clean so the editor should close.
    {
        let tab = app.tabs.active_tab_mut().unwrap();
        let editor = tab.editor.as_mut().unwrap();
        let outcome = dispatch_command(editor, "q");
        // Manually apply the outcome as App::apply_command_outcome would.
        assert_eq!(outcome, CommandOutcome::Close);
    }
    // Simulate the close path.
    app.close_editor();
    assert!(app.tabs.active_tab().unwrap().editor.is_none());
    assert_eq!(app.focus, Focus::Viewer);
}

#[test]
fn q_with_dirty_blocks_and_sets_status_message() {
    let (mut app, _path) = make_app_with_tab("original");
    app.enter_edit_mode();
    // Make it dirty by changing the baseline so the buffer no longer matches.
    {
        let tab = app.tabs.active_tab_mut().unwrap();
        let editor = tab.editor.as_mut().unwrap();
        editor.baseline = "something different".to_string();
        let outcome = dispatch_command(editor, "q");
        assert_eq!(
            outcome,
            CommandOutcome::Handled,
            ":q on dirty buffer must return Handled (not Close)"
        );
        assert!(
            editor.status_message.is_some(),
            "a status message must be set when :q is blocked"
        );
    }
    // Editor must remain open.
    assert!(app.tabs.active_tab().unwrap().editor.is_some());
}

#[test]
fn q_bang_with_dirty_discards_and_returns_to_viewer() {
    let (mut app, _path) = make_app_with_tab("original");
    app.enter_edit_mode();
    {
        let tab = app.tabs.active_tab_mut().unwrap();
        let editor = tab.editor.as_mut().unwrap();
        editor.baseline = "something different".to_string();
        let outcome = dispatch_command(editor, "q!");
        assert_eq!(
            outcome,
            CommandOutcome::Close,
            ":q! must always close even when dirty"
        );
    }
    app.close_editor();
    assert!(app.tabs.active_tab().unwrap().editor.is_none());
    assert_eq!(app.focus, Focus::Viewer);
}

#[test]
fn command_line_captures_chars_until_enter() {
    use crossterm::event::{KeyCode as KC, KeyEvent, KeyModifiers};

    let (mut app, _path) = make_app_with_tab("text");
    app.enter_edit_mode();
    app.focus = Focus::Editor;

    // Press `:` — should start command-line mode (editor is in Normal mode).
    app.handle_editor_key(KeyEvent::new(KC::Char(':'), KeyModifiers::NONE));
    {
        let tab = app.tabs.active_tab().unwrap();
        let editor = tab.editor.as_ref().unwrap();
        assert!(
            editor.command_line.is_some(),
            "':' in Normal mode must start command-line capture"
        );
        assert_eq!(editor.command_line.as_deref(), Some(""));
    }

    // Type 'w'.
    app.handle_editor_key(KeyEvent::new(KC::Char('w'), KeyModifiers::NONE));
    {
        let tab = app.tabs.active_tab().unwrap();
        let editor = tab.editor.as_ref().unwrap();
        assert_eq!(editor.command_line.as_deref(), Some("w"));
    }

    // We can't easily test the Enter path here without an action_tx, so
    // just verify the capture works: 'w' was collected into command_line.
}

#[test]
fn mouse_events_ignored_while_editing() {
    use crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};

    let (mut app, _path) = make_app_with_tab("content");
    app.enter_edit_mode();
    // Precondition: focus must be Editor.
    assert_eq!(app.focus, Focus::Editor);

    // Record the tree selection before the mouse event.
    let selection_before = app.tree.list_state.selected();

    // Simulate a left-click anywhere on screen.
    let click = MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 5,
        row: 5,
        modifiers: KeyModifiers::NONE,
    };
    app.handle_mouse(click);

    // Focus must remain on the editor.
    assert_eq!(app.focus, Focus::Editor, "focus must stay Editor");
    // Tree selection must be unchanged.
    assert_eq!(
        app.tree.list_state.selected(),
        selection_before,
        "tree selection must not change during edit mode"
    );
    // Editor must still be present.
    assert!(
        app.tabs.active_tab().unwrap().editor.is_some(),
        "editor must remain open"
    );
}

// ── enter_edit_mode source-line tests ────────────────────────────────────

/// `enter_edit_mode` must place the edtui cursor on the source line that
/// the viewer cursor's rendered logical line maps to via `source_line_at`.
///
/// We build a Text block whose `source_lines` are [10, 11, 12] and set the
/// viewer cursor to logical line 1.  `source_line_at` returns 11, so the
/// editor cursor row must be 11.
#[test]
fn enter_edit_mode_uses_cursor_for_source_line() {
    use crate::markdown::{DocBlock, HeadingAnchor, LinkInfo};
    use ratatui::text::{Line, Span, Text};

    let mut app = App::new(std::path::PathBuf::from("."), None);

    // Open a tab with dummy content that has as many newlines as the
    // highest source line we reference (line 11 → 12 lines).
    let content: String = {
        use std::fmt::Write as _;
        let mut s = String::new();
        for i in 0..12usize {
            let _ = writeln!(s, "source line {i}");
        }
        s
    };
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();
    std::fs::write(&path, &content).unwrap();

    let (_, _) = app.tabs.open_or_focus(&path, true);
    let palette = crate::theme::Palette::from_theme(crate::theme::Theme::Default);
    let tab = app.tabs.active_tab_mut().unwrap();
    tab.view.load(
        path.clone(),
        "test.md".into(),
        content,
        &palette,
        crate::theme::Theme::Default,
    );

    // Replace the rendered blocks with a hand-crafted Text block whose
    // source_lines are [10, 11, 12].
    let src_lines = vec![10u32, 11, 12];
    let text_lines: Vec<Line<'static>> = src_lines
        .iter()
        .map(|i| Line::from(Span::raw(format!("line {i}"))))
        .collect();
    tab.view.rendered = vec![DocBlock::Text {
        text: Text::from(text_lines),
        links: Vec::<LinkInfo>::new(),
        heading_anchors: Vec::<HeadingAnchor>::new(),
        source_lines: src_lines,
    }];
    tab.view.total_lines = 3;
    // Set cursor to logical line 1 → source_line_at returns 11.
    tab.view.cursor_line = 1;

    app.focus = Focus::Viewer;
    app.enter_edit_mode();

    assert_eq!(app.focus, Focus::Editor, "focus should switch to Editor");
    let tab = app.tabs.active_tab().unwrap();
    let editor = tab.editor.as_ref().expect("editor should be set");
    assert_eq!(
        editor.state.cursor.row, 11,
        "editor cursor row should be the mapped source line (11)"
    );
}

// ── viewer navigation (d/u/gg/G) regression tests ────────────────────────

/// Minimal App with a tab whose view has a known `total_lines` and a
/// configured `view_height`.  Cheaper than `make_app_with_tab` because it
/// does not load + render real markdown content.
fn make_app_with_view(total_lines: u32, view_height: u32) -> App {
    let mut app = App::new(PathBuf::from("."), None);
    let path = PathBuf::from("/fake/nav_test.md");
    app.tabs.open_or_focus(&path, true);
    app.tabs.view_height = view_height;
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.total_lines = total_lines;
        tab.view.cursor_line = 0;
        tab.view.scroll_offset = 0;
    }
    app.focus = Focus::Viewer;
    app
}

#[test]
fn d_key_moves_cursor_half_page_down() {
    let mut app = make_app_with_view(100, 30);
    app.handle_key(KeyCode::Char('d'), KeyModifiers::NONE);
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(
        tab.view.cursor_line, 15,
        "`d` should move the cursor half a page (vh/2 = 15)"
    );
}

#[test]
fn u_key_moves_cursor_half_page_up() {
    let mut app = make_app_with_view(100, 30);
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.cursor_line = 50;
        tab.view.scroll_offset = 35;
    }
    app.handle_key(KeyCode::Char('u'), KeyModifiers::NONE);
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(tab.view.cursor_line, 35, "`u` should move cursor up vh/2");
}

#[test]
fn gg_chord_jumps_cursor_to_top() {
    let mut app = make_app_with_view(100, 30);
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.cursor_line = 50;
        tab.view.scroll_offset = 35;
    }
    app.handle_key(KeyCode::Char('g'), KeyModifiers::NONE);
    app.handle_key(KeyCode::Char('g'), KeyModifiers::NONE);
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(tab.view.cursor_line, 0, "`gg` should jump cursor to 0");
    assert_eq!(tab.view.scroll_offset, 0, "`gg` should reset scroll");
}

#[test]
fn shift_g_jumps_cursor_to_bottom() {
    let mut app = make_app_with_view(100, 30);
    app.handle_key(KeyCode::Char('G'), KeyModifiers::SHIFT);
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(
        tab.view.cursor_line, 99,
        "`G` should land cursor on last line"
    );
}

/// When the cursor is inside a table block, `Enter` must open THAT
/// table rather than the first table visible on screen.
#[test]
fn try_open_table_modal_picks_table_under_cursor() {
    let mut app = App::new(PathBuf::from("."), None);
    let path = PathBuf::from("/fake/tables.md");
    app.tabs.open_or_focus(&path, true);
    app.tabs.view_height = 30;
    app.focus = Focus::Viewer;

    // Layout: [text(3)] [table A(4)] [text(3)] [table B(4)]
    //          0..3      3..7         7..10     10..14
    let blocks = vec![
        make_text_block(&["intro", "text", "here"]),
        make_table_block(10, &["A"], &[&["a-row-0"]]),
        make_text_block(&["middle", "text", "here"]),
        make_table_block(20, &["B"], &[&["b-row-0"]]),
    ];
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.total_lines = blocks.iter().map(DocBlock::height).sum();
        tab.view.rendered = blocks;
        tab.view.scroll_offset = 0;
        tab.view.cursor_line = 12; // inside table B (10..14)
    }

    app.try_open_table_modal();
    let modal = app.table_modal.as_ref().expect("modal must open");
    assert_eq!(
        modal.headers.len(),
        1,
        "expected table B's single header, got {:?}",
        modal.headers
    );
    assert_eq!(
        modal.rows[0][0]
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>(),
        "b-row-0",
        "modal should carry table B's data, not table A's",
    );
}

/// Regression: when the cursor is on prose (not a table), `Enter` should
/// fall back to the first table intersecting the viewport (old behaviour).
#[test]
fn try_open_table_modal_falls_back_to_first_visible_table() {
    let mut app = App::new(PathBuf::from("."), None);
    let path = PathBuf::from("/fake/tables.md");
    app.tabs.open_or_focus(&path, true);
    app.tabs.view_height = 30;
    app.focus = Focus::Viewer;

    let blocks = vec![
        make_text_block(&["intro"]),
        make_table_block(10, &["A"], &[&["a-row-0"]]),
        make_table_block(20, &["B"], &[&["b-row-0"]]),
    ];
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.total_lines = blocks.iter().map(DocBlock::height).sum();
        tab.view.rendered = blocks;
        tab.view.scroll_offset = 0;
        tab.view.cursor_line = 0; // on prose, above any table
    }

    app.try_open_table_modal();
    let modal = app.table_modal.as_ref().expect("modal must open");
    assert_eq!(
        modal.rows[0][0]
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>(),
        "a-row-0",
        "modal should open table A (first visible) when cursor is on prose",
    );
}

#[test]
fn d_key_moves_cursor_with_real_loaded_content() {
    use crate::theme::{Palette, Theme};
    let mut app = App::new(PathBuf::from("."), None);
    let path = PathBuf::from("/fake/nav_test.md");
    app.tabs.open_or_focus(&path, true);
    let content: String = {
        use std::fmt::Write as _;
        let mut s = String::new();
        for i in 0..60usize {
            let _ = write!(s, "paragraph {i}\n\n");
        }
        s
    };
    let palette = Palette::from_theme(Theme::Default);
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.load(
            path.clone(),
            "nav_test.md".to_string(),
            content,
            &palette,
            Theme::Default,
        );
    }
    app.focus = Focus::Viewer;
    app.tabs.view_height = 30;

    let before_cursor = app.tabs.active_tab().unwrap().view.cursor_line;
    let before_total = app.tabs.active_tab().unwrap().view.total_lines;
    let before_vh = app.tabs.view_height;
    app.handle_key(KeyCode::Char('d'), KeyModifiers::NONE);
    let after_cursor = app.tabs.active_tab().unwrap().view.cursor_line;
    assert!(
        before_total > 0,
        "total_lines must be populated (got {before_total})"
    );
    assert!(
        before_vh > 0,
        "view_height must be positive (got {before_vh})"
    );
    assert_ne!(
        before_cursor, after_cursor,
        "`d` should move the cursor (before={before_cursor} after={after_cursor} \
         total_lines={before_total} view_height={before_vh})",
    );
}

// ── doc_search navigation ────────────────────────────────────────────────

/// Build an `App` with an active tab whose `doc_search` state has the
/// given match lines and `current_match`, and whose view has the given
/// `total_lines`.  `view_height` defaults to 20.
fn make_app_with_doc_search(match_lines: Vec<u32>, current_match: usize, total_lines: u32) -> App {
    let mut app = App::new(PathBuf::from("."), None);
    let path = PathBuf::from("/fake/ds_test.md");
    app.tabs.open_or_focus(&path, true);
    app.tabs.view_height = 20;
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.total_lines = total_lines;
        tab.view.cursor_line = 0;
        tab.view.scroll_offset = 0;
        tab.doc_search.match_lines = match_lines;
        tab.doc_search.current_match = current_match;
    }
    app
}

/// `doc_search_next` must advance `current_match`, set `cursor_line` to the
/// new match line, and adjust `scroll_offset` via `scroll_to_cursor`.
#[test]
fn doc_search_next_updates_cursor_and_scroll() {
    // 100-line doc, view_height = 20; match_lines = [5, 20, 35],
    // cursor starts at line 5 (current_match = 0).
    let mut app = make_app_with_doc_search(vec![5, 20, 35], 0, 100);
    {
        // Ensure cursor is already at the first match.
        let tab = app.tabs.active_tab_mut().unwrap();
        tab.view.cursor_line = 5;
    }
    app.doc_search_next();
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(tab.doc_search.current_match, 1);
    assert_eq!(
        tab.view.cursor_line, 20,
        "cursor must move to match line 20"
    );
    // After scroll_to_cursor with view_height=20, scroll_offset = 20 - (20-1) = 1.
    assert_eq!(tab.view.scroll_offset, 1);
}

/// `doc_search_prev` with `current_match == 0` must wrap to the last match.
#[test]
fn doc_search_prev_wraps_to_last_match() {
    let mut app = make_app_with_doc_search(vec![5, 20, 35], 0, 100);
    app.doc_search_prev();
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(tab.doc_search.current_match, 2);
    assert_eq!(tab.view.cursor_line, 35, "cursor must wrap to last match");
}

/// When there are no matches, `doc_search_next` must not change any state.
#[test]
fn doc_search_empty_matches_no_op() {
    let mut app = make_app_with_doc_search(vec![], 0, 100);
    {
        let tab = app.tabs.active_tab_mut().unwrap();
        tab.view.cursor_line = 7;
        tab.view.scroll_offset = 3;
    }
    app.doc_search_next();
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(tab.view.cursor_line, 7, "cursor must not change");
    assert_eq!(tab.view.scroll_offset, 3, "scroll must not change");
}

/// `perform_doc_search` with a matching query must set `cursor_line` to the
/// first match.
///
/// We build rendered blocks that contain "hello" on line 4 (the 5th line
/// of a Text block that starts at the document root) and verify the cursor
/// ends up at absolute line 4.
#[test]
fn perform_doc_search_first_match_moves_cursor() {
    let lines: Vec<&str> = (0..10)
        .map(|i| if i == 4 { "hello world" } else { "other" })
        .collect();
    let mut app = App::new(PathBuf::from("."), None);
    let path = PathBuf::from("/fake/search_test.md");
    app.tabs.open_or_focus(&path, true);
    app.tabs.view_height = 20;
    if let Some(tab) = app.tabs.active_tab_mut() {
        let block = make_text_block(lines.as_slice());
        let total = block.height();
        tab.view.rendered = vec![block];
        tab.view.total_lines = total;
        tab.view.cursor_line = 0;
        tab.view.scroll_offset = 0;
        tab.doc_search.active = true;
        tab.doc_search.query = "hello".to_string();
    }
    app.focus = Focus::Viewer;
    app.perform_doc_search();
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(
        tab.view.cursor_line, 4,
        "cursor must jump to first match at line 4"
    );
}

#[test]
fn watcher_suppresses_reload_within_grace_window() {
    let (mut app, path) = make_app_with_tab("content");
    // Simulate a recent self-save.
    app.last_file_save_at = Some((path.clone(), Instant::now()));
    // reload_changed_tabs requires action_tx; if None it returns early before
    // the suppression check.  We use a channel so the logic actually runs.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Action>();
    app.action_tx = Some(tx);
    app.reload_changed_tabs(std::slice::from_ref(&path));
    // The spawn_blocking must NOT have been called because the path is
    // within the grace window.  Since spawn_blocking is async, we check that
    // no FileReloaded action arrives immediately (the channel should be empty).
    assert!(
        rx.try_recv().is_err(),
        "no FileReloaded should be sent when within the grace window"
    );
}

// ── apply_file_reloaded cursor-preservation ──────────────────────────────

/// A `FileReloaded` event with unchanged content must not reset the cursor.
///
/// On Linux, inotify fires `IN_ACCESS` when a file is *read*, producing a
/// spurious `FilesChanged` → `FileReloaded` round-trip.  The guard in
/// `apply_file_reloaded` compares byte content and skips the reload, so the
/// cursor stays wherever the user left it.
#[test]
fn reload_with_unchanged_content_preserves_cursor() {
    use crate::theme::{Palette, Theme};
    let palette = Palette::from_theme(Theme::Default);
    let content: String = {
        use std::fmt::Write as _;
        let mut s = String::new();
        for i in 0..20usize {
            let _ = write!(s, "line {i}\n\n");
        }
        s
    };
    let path = PathBuf::from("/fake/unchanged.md");

    let mut app = App::new(PathBuf::from("."), None);
    app.tabs.open_or_focus(&path, true);
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.load(
            path.clone(),
            "unchanged.md".to_string(),
            content.clone(),
            &palette,
            Theme::Default,
        );
        tab.view.cursor_line = 10;
        tab.view.scroll_offset = 5;
    }

    // Simulate FileReloaded arriving with identical content.
    app.apply_file_reloaded(path.clone(), content);

    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(
        tab.view.cursor_line, 10,
        "cursor must not reset on spurious reload (unchanged content)"
    );
    assert_eq!(
        tab.view.scroll_offset, 5,
        "scroll must not reset on spurious reload (unchanged content)"
    );
}

/// A `FileReloaded` event with new content must restore the cursor to its
/// old position when that position is still valid (file grew or same size).
#[test]
fn reload_with_changed_content_restores_cursor_when_in_range() {
    use crate::theme::{Palette, Theme};
    let palette = Palette::from_theme(Theme::Default);
    // 20 paragraphs → many display lines.
    let content_v1: String = {
        use std::fmt::Write as _;
        let mut s = String::new();
        for i in 0..20usize {
            let _ = write!(s, "line {i}\n\n");
        }
        s
    };
    let path = PathBuf::from("/fake/changed.md");

    let mut app = App::new(PathBuf::from("."), None);
    app.tabs.open_or_focus(&path, true);
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.load(
            path.clone(),
            "changed.md".to_string(),
            content_v1,
            &palette,
            Theme::Default,
        );
        tab.view.cursor_line = 10;
        tab.view.scroll_offset = 5;
    }

    // New content that is longer than 10 display lines — cursor stays.
    let content_v2: String = {
        use std::fmt::Write as _;
        let mut s = String::new();
        for i in 0..20usize {
            let _ = write!(s, "edited {i}\n\n");
        }
        s
    };
    app.apply_file_reloaded(path.clone(), content_v2);

    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(
        tab.view.cursor_line, 10,
        "cursor must be restored after a genuine reload when still in range"
    );
}

// ── build_yank_text ──────────────────────────────────────────────────────

#[test]
fn build_yank_text_single_line() {
    let content = "alpha\nbeta\ngamma";
    assert_eq!(build_yank_text(content, 1, 1), "beta");
}

#[test]
fn build_yank_text_multi_line() {
    let content = "line0\nline1\nline2\nline3";
    assert_eq!(build_yank_text(content, 1, 3), "line1\nline2\nline3");
}

#[test]
fn build_yank_text_reversed_range() {
    // Range given in reverse order must produce same result as forward range.
    let content = "a\nb\nc";
    assert_eq!(build_yank_text(content, 2, 0), "a\nb\nc");
}

#[test]
fn build_yank_text_past_eof() {
    // Range that extends past the available lines returns whatever is there.
    let content = "x\ny";
    let result = build_yank_text(content, 0, 10);
    assert_eq!(result, "x\ny");
}

#[test]
fn build_yank_text_empty_content() {
    assert_eq!(build_yank_text("", 0, 0), "");
}

// ── Feature 2: Visual mode and yank ─────────────────────────────────────

/// Helper: build an App with a rendered tab (blocks set, not just content string).
fn make_rendered_app(content: &str) -> (App, PathBuf) {
    use crate::theme::{Palette, Theme};
    let palette = Palette::from_theme(Theme::Default);
    let path = PathBuf::from("/fake/yank_test.md");
    let mut app = App::new(PathBuf::from("."), None);
    app.tabs.open_or_focus(&path, true);
    app.tabs.view_height = 20;
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.load(
            path.clone(),
            "yank_test.md".to_string(),
            content.to_string(),
            &palette,
            Theme::Default,
        );
    }
    app.focus = Focus::Viewer;
    (app, path)
}

/// Helper to build a line-mode `VisualRange` for tests.
fn line_vrange(anchor: u32, cursor: u32) -> crate::ui::markdown_view::VisualRange {
    use crate::ui::markdown_view::{VisualMode, VisualRange};
    VisualRange {
        mode: VisualMode::Line,
        anchor_line: anchor,
        anchor_col: 0,
        cursor_line: cursor,
        cursor_col: 0,
    }
}

#[test]
fn capital_v_enters_line_visual_mode() {
    use crate::ui::markdown_view::{VisualMode, VisualRange};
    let (mut app, _path) = make_rendered_app("line0\nline1\nline2");
    // Move cursor to line 2.
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.cursor_line = 2;
    }
    app.handle_key(KeyCode::Char('V'), KeyModifiers::NONE);
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(
        tab.view.visual_mode,
        Some(VisualRange {
            mode: VisualMode::Line,
            anchor_line: 2,
            anchor_col: 0,
            cursor_line: 2,
            cursor_col: 0,
        }),
        "V must enter line visual mode at current cursor"
    );
}

#[test]
fn lowercase_v_enters_char_visual_mode() {
    use crate::ui::markdown_view::{VisualMode, VisualRange};
    let (mut app, _path) = make_rendered_app("line0\nline1\nline2");
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.cursor_line = 1;
        tab.view.cursor_col = 3;
    }
    app.handle_key(KeyCode::Char('v'), KeyModifiers::NONE);
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(
        tab.view.visual_mode,
        Some(VisualRange {
            mode: VisualMode::Char,
            anchor_line: 1,
            anchor_col: 3,
            cursor_line: 1,
            cursor_col: 3,
        }),
        "v must enter char visual mode at current cursor/col"
    );
}

#[test]
fn v_in_visual_mode_exits_visual_mode() {
    let (mut app, _path) = make_rendered_app("line0\nline1\nline2");
    // Enter line visual mode manually, then press V again to exit.
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.visual_mode = Some(line_vrange(1, 2));
    }
    app.handle_key(KeyCode::Char('V'), KeyModifiers::NONE);
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(
        tab.view.visual_mode, None,
        "V in line visual mode must exit it"
    );
}

#[test]
fn esc_in_visual_mode_exits_visual_mode() {
    let (mut app, _path) = make_rendered_app("line0\nline1");
    if let Some(tab) = app.tabs.active_tab_mut() {
        tab.view.visual_mode = Some(line_vrange(0, 1));
    }
    app.handle_key(KeyCode::Esc, KeyModifiers::NONE);
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(tab.view.visual_mode, None, "Esc must exit visual mode");
}

#[test]
fn j_in_visual_mode_extends_range() {
    // Use a controlled tab with known total_lines to avoid renderer side-effects.
    let mut app = App::new(PathBuf::from("."), None);
    let path = PathBuf::from("/fake/visual_j.md");
    app.tabs.open_or_focus(&path, true);
    app.tabs.view_height = 20;
    if let Some(tab) = app.tabs.active_tab_mut() {
        // Build 10 logical lines directly so the cursor clamp works correctly.
        let block = make_text_block(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]);
        let total = block.height();
        tab.view.rendered = vec![block];
        tab.view.total_lines = total;
        tab.view.cursor_line = 2;
        tab.view.visual_mode = Some(line_vrange(2, 2));
    }
    app.focus = Focus::Viewer;
    // Press j to move down.
    app.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
    let tab = app.tabs.active_tab().unwrap();
    let range = tab
        .view
        .visual_mode
        .expect("visual mode must still be active");
    assert_eq!(range.anchor_line, 2, "anchor must stay at 2");
    assert_eq!(range.cursor_line, 3, "cursor must extend to 3 after j");
}

#[test]
fn y_in_visual_mode_yanks_and_exits() {
    // Use a controlled tab with predictable source_lines mapping.
    // make_text_block assigns source_lines = [0, 1, 2, ...] sequentially.
    let content = "alpha\nbeta\ngamma\ndelta";
    let mut app = App::new(PathBuf::from("."), None);
    let path = PathBuf::from("/fake/visual_yank.md");
    app.tabs.open_or_focus(&path, true);
    app.tabs.view_height = 20;
    if let Some(tab) = app.tabs.active_tab_mut() {
        let block = make_text_block(&["alpha", "beta", "gamma", "delta"]);
        let total = block.height();
        tab.view.rendered = vec![block];
        tab.view.total_lines = total;
        tab.view.content = content.to_string();
        tab.view.current_path = Some(path.clone());
        // Select logical lines 1..=2 (source lines 1="beta", 2="gamma").
        tab.view.cursor_line = 1;
        tab.view.visual_mode = Some(line_vrange(1, 2));
    }
    app.focus = Focus::Viewer;
    // Press y — should yank and exit visual mode.
    app.handle_key(KeyCode::Char('y'), KeyModifiers::NONE);
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(
        tab.view.visual_mode, None,
        "y in visual mode must exit visual mode"
    );
    // Verify that the yank text for source lines 1..=2 is correct.
    let top_source = crate::markdown::source_line_at(&tab.view.rendered, 1);
    let bottom_source = crate::markdown::source_line_at(&tab.view.rendered, 2);
    let expected = build_yank_text(content, top_source, bottom_source);
    assert_eq!(
        expected, "beta\ngamma",
        "yank text must span visual selection"
    );
}

// ── New: h/l cursor column movement ─────────────────────────────────────

#[test]
fn h_moves_cursor_col_left() {
    let mut app = App::new(PathBuf::from("."), None);
    let path = PathBuf::from("/fake/hl_test.md");
    app.tabs.open_or_focus(&path, true);
    app.tabs.view_height = 20;
    // Build a line wide enough to have horizontal room.
    if let Some(tab) = app.tabs.active_tab_mut() {
        let block = make_text_block(&["hello world"]);
        tab.view.rendered = vec![block];
        tab.view.total_lines = 1;
        tab.view.cursor_col = 5;
    }
    app.focus = Focus::Viewer;
    app.handle_key(KeyCode::Char('h'), KeyModifiers::NONE);
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(tab.view.cursor_col, 4, "h must decrement cursor_col");
}

#[test]
fn l_moves_cursor_col_right_clamped() {
    let mut app = App::new(PathBuf::from("."), None);
    let path = PathBuf::from("/fake/hl_clamp.md");
    app.tabs.open_or_focus(&path, true);
    app.tabs.view_height = 20;
    // "abc" is 3 cells wide — max cursor_col = 2.
    if let Some(tab) = app.tabs.active_tab_mut() {
        let block = make_text_block(&["abc"]);
        tab.view.rendered = vec![block];
        tab.view.total_lines = 1;
        tab.view.cursor_col = 2; // already at end
    }
    app.focus = Focus::Viewer;
    app.handle_key(KeyCode::Char('l'), KeyModifiers::NONE);
    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(
        tab.view.cursor_col, 2,
        "l at end of line must not exceed line_width-1"
    );
}

// ── Feature 1: confirm_search jumps to match line ───────────────────────

#[test]
fn pending_jump_cleared_after_apply() {
    // Set a pending jump and simulate a FileLoaded action for the same path.
    let path = PathBuf::from("/fake/jump_test.md");
    let content = "line0\nline1\nline2\nline3\nline4";
    let mut app = App::new(PathBuf::from("."), None);
    app.tabs.open_or_focus(&path, true);
    // Seed the tab as empty (simulates a pending load).
    // pending_jump is set to source line 2.
    app.pending_jump = Some((path.clone(), 2));
    // Now simulate FileLoaded arriving.
    app.apply_file_loaded(path.clone(), content.to_string(), true);
    assert!(
        app.pending_jump.is_none(),
        "pending_jump must be cleared after apply_file_loaded"
    );
}

#[test]
fn confirm_search_filename_result_no_jump() {
    // A filename-mode result has first_match_line = None;
    // after the search confirm, pending_jump should remain None.
    use crate::ui::search_modal::{SearchMode, SearchResult};
    let mut app = App::new(PathBuf::from("."), None);
    let path = PathBuf::from("/fake/fn_result.md");
    app.search.active = true;
    app.search.mode = SearchMode::FileName;
    app.search.results = vec![SearchResult {
        path: path.clone(),
        name: "fn_result.md".to_string(),
        match_count: 0,
        preview: String::new(),
        first_match_line: None,
    }];
    app.search.selected_index = 0;
    app.confirm_search();
    assert!(
        app.pending_jump.is_none(),
        "filename result must not set pending_jump"
    );
}

#[test]
fn apply_file_loaded_jumps_cursor_to_source_line() {
    let content = "alpha\nbeta\ngamma\ndelta\nepsilon";
    let path = PathBuf::from("/fake/jump_cursor.md");
    let mut app = App::new(PathBuf::from("."), None);
    app.tabs.open_or_focus(&path, true);
    app.tabs.view_height = 20;

    // Populate the tab with a known block (source_lines = [0,1,2,3,4]).
    if let Some(tab) = app.tabs.active_tab_mut() {
        let block = make_text_block(&["alpha", "beta", "gamma", "delta", "epsilon"]);
        let total = block.height();
        tab.view.rendered = vec![block];
        tab.view.total_lines = total;
        tab.view.content = content.to_string();
        tab.view.current_path = Some(path.clone());
    }

    let expected_logical = {
        let tab = app.tabs.active_tab().unwrap();
        crate::markdown::logical_line_at_source(&tab.view.rendered, 2)
            .expect("controlled block must map source 2 to logical 2")
    };
    assert_eq!(
        expected_logical, 2,
        "make_text_block must yield source_line == logical_line"
    );

    app.pending_jump = Some((path.clone(), 2));
    app.apply_file_loaded(path.clone(), content.to_string(), true);

    let tab = app.tabs.active_tab().unwrap();
    assert_eq!(
        tab.view.cursor_line, expected_logical,
        "cursor_line must land on logical line {expected_logical} for source line 2"
    );
    assert!(app.pending_jump.is_none(), "pending_jump must be consumed");
}

#[test]
fn pending_jump_cleared_on_file_load_failure() {
    // A FileLoadFailed for the matching path must clear pending_jump.
    let path = PathBuf::from("/fake/nonexistent.md");
    let mut app = App::new(PathBuf::from("."), None);
    app.pending_jump = Some((path.clone(), 5));
    app.handle_action(Action::FileLoadFailed { path: path.clone() });
    assert!(
        app.pending_jump.is_none(),
        "pending_jump must be cleared when the matching file fails to load"
    );
}

#[test]
fn pending_jump_not_cleared_on_different_path_failure() {
    // A FileLoadFailed for a different path must not touch pending_jump.
    let path = PathBuf::from("/fake/target.md");
    let other = PathBuf::from("/fake/other.md");
    let mut app = App::new(PathBuf::from("."), None);
    app.pending_jump = Some((path.clone(), 3));
    app.handle_action(Action::FileLoadFailed { path: other });
    assert!(
        app.pending_jump.is_some(),
        "pending_jump must be preserved when a different file fails to load"
    );
}
