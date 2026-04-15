//! Vim-style in-place editor for markdown files.
//!
//! This module provides [`TabEditor`], which holds the edtui state for one open tab,
//! plus helpers to draw the editor overlay and dispatch ex-style commands.

use edtui::{EditorEventHandler, EditorMode, EditorState, EditorTheme, EditorView, Lines};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::App;
use crate::theme::Palette;

// ── TabEditor ────────────────────────────────────────────────────────────────

/// Per-tab editor state.  Present only while the tab is in edit mode.
pub struct TabEditor {
    /// edtui widget state — owns the text buffer and cursor.
    pub state: EditorState,
    /// The text content as of the last successful save (or initial open).
    /// Used to detect unsaved changes ("dirty" state).
    pub baseline: String,
    /// When `Some`, the user has pressed `:` in Normal mode and we are
    /// capturing the ex-command string ourselves (e.g. `"w"`, `"q"`, `"wq"`).
    pub command_line: Option<String>,
    /// A transient status message shown in the editor footer.
    pub status_message: Option<String>,
    /// When `true`, the `FileSaved` handler should close the editor after a
    /// successful write.  Set by the `:wq` path before spawning the async
    /// write task.  Never carries semantic meaning through `status_message`.
    pub close_after_save: bool,
    /// `false` until the first draw has completed.  The first draw of a
    /// freshly-constructed `EditorState` is a no-op for viewport scroll
    /// because edtui's internal `num_rows` is still zero — scrolling can
    /// only start once `num_rows` has been populated.  We therefore render
    /// twice on the first draw: pass one fills `num_rows`, pass two
    /// scrolls the viewport to reveal the initial cursor.
    pub first_draw_done: bool,
}

impl TabEditor {
    /// Construct a new editor pre-loaded with `content`.
    ///
    /// Starts in Normal mode.  The caller may transition to Insert immediately
    /// after construction if desired.
    #[must_use]
    pub fn new(content: String) -> Self {
        let state = EditorState::new(Lines::from(content.as_str()));
        Self {
            state,
            baseline: content,
            command_line: None,
            status_message: None,
            close_after_save: false,
            first_draw_done: false,
        }
    }

    /// Return `true` when the buffer differs from the last-saved baseline.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        extract_text(&self.state) != self.baseline
    }
}

// ── Text extraction ──────────────────────────────────────────────────────────

/// Collect the full text content from an `EditorState` as a single `String`.
///
/// `Lines` is a `Jagged<char>` where each row is a `Vec<char>`.  We iterate
/// the rows, collect each into a `String`, and join them with `'\n'`.
#[must_use]
pub fn extract_text(state: &EditorState) -> String {
    state
        .lines
        .iter_row()
        .map(|row| row.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Command dispatch ─────────────────────────────────────────────────────────

/// The outcome of dispatching an ex command.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    /// The command was handled; editor should remain open.
    Handled,
    /// The command requires a save; caller should initiate async write.
    Save,
    /// The command requires closing the editor (content already saved or discarded).
    Close,
    /// The command requires save then close.
    SaveThenClose,
}

/// Dispatch an ex-style command string (without the leading `:`).
///
/// Returns the appropriate [`CommandOutcome`] so the caller can act on it.
/// Unknown commands set `editor.status_message` and return `Handled`.
pub fn dispatch_command(editor: &mut TabEditor, cmd: &str) -> CommandOutcome {
    match cmd.trim() {
        "w" => CommandOutcome::Save,
        "q" => {
            if editor.is_dirty() {
                editor.status_message = Some("unsaved changes — use :q! to discard".to_string());
                CommandOutcome::Handled
            } else {
                CommandOutcome::Close
            }
        }
        "q!" => CommandOutcome::Close,
        "wq" => CommandOutcome::SaveThenClose,
        other => {
            editor.status_message = Some(format!("unknown command: :{other}"));
            CommandOutcome::Handled
        }
    }
}

// ── Theme ────────────────────────────────────────────────────────────────────

/// Build an [`EditorTheme`] whose colors match the active UI palette.
///
/// Our own footer renders the mode indicator, so we hide edtui's built-in
/// status line to avoid duplication.
#[must_use]
pub fn theme_from_palette(p: &Palette) -> EditorTheme<'static> {
    EditorTheme::default()
        .base(Style::default().fg(p.foreground).bg(p.background))
        .cursor_style(
            Style::default()
                .fg(p.background)
                .bg(p.accent)
                .add_modifier(Modifier::BOLD),
        )
        .selection_style(Style::default().fg(p.selection_fg).bg(p.selection_bg))
        .line_numbers_style(Style::default().fg(p.gutter).bg(p.background))
        .hide_status_line()
}

// ── Draw ─────────────────────────────────────────────────────────────────────

/// Render the editor overlay into `viewer_area`.
///
/// Layout:
/// - top `viewer_area.height - 1` rows: the edtui `EditorView` widget.
/// - bottom 1 row: mode indicator + dirty marker + command-line or status message.
pub fn draw(f: &mut Frame, app: &mut App, viewer_area: Rect) {
    // Snapshot the palette before we take a mutable borrow on the tab.
    let palette = app.palette;
    let Some(tab) = app.tabs.active_tab_mut() else {
        return;
    };
    let Some(editor) = tab.editor.as_mut() else {
        return;
    };

    // Split the area into editor body + 1-line footer.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(viewer_area);

    let editor_area = chunks[0];
    let footer_area = chunks[1];

    // Render edtui with a theme matching the active UI palette.
    //
    // On the very first draw of a freshly-constructed editor, we render
    // twice: pass one populates edtui's internal `num_rows` (which starts
    // at zero and gates viewport scrolling), pass two actually scrolls the
    // viewport so a non-zero initial cursor row is revealed.  Subsequent
    // draws go through the normal single-render path.
    if !editor.first_draw_done {
        let first_pass = EditorView::new(&mut editor.state).theme(theme_from_palette(&palette));
        f.render_widget(first_pass, editor_area);
        editor.first_draw_done = true;
    }
    let editor_view = EditorView::new(&mut editor.state).theme(theme_from_palette(&palette));
    f.render_widget(editor_view, editor_area);

    // Move the terminal's real cursor to where edtui placed it.
    //
    // `cursor_screen_position()` is populated by edtui during the render call
    // above.  It returns `None` before the first render; in that case we fall
    // back to the top-left of the editor area so the cursor is never left at
    // an arbitrary position.
    let cursor_pos = editor
        .state
        .cursor_screen_position()
        .unwrap_or(Position::new(editor_area.x, editor_area.y));
    f.set_cursor_position(cursor_pos);

    // Build the footer line.
    let mode_label = match editor.state.mode {
        EditorMode::Normal => "-- NORMAL --",
        EditorMode::Insert => "-- INSERT --",
        EditorMode::Visual => "-- VISUAL --",
        // Search is an internal edtui mode; display it as NORMAL to avoid confusion.
        EditorMode::Search => "-- NORMAL --",
    };
    let dirty_marker = if editor.is_dirty() { " [+]" } else { "" };

    // Right side of the footer: command buffer, status message, or nothing.
    let right_text = if let Some(ref cmd) = editor.command_line {
        format!(":{cmd}")
    } else if let Some(ref msg) = editor.status_message {
        msg.clone()
    } else {
        String::new()
    };

    let footer_line = Line::from(vec![
        Span::styled(
            format!("{mode_label}{dirty_marker}"),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(right_text, Style::default().fg(palette.accent_alt)),
    ]);

    f.render_widget(
        Paragraph::new(footer_line).style(Style::default().bg(palette.background)),
        footer_area,
    );
}

// ── Key handler helper ───────────────────────────────────────────────────────

/// Forward a crossterm `KeyEvent` to edtui's event handler.
///
/// We create the handler on every call because `EditorEventHandler` is cheap
/// (it holds no runtime state — just a keymap) and storing it on `TabEditor`
/// would require a generic/lifetime annotation that pollutes the struct.
pub fn forward_key_to_edtui(key: crossterm::event::KeyEvent, state: &mut EditorState) {
    let mut handler = EditorEventHandler::default();
    handler.on_key_event(key, state);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_editor(content: &str) -> TabEditor {
        TabEditor::new(content.to_string())
    }

    #[test]
    fn extract_text_roundtrip() {
        let editor = make_editor("hello\nworld");
        assert_eq!(extract_text(&editor.state), "hello\nworld");
    }

    #[test]
    fn extract_text_single_line() {
        let editor = make_editor("single line");
        assert_eq!(extract_text(&editor.state), "single line");
    }

    #[test]
    fn is_dirty_false_on_new_editor() {
        let editor = make_editor("content");
        assert!(!editor.is_dirty());
    }

    #[test]
    fn command_unknown_sets_status_message() {
        let mut editor = make_editor("text");
        let outcome = dispatch_command(&mut editor, "xyz");
        assert_eq!(outcome, CommandOutcome::Handled);
        assert!(editor.status_message.is_some());
        assert!(editor.status_message.unwrap().contains("xyz"));
    }

    #[test]
    fn command_q_clean_returns_close() {
        let mut editor = make_editor("clean");
        let outcome = dispatch_command(&mut editor, "q");
        assert_eq!(outcome, CommandOutcome::Close);
    }

    #[test]
    fn command_q_dirty_returns_handled_and_sets_message() {
        let mut editor = make_editor("original");
        // Make the editor "dirty" by changing the baseline.
        editor.baseline = "different".to_string();
        let outcome = dispatch_command(&mut editor, "q");
        assert_eq!(outcome, CommandOutcome::Handled);
        assert!(editor.status_message.is_some());
    }

    #[test]
    fn command_q_bang_always_closes() {
        let mut editor = make_editor("original");
        editor.baseline = "different".to_string();
        let outcome = dispatch_command(&mut editor, "q!");
        assert_eq!(outcome, CommandOutcome::Close);
    }

    #[test]
    fn command_wq_returns_save_then_close() {
        let mut editor = make_editor("content");
        let outcome = dispatch_command(&mut editor, "wq");
        assert_eq!(outcome, CommandOutcome::SaveThenClose);
    }

    #[test]
    fn command_w_returns_save() {
        let mut editor = make_editor("content");
        let outcome = dispatch_command(&mut editor, "w");
        assert_eq!(outcome, CommandOutcome::Save);
    }

    #[test]
    fn close_after_save_field_defaults_false() {
        let editor = make_editor("content");
        assert!(!editor.close_after_save);
    }

    #[test]
    fn close_after_save_is_independent_of_status_message() {
        let mut editor = make_editor("content");
        editor.close_after_save = true;
        editor.status_message = Some("saved".into());
        assert!(editor.close_after_save);
        assert_eq!(editor.status_message.as_deref(), Some("saved"));
    }
}
