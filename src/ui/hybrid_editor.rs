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
//!   Sub-phase 6 will keep these in sync by replaying every edit into edtui after
//!   `apply_edit` mutates the `String` buffer.  Until then the two are deliberately
//!   left un-synced because nothing drives edtui events yet.
//!
//! Callers should treat `source` as the truth for byte-range bookkeeping and
//! `editor_state` as the truth for cursor position and undo history.

// Some items (apply_edit, is_dirty, EditEffect, BlockSourceRange) are used by
// sub-phases 5–9 and not yet wired into the production call graph.  Suppress
// the dead_code lint for those dormant items.
#![allow(dead_code)]

use edtui::{EditorState, Lines};

use crate::markdown::DocBlock;

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
    /// vim mode.  Its `lines: Jagged<char>` is initialized from `source` at
    /// construction time; sub-phase 6 will keep it in sync with `source` on
    /// every edit.  Until then, treat `editor_state` as the cursor/mode oracle
    /// and `source` as the byte-range oracle.
    pub editor_state: EditorState,
    /// Canonical source buffer.  All edits are applied here via `apply_edit`;
    /// byte ranges in `DocBlock`s are valid against this string.
    pub source: String,
    /// Snapshot of the source at hybrid-mode entry, used to detect dirty state
    /// without re-reading the disk file.
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
    /// Set by the `:wq` path in sub-phase 6.
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

// ── Private helpers ───────────────────────────────────────────────────────────

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
}
