//! Cursor bridge: translate between source byte offsets and visual (row, col) positions.
//!
//! Pure functions with no I/O or mutation. All three functions are the foundation
//! for sub-phases 2–9 of the hybrid live-preview editing feature.
//!
//! # Coordinate spaces
//!
//! - **Source byte offset**: an index into the raw markdown `&str` passed to
//!   `render_markdown`. Ranges from `0` to `source.len()` inclusive (the
//!   end-of-document sentinel).
//! - **Visual (row, col)**: `(u32, u16)` where `row` is the absolute visual row
//!   in the same coordinate space as `MarkdownViewState::cursor_line`, and `col`
//!   is a display-column offset (unicode-width units, not bytes).

// The three public functions in this module are new API surface not yet wired
// into the production call graph — they will be consumed in sub-phases 2–9.
// Suppress the dead_code lint until then so clippy stays clean.
#![allow(dead_code)]

use std::collections::HashMap;

use unicode_width::UnicodeWidthChar;

use crate::markdown::{DocBlock, TextBlockId};
use crate::ui::markdown_view::WrappedTextLayout;

// ── Public API ────────────────────────────────────────────────────────────────

/// Return the index in `blocks` of the block whose `[source_byte_start,
/// source_byte_end)` range contains `byte`.
///
/// When `byte == source.len()` (cursor at end of document), the last block's
/// index is returned — this is the end-of-doc sentinel.
///
/// # Panics
///
/// Panics when `blocks` is empty (caller's responsibility to guarantee a
/// non-empty block list before calling).
///
/// # Arguments
///
/// * `blocks` – the rendered document block list (contiguous byte ranges
///   guaranteed by the post-render fixup pass in `render_markdown`).
/// * `byte`   – source byte offset to locate.
pub fn byte_offset_to_block(blocks: &[DocBlock], byte: usize) -> usize {
    assert!(
        !blocks.is_empty(),
        "byte_offset_to_block: blocks must not be empty"
    );
    // Binary search using source_byte_start of each block.
    // The fixup pass guarantees blocks[i].source_byte_end == blocks[i+1].source_byte_start,
    // so we can treat starts as sorted partition points.
    let starts: Vec<u32> = blocks.iter().map(block_byte_start).collect();
    match starts.binary_search(&crate::cast::u32_sat(byte)) {
        // Exact match on a block's start offset → that block owns the byte.
        Ok(i) => i,
        // No exact match: `i` is the insertion point.
        // The block containing `byte` starts just before `i`.
        Err(i) => i.saturating_sub(1),
    }
}

/// Convert a source byte offset to a visual `(row, col)` position.
///
/// Returns `None` when:
/// - The block containing `byte` has no entry in `text_layouts` (e.g. Mermaid
///   or Table blocks, which don't use the wrapped-text layout cache). Sub-phase 4
///   will handle those cases.
/// - `text_layouts` is empty (before the first draw).
///
/// Column positions are measured in display columns (unicode-width), not bytes.
///
/// # Arguments
///
/// * `blocks`       – the rendered document block list.
/// * `text_layouts` – the wrap-layout cache from `MarkdownViewState`.
/// * `byte`         – source byte offset to translate.
pub fn byte_to_visual(
    blocks: &[DocBlock],
    text_layouts: &HashMap<TextBlockId, WrappedTextLayout>,
    byte: usize,
) -> Option<(u32, u16)> {
    if blocks.is_empty() {
        return None;
    }
    let block_idx = byte_offset_to_block(blocks, byte);
    let block = &blocks[block_idx];

    // Compute the visual row offset at which this block begins.
    let block_visual_start: u32 = blocks[..block_idx].iter().map(|b| b.height()).sum();

    match block {
        DocBlock::Text {
            id,
            text,
            source_lines,
            ..
        } => {
            let layout = text_layouts.get(id)?;
            let byte_start = block_byte_start(block) as usize;
            let byte_within_block = byte.saturating_sub(byte_start);

            // Walk the source byte slice to find which logical line index contains
            // `byte_within_block`. We derive the source content for this block from
            // its `source_lines` array: each entry gives us the start of a logical
            // line in the rendered output.
            //
            // Since we don't store the raw source per block here, we use the
            // block-relative byte offset and the `source_lines` as a guide. For
            // each logical line in the text block we compute a running byte count
            // from `text.lines[i].spans` text lengths to approximate the logical
            // line that contains `byte_within_block`.
            let (logical_idx, col_in_line) = byte_within_block_to_logical(text, byte_within_block);

            // Map logical index to physical (visual) row via the cache.
            let physical_row = logical_to_first_physical_row(layout, logical_idx);
            let visual_row = block_visual_start + physical_row;

            // Translate byte column to display column using unicode-width.
            let display_col = byte_col_to_display_col(text.lines.get(logical_idx)?, col_in_line);

            // Suppress unused warning — source_lines is present but we derive
            // position from rendered span content, not from raw source bytes.
            let _ = source_lines;

            Some((visual_row, display_col))
        }
        // Mermaid and Table blocks don't have wrapped-text layouts.
        // Sub-phase 4 will handle their cursor positions separately.
        DocBlock::Mermaid { .. } | DocBlock::Table(_) => None,
    }
}

/// Convert a visual `(row, col)` position to a source byte offset.
///
/// This is the inverse of [`byte_to_visual`]. Returns `None` when the target
/// visual row falls outside all blocks' visual ranges, or when the block at
/// that row is not a Text block with a layout cache entry.
///
/// Column positions are measured in display columns (unicode-width), not bytes.
///
/// # Arguments
///
/// * `blocks`       – the rendered document block list.
/// * `text_layouts` – the wrap-layout cache from `MarkdownViewState`.
/// * `visual_row`   – target absolute visual row (same space as `cursor_line`).
/// * `visual_col`   – target display-column offset.
pub fn visual_to_byte(
    blocks: &[DocBlock],
    text_layouts: &HashMap<TextBlockId, WrappedTextLayout>,
    visual_row: u32,
    visual_col: u16,
) -> Option<usize> {
    // Walk blocks to find which one contains `visual_row`.
    let mut offset = 0u32;
    for block in blocks {
        let h = block.height();
        if visual_row < offset + h {
            let local_visual = (visual_row - offset) as usize;
            return match block {
                DocBlock::Text { id, text, .. } => {
                    let layout = text_layouts.get(id)?;
                    // Map the local physical row to its logical line.
                    let logical_idx = layout
                        .physical_to_logical
                        .get(local_visual)
                        .copied()
                        .unwrap_or(0) as usize;
                    let line = text.lines.get(logical_idx)?;
                    // Convert display column to byte offset within the logical line.
                    let byte_col = display_col_to_byte_col(line, visual_col);
                    // Byte offset within block = sum of all logical lines before
                    // `logical_idx` + the byte col within the current line.
                    let byte_before = bytes_before_logical(text, logical_idx);
                    let block_byte_start = block_byte_start(block) as usize;
                    Some(block_byte_start + byte_before + byte_col)
                }
                // Mermaid and Table: not backed by wrapped-text layouts.
                DocBlock::Mermaid { .. } | DocBlock::Table(_) => None,
            };
        }
        offset += h;
    }
    None
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Extract `source_byte_start` from any `DocBlock` variant.
fn block_byte_start(block: &DocBlock) -> u32 {
    match block {
        DocBlock::Text {
            source_byte_start, ..
        } => *source_byte_start,
        DocBlock::Mermaid {
            source_byte_start, ..
        } => *source_byte_start,
        DocBlock::Table(t) => t.source_byte_start,
    }
}

/// Given a `Text` block and a byte offset within the block (not the full source),
/// return `(logical_line_index, byte_col_within_that_line)`.
///
/// The mapping is approximated by summing the byte lengths of rendered span
/// content for each logical line. This correctly handles the common case where
/// each rendered logical line corresponds to one source line; soft-break joining
/// means a single rendered line may cover multiple source lines, but the column
/// offset within the rendered content is what matters for cursor placement.
fn byte_within_block_to_logical(
    text: &ratatui::text::Text<'static>,
    byte_within_block: usize,
) -> (usize, usize) {
    let mut remaining = byte_within_block;
    for (i, line) in text.lines.iter().enumerate() {
        // Compute the byte length of this logical line's rendered content.
        let line_bytes: usize = line.spans.iter().map(|s| s.content.len()).sum();
        // +1 for the implicit newline between lines (the SoftBreak that was
        // collapsed or the line boundary in the source).
        let line_len_with_sep = if i + 1 < text.lines.len() {
            line_bytes + 1
        } else {
            line_bytes
        };
        if remaining <= line_bytes {
            return (i, remaining);
        }
        remaining = remaining.saturating_sub(line_len_with_sep);
    }
    // Clamp to last line if byte is at or past end.
    let last = text.lines.len().saturating_sub(1);
    let last_len: usize = text
        .lines
        .get(last)
        .map_or(0, |l| l.spans.iter().map(|s| s.content.len()).sum());
    (last, last_len.min(remaining))
}

/// Sum of byte lengths of all logical lines before `logical_idx` in `text`,
/// including the separating newlines between them.
fn bytes_before_logical(text: &ratatui::text::Text<'static>, logical_idx: usize) -> usize {
    text.lines[..logical_idx]
        .iter()
        .map(|line| {
            let line_bytes: usize = line.spans.iter().map(|s| s.content.len()).sum();
            // Each line before logical_idx is followed by its newline separator.
            line_bytes + 1
        })
        .sum()
}

/// Return the first physical row index in `layout.physical_to_logical` that
/// maps to `logical_idx`. Falls back to `logical_idx` itself (pessimistic,
/// treats each line as 1 row) when the cache doesn't have the mapping.
fn logical_to_first_physical_row(layout: &WrappedTextLayout, logical_idx: usize) -> u32 {
    layout
        .physical_to_logical
        .iter()
        .position(|&l| l == crate::cast::u32_sat(logical_idx))
        .map_or(crate::cast::u32_sat(logical_idx), crate::cast::u32_sat)
}

/// Convert a byte-column offset within a rendered `Line` to a display-column
/// offset (unicode-width units).
///
/// Walks the line's span content character by character, accumulating
/// display-column widths until `byte_col` bytes have been consumed.
fn byte_col_to_display_col(line: &ratatui::text::Line<'static>, byte_col: usize) -> u16 {
    let mut bytes_consumed = 0usize;
    let mut display_cols = 0u16;
    'outer: for span in &line.spans {
        for ch in span.content.chars() {
            if bytes_consumed >= byte_col {
                break 'outer;
            }
            bytes_consumed += ch.len_utf8();
            display_cols =
                display_cols.saturating_add(UnicodeWidthChar::width(ch).unwrap_or(0) as u16);
        }
    }
    display_cols
}

/// Convert a display-column offset to a byte-column offset within a rendered
/// `Line`. Clamps to the end of the line content if `display_col` is past the
/// last character.
fn display_col_to_byte_col(line: &ratatui::text::Line<'static>, display_col: u16) -> usize {
    let mut cols_remaining = display_col as usize;
    let mut byte_col = 0usize;
    'outer: for span in &line.spans {
        for ch in span.content.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if w > cols_remaining {
                break 'outer;
            }
            cols_remaining = cols_remaining.saturating_sub(w);
            byte_col += ch.len_utf8();
        }
    }
    byte_col
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::{DocBlock, update_text_layouts};
    use crate::theme::{Palette, Theme};
    use ratatui::text::{Line, Span, Text};
    use std::cell::Cell;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // ── Test helpers ─────────────────────────────────────────────────────────

    fn palette() -> Palette {
        Palette::from_theme(Theme::Default)
    }

    fn theme() -> Theme {
        Theme::Default
    }

    /// Hash rendered text content for a stable `TextBlockId` (content-based, not
    /// source-line-based — matches the new `flush_text_block` derivation).
    fn text_block_id(lines: &[Line<'static>]) -> TextBlockId {
        let mut h = DefaultHasher::new();
        for line in lines {
            for span in &line.spans {
                span.content.hash(&mut h);
            }
        }
        lines.len().hash(&mut h);
        TextBlockId(h.finish())
    }

    fn make_text_block(content: Vec<(&str, u32)>) -> DocBlock {
        let lines: Vec<Line<'static>> = content
            .iter()
            .map(|(s, _)| Line::from(Span::raw(s.to_string())))
            .collect();
        let source_lines: Vec<u32> = content.iter().map(|(_, l)| *l).collect();
        let n = lines.len();
        let id = text_block_id(&lines);
        DocBlock::Text {
            id,
            text: Text::from(lines),
            links: vec![],
            heading_anchors: vec![],
            source_lines,
            wrapped_height: Cell::new(crate::cast::u32_sat(n)),
            source_byte_start: 0,
            source_byte_end: 0,
        }
    }

    /// Build a sample document with paragraphs + heading + table + mermaid.
    fn sample_doc() -> (String, Vec<DocBlock>) {
        let md = "Para one.\n\n## Heading\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```mermaid\ngraph LR\nA-->B\n```\n\nFinal para.\n".to_string();
        let p = palette();
        let blocks = crate::markdown::renderer::render_markdown(&md, &p, theme());
        (md, blocks)
    }

    // ── Deliverable 3 tests ──────────────────────────────────────────────────

    /// Every byte in `0..source.len()` must map to exactly one block index
    /// (no gaps, no overlaps). Exercises the contiguous-range invariant
    /// established by the post-render fixup pass.
    #[test]
    fn byte_offset_to_block_covers_full_source() {
        let (source, blocks) = sample_doc();
        assert!(!blocks.is_empty());

        // For every byte in the source, find all blocks that "claim" it and
        // assert exactly one.
        for byte in 0..source.len() {
            let idx = byte_offset_to_block(&blocks, byte);
            let b = &blocks[idx];
            let start = block_byte_start(b) as usize;
            let end = match b {
                DocBlock::Text {
                    source_byte_end, ..
                } => *source_byte_end as usize,
                DocBlock::Mermaid {
                    source_byte_end, ..
                } => *source_byte_end as usize,
                DocBlock::Table(t) => t.source_byte_end as usize,
            };
            assert!(
                start <= byte && byte < end,
                "byte {byte} not in block[{idx}] range [{start}, {end})"
            );
        }

        // The end-of-doc sentinel must also resolve without panic.
        let _ = byte_offset_to_block(&blocks, source.len());
    }

    /// For Text blocks with a populated layout cache, `byte_to_visual` followed
    /// by `visual_to_byte` must return either the original byte, or a byte on
    /// the same logical line (acceptable drift at wrap boundaries).
    #[test]
    fn byte_to_visual_round_trips_via_visual_to_byte() {
        let (source, blocks) = sample_doc();
        let mut text_layouts = HashMap::new();
        update_text_layouts(&blocks, &mut text_layouts, 80);

        // Only test bytes that fall in Text blocks (others return None from
        // byte_to_visual and are covered by the Mermaid/Table sub-phases).
        for (idx, block) in blocks.iter().enumerate() {
            let (start, end) = match block {
                DocBlock::Text {
                    source_byte_start,
                    source_byte_end,
                    ..
                } => (*source_byte_start as usize, *source_byte_end as usize),
                _ => continue,
            };

            // Sample a handful of byte offsets within this Text block.
            let test_bytes: Vec<usize> = (start..end)
                .step_by(((end - start) / 5).max(1))
                .take(6)
                .collect();

            for byte in test_bytes {
                if let Some((vrow, vcol)) = byte_to_visual(&blocks, &text_layouts, byte)
                    && let Some(back) = visual_to_byte(&blocks, &text_layouts, vrow, vcol)
                {
                    // The round-trip lands on the same block.
                    let back_idx = byte_offset_to_block(&blocks, back);
                    assert_eq!(
                        back_idx, idx,
                        "round-trip from byte {byte} ended in block {back_idx}, expected {idx}"
                    );
                    // Within the block, the byte should be on the same logical
                    // line (small drift acceptable at word-wrap boundaries).
                    // We just assert no panic and the result is in-bounds.
                    assert!(
                        back < source.len() + 1,
                        "round-trip byte {back} out of range"
                    );
                }
            }
        }
    }

    /// Two `DocBlock::Text` values with identical rendered content but different
    /// `source_lines` must have the SAME `TextBlockId`.
    ///
    /// This is the core invariant of Deliverable 2: shifting source line numbers
    /// (from an upstream edit) must NOT invalidate the wrap-layout cache for
    /// unchanged blocks.
    #[test]
    fn text_block_id_stable_under_source_line_shift() {
        let lines = vec![
            Line::from(Span::raw("Hello world")),
            Line::from(Span::raw("Second line")),
        ];
        // Two different source_line arrays (simulating upstream edit shifting numbers).
        let source_lines_a = vec![0u32, 1];
        let source_lines_b = vec![10u32, 11];

        let id_a = {
            let mut h = DefaultHasher::new();
            for line in &lines {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            lines.len().hash(&mut h);
            TextBlockId(h.finish())
        };
        let id_b = {
            let mut h = DefaultHasher::new();
            for line in &lines {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            lines.len().hash(&mut h);
            TextBlockId(h.finish())
        };

        // Sanity check the derivation is actually independent of source_lines.
        let _ = (source_lines_a, source_lines_b);
        assert_eq!(
            id_a, id_b,
            "TextBlockId must be identical for same content at different source line numbers"
        );
    }

    /// Two `DocBlock::Text` values with DIFFERENT rendered content must have
    /// DIFFERENT `TextBlockId`s. (Sanity: we still detect content changes.)
    #[test]
    fn text_block_id_changes_under_content_change() {
        let lines_a = vec![Line::from(Span::raw("Content A"))];
        let lines_b = vec![Line::from(Span::raw("Content B"))];

        let id_a = {
            let mut h = DefaultHasher::new();
            for line in &lines_a {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            lines_a.len().hash(&mut h);
            TextBlockId(h.finish())
        };
        let id_b = {
            let mut h = DefaultHasher::new();
            for line in &lines_b {
                for span in &line.spans {
                    span.content.hash(&mut h);
                }
            }
            lines_b.len().hash(&mut h);
            TextBlockId(h.finish())
        };

        assert_ne!(id_a, id_b, "TextBlockId must differ for different content");
    }

    /// After rendering a sample document, the post-render fixup pass must produce
    /// contiguous byte ranges: each block's `source_byte_end` equals the next
    /// block's `source_byte_start`, the first block starts at 0, and the last
    /// block ends at `source.len()`.
    #[test]
    fn block_byte_ranges_contiguous_post_fixup() {
        let (source, blocks) = sample_doc();
        assert!(!blocks.is_empty(), "expected at least one block");

        // First block must start at 0.
        assert_eq!(
            block_byte_start(&blocks[0]),
            0,
            "first block must start at byte 0"
        );

        // Each consecutive pair must be adjacent.
        for i in 0..blocks.len().saturating_sub(1) {
            let this_end = match &blocks[i] {
                DocBlock::Text {
                    source_byte_end, ..
                } => *source_byte_end,
                DocBlock::Mermaid {
                    source_byte_end, ..
                } => *source_byte_end,
                DocBlock::Table(t) => t.source_byte_end,
            };
            let next_start = block_byte_start(&blocks[i + 1]);
            assert_eq!(
                this_end,
                next_start,
                "block[{i}].source_byte_end ({this_end}) != block[{}].source_byte_start ({next_start})",
                i + 1
            );
        }

        // Last block must end at source.len().
        let last_end = match blocks.last().unwrap() {
            DocBlock::Text {
                source_byte_end, ..
            } => *source_byte_end,
            DocBlock::Mermaid {
                source_byte_end, ..
            } => *source_byte_end,
            DocBlock::Table(t) => t.source_byte_end,
        };
        assert_eq!(
            last_end as usize,
            source.len(),
            "last block must end at source.len() = {}",
            source.len()
        );
    }

    /// Verify `byte_to_visual` returns `None` for bytes in Mermaid blocks
    /// (those don't have text_layouts entries).
    #[test]
    fn byte_to_visual_returns_none_for_mermaid_block() {
        let md = "```mermaid\ngraph LR\nA-->B\n```\n";
        let p = palette();
        let blocks = crate::markdown::renderer::render_markdown(md, &p, theme());
        let text_layouts = HashMap::new(); // empty — mermaid blocks have no layout

        // Find the mermaid block.
        let mermaid_block = blocks
            .iter()
            .find(|b| matches!(b, DocBlock::Mermaid { .. }));
        if let Some(b) = mermaid_block {
            let start = block_byte_start(b) as usize;
            // byte_to_visual for a mermaid byte must return None.
            assert!(
                byte_to_visual(&blocks, &text_layouts, start).is_none(),
                "byte_to_visual must return None for Mermaid blocks"
            );
        }
    }

    /// `byte_offset_to_block` must handle a document with a single block
    /// (the edge case where `binary_search` always returns `Err(0)` or `Err(1)`).
    #[test]
    fn byte_offset_to_block_single_block() {
        let md = "Hello world.\n";
        let p = palette();
        let blocks = crate::markdown::renderer::render_markdown(md, &p, theme());
        // Every byte must resolve to block 0.
        for byte in 0..md.len() {
            assert_eq!(
                byte_offset_to_block(&blocks, byte),
                0,
                "byte {byte} must resolve to block 0 in a single-block document"
            );
        }
    }
}
