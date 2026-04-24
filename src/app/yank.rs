/// Yank (clipboard copy) helpers for the viewer.
///
/// All methods are part of `impl App`.
// Submodule of app — intentionally imports all parent symbols.
#[allow(clippy::wildcard_imports)]
use super::*;

impl App {
    /// Copy the source-level text of the current cursor line to the system
    /// clipboard via OSC 52.  Invoked by the `yy` chord in the viewer.
    pub(super) fn yank_current_line(&mut self) {
        let Some(tab) = self.tabs.active_tab() else {
            return;
        };
        let target_source = crate::markdown::source_line_at(
            &tab.view.rendered,
            tab.view.cursor_line,
            &tab.view.text_layouts,
            &tab.view.table_layouts,
        );
        // `content` is the raw markdown; we index into its lines.
        let content = tab.view.content.clone();
        if let Some(line) = content.lines().nth(target_source as usize) {
            copy_to_clipboard(line);
        }
    }

    /// Copy the source-level text covered by the current visual-line selection
    /// to the system clipboard, then exit visual mode.  Invoked by `y` in visual mode.
    pub(super) fn yank_visual_selection(&mut self) {
        use crate::ui::markdown_view::{VisualMode, extract_line_text_range};
        let Some(tab) = self.tabs.active_tab_mut() else {
            return;
        };
        let Some(range) = tab.view.visual_mode else {
            return;
        };
        let text = match range.mode {
            VisualMode::Line => {
                // Line mode: yank whole source lines (existing behaviour).
                let top_source = crate::markdown::source_line_at(
                    &tab.view.rendered,
                    range.top_line(),
                    &tab.view.text_layouts,
                    &tab.view.table_layouts,
                );
                let bottom_source = crate::markdown::source_line_at(
                    &tab.view.rendered,
                    range.bottom_line(),
                    &tab.view.text_layouts,
                    &tab.view.table_layouts,
                );
                build_yank_text(&tab.view.content, top_source, bottom_source)
            }
            VisualMode::Char => {
                // Char mode: extract rendered text from only the selected
                // columns. Walks each VISUAL row in [top, bottom] and
                // extracts the column range reported by char_range_on_line.
                //
                // Phase 3 invariant: `cursor_line` and `range.top_line()` /
                // `range.bottom_line()` live in visual rows, and a Text
                // block's `block.height()` is its wrapped row count. We
                // must iterate the cached wrapped rows in lockstep, NOT
                // `text.lines` (logical lines) — those agree only for
                // unwrapped paragraphs.
                let mut parts: Vec<String> = Vec::new();
                let mut block_offset = 0u32;
                let top = range.top_line();
                let bottom = range.bottom_line();
                'blocks: for block in &tab.view.rendered {
                    let height = block.height();
                    let block_end = block_offset + height;
                    if block_end <= top {
                        block_offset = block_end;
                        continue;
                    }
                    if block_offset > bottom {
                        break;
                    }
                    if let crate::markdown::DocBlock::Text { id, .. } = block
                        && let Some(layout) = tab.view.text_layouts.get(id)
                    {
                        for (local_visual, wrapped) in layout.wrapped.iter().enumerate() {
                            let abs = block_offset + crate::cast::u32_sat(local_visual);
                            if abs > bottom {
                                break 'blocks;
                            }
                            if let Some((sc, ec)) = range.char_range_on_line(abs, wrapped.width) {
                                let line = wrapped.to_ratatui_line();
                                parts.push(extract_line_text_range(&line, sc, ec));
                            }
                        }
                    }
                    block_offset = block_end;
                }
                parts.join("\n")
            }
        };
        copy_to_clipboard(&text);
        tab.view.visual_mode = None;
    }
}
