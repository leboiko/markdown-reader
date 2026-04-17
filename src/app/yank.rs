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
        let target_source =
            crate::markdown::source_line_at(&tab.view.rendered, tab.view.cursor_line);
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
                let top_source =
                    crate::markdown::source_line_at(&tab.view.rendered, range.top_line());
                let bottom_source =
                    crate::markdown::source_line_at(&tab.view.rendered, range.bottom_line());
                build_yank_text(&tab.view.content, top_source, bottom_source)
            }
            VisualMode::Char => {
                // Char mode: extract rendered text from only the selected columns.
                // Walk each line in [top_line, bottom_line] and extract the column
                // range reported by char_range_on_line.
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
                    if let crate::markdown::DocBlock::Text { text, .. } = block {
                        for (local_idx, line) in text.lines.iter().enumerate() {
                            let abs = block_offset + crate::cast::u32_sat(local_idx);
                            if abs > bottom {
                                break 'blocks;
                            }
                            // Compute display width of this line from its spans.
                            let line_width: u16 = line
                                .spans
                                .iter()
                                .map(|s| {
                                    crate::cast::u16_sat(
                                        unicode_width::UnicodeWidthStr::width(s.content.as_ref()),
                                    )
                                })
                                .fold(0u16, u16::saturating_add);
                            if let Some((sc, ec)) = range.char_range_on_line(abs, line_width) {
                                parts.push(extract_line_text_range(line, sc, ec));
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
