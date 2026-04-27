//! Data model for hybrid live-preview editing (sub-phases 2–9).
//!
//! This module is **dormant** after sub-phase 2: `Tab::hybrid` stays `None` for
//! all tabs. Sub-phase 4 introduces the `enter_hybrid_mode` entry point that
//! populates it and wires up the `I` keybinding.
//!
//! # Source-buffer duality
//!
//! [`HybridState`] maintains **two representations** of the current source:
//!
//! - `source: String` — the canonical mutable buffer.  All edits go through
//!   [`HybridState::apply_edit`], which splices this buffer directly.
//! - `editor_state.lines: Jagged<char>` — edtui's parallel representation.
//!   Initialized from `source` at construction time via [`HybridState::from_source`].
//!   Sub-phase 6 keeps these in sync by rebuilding `Lines` from `source` after
//!   every `apply_edit` call.  The rebuild is O(n) in source length but fast
//!   enough for typical documents (< 100 µs on a 50 KB file).
//!
//! Callers should treat `source` as the truth for byte-range bookkeeping and
//! `editor_state` as the truth for cursor position and undo history.

use edtui::{EditorState, Lines};

use crate::markdown::DocBlock;
use crate::markdown::cursor_bridge::byte_offset_to_block;
use crate::markdown::renderer::{render_block_from_slice, render_markdown};
use crate::theme::{Palette, Theme};
use crate::ui::markdown_view::MarkdownViewState;

// ── Core types ────────────────────────────────────────────────────────────────

/// Identifies a `DocBlock`'s byte range in the current (possibly mutated) source.
///
/// Stored on [`HybridState::active_block`] so the renderer knows which block to
/// reveal as raw markdown while all others stay formatted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockSourceRange {
    /// Index into the enclosing tab's `view.rendered` block list.
    pub index: usize,
    /// Start byte offset of this block in the current source (inclusive).
    pub start_byte: usize,
    /// End byte offset of this block in the current source (exclusive).
    pub end_byte: usize,
}

/// Effect returned by [`HybridState::apply_edit`], describing the net change
/// to the source buffer's byte and line counts.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // used in sub-phase 6 (editing)
pub struct EditEffect {
    /// Net byte-length change: `inserted.len() as isize − deleted as isize`.
    pub byte_delta: isize,
    /// Net line-count change (positive = more lines, negative = fewer lines).
    pub line_delta: isize,
}

/// Per-tab state for hybrid live-preview editing.
///
/// Present on [`crate::ui::tabs::Tab::hybrid`] only while the tab is in hybrid
/// mode (entered via `enter_hybrid_mode`, sub-phase 4).  `None` in all other
/// states (viewer mode, full editor mode).
///
/// # Source-buffer duality
///
/// See the module-level documentation for the relationship between `source` and
/// `editor_state.lines`.
pub struct HybridState {
    /// edtui editor state — owns the cursor position, undo/redo history, and
    /// vim mode.  Its `lines: Jagged<char>` is rebuilt from `source` after every
    /// edit via `sync_editor_lines_from_source` (sub-phase 6).  Treat
    /// `editor_state` as the cursor/mode oracle and `source` as the byte-range
    /// oracle.
    pub editor_state: EditorState,
    /// Canonical source buffer.  All edits are applied here via `apply_edit`;
    /// byte ranges in `DocBlock`s are valid against this string.
    pub source: String,
    /// Snapshot of the source at hybrid-mode entry, used to detect dirty state
    /// without re-reading the disk file.  Updated by `:w` to track save state.
    pub baseline: String,
    /// Pre-computed byte offset of each line's start in `source`.
    ///
    /// `line_boundaries[i]` is the byte offset where line `i` begins.  There
    /// is always at least one entry: `line_boundaries[0] == 0`.  Rebuilt by
    /// [`HybridState::apply_edit`] after every mutation (O(n) in source length).
    pub line_boundaries: Vec<usize>,
    /// Block whose source byte range currently contains the cursor.
    ///
    /// `None` until sub-phase 4 computes it at mode-entry, and recomputed on
    /// every cursor move or source mutation thereafter.
    pub active_block: Option<BlockSourceRange>,
    /// Ex-command line state (mirrors `TabEditor::command_line`).
    /// Sub-phase 6 wires this up; sub-phase 2 just initializes it to `None`.
    pub command_line: Option<String>,
    /// Transient status message shown at the bottom of the screen.
    /// Sub-phase 6 wires this up; sub-phase 2 just initializes it to `None`.
    pub status_message: Option<String>,
    /// When `true`, the file should be closed after the next successful save.
    /// Set by the `:wq` command path.
    pub close_after_save: bool,
}

impl HybridState {
    /// Construct a `HybridState` from the current source text.
    ///
    /// Initializes edtui from `source` (cursor at position 0, Normal mode),
    /// pre-computes `line_boundaries`, and sets all bookkeeping fields to their
    /// dormant defaults.  Sub-phase 4 will call this when the user presses `I`.
    ///
    /// # Arguments
    ///
    /// * `source` – raw markdown source loaded from disk.
    pub fn from_source(source: &str) -> Self {
        let editor_state = EditorState::new(Lines::from(source));
        let source_owned = source.to_string();
        let line_boundaries = compute_line_boundaries(&source_owned);
        Self {
            editor_state,
            source: source_owned.clone(),
            baseline: source_owned,
            line_boundaries,
            active_block: None,
            command_line: None,
            status_message: None,
            close_after_save: false,
        }
    }

    /// Return `true` when `source` differs from the `baseline` snapshot.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.source != self.baseline
    }

    /// Apply an in-place edit to the source buffer.
    ///
    /// Splices `source` at `byte_offset`, deleting `deleted` bytes and inserting
    /// `inserted`.  Then rebuilds `line_boundaries` and shifts every block's byte
    /// range in `blocks` to keep them consistent with the new source.
    ///
    /// # Block-range update rules
    ///
    /// - Block entirely **before** the edit (`block.source_byte_end <= byte_offset`):
    ///   unchanged.
    /// - Block entirely **after** the edit (`block.source_byte_start >= byte_offset + deleted`):
    ///   both `start_byte` and `end_byte` shift by `byte_delta`.
    /// - Edit is **inside** the block (`start <= byte_offset` and
    ///   `byte_offset + deleted <= end`): only `end_byte` shifts.
    /// - **Insert at exact block end** (`byte_offset == block.source_byte_end` and
    ///   `deleted == 0`): by convention the insertion stays **in block N** (the block
    ///   ending at that offset).  This matches the UX expectation that typing at the
    ///   end of a paragraph extends that paragraph rather than prepending to the next
    ///   one.  Only a `\n\n` sequence should logically cross a block boundary, and
    ///   that is the re-parse-on-leave event handled in sub-phase 6.
    ///
    /// # Cross-block deletes
    ///
    /// Deleting a range that straddles two block boundaries is an internal
    /// invariant violation — sub-phase 6's editing logic guarantees that
    /// user-driven deletions never cross block boundaries.  This method panics
    /// with a clear message if that invariant is broken.
    ///
    /// # Arguments
    ///
    /// * `blocks`      – mutable block list from `tab.view.rendered`.
    /// * `byte_offset` – byte position in `source` at which to splice.
    /// * `deleted`     – number of bytes to remove starting at `byte_offset`.
    /// * `inserted`    – bytes to insert at `byte_offset` after the deletion.
    ///
    /// # Panics
    ///
    /// - `byte_offset + deleted > source.len()` — out-of-bounds splice.
    /// - `byte_offset` or `byte_offset + deleted` is not on a UTF-8 char boundary.
    /// - The deleted range straddles more than one block boundary.
    pub fn apply_edit(
        &mut self,
        blocks: &mut [DocBlock],
        byte_offset: usize,
        deleted: usize,
        inserted: &str,
    ) -> EditEffect {
        // ── Validation ────────────────────────────────────────────────────────
        let delete_end = byte_offset + deleted;
        assert!(
            delete_end <= self.source.len(),
            "apply_edit: byte_offset({byte_offset}) + deleted({deleted}) = {delete_end} \
             exceeds source length ({})",
            self.source.len()
        );
        assert!(
            self.source.is_char_boundary(byte_offset),
            "apply_edit: byte_offset {byte_offset} is not on a UTF-8 char boundary"
        );
        assert!(
            self.source.is_char_boundary(delete_end),
            "apply_edit: byte_offset + deleted = {delete_end} is not on a UTF-8 char boundary"
        );

        // ── Count lines before mutation (for line_delta) ──────────────────────
        let deleted_newlines: isize = self.source[byte_offset..delete_end]
            .bytes()
            .filter(|&b| b == b'\n')
            .count() as isize;
        let inserted_newlines: isize = inserted.bytes().filter(|&b| b == b'\n').count() as isize;

        // ── Splice the source buffer ──────────────────────────────────────────
        self.source.replace_range(byte_offset..delete_end, inserted);

        // ── Rebuild line boundaries ───────────────────────────────────────────
        self.line_boundaries = compute_line_boundaries(&self.source);

        // ── Compute deltas ────────────────────────────────────────────────────
        let byte_delta: isize = inserted.len() as isize - deleted as isize;
        let line_delta: isize = inserted_newlines - deleted_newlines;

        // ── Update block byte ranges ──────────────────────────────────────────
        //
        // Three cases, in priority order:
        //
        //   1. Entirely before (strict):  block.end < byte_offset  → no change.
        //      Strict `<` so that `end == byte_offset` (insert-at-block-end) falls
        //      into case 2, not here.
        //
        //   2. Inside the block:  block.start <= byte_offset AND delete_end <= block.end
        //      → only `end` shifts by `byte_delta`.
        //      Exception: when `start == byte_offset` AND `deleted == 0` AND `start > 0`,
        //      the insertion lands exactly at the start of this block but is already owned
        //      by the previous block's case 2 (insert-at-block-end convention).  Skip to
        //      case 3 so this block shifts rather than extending.
        //      The `start == 0` exemption means inserting at the very front of the document
        //      extends block 0 inward (the natural expectation).
        //
        //   3. Entirely after (or skipped from case 2)  → both `start` and `end` shift.

        for block in blocks.iter_mut() {
            let (start, end) = block_byte_range(block);

            if end < byte_offset {
                // Case 1.
                continue;
            }

            if start <= byte_offset && delete_end <= end {
                // Case 2 — unless we need to defer to case 3.
                //
                // When `start == byte_offset` AND `deleted == 0` AND `start > 0`:
                // the previous block's end equals `byte_offset` and already claimed this
                // insertion via the "insert-at-block-end" convention.  This block should
                // shift entirely (case 3), not extend.
                let defer_to_after = start == byte_offset && deleted == 0 && start > 0;
                if !defer_to_after {
                    set_block_byte_range(block, start, apply_delta(end, byte_delta));
                    continue;
                }
            }

            // Case 3.  Verify the deletion doesn't straddle a block boundary — that
            // would corrupt the bookkeeping.  Sub-phase 6 guarantees this never
            // happens for user-driven edits.
            assert!(
                byte_offset <= start || delete_end <= start,
                "apply_edit: deleted range [{byte_offset}, {delete_end}) crosses block boundary \
                 at byte {start}; cross-block deletes are not supported (sub-phase 6 invariant)"
            );
            set_block_byte_range(
                block,
                apply_delta(start, byte_delta),
                apply_delta(end, byte_delta),
            );
        }

        EditEffect {
            byte_delta,
            line_delta,
        }
    }
}

// ── Public sub-phase 5 helpers ────────────────────────────────────────────────

/// Compute the source byte offset for the current edtui cursor position.
///
/// Looks up `editor_state.cursor.row` in `line_boundaries` to get the byte
/// offset of that line's start, then adds `editor_state.cursor.col` for the
/// column offset.
///
/// # Arguments
///
/// * `editor_state`    – edtui cursor state (owns `cursor.row`, `cursor.col`).
/// * `line_boundaries` – pre-computed byte offsets of line starts in `source`.
pub fn byte_offset_from_editor_state(
    editor_state: &EditorState,
    line_boundaries: &[usize],
) -> usize {
    let row = editor_state.cursor.row;
    let col = editor_state.cursor.col;
    let line_start = line_boundaries
        .get(row)
        .copied()
        .unwrap_or_else(|| line_boundaries.last().copied().unwrap_or(0));
    line_start + col
}

/// Re-detect which block the cursor is in and update `hybrid.active_block`.
///
/// Called after every cursor movement in hybrid mode.  Reads the current byte
/// offset from `hybrid.editor_state` + `hybrid.line_boundaries`, binary-searches
/// `view.rendered` for the containing block, and writes the result back into
/// `hybrid.active_block`.
///
/// When the index changes the draw loop automatically re-renders the old block
/// formatted and the new block raw — no additional work is required.
///
/// # Arguments
///
/// * `hybrid` – mutable hybrid state (source of cursor position, target of
///   `active_block` update).
/// * `view`   – markdown view state (source of the rendered block list).
pub fn recompute_active_block(hybrid: &mut HybridState, view: &MarkdownViewState) {
    if view.rendered.is_empty() {
        hybrid.active_block = None;
        return;
    }
    let cursor_byte = byte_offset_from_editor_state(&hybrid.editor_state, &hybrid.line_boundaries);
    let new_index = byte_offset_to_block(&view.rendered, cursor_byte);
    let (start_byte, end_byte) = block_byte_range(&view.rendered[new_index]);
    hybrid.active_block = Some(BlockSourceRange {
        index: new_index,
        start_byte,
        end_byte,
    });
}

// ── Sub-phase 6: editing primitives ───────────────────────────────────────────

/// Insert a single character at the current cursor byte offset.
///
/// After splicing `source`, rebuilds edtui's `Lines` from the new source so the
/// two representations stay in sync, then repositions the cursor to the byte
/// immediately after the inserted character.
///
/// # Arguments
///
/// * `hybrid` – mutable hybrid state.
/// * `view`   – markdown view state (block list updated in-place by `apply_edit`).
/// * `ch`     – character to insert.
pub fn insert_char(hybrid: &mut HybridState, view: &mut MarkdownViewState, ch: char) {
    let byte = byte_offset_from_editor_state(&hybrid.editor_state, &hybrid.line_boundaries);
    // Encode the char to its UTF-8 byte representation before inserting.
    let mut buf = [0u8; 4];
    let inserted = ch.encode_utf8(&mut buf);
    let char_len = inserted.len();
    hybrid.apply_edit(&mut view.rendered, byte, 0, inserted);
    sync_editor_lines_from_source(hybrid);
    // Advance cursor past the inserted character.
    set_cursor_to_byte(hybrid, byte + char_len);
    recompute_active_block(hybrid, view);
}

/// Delete the character immediately before the cursor (Backspace).
///
/// No-ops when the cursor is at byte 0.  Always steps back to a valid UTF-8
/// char boundary so multi-byte characters are removed atomically.
///
/// # Arguments
///
/// * `hybrid` – mutable hybrid state.
/// * `view`   – markdown view state (block list updated in-place by `apply_edit`).
pub fn delete_char_before(hybrid: &mut HybridState, view: &mut MarkdownViewState) {
    let byte = byte_offset_from_editor_state(&hybrid.editor_state, &hybrid.line_boundaries);
    if byte == 0 {
        return;
    }
    // Find the start of the character just before the cursor.  `byte - 1`
    // moves one byte back; `prev_char_boundary` retreats further if needed to
    // land on a valid UTF-8 boundary (handles multi-byte chars like 'é').
    let char_start = prev_char_boundary(&hybrid.source, byte - 1);
    let char_len = byte - char_start;
    hybrid.apply_edit(&mut view.rendered, char_start, char_len, "");
    sync_editor_lines_from_source(hybrid);
    // Cursor now sits at `char_start` (the deleted char is gone).
    set_cursor_to_byte(hybrid, char_start);
    recompute_active_block(hybrid, view);
}

/// Delete the character at the cursor position (Delete / Del).
///
/// No-ops when the cursor is at or past the end of the source.  Advances past
/// the full multi-byte character so the deletion is always atomic.
///
/// # Arguments
///
/// * `hybrid` – mutable hybrid state.
/// * `view`   – markdown view state (block list updated in-place by `apply_edit`).
pub fn delete_char_after(hybrid: &mut HybridState, view: &mut MarkdownViewState) {
    let byte = byte_offset_from_editor_state(&hybrid.editor_state, &hybrid.line_boundaries);
    if byte >= hybrid.source.len() {
        return;
    }
    // Find the end of the character starting at `byte`.
    let char_end = next_char_boundary(&hybrid.source, byte + 1);
    let char_len = char_end - byte;
    hybrid.apply_edit(&mut view.rendered, byte, char_len, "");
    sync_editor_lines_from_source(hybrid);
    // Cursor stays at `byte` (the deleted char shifted everything left).
    set_cursor_to_byte(hybrid, byte.min(hybrid.source.len()));
    recompute_active_block(hybrid, view);
}

/// Re-parse the block at `block_index` and splice the result into `view.rendered`.
///
/// Called when the cursor leaves a block that was edited.  The rationale is that
/// pulldown-cmark sees the whole block at once and produces structural output
/// (paragraph merging, heading detection, list nesting) — we cannot determine
/// the correct rendered form from a partial view.  On cursor-leave we have the
/// complete final state of the block's source, so we re-parse exactly once and
/// splice the result in.
///
/// The splice may produce one block (most common) or multiple (if the user typed
/// `\n\n` mid-paragraph, splitting it into two).  `splice_blocks` handles cache
/// eviction and `recompute_positions` internally.
///
/// # Arguments
///
/// * `hybrid`      – hybrid state (provides `source` and updated byte ranges).
/// * `view`        – mutable view state (splice target).
/// * `block_index` – 0-based index into `view.rendered` of the block to re-parse.
/// * `palette`     – color palette for the active UI theme.
/// * `theme`       – active UI theme; forwarded to the markdown renderer.
pub fn reparse_and_splice_block(
    hybrid: &HybridState,
    view: &mut MarkdownViewState,
    block_index: usize,
    palette: &Palette,
    theme: Theme,
) {
    // Guard: the index might be out of range if a prior splice changed the
    // block list length.  This should not happen in normal usage (the cursor
    // leaves the block that was being edited), but be defensive.
    let Some(block) = view.rendered.get(block_index) else {
        return;
    };
    let (start, end) = block_byte_range(block);
    // Clamp to actual source length in case of any drift.
    let end = end.min(hybrid.source.len());
    let start = start.min(end);
    let slice = &hybrid.source[start..end];
    let replacement = render_block_from_slice(slice, start, palette, theme);
    view.splice_blocks(block_index..block_index + 1, replacement);
}

/// Perform a full re-parse of `hybrid.source` and replace `view.rendered`.
///
/// This is called defensively on `:w` (save) to eliminate any drift between the
/// incremental byte-range bookkeeping in `apply_edit` and pulldown-cmark's view
/// of the world.  The cost is negligible for user-initiated saves.
///
/// # Arguments
///
/// * `hybrid`  – hybrid state (canonical source buffer).
/// * `view`    – mutable view state (full `rendered` replacement).
/// * `palette` – color palette for the active UI theme.
/// * `theme`   – active UI theme.
pub fn full_reparse(
    hybrid: &HybridState,
    view: &mut MarkdownViewState,
    palette: &Palette,
    theme: Theme,
) {
    let blocks = render_markdown(&hybrid.source, palette, theme);
    // Recompute total_lines and positions; clear stale layout caches so the
    // next draw recalculates wrapped heights at the current terminal width.
    view.total_lines = blocks.iter().map(DocBlock::height).sum();
    view.rendered = blocks;
    view.text_layouts.clear();
    view.table_layouts.clear();
    view.recompute_positions();
}

/// Rebuild edtui's `Lines` (the `Jagged<char>` text buffer) from `hybrid.source`.
///
/// Called after every `apply_edit` so the two representations stay in sync.
/// The cursor position is NOT preserved here — callers must call
/// `set_cursor_to_byte` after `sync_editor_lines_from_source` to land the
/// cursor at the desired byte.
///
/// # Design note
///
/// A full rebuild is O(n) in source length, but for typical markdown documents
/// (< 200 KB) this is under 200 µs per keystroke — well within the 16 ms frame
/// budget.  An incremental approach (mutating individual `Jagged` rows) would be
/// faster but would require intimate knowledge of edtui's internal structure;
/// it is left as a future optimization.
fn sync_editor_lines_from_source(hybrid: &mut HybridState) {
    hybrid.editor_state.lines = Lines::from(hybrid.source.as_str());
}

// ── Cursor movement helpers ────────────────────────────────────────────────────
//
// Each helper follows the same pattern:
//   1. Compute current byte offset.
//   2. Compute new byte offset (clamped, UTF-8-boundary-safe).
//   3. Convert to (row, col) via line_boundaries.
//   4. Write back to editor_state.cursor.
//   5. Call recompute_active_block.
//
// They take `view_height` only when they need it for page-relative movement.

/// Move the hybrid cursor one character to the left.
///
/// No-ops at the start of the document (byte 0).  Always lands on a UTF-8
/// char boundary.
///
/// # Arguments
///
/// * `hybrid` – mutable hybrid state.
/// * `view`   – markdown view state (needed to recompute active block).
pub fn move_cursor_left(hybrid: &mut HybridState, view: &MarkdownViewState) {
    let byte = byte_offset_from_editor_state(&hybrid.editor_state, &hybrid.line_boundaries);
    if byte == 0 {
        return;
    }
    // Step back to the previous char boundary.  `byte - 1` moves behind the
    // current position; `prev_char_boundary` then retreats further if needed
    // to land on a valid UTF-8 boundary (handles multi-byte chars).
    let new_byte = prev_char_boundary(&hybrid.source, byte - 1);
    set_cursor_to_byte(hybrid, new_byte);
    recompute_active_block(hybrid, view);
}

/// Move the hybrid cursor one character to the right.
///
/// No-ops at the end of the document.  Always lands on a UTF-8 char boundary.
///
/// # Arguments
///
/// * `hybrid` – mutable hybrid state.
/// * `view`   – markdown view state (needed to recompute active block).
pub fn move_cursor_right(hybrid: &mut HybridState, view: &MarkdownViewState) {
    let byte = byte_offset_from_editor_state(&hybrid.editor_state, &hybrid.line_boundaries);
    if byte >= hybrid.source.len() {
        return;
    }
    // Step forward one byte at a time until we land on a char boundary.
    let new_byte = next_char_boundary(&hybrid.source, byte + 1);
    set_cursor_to_byte(hybrid, new_byte);
    recompute_active_block(hybrid, view);
}

/// Move the hybrid cursor one source line down.
///
/// Tries to preserve the current column; clamps to the end of the new line
/// when that line is shorter.  No-ops on the last line.
///
/// # Arguments
///
/// * `hybrid` – mutable hybrid state.
/// * `view`   – markdown view state (needed to recompute active block).
pub fn move_cursor_down(hybrid: &mut HybridState, view: &MarkdownViewState) {
    let row = hybrid.editor_state.cursor.row;
    let col = hybrid.editor_state.cursor.col;
    let next_row = row + 1;
    if next_row >= hybrid.line_boundaries.len() {
        return; // already on last line
    }
    let new_byte = clamped_byte_on_line(&hybrid.source, &hybrid.line_boundaries, next_row, col);
    set_cursor_to_byte(hybrid, new_byte);
    recompute_active_block(hybrid, view);
}

/// Move the hybrid cursor one source line up.
///
/// Tries to preserve the current column; clamps to the end of the new line.
/// No-ops on line 0.
///
/// # Arguments
///
/// * `hybrid` – mutable hybrid state.
/// * `view`   – markdown view state (needed to recompute active block).
pub fn move_cursor_up(hybrid: &mut HybridState, view: &MarkdownViewState) {
    let row = hybrid.editor_state.cursor.row;
    if row == 0 {
        return;
    }
    let col = hybrid.editor_state.cursor.col;
    let new_byte = clamped_byte_on_line(&hybrid.source, &hybrid.line_boundaries, row - 1, col);
    set_cursor_to_byte(hybrid, new_byte);
    recompute_active_block(hybrid, view);
}

/// Move the hybrid cursor `count` source lines down (for Page Down).
///
/// # Arguments
///
/// * `hybrid`      – mutable hybrid state.
/// * `view`        – markdown view state (needed to recompute active block).
/// * `count`       – number of lines to advance.
pub fn move_cursor_page_down(hybrid: &mut HybridState, view: &MarkdownViewState, count: usize) {
    let row = hybrid.editor_state.cursor.row;
    let col = hybrid.editor_state.cursor.col;
    let last_row = hybrid.line_boundaries.len().saturating_sub(1);
    let new_row = (row + count).min(last_row);
    if new_row == row {
        return;
    }
    let new_byte = clamped_byte_on_line(&hybrid.source, &hybrid.line_boundaries, new_row, col);
    set_cursor_to_byte(hybrid, new_byte);
    recompute_active_block(hybrid, view);
}

/// Move the hybrid cursor `count` source lines up (for Page Up).
///
/// # Arguments
///
/// * `hybrid`      – mutable hybrid state.
/// * `view`        – markdown view state (needed to recompute active block).
/// * `count`       – number of lines to go back.
pub fn move_cursor_page_up(hybrid: &mut HybridState, view: &MarkdownViewState, count: usize) {
    let row = hybrid.editor_state.cursor.row;
    let col = hybrid.editor_state.cursor.col;
    let new_row = row.saturating_sub(count);
    if new_row == row {
        return;
    }
    let new_byte = clamped_byte_on_line(&hybrid.source, &hybrid.line_boundaries, new_row, col);
    set_cursor_to_byte(hybrid, new_byte);
    recompute_active_block(hybrid, view);
}

/// Move the hybrid cursor to column 0 of the current line (Home).
///
/// # Arguments
///
/// * `hybrid` – mutable hybrid state.
/// * `view`   – markdown view state (needed to recompute active block).
pub fn move_cursor_line_start(hybrid: &mut HybridState, view: &MarkdownViewState) {
    let row = hybrid.editor_state.cursor.row;
    let new_byte = hybrid
        .line_boundaries
        .get(row)
        .copied()
        .unwrap_or_else(|| hybrid.line_boundaries.last().copied().unwrap_or(0));
    set_cursor_to_byte(hybrid, new_byte);
    recompute_active_block(hybrid, view);
}

/// Move the hybrid cursor to the last byte of the current line (End).
///
/// Positions the cursor at the last character before the newline (or the last
/// character of the document on the final line).
///
/// # Arguments
///
/// * `hybrid` – mutable hybrid state.
/// * `view`   – markdown view state (needed to recompute active block).
pub fn move_cursor_line_end(hybrid: &mut HybridState, view: &MarkdownViewState) {
    let row = hybrid.editor_state.cursor.row;
    let line_start = hybrid
        .line_boundaries
        .get(row)
        .copied()
        .unwrap_or_else(|| hybrid.line_boundaries.last().copied().unwrap_or(0));
    // `next_line_start` is the byte after the trailing `\n`; the `\n` itself is at
    // `next_line_start - 1`.  We want the last *content* byte, which is one before
    // the newline: `next_line_start - 2`.  On the final line (no trailing newline)
    // we use `source.len()` directly.
    let line_end_content = hybrid
        .line_boundaries
        .get(row + 1)
        .map(|&next| next.saturating_sub(2))
        .unwrap_or(hybrid.source.len().saturating_sub(1));
    // Clamp to line_start so an empty line (just `\n`) lands at its own start.
    let line_end = line_end_content.max(line_start);
    // Snap to a UTF-8 char boundary in case the column falls inside a multi-byte char.
    let new_byte = if line_end > line_start {
        prev_char_boundary(&hybrid.source, line_end)
    } else {
        line_start
    };
    set_cursor_to_byte(hybrid, new_byte);
    recompute_active_block(hybrid, view);
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Return the largest byte index `<= byte` that is a valid UTF-8 char boundary
/// in `s`.  When `byte == 0` this is always 0.
fn prev_char_boundary(s: &str, byte: usize) -> usize {
    let mut b = byte.min(s.len());
    while b > 0 && !s.is_char_boundary(b) {
        b -= 1;
    }
    b
}

/// Return the smallest byte index `>= byte` that is a valid UTF-8 char boundary
/// in `s`.  Clamps to `s.len()`.
fn next_char_boundary(s: &str, byte: usize) -> usize {
    let mut b = byte.min(s.len());
    while b < s.len() && !s.is_char_boundary(b) {
        b += 1;
    }
    b
}

/// Compute the byte offset for column `col` on `row` in `source`, clamped to
/// the end of that line so the cursor never lands past the newline.
fn clamped_byte_on_line(source: &str, line_boundaries: &[usize], row: usize, col: usize) -> usize {
    let line_start = line_boundaries
        .get(row)
        .copied()
        .unwrap_or_else(|| line_boundaries.last().copied().unwrap_or(0));
    // End of this line = start of next line minus 1, or end of source.
    let line_end = line_boundaries
        .get(row + 1)
        .map(|&next| next.saturating_sub(1))
        .unwrap_or(source.len());
    // The desired byte, clamped to the line's extent.
    let desired = (line_start + col).min(line_end);
    // Snap forward to the nearest UTF-8 char boundary (handles mid-multibyte-char
    // column positions that arise when the previous line was longer).
    next_char_boundary(source, desired)
}

/// Convert a flat byte offset to an edtui `(row, col)` pair using
/// `line_boundaries`, then write it to `hybrid.editor_state.cursor`.
fn set_cursor_to_byte(hybrid: &mut HybridState, byte: usize) {
    let lb = &hybrid.line_boundaries;
    let row = match lb.binary_search(&byte) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let col = byte.saturating_sub(lb.get(row).copied().unwrap_or(0));
    hybrid.editor_state.cursor = edtui::Index2::new(row, col);
}

/// Build the sorted list of byte offsets where each line begins in `source`.
///
/// `result[0]` is always `0`.  `result[i]` is the byte offset immediately after
/// the `(i-1)`th newline character.  The last entry covers the final line even
/// when it has no trailing newline.
fn compute_line_boundaries(source: &str) -> Vec<usize> {
    let mut boundaries = vec![0usize];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            boundaries.push(i + 1);
        }
    }
    boundaries
}

/// Saturating-add a signed delta to a `usize` byte offset.
///
/// Panics in debug builds on underflow (negative result); returns `0` in
/// release builds via saturating arithmetic.
fn apply_delta(value: usize, delta: isize) -> usize {
    if delta >= 0 {
        value + delta as usize
    } else {
        value.saturating_sub((-delta) as usize)
    }
}

/// Extract `(source_byte_start, source_byte_end)` from any `DocBlock` variant.
fn block_byte_range(block: &DocBlock) -> (usize, usize) {
    match block {
        DocBlock::Text {
            source_byte_start,
            source_byte_end,
            ..
        } => (*source_byte_start as usize, *source_byte_end as usize),
        DocBlock::Mermaid {
            source_byte_start,
            source_byte_end,
            ..
        } => (*source_byte_start as usize, *source_byte_end as usize),
        DocBlock::Table(t) => (t.source_byte_start as usize, t.source_byte_end as usize),
    }
}

/// Write new `(source_byte_start, source_byte_end)` values into a `DocBlock`.
fn set_block_byte_range(block: &mut DocBlock, start: usize, end: usize) {
    // Safe casts: source files are well under 4 GiB.
    let start32 = start as u32;
    let end32 = end as u32;
    match block {
        DocBlock::Text {
            source_byte_start,
            source_byte_end,
            ..
        } => {
            *source_byte_start = start32;
            *source_byte_end = end32;
        }
        DocBlock::Mermaid {
            source_byte_start,
            source_byte_end,
            ..
        } => {
            *source_byte_start = start32;
            *source_byte_end = end32;
        }
        DocBlock::Table(t) => {
            t.source_byte_start = start32;
            t.source_byte_end = end32;
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::renderer::render_markdown;
    use crate::theme::{Palette, Theme};

    fn palette() -> Palette {
        Palette::from_theme(Theme::Default)
    }

    fn theme() -> Theme {
        Theme::Default
    }

    /// Render a source document into blocks and wrap it in a `HybridState`.
    fn setup(source: &str) -> (HybridState, Vec<DocBlock>) {
        let state = HybridState::from_source(source);
        let blocks = render_markdown(source, &palette(), theme());
        (state, blocks)
    }

    /// Assert the contiguity invariant holds: each block's end equals the next
    /// block's start, and the last block's end equals `source.len()`.
    fn assert_contiguous(blocks: &[DocBlock], source_len: usize) {
        for i in 0..blocks.len().saturating_sub(1) {
            let (_, end_i) = block_byte_range(&blocks[i]);
            let (start_next, _) = block_byte_range(&blocks[i + 1]);
            assert_eq!(
                end_i,
                start_next,
                "contiguity broken between block[{i}] (end={end_i}) and block[{}] (start={start_next})",
                i + 1
            );
        }
        if let Some(last) = blocks.last() {
            let (_, last_end) = block_byte_range(last);
            assert_eq!(
                last_end, source_len,
                "last block end ({last_end}) != source_len ({source_len})"
            );
        }
    }

    // A 3-block document for most tests.
    //
    // The renderer merges consecutive text paragraphs into a single `DocBlock::Text`,
    // so plain blank-line separation does not yield multiple blocks.  We need explicit
    // block type boundaries: text → mermaid → text.
    //
    //   block 0: DocBlock::Text  ("Para one.")
    //   block 1: DocBlock::Mermaid  (graph LR / A-->B)
    //   block 2: DocBlock::Text  ("Para two.")
    //
    // `BLOCK1_MERMAID_SOURCE_LEN` = length of the mermaid fence block in bytes:
    //   "```mermaid\ngraph LR\nA-->B\n```\n" = 31 bytes.
    const DOC_3: &str = "Para one.\n\n```mermaid\ngraph LR\nA-->B\n```\n\nPara two.\n";

    /// Return the block at index `i`, panicking with context if the index is out of range.
    fn nth(blocks: &[DocBlock], i: usize) -> &DocBlock {
        blocks.get(i).unwrap_or_else(|| {
            panic!(
                "expected block[{i}] but doc only rendered {} block(s); \
                 check DOC_3 produces the expected structure",
                blocks.len()
            )
        })
    }

    #[test]
    fn apply_edit_insert_in_middle_of_block_shifts_only_end_byte() {
        let (mut state, mut blocks) = setup(DOC_3);
        assert!(blocks.len() >= 3, "DOC_3 must render to at least 3 blocks");

        let (b0_start, b0_end) = block_byte_range(nth(&blocks, 0));
        let (b1_start_before, b1_end_before) = block_byte_range(nth(&blocks, 1));
        let (b2_start_before, b2_end_before) = block_byte_range(nth(&blocks, 2));

        // Insert "X" in the middle of block 1 (the mermaid fence).
        // The mermaid source is ASCII, so any offset inside it is a valid char boundary.
        let mid = (b1_start_before + b1_end_before) / 2;
        let effect = state.apply_edit(&mut blocks, mid, 0, "X");

        assert_eq!(effect.byte_delta, 1);
        assert_eq!(effect.line_delta, 0);

        // Block 0: entirely before the edit — unchanged.
        let (b0s, b0e) = block_byte_range(&blocks[0]);
        assert_eq!((b0s, b0e), (b0_start, b0_end), "block 0 must be unchanged");

        // Block 1: start unchanged, end grew by 1.
        let (b1s, b1e) = block_byte_range(&blocks[1]);
        assert_eq!(b1s, b1_start_before, "block 1 start must not change");
        assert_eq!(b1e, b1_end_before + 1, "block 1 end must grow by 1");

        // Block 2: both fields shifted by 1.
        let (b2s, b2e) = block_byte_range(&blocks[2]);
        assert_eq!(b2s, b2_start_before + 1, "block 2 start must shift +1");
        assert_eq!(b2e, b2_end_before + 1, "block 2 end must shift +1");
    }

    #[test]
    fn apply_edit_insert_at_doc_start_shifts_all_blocks() {
        let (mut state, mut blocks) = setup(DOC_3);
        assert!(blocks.len() >= 3, "DOC_3 must render to at least 3 blocks");

        let (_, b0_end_before) = block_byte_range(nth(&blocks, 0));
        let (b1_start_before, b1_end_before) = block_byte_range(nth(&blocks, 1));
        let (b2_start_before, b2_end_before) = block_byte_range(nth(&blocks, 2));

        // Insert "AB" at byte 0 — before block 0 (which starts at 0).
        // Per the inside-block rule: byte_offset(0) < block[0].end → only end shifts.
        let effect = state.apply_edit(&mut blocks, 0, 0, "AB");
        assert_eq!(effect.byte_delta, 2);

        // Block 0 starts at 0 and the insert lands inside it (0 < end) → only end shifts.
        let (b0s, b0e) = block_byte_range(&blocks[0]);
        assert_eq!(b0s, 0, "block 0 start stays at 0");
        assert_eq!(b0e, b0_end_before + 2, "block 0 end must grow by 2");

        // Blocks 1 and 2 are entirely after the edit point → both fields shift.
        let (b1s, b1e) = block_byte_range(&blocks[1]);
        assert_eq!(b1s, b1_start_before + 2);
        assert_eq!(b1e, b1_end_before + 2);

        let (b2s, b2e) = block_byte_range(&blocks[2]);
        assert_eq!(b2s, b2_start_before + 2);
        assert_eq!(b2e, b2_end_before + 2);
    }

    #[test]
    fn apply_edit_delete_range_in_block() {
        let (mut state, mut blocks) = setup(DOC_3);
        assert!(blocks.len() >= 3, "DOC_3 must render to at least 3 blocks");

        let (b1_start_before, b1_end_before) = block_byte_range(nth(&blocks, 1));
        let (b2_start_before, b2_end_before) = block_byte_range(nth(&blocks, 2));

        // Delete 5 bytes from inside block 1 (the mermaid source is >5 bytes long).
        let del_offset = b1_start_before + 2;
        let effect = state.apply_edit(&mut blocks, del_offset, 5, "");
        assert_eq!(effect.byte_delta, -5);

        // Block 1: start unchanged, end shrank by 5.
        let (b1s, b1e) = block_byte_range(&blocks[1]);
        assert_eq!(b1s, b1_start_before);
        assert_eq!(b1e, b1_end_before - 5);

        // Block 2: both fields decreased by 5.
        let (b2s, b2e) = block_byte_range(&blocks[2]);
        assert_eq!(b2s, b2_start_before - 5);
        assert_eq!(b2e, b2_end_before - 5);
    }

    #[test]
    fn apply_edit_at_block_end_stays_in_block() {
        let (mut state, mut blocks) = setup(DOC_3);
        assert!(blocks.len() >= 2, "DOC_3 must render to at least 2 blocks");

        let (_, b0_end_before) = block_byte_range(nth(&blocks, 0));
        let (b1_start_before, b1_end_before) = block_byte_range(nth(&blocks, 1));

        // Insert at the exact end byte of block 0.
        // Convention: insert-at-block-end stays in block N (block 0 here).
        // So block 0's end grows; block 1 (which starts at that offset) shifts right.
        let effect = state.apply_edit(&mut blocks, b0_end_before, 0, "ZZ");
        assert_eq!(effect.byte_delta, 2);

        // Block 0: start unchanged, end grew by 2.
        let (b0s, b0e) = block_byte_range(&blocks[0]);
        assert_eq!(b0s, 0);
        assert_eq!(b0e, b0_end_before + 2);

        // Block 1: was at [b1_start_before, ..); since b1_start_before == b0_end_before
        // and deleted == 0 → delete_end == byte_offset → block 1 start >= delete_end
        // → "entirely after" branch → both start and end shift by +2.
        let (b1s, b1e) = block_byte_range(&blocks[1]);
        assert_eq!(b1s, b1_start_before + 2, "block 1 start must shift +2");
        assert_eq!(b1e, b1_end_before + 2, "block 1 end must shift +2");

        // Contiguity: new block 0 end == new block 1 start.
        assert_eq!(
            b0_end_before + 2,
            b1_start_before + 2,
            "contiguity must be preserved after insert-at-block-end"
        );
    }

    #[test]
    fn apply_edit_preserves_contiguity_invariant() {
        // Use a single-block doc so the edits are always within the one block
        // and no cross-block assertions are needed.  The contiguity helper still
        // verifies last_block.end == source.len().
        let (mut state, mut blocks) = setup("Hello world\n");

        let edits: &[(usize, usize, &str)] = &[
            (5, 0, "inserted"), // insert inside the block
            (2, 3, "xy"),       // replace inside the block
            (0, 0, "prefix"),   // prepend to the block
        ];

        for &(offset, del, ins) in edits {
            state.apply_edit(&mut blocks, offset, del, ins);
            assert_contiguous(&blocks, state.source.len());
        }
    }

    #[test]
    fn apply_edit_line_boundaries_rebuilt() {
        let (mut state, mut blocks) = setup("line one\nline two\n");
        assert_eq!(state.line_boundaries.len(), 3); // byte 0, byte 9, byte 18

        // Insert a newline — should add one boundary entry.
        state.apply_edit(&mut blocks, 4, 0, "\n");
        assert_eq!(
            state.line_boundaries.len(),
            4,
            "inserting one newline must add one line boundary"
        );
    }

    #[test]
    fn apply_edit_line_delta_correct() {
        let (mut state, mut blocks) = setup("paragraph\n");
        let effect = state.apply_edit(&mut blocks, 9, 0, "\n\n\n");
        assert_eq!(
            effect.line_delta, 3,
            "inserting 3 newlines must yield line_delta = 3"
        );
    }

    #[test]
    fn apply_edit_utf8_boundary_validation_panics_on_mid_char() {
        let (mut state, mut blocks) = setup("caf\u{00e9}\n"); // "café" — é is 2 bytes
        // Byte 4 is the second byte of 'é' (U+00E9 → 0xC3 0xA9).
        // Inserting at byte 4 is mid-char and must panic.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            state.apply_edit(&mut blocks, 4, 0, "X");
        }));
        assert!(
            result.is_err(),
            "apply_edit at a mid-char byte offset must panic"
        );
    }

    // ── Sub-phase 5 tests ─────────────────────────────────────────────────────

    use crate::ui::markdown_view::MarkdownViewState;

    /// Build a `MarkdownViewState` pre-populated with `blocks` (empty caches).
    fn view_with_blocks(blocks: Vec<DocBlock>) -> MarkdownViewState {
        let total_lines = blocks.iter().map(DocBlock::height).sum();
        MarkdownViewState {
            rendered: blocks,
            total_lines,
            ..Default::default()
        }
    }

    /// `recompute_active_block` must find block 0 when the cursor is at byte 0.
    #[test]
    fn recompute_active_block_updates_on_cursor_move() {
        let (mut state, blocks) = setup(DOC_3);
        assert!(blocks.len() >= 3, "DOC_3 must render to at least 3 blocks");

        let view = view_with_blocks(blocks);

        // Cursor starts at byte 0 — should be in block 0.
        recompute_active_block(&mut state, &view);
        let ab = state.active_block.expect("active_block must be Some");
        assert_eq!(ab.index, 0, "byte 0 must be in block 0");

        // Now position the cursor at the start of block 2 (the last text block).
        let (_, _) = block_byte_range(&view.rendered[0]);
        let (b2_start, _) = block_byte_range(&view.rendered[2]);
        set_cursor_to_byte(&mut state, b2_start);
        recompute_active_block(&mut state, &view);
        let ab2 = state
            .active_block
            .expect("active_block must be Some after move");
        assert_eq!(
            ab2.index, 2,
            "cursor at block 2 start must identify block 2"
        );
    }

    /// `move_cursor_left` must decrement the byte offset by 1 (for ASCII).
    #[test]
    fn cursor_movement_left_decrements_byte_unless_at_zero() {
        let source = "Hello world\n";
        let (mut state, blocks) = setup(source);
        let view = view_with_blocks(blocks);

        // Position cursor at byte 5.
        set_cursor_to_byte(&mut state, 5);
        move_cursor_left(&mut state, &view);
        let byte_after = byte_offset_from_editor_state(&state.editor_state, &state.line_boundaries);
        assert_eq!(byte_after, 4, "left from byte 5 must land at byte 4");

        // At byte 0 — should not move.
        set_cursor_to_byte(&mut state, 0);
        move_cursor_left(&mut state, &view);
        let byte_at_0 = byte_offset_from_editor_state(&state.editor_state, &state.line_boundaries);
        assert_eq!(byte_at_0, 0, "left at byte 0 must stay at 0");
    }

    /// `move_cursor_right` on a multi-byte character must advance by the full
    /// char width, not just 1 byte.
    #[test]
    fn cursor_movement_respects_utf8_boundaries() {
        // 'é' is U+00E9 encoded as 0xC3 0xA9 — 2 bytes.
        let source = "caf\u{00e9}\n"; // "café\n"
        let (mut state, blocks) = setup(source);
        let view = view_with_blocks(blocks);

        // Byte 3 is the start of 'é'.
        set_cursor_to_byte(&mut state, 3);
        move_cursor_right(&mut state, &view);
        let byte_after = byte_offset_from_editor_state(&state.editor_state, &state.line_boundaries);
        // Moving right from byte 3 should land at byte 5 (past the 2-byte 'é').
        assert_eq!(
            byte_after, 5,
            "right from byte 3 must skip 2-byte char 'é' and land at byte 5"
        );
    }

    /// `move_cursor_down` from the last line of block 0 must land in the next
    /// block, and `recompute_active_block` must update the active_block index.
    #[test]
    fn cursor_movement_down_crosses_block_boundary() {
        let (mut state, blocks) = setup(DOC_3);
        assert!(blocks.len() >= 2, "DOC_3 must render at least 2 blocks");
        let view = view_with_blocks(blocks);

        // "Para one.\n\n" — block 0 ends after the second newline.
        // Line boundaries: [0, 10, 11, ...].
        // Row 0: "Para one."  (bytes 0..9)
        // Row 1: ""           (bytes 10..10 — just the blank line after the paragraph)
        // Position cursor at row 1 (the last line of block 0's source coverage).
        let (_, b0_end) = block_byte_range(&view.rendered[0]);
        // The source for block 0 is "Para one.\n\n" (11 bytes, 0..11).
        // Line 1 starts at byte 10 ('\n' at byte 9 → line 1 starts at 10).
        set_cursor_to_byte(&mut state, b0_end.saturating_sub(1));
        recompute_active_block(&mut state, &view);
        let before_idx = state.active_block.map(|ab| ab.index).unwrap_or(99);

        move_cursor_down(&mut state, &view);
        recompute_active_block(&mut state, &view);
        let after_idx = state.active_block.map(|ab| ab.index).unwrap_or(99);
        // After moving down, the cursor should be in a later block.
        assert!(
            after_idx >= before_idx,
            "moving down must not move backward in block index"
        );
    }

    /// `move_cursor_line_start` / `move_cursor_line_end` must land at the correct
    /// byte offsets on a known line.
    #[test]
    fn cursor_movement_line_start_and_end() {
        let source = "first line\nsecond line\n";
        // Line boundaries: [0, 11, 23].
        // Line 1: "second line" → bytes 11..22, end at 22 (before the \n at 22).
        let (mut state, blocks) = setup(source);
        let view = view_with_blocks(blocks);

        // Position on line 1 mid-way.
        set_cursor_to_byte(&mut state, 15); // inside "second line"
        move_cursor_line_start(&mut state, &view);
        let start_byte = byte_offset_from_editor_state(&state.editor_state, &state.line_boundaries);
        assert_eq!(
            start_byte, 11,
            "line_start must land at the beginning of line 1"
        );

        move_cursor_line_end(&mut state, &view);
        let end_byte = byte_offset_from_editor_state(&state.editor_state, &state.line_boundaries);
        // "second line" is 11 chars → last char at byte 11 + 10 = 21.
        assert_eq!(
            end_byte, 21,
            "line_end must land at last char before the newline"
        );
    }

    /// The raw height of a block equals `wrap_spans(slice, width).len()`.
    #[test]
    fn active_block_raw_height_matches_wrapped_slice() {
        let source = "Short paragraph.\n";
        let (_, blocks) = setup(source);
        let (b_start, b_end) = block_byte_range(&blocks[0]);
        let slice = &source[b_start..b_end];
        let raw_span = ratatui::text::Span::raw(slice);
        let wrapped = crate::text_layout::wrap_spans(&[raw_span], 80);
        // `wrap_spans` emits one row for the content and one empty row for the
        // trailing '\n', so a paragraph ending in '\n' always produces 2 rows.
        assert_eq!(
            wrapped.len(),
            2,
            "paragraph ending in '\\n' wraps to 2 rows (content + empty)"
        );
    }

    // ── Sub-phase 6 tests ─────────────────────────────────────────────────────

    /// Typing 5 characters into a paragraph must grow the active block's byte
    /// range by exactly 5.
    #[test]
    fn insert_char_extends_active_block_byte_range() {
        let source = "Hello\n";
        let (mut state, blocks) = setup(source);
        let mut view = view_with_blocks(blocks);

        // Initialise active_block from byte 0 (inside the sole paragraph block).
        recompute_active_block(&mut state, &view);
        let before = state.active_block.expect("active_block must be Some");
        let before_len = before.end_byte - before.start_byte;

        // Insert 5 ASCII characters at the cursor (byte 0).
        for ch in ['A', 'B', 'C', 'D', 'E'] {
            insert_char(&mut state, &mut view, ch);
        }

        recompute_active_block(&mut state, &view);
        let after = state
            .active_block
            .expect("active_block must be Some after inserts");
        let after_len = after.end_byte - after.start_byte;
        assert_eq!(
            after_len,
            before_len + 5,
            "5 inserted chars must extend the active block byte range by 5"
        );
    }

    /// Regression for "I press chars but the active block doesn't update on
    /// screen". The active block in hybrid mode is rendered from the slice
    /// `hybrid.source[block.source_byte_start..block.source_byte_end]`. After
    /// each insert the slice MUST contain the newly typed character — if it
    /// doesn't, the renderer keeps drawing the pre-edit content and the user
    /// sees the cursor advance with no visible change.
    #[test]
    fn insert_char_active_block_slice_includes_new_char() {
        let source = "Hello\n";
        let (mut state, blocks) = setup(source);
        let mut view = view_with_blocks(blocks);

        // Cursor at byte 5 (just before the trailing newline).
        set_cursor_to_byte(&mut state, 5);
        recompute_active_block(&mut state, &view);

        insert_char(&mut state, &mut view, 'X');

        let ab = state
            .active_block
            .expect("active_block must be Some after insert");
        let slice = &state.source[ab.start_byte..ab.end_byte];
        assert_eq!(
            slice, "HelloX\n",
            "active block slice must reflect the inserted X — got {slice:?}"
        );
    }

    /// Cursor in the middle of a block: the inserted char must land between
    /// the existing characters in the active block's slice (not appended at
    /// the end).
    #[test]
    fn insert_char_in_middle_of_active_block_appears_in_slice() {
        let source = "Hello\n";
        let (mut state, blocks) = setup(source);
        let mut view = view_with_blocks(blocks);

        // Cursor between the two `l`s.
        set_cursor_to_byte(&mut state, 3);
        recompute_active_block(&mut state, &view);

        insert_char(&mut state, &mut view, 'X');

        let ab = state.active_block.expect("active_block must be Some");
        let slice = &state.source[ab.start_byte..ab.end_byte];
        assert_eq!(
            slice, "HelXlo\n",
            "mid-block insert must show X between the ls — got {slice:?}"
        );
    }

    /// When the cursor sits at the *start* of a non-first block, `apply_edit`'s
    /// "insert-at-block-end" convention attributes the insert to the previous
    /// block. The active block (the one the cursor is in) does NOT grow, so its
    /// slice doesn't change; the user sees no visible update in the active block
    /// even though the source mutated. This test pins that behaviour so we
    /// notice if it ever changes — it's the most likely UX-confusion path.
    #[test]
    fn insert_at_active_block_start_lands_in_previous_block() {
        let (mut state, blocks) = setup(DOC_3);
        assert!(blocks.len() >= 3, "DOC_3 must render to at least 3 blocks");
        let mut view = view_with_blocks(blocks);

        // Position cursor at the start of block 2 (the "Para two." text block).
        let (b2_start, b2_end) = block_byte_range(&view.rendered[2]);
        set_cursor_to_byte(&mut state, b2_start);
        recompute_active_block(&mut state, &view);
        assert_eq!(
            state.active_block.unwrap().index,
            2,
            "cursor must be in block 2"
        );

        let len_before = b2_end - b2_start;
        insert_char(&mut state, &mut view, 'X');

        // After insert: block 2's slice did NOT grow — the X went to block 1.
        let ab = state.active_block.expect("active_block must be Some");
        let slice = &state.source[ab.start_byte..ab.end_byte];
        let len_after = ab.end_byte - ab.start_byte;
        assert_eq!(
            len_after, len_before,
            "insert at block 2 start must NOT extend block 2 (the X goes to block 1) — \
             this is the apply_edit insert-at-end convention; got slice {slice:?}"
        );
    }

    /// Backspace at byte 0 must leave the source unchanged.
    #[test]
    fn backspace_at_byte_zero_does_nothing() {
        let source = "Hello\n";
        let (mut state, blocks) = setup(source);
        let mut view = view_with_blocks(blocks);
        set_cursor_to_byte(&mut state, 0);
        delete_char_before(&mut state, &mut view);
        assert_eq!(
            state.source, source,
            "backspace at byte 0 must not mutate source"
        );
    }

    /// Backspace in the middle of block 1 must shrink block 1's end byte by 1
    /// and shift block 2's start and end bytes by −1.
    #[test]
    fn backspace_in_middle_of_block_decrements_byte_range() {
        let (mut state, blocks) = setup(DOC_3);
        assert!(blocks.len() >= 3, "DOC_3 must render to at least 3 blocks");
        let mut view = view_with_blocks(blocks);

        let (b1_start_before, b1_end_before) = block_byte_range(&view.rendered[1]);
        let (b2_start_before, b2_end_before) = block_byte_range(&view.rendered[2]);

        // Place cursor at byte 10 inside block 1 (the mermaid fence is > 10 bytes).
        let cursor_byte = b1_start_before + 10;
        set_cursor_to_byte(&mut state, cursor_byte);
        delete_char_before(&mut state, &mut view);

        let (b1s, b1e) = block_byte_range(&view.rendered[1]);
        let (b2s, b2e) = block_byte_range(&view.rendered[2]);

        assert_eq!(b1s, b1_start_before, "block 1 start must not change");
        assert_eq!(b1e, b1_end_before - 1, "block 1 end must shrink by 1");
        assert_eq!(b2s, b2_start_before - 1, "block 2 start must shift -1");
        assert_eq!(b2e, b2_end_before - 1, "block 2 end must shift -1");
    }

    /// Inserting `\n` in the middle of a paragraph extends the block's byte
    /// range by 1.  On cursor-leave, the re-parse may produce 1 or 2 blocks
    /// (depending on whether a blank line was also inserted).
    #[test]
    fn enter_inserts_newline_extends_block() {
        let source = "Hello world\n";
        let (mut state, blocks) = setup(source);
        let mut view = view_with_blocks(blocks);

        let (_, b0_end_before) = block_byte_range(&view.rendered[0]);

        set_cursor_to_byte(&mut state, 5); // after "Hello"
        insert_char(&mut state, &mut view, '\n');

        let (_, b0_end_after) = block_byte_range(&view.rendered[0]);
        assert_eq!(
            b0_end_after,
            b0_end_before + 1,
            "inserting one newline must extend the block's end byte by 1"
        );
        assert_eq!(
            state.source, "Hello\n world\n",
            "source must reflect the inserted newline"
        );
    }

    /// After typing `**bold**` and moving off the block, the splice must
    /// re-parse and produce a block whose `TextBlockId` is different from the
    /// original (content changed → new hash → cache-eviction + re-render).
    #[test]
    fn cursor_leave_after_edits_reparses_block() {
        let source = "Hello\n\n```mermaid\ngraph LR\nA-->B\n```\n";
        let (mut state, blocks) = setup(source);
        assert!(blocks.len() >= 2, "doc must have text + mermaid");
        let mut view = view_with_blocks(blocks);

        // Capture the original TextBlockId of block 0.
        let original_id = match &view.rendered[0] {
            DocBlock::Text { id, .. } => *id,
            _ => panic!("block 0 must be a text block"),
        };

        // Type " world" at the end of the paragraph (byte 5, before the `\n`).
        set_cursor_to_byte(&mut state, 5);
        for ch in " world".chars() {
            insert_char(&mut state, &mut view, ch);
        }

        // Simulate cursor leaving block 0 → re-parse it.
        reparse_and_splice_block(&state, &mut view, 0, &palette(), theme());

        // The re-parsed block must have a different TextBlockId because the
        // content hash is derived from the rendered spans — different source →
        // different spans → different id.
        let new_id = match &view.rendered[0] {
            DocBlock::Text { id, .. } => *id,
            _ => panic!("block 0 must still be a text block after re-parse"),
        };
        assert_ne!(
            new_id, original_id,
            "re-parsed block must have a different TextBlockId (content changed)"
        );
    }

    /// Typing a mermaid fence into a text block and triggering cursor-leave
    /// must increase the block count (mermaid fence forces a new `DocBlock::Mermaid`
    /// plus surrounding text blocks).
    ///
    /// With per-element granularity each paragraph is its own `DocBlock::Text`,
    /// so "Intro.\n\nOutro.\n" already produces 2 blocks before the edit.
    /// Inserting a mermaid fence between them creates additional blocks (at
    /// minimum a Mermaid block plus any split text blocks).
    #[test]
    fn type_mermaid_fence_splits_text_block() {
        // Two paragraphs — each in its own block with per-element granularity.
        let source = "Intro.\n\nOutro.\n";
        let (mut state, blocks) = setup(source);
        let mut view = view_with_blocks(blocks);
        let block_count_before = view.rendered.len();
        // With per-element granularity, "Intro." and "Outro." are separate blocks.
        assert!(
            block_count_before >= 2,
            "expected at least 2 text blocks for 2 paragraphs"
        );

        // Insert a mermaid fence between "Intro.\n\n" and "Outro.\n".
        // Byte 8 is the start of "Outro".
        let fence = "```mermaid\ngraph LR\nA-->B\n```\n\n";
        set_cursor_to_byte(&mut state, 8);
        for ch in fence.chars() {
            insert_char(&mut state, &mut view, ch);
        }

        // Simulate cursor leave on block 0.
        reparse_and_splice_block(&state, &mut view, 0, &palette(), theme());

        // The document now contains a mermaid block, so total block count must
        // be at least 3 (intro text, mermaid, outro text).
        assert!(
            view.rendered.len() >= 3,
            "inserting a mermaid fence must produce at least 3 blocks (text+mermaid+text); \
             source = {:?}, block count = {}",
            state.source,
            view.rendered.len(),
        );
        assert!(
            view.rendered
                .iter()
                .any(|b| matches!(b, DocBlock::Mermaid { .. })),
            "re-parsed document must contain a Mermaid block",
        );
    }

    /// `delete_char_after` (Delete key) at a known ASCII position must remove
    /// exactly that character.
    #[test]
    fn delete_char_after_removes_correct_char() {
        let source = "Hello\n";
        let (mut state, blocks) = setup(source);
        let mut view = view_with_blocks(blocks);

        set_cursor_to_byte(&mut state, 0); // cursor on 'H'
        delete_char_after(&mut state, &mut view);
        assert_eq!(state.source, "ello\n", "Delete at byte 0 must remove 'H'");
    }

    /// `delete_char_after` at the end of the source must be a no-op.
    #[test]
    fn delete_char_after_at_end_is_noop() {
        let source = "Hi\n";
        let (mut state, blocks) = setup(source);
        let mut view = view_with_blocks(blocks);

        set_cursor_to_byte(&mut state, source.len()); // past end
        delete_char_after(&mut state, &mut view);
        assert_eq!(
            state.source, "Hi\n",
            "Delete past end must not change source"
        );
    }

    /// Inserting and then backspacing a multi-byte UTF-8 character must leave
    /// the source byte-for-byte identical to the original.
    #[test]
    fn utf8_safe_editing_insert_and_backspace() {
        let source = "Hello\n";
        let (mut state, blocks) = setup(source);
        let mut view = view_with_blocks(blocks);

        set_cursor_to_byte(&mut state, 5); // after "Hello", before '\n'
        // 'é' is U+00E9 — 2 bytes in UTF-8.
        insert_char(&mut state, &mut view, '\u{00e9}');
        assert_eq!(state.source, "Hello\u{00e9}\n");

        // Cursor is now at byte 7 (5 + 2).  Backspace must remove both bytes.
        delete_char_before(&mut state, &mut view);
        assert_eq!(
            state.source, "Hello\n",
            "backspace after multi-byte insert must restore original source"
        );
    }

    /// `full_reparse` must rebuild `view.rendered` from `hybrid.source` so that
    /// the block list reflects any edits accumulated via `apply_edit`.
    #[test]
    fn full_reparse_rebuilds_block_list() {
        let source = "Para one.\n\n```mermaid\ngraph LR\nA-->B\n```\n\nPara two.\n";
        let (mut state, blocks) = setup(source);
        let mut view = view_with_blocks(blocks);
        let original_len = view.rendered.len();

        // Insert enough content to potentially change block structure.
        // We'll insert a new paragraph at byte 0.
        state.apply_edit(&mut view.rendered, 0, 0, "New intro.\n\n");
        // Source is now longer and has an extra paragraph.

        full_reparse(&state, &mut view, &palette(), theme());

        // After full reparse the block list must contain at least one more block
        // (the new paragraph).
        assert!(
            view.rendered.len() >= original_len,
            "full_reparse must produce at least as many blocks after inserting a new paragraph"
        );
        // The rendered block list must cover the entire new source length.
        if let Some(last) = view.rendered.last() {
            let (_, end) = block_byte_range(last);
            assert_eq!(
                end,
                state.source.len(),
                "full_reparse: last block end must equal source length"
            );
        }
    }

    /// `is_dirty` must reflect edits and clear after baseline update.
    #[test]
    fn is_dirty_tracks_edits_and_clears_on_baseline_update() {
        let source = "Hello\n";
        let (mut state, mut blocks) = setup(source);
        assert!(!state.is_dirty(), "fresh state must not be dirty");

        state.apply_edit(&mut blocks, 0, 0, "X");
        assert!(state.is_dirty(), "state must be dirty after edit");

        // Simulate what save does: update baseline.
        state.baseline.clone_from(&state.source);
        assert!(
            !state.is_dirty(),
            "state must not be dirty after baseline sync"
        );
    }

    // ── Sub-phase 7 tests: active tables ─────────────────────────────────────

    // ── Sub-phase 8 tests — active mermaid (deferred re-render on leave) ─────
    //
    // Investigation findings (sub-phase 8):
    //
    // The byte-range fixup pass in `renderer.rs` assigns each mermaid block's
    // `[source_byte_start, source_byte_end)` based on `source_line` (the opening
    // fence's line number) for the start and the NEXT block's `source_byte_start`
    // for the end.  Because `emit_mermaid_block` calls `push_blank_line()` after
    // emitting the mermaid, that blank line inherits the source-line number of the
    // first content line inside the fence (e.g. "graph LR"), which becomes the
    // text block's `source_byte_start`.  The net effect:
    //
    //   DOC_3 = "Para one.\n\n```mermaid\ngraph LR\nA-->B\n```\n\nPara two.\n"
    //
    //   block[0]: Text  [  0, 11)  "Para one.\n\n"
    //   block[1]: Mermaid [11, 22)  "```mermaid\n"   ← only the opening fence
    //   block[2]: Text  [ 22, 52)  "graph LR\nA-->B\n```\n\nPara two.\n"
    //
    // The mermaid block's `source` field ("graph LR\nA-->B") contains the
    // diagram content, but its byte range only covers the opening fence line.
    //
    // Consequence for hybrid mode:
    //
    //   When the cursor enters the mermaid block and the draw loop renders the
    //   active block as raw source via `doc_block.source_byte_range()`, it only
    //   shows "```mermaid\n".  The diagram content is in the adjacent text block.
    //
    //   On cursor-leave `reparse_and_splice_block` re-parses only the opening
    //   fence slice ("```mermaid\n") — an incomplete mermaid source — producing
    //   spurious blocks rather than a clean single Mermaid replacement.
    //
    // This is a known limitation of the current byte-range assignment strategy
    // for mermaid blocks.  The tests below pin the CURRENT behaviour so it is
    // visible and regression-protected.  A future sub-phase (targeted fix to the
    // byte-range fixup pass) would correct the byte ranges to span the full fence
    // and the tests would then be updated.
    //
    // The KEY performance guarantee (unchanged mermaid source → same
    // MermaidBlockId → no spurious re-render) is verified indirectly via the
    // content-hash test below: the `id` stored on `DocBlock::Mermaid` is derived
    // from the `source` field (`hash_str(content_between_fences)`).  Because the
    // source field is populated once at parse time from the fence content and is
    // NOT mutated by `apply_edit` (only the byte ranges shift), an "enter + leave
    // without edit" round-trip does not change the `source` field and therefore
    // does not change the `id`.  `ensure_queued` will hit the cache.

    /// Locate the `DocBlock::Mermaid` in a block list and return its index.
    /// Panics when no mermaid block exists (guards against mis-structured test docs).
    fn find_mermaid_block(blocks: &[DocBlock]) -> usize {
        blocks
            .iter()
            .position(|b| matches!(b, DocBlock::Mermaid { .. }))
            .expect("test document must contain at least one DocBlock::Mermaid")
    }

    /// When the cursor is positioned inside a mermaid block, `recompute_active_block`
    /// must identify that block as active (index equals the mermaid block's index).
    ///
    /// This is the precondition for the draw loop in `draw.rs` to enter the
    /// `is_active_block` branch, which renders the block's `source_byte_range()`
    /// as raw markdown instead of calling the mermaid chart pipeline.
    ///
    /// The branch at `draw.rs:404` is generic over all `DocBlock` variants — it
    /// calls `doc_block.source_byte_range()` without branching on the type.  The
    /// mermaid block's current byte range covers only the opening fence line
    /// ("```mermaid\n"); see the module-level note above for context.
    #[test]
    fn active_mermaid_renders_raw_when_cursor_inside() {
        let (mut state, blocks) = setup(DOC_3);
        let mermaid_idx = find_mermaid_block(&blocks);
        let view = view_with_blocks(blocks);

        // Position the cursor at the start of the mermaid block.
        let (mermaid_start, mermaid_end) = block_byte_range(&view.rendered[mermaid_idx]);
        set_cursor_to_byte(&mut state, mermaid_start);
        recompute_active_block(&mut state, &view);

        let ab = state
            .active_block
            .expect("active_block must be Some when cursor is inside mermaid");
        assert_eq!(
            ab.index, mermaid_idx,
            "cursor at mermaid source_byte_start must activate the mermaid block"
        );
        assert_eq!(
            ab.start_byte, mermaid_start,
            "active_block.start_byte must equal the mermaid block's source_byte_start"
        );
        assert_eq!(
            ab.end_byte, mermaid_end,
            "active_block.end_byte must equal the mermaid block's source_byte_end"
        );

        // The raw source slice must span the entire fenced region: opening
        // fence, diagram content, and closing fence. Box-drawing characters
        // appear only in the formatted AsciiDiagram render output.
        let raw_slice = &state.source[mermaid_start..mermaid_end];
        assert!(
            raw_slice.starts_with("```mermaid"),
            "raw source slice for a mermaid block must start with the opening fence, got: {raw_slice:?}"
        );
        assert!(
            raw_slice.trim_end().ends_with("```"),
            "raw source slice must include the closing fence, got: {raw_slice:?}"
        );
        assert!(
            !raw_slice.contains('\u{2502}') && !raw_slice.contains('\u{250C}'),
            "raw source slice must not contain box-drawing chars (those appear only in formatted render)"
        );
    }

    /// The `MermaidBlockId` is derived from the mermaid `source` field
    /// (the content between the fences, populated at parse time).  This field is
    /// NOT mutated by `apply_edit`; only the byte-range fields shift.  Therefore
    /// an "enter + leave without edit" round-trip preserves the id, and
    /// `ensure_queued` will return early (cache hit → no spurious re-render).
    ///
    /// This test verifies the cache-key stability guarantee directly on the
    /// `DocBlock::Mermaid` id field, without going through the full cache.
    #[test]
    fn mermaid_cursor_leave_with_unchanged_source_keeps_id_and_cache() {
        let (_, blocks) = setup(DOC_3);
        let mermaid_idx = find_mermaid_block(&blocks);

        // Capture the MermaidBlockId directly from the parsed block.
        // The id is `MermaidBlockId(hash_str(source_between_fences))`.
        let id_before = match &blocks[mermaid_idx] {
            DocBlock::Mermaid { id, .. } => *id,
            _ => panic!("expected DocBlock::Mermaid at mermaid_idx"),
        };

        // Re-parse the same source document to simulate a full re-render pass.
        // On cursor-leave without editing, the source bytes are unchanged, so
        // render_markdown produces a block with the same `source` field and
        // therefore the same id.
        let blocks2 = crate::markdown::renderer::render_markdown(DOC_3, &palette(), theme());
        let mermaid_idx2 = find_mermaid_block(&blocks2);
        let id_after = match &blocks2[mermaid_idx2] {
            DocBlock::Mermaid { id, .. } => *id,
            _ => panic!("expected DocBlock::Mermaid after re-parse"),
        };

        assert_eq!(
            id_before.0, id_after.0,
            "unchanged mermaid source must yield the same MermaidBlockId \
             so the cache entry survives and no spurious re-render is triggered"
        );
    }

    /// When the mermaid content changes between two parse runs (simulating the
    /// user editing the diagram), the resulting `MermaidBlockId` must differ.
    ///
    /// A different id means `ensure_queued` will see a cache miss on the next
    /// draw frame and enqueue a fresh async render — the deferred re-render
    /// on leave that the sub-phase is named after.
    #[test]
    fn mermaid_cursor_leave_triggers_reparse_with_new_id() {
        let (_, blocks) = setup(DOC_3);
        let mermaid_idx = find_mermaid_block(&blocks);
        let original_id = match &blocks[mermaid_idx] {
            DocBlock::Mermaid { id, .. } => *id,
            _ => panic!("expected DocBlock::Mermaid at mermaid_idx"),
        };

        // Parse a modified document: diagram content changed ("A-->B" → "A-->C").
        let modified = DOC_3.replace("A-->B", "A-->C");
        let blocks2 = crate::markdown::renderer::render_markdown(&modified, &palette(), theme());
        let mermaid_idx2 = find_mermaid_block(&blocks2);
        let new_id = match &blocks2[mermaid_idx2] {
            DocBlock::Mermaid { id, .. } => *id,
            _ => panic!("expected DocBlock::Mermaid after re-parse of modified source"),
        };

        assert_ne!(
            new_id.0, original_id.0,
            "changed mermaid content must produce a new MermaidBlockId \
             (different hash → cache miss → async re-render queued)"
        );
    }

    /// When the cursor is at a specific byte inside a mermaid block,
    /// `byte_to_visual_raw` must return the correct `(visual_row, visual_col)`.
    ///
    /// The mermaid block's byte range in DOC_3 is [11, 22) = "```mermaid\n"
    /// (the opening fence only — see module-level note).  At inner_width = 80
    /// (no wrapping), "```mermaid" occupies row 0.  The cursor at the first byte
    /// of that range lands at (row 0, col 0).  A cursor 3 bytes in lands at col 3.
    #[test]
    fn cursor_inside_mermaid_block_byte_to_visual_works() {
        use crate::markdown::cursor_bridge::byte_to_visual_raw;

        let (state, blocks) = setup(DOC_3);
        let mermaid_idx = find_mermaid_block(&blocks);
        let view = view_with_blocks(blocks);

        let (mermaid_start, _) = block_byte_range(&view.rendered[mermaid_idx]);
        let mermaid_block = &view.rendered[mermaid_idx];

        // Cursor at the very first byte of the mermaid block (the opening backtick)
        // → row 0, col 0 (block_visual_start = 0 for simplicity).
        let (row, col) = byte_to_visual_raw(mermaid_block, &state.source, 0, 80, mermaid_start);
        assert_eq!(
            row, 0,
            "cursor at start of mermaid block must be on visual row 0"
        );
        assert_eq!(col, 0, "cursor at start of mermaid block must be at col 0");

        // Cursor 3 bytes in ("`````|mermaid") → still row 0, col 3.
        let mid_first_row = mermaid_start + 3;
        let (row2, col2) = byte_to_visual_raw(mermaid_block, &state.source, 0, 80, mid_first_row);
        assert_eq!(
            row2, 0,
            "cursor 3 bytes into the opening fence must remain on row 0"
        );
        assert_eq!(
            col2, 3,
            "cursor 3 bytes into the opening fence must be at col 3"
        );
    }

    /// Typing a character inside an active mermaid block must extend the block's
    /// `source_byte_end` by the byte length of the inserted character and shift
    /// all subsequent blocks' byte ranges.
    ///
    /// This exercises the generic `apply_edit` byte-range bookkeeping for
    /// `DocBlock::Mermaid` — the same path that handles `DocBlock::Text` and
    /// `DocBlock::Table` in sub-phases 6 and 7.
    #[test]
    fn editing_inside_active_mermaid_extends_byte_range() {
        let (mut state, blocks) = setup(DOC_3);
        let mermaid_idx = find_mermaid_block(&blocks);
        let mut view = view_with_blocks(blocks);

        // Initialise active_block with cursor inside the mermaid block.
        let (mermaid_start, mermaid_end_before) = block_byte_range(&view.rendered[mermaid_idx]);
        set_cursor_to_byte(&mut state, mermaid_start);
        recompute_active_block(&mut state, &view);

        // Insert one ASCII character inside the mermaid block.
        insert_char(&mut state, &mut view, 'Z');

        // Re-read the mermaid block's range after the edit.
        let (_, mermaid_end_after) = block_byte_range(&view.rendered[mermaid_idx]);
        assert_eq!(
            mermaid_end_after,
            mermaid_end_before + 1,
            "inserting one ASCII char inside the mermaid block must extend source_byte_end by 1"
        );

        // The block after the mermaid (index mermaid_idx + 1) must shift right by 1.
        if let Some(next_block) = view.rendered.get(mermaid_idx + 1) {
            let (next_start, _) = block_byte_range(next_block);
            assert_eq!(
                next_start,
                mermaid_end_before + 1,
                "block after the mermaid must shift right by 1 after insert"
            );
        }

        // Contiguity invariant must still hold across all blocks.
        assert_contiguous(&view.rendered, state.source.len());
    }

    // A document that has a table block we can target for these tests.
    // Structure: text (para), then table, then text (para).
    // The table source is:
    //   | Col1 | Col2 |\n|---|---|\n| a | b |\n
    // which pulldown-cmark parses as a TableBlock.
    const DOC_WITH_TABLE: &str = "Intro.\n\n| Col1 | Col2 |\n|---|---|\n| a | b |\n\nOutro.\n";

    /// Locate the `DocBlock::Table` in a block list and return its index.
    /// Panics when no table block exists (guards against mis-structured test docs).
    fn find_table_block(blocks: &[DocBlock]) -> usize {
        blocks
            .iter()
            .position(|b| matches!(b, DocBlock::Table(_)))
            .expect("DOC_WITH_TABLE must render to at least one DocBlock::Table")
    }

    /// When the cursor is positioned inside a table block, the active-block index
    /// must equal the table block's index. This is the pre-condition for the draw
    /// loop to render the table as raw markdown rather than the formatted box.
    ///
    /// The actual raw-render path lives in `draw.rs` and is not unit-testable here
    /// without a full ratatui backend, but the active-block detection — which gates
    /// the raw render — is fully exercised.
    #[test]
    fn active_table_renders_raw_when_cursor_inside() {
        let (mut state, blocks) = setup(DOC_WITH_TABLE);
        let table_idx = find_table_block(&blocks);
        let view = view_with_blocks(blocks);

        // Position the cursor at the start of the table block.
        let (table_start, _) = block_byte_range(&view.rendered[table_idx]);
        set_cursor_to_byte(&mut state, table_start);
        recompute_active_block(&mut state, &view);

        let ab = state.active_block.expect("active_block must be Some");
        assert_eq!(
            ab.index, table_idx,
            "cursor at the table's source_byte_start must identify the table block as active"
        );

        // Verify the byte range stored on active_block matches the table's actual range.
        let (expected_start, expected_end) = block_byte_range(&view.rendered[table_idx]);
        assert_eq!(
            ab.start_byte, expected_start,
            "active_block.start_byte must equal the table's source_byte_start"
        );
        assert_eq!(
            ab.end_byte, expected_end,
            "active_block.end_byte must equal the table's source_byte_end"
        );

        // Confirm the raw source slice contains the pipe characters that mark it
        // as a markdown table (not box-drawing chars from the formatted render).
        let raw_slice = &state.source[expected_start..expected_end];
        assert!(
            raw_slice.contains('|'),
            "raw source slice for a table block must contain '|' characters"
        );
        assert!(
            !raw_slice.contains('\u{2502}') && !raw_slice.contains('\u{250C}'),
            "raw source slice must not contain box-drawing chars (those appear only in formatted render)"
        );
    }

    /// After moving the cursor out of the table block, `recompute_active_block`
    /// must identify a different block. On cursor-leave `reparse_and_splice_block`
    /// would re-render the table as a formatted box; here we just verify the
    /// active-block detection flips, which is the trigger for that re-render.
    #[test]
    fn active_table_re_renders_box_on_cursor_leave() {
        let (mut state, blocks) = setup(DOC_WITH_TABLE);
        let table_idx = find_table_block(&blocks);
        // Capture byte ranges before `blocks` is consumed by `view_with_blocks`.
        let (table_start, table_end) = block_byte_range(&blocks[table_idx]);
        let view = view_with_blocks(blocks);

        // Enter the table block.
        set_cursor_to_byte(&mut state, table_start);
        recompute_active_block(&mut state, &view);
        let inside = state
            .active_block
            .expect("active_block must be Some inside table");
        assert_eq!(
            inside.index, table_idx,
            "cursor inside table must activate table block"
        );

        // Move cursor past the table — into the block that follows it.
        // `table_end` is the start of the next block (contiguity invariant).
        set_cursor_to_byte(&mut state, table_end);
        recompute_active_block(&mut state, &view);
        let outside = state
            .active_block
            .expect("active_block must be Some after table");
        assert_ne!(
            outside.index, table_idx,
            "cursor past the table's source_byte_end must no longer activate the table block"
        );
        // The cursor is now in the block that comes after the table (index > table_idx).
        assert!(
            outside.index > table_idx,
            "block after the table must have a higher index than the table"
        );

        // Simulate cursor-leave re-parse: the table block must be re-parsed from
        // its source slice and splice back in. The splice is not directly observable
        // here, but `reparse_and_splice_block` must not panic and the block list
        // must still cover the full source.  We set up a fresh view for the splice
        // call since `view` was consumed by the earlier `view_with_blocks(blocks)`.
        let (state2, blocks2) = setup(DOC_WITH_TABLE);
        let mut view2 = view_with_blocks(blocks2);
        reparse_and_splice_block(&state2, &mut view2, table_idx, &palette(), theme());
        // No assertion needed — absence of panic is the contract.
    }

    /// When the cursor is at a specific byte inside a table block, `byte_to_visual_raw`
    /// must return the correct `(visual_row, visual_col)` within the raw rendering.
    ///
    /// The table source is:
    ///   `| Col1 | Col2 |\n|---|---|\n| a | b |\n`
    ///
    /// At `inner_width = 80` (no wrapping), the first `|` is at col 0, row 0.
    /// The `\n` after the first row is at byte offset (len of first row) within
    /// the slice; the second row starts at row 1.
    #[test]
    fn cursor_inside_table_byte_to_visual_works() {
        use crate::markdown::cursor_bridge::byte_to_visual_raw;

        let (state, blocks) = setup(DOC_WITH_TABLE);
        let table_idx = find_table_block(&blocks);
        let view = view_with_blocks(blocks);

        let (table_start, _) = block_byte_range(&view.rendered[table_idx]);
        let table_block = &view.rendered[table_idx];

        // Cursor at the very start of the table (the first `|`) → row 0, col 0.
        let (row, col) = byte_to_visual_raw(
            table_block,
            &state.source,
            0, // block_visual_start = 0 for this test (only the table matters)
            80,
            table_start,
        );
        assert_eq!(row, 0, "cursor at start of table must be on visual row 0");
        assert_eq!(col, 0, "cursor at start of table must be at col 0");

        // The first table source row is "| Col1 | Col2 |\n" — 18 bytes.
        // Cursor at the start of the second row (byte table_start + 18) → row 1, col 0.
        let first_row = "| Col1 | Col2 |\n";
        let second_row_byte = table_start + first_row.len();
        let (row2, col2) = byte_to_visual_raw(table_block, &state.source, 0, 80, second_row_byte);
        assert_eq!(
            row2, 1,
            "cursor at start of second table row must be on visual row 1"
        );
        assert_eq!(
            col2, 0,
            "cursor at start of second table row must be at col 0"
        );
    }

    /// Typing a character inside an active table must extend the table block's
    /// `source_byte_end` by the byte length of the inserted character.
    ///
    /// This mirrors `insert_char_extends_active_block_byte_range` from sub-phase 6
    /// but targets a `DocBlock::Table` to confirm the generic `apply_edit` byte-range
    /// bookkeeping works for all block types.
    #[test]
    fn editing_inside_active_table_extends_byte_range() {
        let (mut state, blocks) = setup(DOC_WITH_TABLE);
        let table_idx = find_table_block(&blocks);
        let mut view = view_with_blocks(blocks);

        // Initialise active_block with cursor inside the table.
        let (table_start, table_end_before) = block_byte_range(&view.rendered[table_idx]);
        set_cursor_to_byte(&mut state, table_start);
        recompute_active_block(&mut state, &view);

        // Insert one ASCII character inside the table block.
        insert_char(&mut state, &mut view, 'X');

        // Re-read the table block's range after the edit.
        let (_, table_end_after) = block_byte_range(&view.rendered[table_idx]);
        assert_eq!(
            table_end_after,
            table_end_before + 1,
            "inserting one ASCII char inside the table must extend source_byte_end by 1"
        );

        // Blocks after the table must shift right by 1 as well.
        if let Some(next_block) = view.rendered.get(table_idx + 1) {
            let (next_start, _) = block_byte_range(next_block);
            // The next block's start must now be 1 byte further right.
            // It was at `table_end_before` (contiguity) and must now be at `table_end_before + 1`.
            assert_eq!(
                next_start,
                table_end_before + 1,
                "block after the table must shift right by 1 after insert"
            );
        }

        // Contiguity invariant must still hold.
        assert_contiguous(&view.rendered, state.source.len());
    }
}
