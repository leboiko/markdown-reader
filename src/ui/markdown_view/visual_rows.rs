use crate::markdown::DocBlock;
use ratatui::text::Line;
use unicode_width::UnicodeWidthStr;

/// Compute the number of terminal rows a single rendered `Line` occupies when
/// wrapped to `content_width` columns.
///
/// ratatui's `Paragraph::wrap` word-wraps at `content_width`. A line that is
/// shorter than or equal to `content_width` occupies exactly 1 row. Lines wider
/// than `content_width` overflow into additional rows; we calculate the count
/// with ceiling division. Empty lines (zero width) still occupy 1 row.
pub fn line_visual_rows(line: &Line, content_width: u16) -> u32 {
    if content_width == 0 {
        return 1;
    }
    let width: usize = line
        .spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum();
    if width == 0 {
        return 1;
    }
    let cw = content_width as usize;
    crate::cast::u32_sat(width.div_ceil(cw))
}

/// Translate a visual row within the viewport to the absolute logical document
/// line it corresponds to.
///
/// The viewer uses `Paragraph::wrap(Wrap { trim: false })`, so a single logical
/// `Line` that is wider than the content area wraps to multiple visual rows.
/// This means the naive formula `scroll_offset + visual_row_offset` is only
/// correct when every logical line fits on exactly one visual row.
///
/// This function walks the rendered blocks starting at `scroll_offset` and
/// counts visual rows (accounting for wrapping) until it reaches `visual_row`,
/// then returns the logical line index at that position.
///
/// # Arguments
///
/// * `blocks` – the rendered document blocks
/// * `scroll_offset` – the logical document line at the top of the viewport
/// * `visual_row` – 0-based row offset from the top of the content area
/// * `content_width` – width in terminal columns available for text (excluding
///   the gutter when line numbers are shown)
pub fn visual_row_to_logical_line(
    blocks: &[DocBlock],
    scroll_offset: u32,
    visual_row: u32,
    content_width: u16,
) -> u32 {
    let mut remaining_visual = visual_row;
    let mut block_offset = 0u32;

    for block in blocks {
        let block_height = block.height();
        let block_end = block_offset + block_height;

        // Skip blocks that end before the scroll offset.
        if block_end <= scroll_offset {
            block_offset = block_end;
            continue;
        }

        // The first logical line within this block that is visible.
        let clip_start = scroll_offset.saturating_sub(block_offset) as usize;

        match block {
            DocBlock::Text { text, .. } => {
                for (idx, line) in text.lines.iter().enumerate().skip(clip_start) {
                    let rows = line_visual_rows(line, content_width);
                    if remaining_visual < rows {
                        // The clicked row is inside this logical line.
                        return block_offset + crate::cast::u32_sat(idx);
                    }
                    remaining_visual -= rows;
                }
            }
            // Mermaid and Table blocks are opaque (no internal logical lines
            // that can hold links), so treat each visible row as 1 unit.
            DocBlock::Mermaid { cell_height, .. } => {
                let visible_rows = cell_height
                    .get()
                    .saturating_sub(crate::cast::u32_sat(clip_start));
                if remaining_visual < visible_rows {
                    // Inside a mermaid block — no links here; return a sentinel
                    // that won't match any link line.
                    return u32::MAX;
                }
                remaining_visual -= visible_rows;
            }
            DocBlock::Table(t) => {
                let visible_rows = t
                    .rendered_height
                    .saturating_sub(crate::cast::u32_sat(clip_start));
                if remaining_visual < visible_rows {
                    return u32::MAX;
                }
                remaining_visual -= visible_rows;
            }
        }

        block_offset = block_end;
    }

    // Fell off the end — return a value that won't match any link.
    u32::MAX
}
