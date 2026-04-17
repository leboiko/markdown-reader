/// Table-modal open logic for `App`.
///
/// All methods are part of `impl App`.
// Submodule of app — intentionally imports all parent symbols.
#[allow(clippy::wildcard_imports)]
use super::*;

impl App {
    /// Open the table modal if the block at (or nearest to) the cursor is a table.
    ///
    /// Prefers the table the cursor is currently inside.  Falls back to the
    /// first table that intersects the viewport when the cursor is on prose —
    /// this preserves the old click-anywhere-to-expand behaviour for files
    /// where the cursor hasn't been moved into a table yet.
    pub(super) fn try_open_table_modal(&mut self) {
        let view_height = self.tabs.view_height;
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        let viewport_start = tab.view.scroll_offset;
        let viewport_end = viewport_start + view_height;
        let cursor_line = tab.view.cursor_line;

        let mut cursor_match: Option<&crate::markdown::TableBlock> = None;
        let mut viewport_match: Option<&crate::markdown::TableBlock> = None;
        let mut block_start = 0u32;
        for doc_block in &tab.view.rendered {
            let block_end = block_start + doc_block.height();
            if let crate::markdown::DocBlock::Table(table) = doc_block {
                if cursor_line >= block_start && cursor_line < block_end {
                    cursor_match = Some(table);
                    break;
                }
                if viewport_match.is_none()
                    && block_end > viewport_start
                    && block_start < viewport_end
                {
                    viewport_match = Some(table);
                }
            }
            block_start = block_end;
            if block_start >= viewport_end && cursor_match.is_none() {
                // No more blocks can intersect the viewport; only keep
                // scanning if we still need to find a cursor match.
                if cursor_line < block_start {
                    break;
                }
            }
        }

        let Some(table) = cursor_match.or(viewport_match) else {
            return;
        };
        self.table_modal = Some(TableModalState {
            tab_id: tab.id,
            h_scroll: 0,
            v_scroll: 0,
            headers: table.headers.clone(),
            rows: table.rows.clone(),
            alignments: table.alignments.clone(),
            natural_widths: table.natural_widths.clone(),
        });
        self.focus = Focus::TableModal;
    }

    /// If the click coordinates land on an internal `#anchor` link, scroll to
    /// the matching heading. External links are ignored silently.
    ///
    /// `viewer_rect` is the outer border rect of the viewer panel; the inner
    /// content area starts one cell inside on each side.
    pub(super) fn try_follow_link_click(
        &mut self,
        viewer_rect: ratatui::layout::Rect,
        col: u16,
        row: u16,
    ) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };

        // The content inner rect (inside the 1-cell border).
        let inner_x = viewer_rect.x + 1;
        let inner_y = viewer_rect.y + 1;

        if row < inner_y || col < inner_x {
            return;
        }

        let scroll_offset = tab.view.scroll_offset;
        let visual_row = u32::from(row - inner_y);

        // Subtract the gutter width when line numbers are shown. The formula
        // matches render_text_with_gutter so click positions align with text.
        let content_col = if self.show_line_numbers {
            let total_lines = tab.view.total_lines.max(10);
            let num_digits = crate::cast::u16_from_u32((total_lines.ilog10() + 1).max(4));
            let gutter_width = num_digits + 3;
            (col - inner_x).saturating_sub(gutter_width)
        } else {
            col - inner_x
        };

        // `layout_width` is the text content width (excluding the gutter).
        // `Paragraph::wrap` wraps at this width, so logical lines that are
        // wider than `layout_width` occupy multiple visual rows. We must
        // account for this wrapping to convert the clicked visual row back to
        // the correct logical document line.
        let content_width = tab.view.layout_width;
        let clicked_line = crate::ui::markdown_view::visual_row_to_logical_line(
            &tab.view.rendered,
            scroll_offset,
            visual_row,
            content_width,
        );

        let anchor = tab
            .view
            .links
            .iter()
            .find(|l| {
                l.line == clicked_line
                    && content_col >= l.col_start
                    && content_col < l.col_end
                    && l.url.starts_with('#')
            })
            .map(|l| l.url[1..].to_string());

        if let Some(anchor) = anchor {
            let target_line = tab
                .view
                .heading_anchors
                .iter()
                .find(|a| a.anchor == anchor)
                .map(|a| a.line);
            if let Some(line) = target_line {
                let vh = self.tabs.view_height;
                if let Some(tab) = self.tabs.active_tab_mut() {
                    // Set the cursor to the heading line itself, then scroll
                    // so 2 lines of context appear above it.
                    tab.view.cursor_line = line.min(tab.view.total_lines.saturating_sub(1));
                    let max = tab.view.total_lines.saturating_sub(vh / 2);
                    tab.view.scroll_offset = line.saturating_sub(2).min(max);
                }
            }
        }
    }
}
