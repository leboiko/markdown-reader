use super::gutter::render_text_with_gutter;
use super::highlight::{apply_block_highlight, highlight_matches};
use super::mermaid_draw::{MermaidDrawParams, draw_mermaid_block};
use super::state::VisualRange;
use crate::action::Action;
use crate::app::App;
use crate::markdown::{DocBlock, MermaidBlockId, update_mermaid_heights};
use crate::mermaid::MermaidRenderConfig;
use crate::ui::table_render::layout_table;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::Text,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::borrow::Cow;

/// How many display lines above and below the viewport to prefetch mermaid
/// renders. Large enough that normal scrolling rarely hits an unrendered
/// placeholder; small enough that unused diagrams don't waste CPU.
const LAZY_RENDER_LOOKAHEAD: u32 = 50;

// ---------------------------------------------------------------------------
// Draw-instruction types — private to this module; used only in `draw`.
// ---------------------------------------------------------------------------

/// Deferred text-block render instruction.
struct TextDraw {
    y: u16,
    height: u16,
    text: Text<'static>,
    first_line_number: u32,
}

/// Deferred mermaid-block render instruction.
struct MermaidDraw {
    y: u16,
    height: u16,
    fully_visible: bool,
    id: MermaidBlockId,
    source: String,
    /// Absolute logical-line index where this block starts in the document.
    block_start: u32,
    /// Total height of this block in logical lines.
    block_height: u32,
    /// Visual selection at the time of the draw instruction capture.
    visual_mode: Option<VisualRange>,
}

/// Render the markdown preview panel into `area`.
///
/// # Arguments
///
/// * `f`       – the ratatui frame to render into.
/// * `app`     – mutable application state (tabs, caches, settings).
/// * `area`    – the terminal rectangle allocated to this panel.
/// * `focused` – whether the viewer panel currently has keyboard focus.
#[allow(clippy::many_single_char_names, clippy::too_many_lines)]
pub fn draw(f: &mut Frame, app: &mut App, area: Rect, focused: bool) {
    let p = app.palette;

    let active_tab = app.tabs.active_tab();
    let file_name = active_tab.map_or("", |t| t.view.file_name.as_str());

    // Build the title string before the block so its lifetime covers the block
    // borrow.  It is only used in the bordered path, but must be declared in
    // the outer scope regardless so the borrow checker is satisfied.
    let title: Cow<str> = if file_name.is_empty() {
        Cow::Borrowed(" Preview ")
    } else {
        Cow::Owned(format!(" {file_name} "))
    };

    // When the tree is hidden the viewer expands to the full terminal width.
    // Drawing borders in that state wastes 2 columns and 2 rows, and the border
    // box looks odd with nothing alongside it.  Skip borders entirely and let
    // the tab bar (which already spans the full width) serve as the visual
    // separator.
    let block = if app.tree_hidden {
        Block::default().style(Style::default().bg(p.background))
    } else {
        let border_style = if focused {
            p.border_focused_style()
        } else {
            p.border_style()
        };
        Block::default()
            .title(title.as_ref())
            .title_style(p.title_style())
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(Style::default().bg(p.background))
    };

    // When borderless the inner area equals the outer area (no 1-cell border on
    // each edge), so the viewport is 2 rows taller than in bordered mode.
    app.tabs.view_height = u32::from(block.inner(area).height);

    let has_content = app
        .tabs
        .active_tab()
        .is_some_and(|t| !t.view.content.is_empty());

    if !has_content {
        let empty = Paragraph::new("No file selected. Select a markdown file from the tree.")
            .style(p.dim_style().bg(p.background))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    let view_height = app.tabs.view_height;
    let inner = block.inner(area);
    f.render_widget(block, area);

    // When line numbers are on, the gutter steals cells from the left of the
    // content area. Tables must be laid out against the actual content width
    // or their rows wrap inside ratatui's Paragraph and the grid breaks.
    let effective_width = if app.show_line_numbers {
        let estimate = app
            .tabs
            .active_tab()
            .map_or(10, |t| t.view.total_lines.max(10));
        let num_digits = crate::cast::u16_from_u32((estimate.ilog10() + 1).max(4));
        let gutter_width = num_digits + 3;
        inner.width.saturating_sub(gutter_width)
    } else {
        inner.width
    };

    // If the effective width has changed, all table layout caches are stale.
    // Recompute heights for every table block and update total_lines.
    {
        // Safety: `has_content` is true so there is always an active tab here.
        // Use a guard instead of `.unwrap()` to satisfy the no-unwrap rule.
        let Some(tab) = app.tabs.active_tab_mut() else {
            return;
        };
        if tab.view.layout_width == effective_width {
            // Populate cache for any tables not yet laid out (e.g. first draw).
            // Track whether any new table was added so we know to recompute positions.
            let mut layout_changed = false;
            for doc_block in &mut tab.view.rendered {
                if let DocBlock::Table(table) = doc_block
                    && let std::collections::hash_map::Entry::Vacant(e) =
                        tab.view.table_layouts.entry(table.id)
                {
                    let (text, height, _was_truncated) = layout_table(table, effective_width, &p);
                    table.rendered_height = height;
                    e.insert(super::state::TableLayout { text });
                    layout_changed = true;
                }
            }
            // update_mermaid_heights returns true when any block's height changed.
            // Only recompute positions (O(blocks)) when something actually moved —
            // calling it unconditionally every frame was the source of UI freezes on
            // large documents.
            let mermaid_changed = update_mermaid_heights(
                &tab.view.rendered,
                &app.mermaid_cache,
                app.mermaid_max_height,
            );
            if layout_changed || mermaid_changed {
                tab.view.total_lines = tab
                    .view
                    .rendered
                    .iter()
                    .map(|b: &DocBlock| b.height())
                    .sum();
                tab.view.recompute_positions();
                let max_scroll = tab.view.total_lines.saturating_sub(view_height / 2);
                tab.view.scroll_offset = tab.view.scroll_offset.min(max_scroll);
            }
        } else {
            tab.view.layout_width = effective_width;
            tab.view.table_layouts.clear();

            for doc_block in &mut tab.view.rendered {
                if let DocBlock::Table(table) = doc_block {
                    let (text, height, _was_truncated) = layout_table(table, effective_width, &p);
                    table.rendered_height = height;
                    tab.view
                        .table_layouts
                        .insert(table.id, super::state::TableLayout { text });
                }
            }

            // Width changed — all block heights may have shifted; always recompute.
            update_mermaid_heights(
                &tab.view.rendered,
                &app.mermaid_cache,
                app.mermaid_max_height,
            );
            tab.view.total_lines = tab
                .view
                .rendered
                .iter()
                .map(|b: &DocBlock| b.height())
                .sum();
            tab.view.recompute_positions();
            let max_scroll = tab.view.total_lines.saturating_sub(view_height / 2);
            tab.view.scroll_offset = tab.view.scroll_offset.min(max_scroll);
        }
    }

    // Safety: `has_content` is true so there is always an active tab here.
    let Some(tab) = app.tabs.active_tab() else {
        return;
    };
    let scroll_offset = tab.view.scroll_offset;
    let cursor_line = tab.view.cursor_line;
    // Copy the visual selection so we can use it while iterating over blocks
    // without holding a borrow into `app.tabs`.
    let visual_mode = tab.view.visual_mode;

    let doc_search_query =
        if !tab.doc_search.query.is_empty() && !tab.doc_search.match_lines.is_empty() {
            Some((
                tab.doc_search.query.clone(),
                tab.doc_search
                    .match_lines
                    .get(tab.doc_search.current_match)
                    .copied(),
            ))
        } else {
            None
        };

    // Build a flat list of (block_start_line, block) to find which blocks
    // intersect [scroll_offset, scroll_offset + view_height).
    let viewport_end = scroll_offset + view_height;

    // Mermaid blocks within this extended window are queued for rendering even
    // if not yet visible, so that scrolling rarely hits an unrendered placeholder.
    let lookahead_start = scroll_offset.saturating_sub(LAZY_RENDER_LOOKAHEAD);
    let lookahead_end = viewport_end + LAZY_RENDER_LOOKAHEAD;

    // We can't hold a borrow into `app.tabs` while also accessing
    // `app.mermaid_cache`, so we collect rendering instructions first.
    let mut text_draws: Vec<TextDraw> = Vec::new();
    let mut mermaid_draws: Vec<MermaidDraw> = Vec::new();
    let mut mermaid_to_queue: Vec<(MermaidBlockId, String)> = Vec::new();

    {
        // Safety: same guard — active tab is guaranteed by `has_content`.
        let Some(tab) = app.tabs.active_tab() else {
            return;
        };
        let mut block_start = 0u32;

        for doc_block in &tab.view.rendered {
            let block_height: u32 = DocBlock::height(doc_block);
            let block_end = block_start + block_height;

            // Queue mermaid blocks within the lookahead window.
            if let DocBlock::Mermaid { id, source, .. } = doc_block
                && block_end > lookahead_start
                && block_start < lookahead_end
            {
                mermaid_to_queue.push((MermaidBlockId(id.0), source.clone()));
            }

            if block_end > scroll_offset && block_start < viewport_end {
                // Lines within this block that are visible.
                let clip_start = scroll_offset.saturating_sub(block_start);
                let clip_end = (viewport_end - block_start).min(block_height);
                let visible_lines = clip_end.saturating_sub(clip_start);

                // Y offset in the inner rect.
                let y_in_viewport = block_start.saturating_sub(scroll_offset);
                let rect_y = inner
                    .y
                    .saturating_add(crate::cast::u16_from_u32(y_in_viewport));

                if rect_y < inner.y + inner.height && visible_lines > 0 {
                    let draw_height = crate::cast::u16_from_u32(
                        visible_lines.min(u32::from(inner.y + inner.height - rect_y)),
                    );

                    match doc_block {
                        DocBlock::Text { text, .. } => {
                            // Slice only the visible lines from this Text block.
                            let start = clip_start as usize;
                            let end = (clip_start + visible_lines)
                                .min(crate::cast::u32_sat(text.lines.len()))
                                as usize;
                            let mut visible_text = if let Some((query, current_line)) =
                                &doc_search_query
                            {
                                let full_text =
                                    highlight_matches(text, query, *current_line, block_start, &p);
                                let sliced_lines = full_text.lines[start..end].to_vec();
                                Text::from(sliced_lines)
                            } else {
                                let sliced_lines = text.lines[start..end].to_vec();
                                Text::from(sliced_lines)
                            };

                            // Apply highlight(s) when the viewer has focus.
                            // In visual mode every line in the selection gets highlighted;
                            // in normal mode only the single cursor row is highlighted.
                            let block_end = block_start + block_height;
                            if focused {
                                apply_block_highlight(
                                    &mut visible_text.lines,
                                    visual_mode,
                                    cursor_line,
                                    block_start,
                                    block_end,
                                    start,
                                    p.selection_bg,
                                );
                            }

                            text_draws.push(TextDraw {
                                y: rect_y,
                                height: draw_height,
                                text: visible_text,
                                first_line_number: block_start + clip_start + 1,
                            });
                        }
                        DocBlock::Mermaid { id, source, .. } => {
                            // Render the image when the block is as visible as
                            // it can get: fully visible for small blocks, or
                            // filling the viewport for blocks taller than the
                            // viewport. Show a placeholder only while the block
                            // is entering/exiting the viewport edges.
                            let max_renderable = block_height.min(u32::from(inner.height));
                            let fully_visible = visible_lines >= max_renderable
                                && u32::from(draw_height) >= max_renderable;
                            mermaid_draws.push(MermaidDraw {
                                y: rect_y,
                                height: draw_height,
                                fully_visible,
                                id: MermaidBlockId(id.0),
                                source: source.clone(),
                                block_start,
                                block_height,
                                visual_mode,
                            });
                        }
                        DocBlock::Table(table) => {
                            // Slice visible lines from the cached rendered text.
                            if let Some(cached) = tab.view.table_layouts.get(&table.id) {
                                let start = clip_start as usize;
                                let end = (clip_start + visible_lines)
                                    .min(crate::cast::u32_sat(cached.text.lines.len()))
                                    as usize;
                                let mut visible_text =
                                    if let Some((query, current_line)) = &doc_search_query {
                                        let full = highlight_matches(
                                            &cached.text,
                                            query,
                                            *current_line,
                                            block_start,
                                            &p,
                                        );
                                        Text::from(full.lines[start..end].to_vec())
                                    } else {
                                        Text::from(cached.text.lines[start..end].to_vec())
                                    };
                                // Apply highlight(s) when the viewer has focus.
                                // In visual mode every line in the selection range is
                                // highlighted; in normal mode only the cursor row.
                                let block_end = block_start + block_height;
                                if focused {
                                    apply_block_highlight(
                                        &mut visible_text.lines,
                                        visual_mode,
                                        cursor_line,
                                        block_start,
                                        block_end,
                                        start,
                                        p.selection_bg,
                                    );
                                }
                                text_draws.push(TextDraw {
                                    y: rect_y,
                                    height: draw_height,
                                    text: visible_text,
                                    first_line_number: block_start + clip_start + 1,
                                });
                            }
                        }
                    }
                }
            }

            block_start = block_end;
            if block_start >= lookahead_end {
                break;
            }
        }
    }

    // Queue any mermaid diagrams in the lookahead window that haven't been
    // rendered yet. This is the only site that calls ensure_queued — rendering
    // is fully lazy and driven by viewport proximity.
    if let Some(tx) = &app.action_tx {
        let in_tmux = std::env::var("TMUX").is_ok();
        let tx: tokio::sync::mpsc::UnboundedSender<Action> = Clone::clone(tx);
        let bg_rgb = match p.background {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => (0, 0, 0),
        };
        let render_cfg = MermaidRenderConfig {
            picker: app.picker.as_ref(),
            action_tx: &tx,
            in_tmux,
            bg_rgb,
            mode: app.mermaid_mode,
            max_height: app.mermaid_max_height,
        };
        for (id, source) in mermaid_to_queue {
            app.mermaid_cache.ensure_queued(id, &source, &render_cfg);
        }
    }

    let total_doc_lines = app.tabs.active_tab().map_or(0, |t| t.view.total_lines);

    // Render text blocks.
    for td in text_draws {
        let rect = Rect {
            x: inner.x,
            y: td.y,
            width: inner.width,
            height: td.height,
        };
        if app.show_line_numbers {
            render_text_with_gutter(f, rect, td.text, td.first_line_number, total_doc_lines, &p);
        } else {
            let para = Paragraph::new(td.text).wrap(Wrap { trim: false });
            f.render_widget(para, rect);
        }
    }

    // Render mermaid blocks.
    for md in mermaid_draws {
        let rect = Rect {
            x: inner.x,
            y: md.y,
            width: inner.width,
            height: md.height,
        };
        let params = MermaidDrawParams {
            fully_visible: md.fully_visible,
            id: md.id,
            source: &md.source,
            focused,
            cursor_line,
            block_start: md.block_start,
            block_end: md.block_start + md.block_height,
            visual_mode: md.visual_mode,
        };
        draw_mermaid_block(f, app, rect, &p, &params);
    }
}
