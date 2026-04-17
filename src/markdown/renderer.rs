use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};
use std::cell::Cell;
use unicode_width::UnicodeWidthStr;

use crate::markdown::{
    CellSpans, DocBlock, HeadingAnchor, LinkInfo, MermaidBlockId, TableBlock, TableBlockId,
    cell_display_width, cell_to_string, heading_to_anchor, highlight::highlight_code,
};
use crate::mermaid::DEFAULT_MERMAID_HEIGHT;
use crate::theme::{Palette, Theme};

/// Render a markdown string into a sequence of [`DocBlock`] values.
///
/// Mermaid fenced code blocks produce [`DocBlock::Mermaid`] entries; all other
/// content is grouped into [`DocBlock::Text`] runs. Consecutive text lines are
/// merged so there is at most one `Text` block between two `Mermaid` blocks.
///
/// `DocBlock::Text` blocks carry embedded [`LinkInfo`] and [`HeadingAnchor`]
/// slices whose `line` fields are relative to the block's start. Callers
/// convert them to absolute display lines by adding the block's cumulative
/// offset (see `MarkdownViewState::load`).
///
/// Each rendered logical line also carries a 0-indexed source line derived from
/// pulldown-cmark's byte-offset spans, enabling the viewer cursor to map back
/// to the exact source line when entering edit mode.
///
/// # Arguments
///
/// * `content` – raw markdown source.
/// * `palette` – color palette for the active UI theme.
/// * `theme` – the active UI theme; used to select the matching syntect
///   highlighting theme for fenced code blocks.
pub fn render_markdown(content: &str, palette: &Palette, theme: Theme) -> Vec<DocBlock> {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH;
    let parser = Parser::new_ext(content, opts);
    let renderer = MdRenderer::new(palette, theme);
    renderer.render(content, parser)
}

/// Pre-compute line-start byte offsets for `content`.
///
/// `line_boundaries[i]` is the byte offset where line `i` starts (0-indexed).
/// There is always at least one entry: `line_boundaries[0] == 0`.
fn build_line_boundaries(content: &str) -> Vec<usize> {
    let mut boundaries = vec![0];
    for (i, b) in content.as_bytes().iter().enumerate() {
        if *b == b'\n' {
            boundaries.push(i + 1);
        }
    }
    boundaries
}

/// Given a byte offset into the source, return the 0-indexed source line.
///
/// Uses a binary search into the pre-computed `boundaries` slice.
fn byte_offset_to_line(offset: usize, boundaries: &[usize]) -> u32 {
    match boundaries.binary_search(&offset) {
        // Exact match: the offset is itself the start of a line.
        Ok(i) => crate::cast::u32_sat(i),
        // No exact match: `i` is the insertion point — the line that started
        // before `offset` is at index `i - 1`.
        Err(i) => crate::cast::u32_sat(i.saturating_sub(1)),
    }
}

// ── Internal renderer ────────────────────────────────────────────────────────

#[allow(clippy::struct_excessive_bools)]
struct MdRenderer {
    /// Accumulates lines for the current `Text` block.
    lines: Vec<Line<'static>>,
    /// Completed blocks emitted so far.
    blocks: Vec<DocBlock>,
    current_spans: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    list_depth: usize,
    list_counters: Vec<Option<u64>>,
    in_code_block: bool,
    /// `Some(lang)` when inside a fenced block — `None` for indented blocks.
    code_block_lang: Option<String>,
    code_block_content: Vec<String>,
    in_heading: bool,
    heading_level: u8,
    in_blockquote: bool,
    in_table: bool,
    table_alignments: Vec<pulldown_cmark::Alignment>,
    table_row: Vec<CellSpans>,
    table_rows: Vec<Vec<CellSpans>>,
    table_header_row: Option<Vec<CellSpans>>,
    table_header: bool,
    /// URL of the link currently being rendered; set on `Start(Link)`, cleared
    /// on `TagEnd::Link` after recording the span range.
    current_link_url: Option<String>,
    /// Byte-column at which the current link's text begins in `current_spans`.
    /// Measured as the sum of span content lengths before the link started.
    link_col_start: u16,
    /// Links collected within the current pending `Text` block (block-relative).
    pending_links: Vec<LinkInfo>,
    /// Accumulated text of the heading currently being rendered.
    heading_text: String,
    /// Heading anchors accumulated for the current pending `Text` block.
    pending_heading_anchors: Vec<HeadingAnchor>,
    h1: Color,
    h2: Color,
    h3: Color,
    heading_other: Color,
    inline_code: Color,
    code_fg: Color,
    code_bg: Color,
    code_border: Color,
    link: Color,
    list_marker: Color,
    task_marker: Color,
    block_quote_fg: Color,
    block_quote_border: Color,
    dim: Color,
    /// Syntect theme name corresponding to the active UI theme. Used to
    /// resolve the correct token colors when highlighting fenced code blocks.
    syntax_theme_name: &'static str,

    // ── Source-line tracking ─────────────────────────────────────────────────
    /// Start byte offset of each source line: `line_boundaries[i]` is the byte
    /// offset where line `i` begins. Built once from `content` at render start.
    line_boundaries: Vec<usize>,
    /// 0-indexed source line of the most-recently processed event.
    /// Updated before dispatching each `(event, span)` pair.
    current_source_line: u32,
    /// Parallel to `self.lines` — one entry per rendered logical line.
    /// Invariant: `current_source_lines.len() == lines.len()` after every
    /// `flush_line` / `push_blank_line` call.
    current_source_lines: Vec<u32>,
    /// Byte offset of the opening fence of the current code block.
    /// Set on `Start(Tag::CodeBlock)` from `span.start`.
    code_block_fence_offset: Option<usize>,
    /// 0-indexed source line where the current code block's opening fence sits.
    code_block_start_line: u32,
    /// 0-indexed source line where the current table's opening row sits.
    table_start_line: u32,
    /// Source line of the table row currently being accumulated.
    /// Captured from `Start(Tag::TableRow)`'s `span.start`.
    current_table_row_source_line: u32,
    /// Source lines for every logical row in the current table.
    /// Index 0 is the header row; indices `1..=table_rows.len()` are body rows.
    /// Flushed into `TableBlock::row_source_lines` in `emit_table_block`.
    table_row_source_lines: Vec<u32>,
}

impl MdRenderer {
    fn new(palette: &Palette, theme: Theme) -> Self {
        Self {
            lines: Vec::new(),
            blocks: Vec::new(),
            current_spans: Vec::new(),
            style_stack: vec![Style::default().fg(palette.foreground)],
            list_depth: 0,
            list_counters: Vec::new(),
            in_code_block: false,
            code_block_lang: None,
            code_block_content: Vec::new(),
            in_heading: false,
            heading_level: 0,
            in_blockquote: false,
            in_table: false,
            table_alignments: Vec::new(),
            table_row: Vec::new(),
            table_rows: Vec::new(),
            table_header_row: None,
            table_header: false,
            current_link_url: None,
            link_col_start: 0,
            pending_links: Vec::new(),
            heading_text: String::new(),
            pending_heading_anchors: Vec::new(),
            h1: palette.h1,
            h2: palette.h2,
            h3: palette.h3,
            heading_other: palette.heading_other,
            inline_code: palette.inline_code,
            code_fg: palette.code_fg,
            code_bg: palette.code_bg,
            code_border: palette.code_border,
            link: palette.link,
            list_marker: palette.list_marker,
            task_marker: palette.task_marker,
            block_quote_fg: palette.block_quote_fg,
            block_quote_border: palette.block_quote_border,
            dim: palette.dim,
            syntax_theme_name: theme.syntax_theme_name(),
            line_boundaries: Vec::new(),
            current_source_line: 0,
            current_source_lines: Vec::new(),
            code_block_fence_offset: None,
            code_block_start_line: 0,
            table_start_line: 0,
            current_table_row_source_line: 0,
            table_row_source_lines: Vec::new(),
        }
    }

    fn current_style(&self) -> Style {
        self.style_stack.last().copied().unwrap_or_default()
    }

    fn push_style(&mut self, modifier: Style) {
        let base = self.current_style();
        self.style_stack.push(base.patch(modifier));
    }

    fn pop_style(&mut self) {
        if self.style_stack.len() > 1 {
            self.style_stack.pop();
        }
    }

    fn flush_line(&mut self) {
        if self.in_table {
            return;
        }
        let spans = std::mem::take(&mut self.current_spans);
        if self.in_blockquote && !self.in_code_block {
            let mut bq_spans = vec![Span::styled(
                "│ ".to_string(),
                Style::default().fg(self.block_quote_border),
            )];
            bq_spans.extend(spans);
            self.lines.push(Line::from(bq_spans));
        } else {
            self.lines.push(Line::from(spans));
        }
        // Maintain the parallel source_lines invariant.
        self.current_source_lines.push(self.current_source_line);
    }

    fn push_blank_line(&mut self) {
        if self.in_table {
            return;
        }
        self.lines.push(Line::from(""));
        // Blank lines inherit the source line of the surrounding context.
        self.current_source_lines.push(self.current_source_line);
    }

    /// Drain `self.lines` into a `DocBlock::Text` if there are any pending lines.
    ///
    /// Any links and heading anchors accumulated are moved into the block;
    /// their `line` fields are already relative to this block's start.
    ///
    /// Invariant: `source_lines.len() == text.lines.len()` is enforced by a
    /// debug assertion before pushing the block.
    #[allow(clippy::similar_names)]
    fn flush_text_block(&mut self) {
        if self.lines.is_empty() {
            // Drop orphaned source_lines that accumulated without any matching
            // rendered line (can happen around pure-table sections).
            self.current_source_lines.clear();
        } else {
            let lines = std::mem::take(&mut self.lines);
            let source_lines = std::mem::take(&mut self.current_source_lines);
            let links = std::mem::take(&mut self.pending_links);
            let heading_anchors = std::mem::take(&mut self.pending_heading_anchors);
            // In debug builds, catch any mismatch between rendered lines and
            // their source-line annotations immediately.
            debug_assert_eq!(
                lines.len(),
                source_lines.len(),
                "source_lines length {} != lines length {}",
                source_lines.len(),
                lines.len(),
            );
            self.blocks.push(DocBlock::Text {
                text: Text::from(lines),
                links,
                heading_anchors,
                source_lines,
            });
        }
    }

    /// Sum of the display widths of all spans currently in `current_spans`.
    ///
    /// Used to compute `col_start` / `col_end` for link hit-testing. We use
    /// char count rather than byte count because ratatui column positions are
    /// character-based; for ASCII-only link text this is identical to byte count.
    fn current_col_width(&self) -> u16 {
        self.current_spans
            .iter()
            .map(|s| crate::cast::u16_sat(s.content.chars().count()))
            .sum()
    }

    /// Drive the render loop.
    ///
    /// `content` is the raw markdown string; it is used to build the
    /// `line_boundaries` table for byte-offset-to-line translation.
    /// `parser` is the pulldown-cmark parser constructed from the same string.
    #[allow(clippy::too_many_lines)]
    fn render(mut self, content: &str, parser: Parser) -> Vec<DocBlock> {
        // Build the line boundary table once.  O(n) in the source length.
        self.line_boundaries = build_line_boundaries(content);

        // Use the offset iterator so every event carries a byte-range into
        // the original source, letting us map events back to source lines.
        for (event, span) in parser.into_offset_iter() {
            // Stamp the current source line from the event's start offset
            // before dispatching.  All `lines.push` paths below inherit this.
            self.current_source_line = byte_offset_to_line(span.start, &self.line_boundaries);

            match event {
                Event::Start(tag) => self.start_tag(tag, &span),
                Event::End(tag) => self.end_tag(tag),
                Event::Text(text) => self.handle_text(&text),
                Event::Code(code) => {
                    let style = self
                        .current_style()
                        .fg(self.inline_code)
                        .add_modifier(Modifier::BOLD);
                    self.current_spans
                        .push(Span::styled(format!("`{code}`"), style));
                }
                Event::SoftBreak => {
                    self.current_spans
                        .push(Span::styled(" ".to_string(), self.current_style()));
                }
                Event::HardBreak => {
                    self.flush_line();
                }
                Event::Rule => {
                    self.flush_line();
                    self.lines.push(Line::from(Span::styled(
                        "─".repeat(60),
                        Style::default().fg(self.dim),
                    )));
                    // Rule line and the blank after it both map to current source line.
                    self.current_source_lines.push(self.current_source_line);
                    self.push_blank_line();
                }
                Event::TaskListMarker(checked) => {
                    let marker = if checked { "☑ " } else { "☐ " };
                    self.current_spans.push(Span::styled(
                        marker.to_string(),
                        Style::default().fg(self.task_marker),
                    ));
                }
                Event::InlineMath(math) => {
                    // Convert LaTeX to Unicode approximation and render
                    // inline, styled like inline code but in italic so
                    // readers can tell math from code at a glance.
                    let rendered = crate::markdown::math::latex_to_unicode(&math);
                    let style = self
                        .current_style()
                        .fg(self.inline_code)
                        .add_modifier(Modifier::ITALIC);
                    self.current_spans.push(Span::styled(rendered, style));
                }
                Event::DisplayMath(math) => {
                    // Convert LaTeX to Unicode and render as a bordered
                    // block labelled "math", mirroring the code-block
                    // frame.
                    let rendered = crate::markdown::math::latex_to_unicode(&math);
                    self.flush_line();
                    let border_style = Style::default().fg(self.code_border);
                    let math_style = Style::default()
                        .fg(self.code_fg)
                        .bg(self.code_bg)
                        .add_modifier(Modifier::ITALIC);
                    let math_lines: Vec<&str> = rendered.lines().collect();
                    let max_width = math_lines
                        .iter()
                        .map(|l| UnicodeWidthStr::width(*l))
                        .max()
                        .unwrap_or(0)
                        .max(20);
                    let inner_width = max_width + 1;
                    let label = " math ";

                    self.push_blank_line();
                    // Top border with "math" label.
                    self.lines.push(Line::from(vec![
                        Span::styled("╭".to_string(), border_style),
                        Span::styled(
                            label.to_string(),
                            Style::default()
                                .fg(self.inline_code)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(
                                "{}╮",
                                "─".repeat(inner_width + 1 - label.len().min(inner_width))
                            ),
                            border_style,
                        ),
                    ]));
                    self.current_source_lines.push(self.current_source_line);
                    // Content lines.
                    for line in &math_lines {
                        self.lines.push(Line::from(vec![
                            Span::styled(
                                "│ ".to_string(),
                                Style::default().fg(self.code_border).bg(self.code_bg),
                            ),
                            Span::styled(format!("{line:<inner_width$}"), math_style),
                            Span::styled(
                                "│".to_string(),
                                Style::default().fg(self.code_border).bg(self.code_bg),
                            ),
                        ]));
                        self.current_source_lines.push(self.current_source_line);
                    }
                    // Bottom border.
                    self.lines.push(Line::from(Span::styled(
                        format!("╰{}╯", "─".repeat(inner_width + 1)),
                        border_style,
                    )));
                    self.current_source_lines.push(self.current_source_line);
                    self.push_blank_line();
                }
                _ => {}
            }
        }
        if !self.current_spans.is_empty() {
            self.flush_line();
        }
        self.flush_text_block();
        self.blocks
    }

    #[allow(clippy::too_many_lines)]
    #[allow(clippy::match_same_arms)]
    fn start_tag(&mut self, tag: Tag, span: &Range<usize>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.in_heading = true;
                self.heading_level = level as u8;
                self.heading_text.clear();
                let color = match level {
                    pulldown_cmark::HeadingLevel::H1 => self.h1,
                    pulldown_cmark::HeadingLevel::H2 => self.h2,
                    pulldown_cmark::HeadingLevel::H3 => self.h3,
                    _ => self.heading_other,
                };
                let mut style = Style::default().fg(color).add_modifier(Modifier::BOLD);
                if level == pulldown_cmark::HeadingLevel::H1 {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                self.push_style(style);
                let prefix = match level {
                    pulldown_cmark::HeadingLevel::H1 => "█ ",
                    pulldown_cmark::HeadingLevel::H2 => "▌ ",
                    pulldown_cmark::HeadingLevel::H3 => "▎ ",
                    _ => "  ",
                };
                self.current_spans
                    .push(Span::styled(prefix.to_string(), self.current_style()));
            }
            Tag::Paragraph => {}
            Tag::BlockQuote(_) => {
                self.in_blockquote = true;
                self.push_style(Style::default().fg(self.block_quote_fg));
            }
            Tag::CodeBlock(kind) => {
                self.in_code_block = true;
                self.code_block_lang = match &kind {
                    CodeBlockKind::Fenced(lang) => {
                        let s = lang.trim().to_lowercase();
                        if s.is_empty() { None } else { Some(s) }
                    }
                    CodeBlockKind::Indented => None,
                };
                self.code_block_content.clear();
                // Record the fence's byte offset and resolve its source line.
                self.code_block_fence_offset = Some(span.start);
                self.code_block_start_line = byte_offset_to_line(span.start, &self.line_boundaries);
                self.flush_line();
            }
            Tag::List(start) => {
                self.list_depth += 1;
                self.list_counters.push(start);
            }
            Tag::Item => {
                let indent = "  ".repeat(self.list_depth.saturating_sub(1));
                let bullet = if let Some(counter) = self.list_counters.last_mut() {
                    if let Some(n) = counter {
                        let bullet = format!("{indent}{n}. ");
                        *n += 1;
                        bullet
                    } else {
                        let marker = match self.list_depth {
                            1 => "•",
                            2 => "◦",
                            _ => "▪",
                        };
                        format!("{indent}{marker} ")
                    }
                } else {
                    format!("{indent}• ")
                };
                self.current_spans
                    .push(Span::styled(bullet, Style::default().fg(self.list_marker)));
            }
            Tag::Emphasis => {
                self.push_style(Style::default().add_modifier(Modifier::ITALIC));
            }
            Tag::Strong => {
                self.push_style(Style::default().add_modifier(Modifier::BOLD));
            }
            Tag::Strikethrough => {
                self.push_style(Style::default().add_modifier(Modifier::CROSSED_OUT));
            }
            Tag::Link { dest_url, .. } => {
                self.link_col_start = self.current_col_width();
                self.current_link_url = Some(dest_url.into_string());
                self.push_style(
                    Style::default()
                        .fg(self.link)
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            Tag::Table(alignments) => {
                self.in_table = true;
                self.table_alignments = alignments;
                self.table_rows.clear();
                self.table_header_row = None;
                self.table_start_line = byte_offset_to_line(span.start, &self.line_boundaries);
                self.flush_line();
            }
            Tag::TableHead => {
                self.table_header = true;
                self.table_row.clear();
                // pulldown-cmark does NOT emit `Tag::TableRow` for a table's
                // header — the header's cells live directly inside
                // `TableHead`. Capture the source line here so `TagEnd::TableHead`
                // has the right value to push onto `table_row_source_lines`.
                self.current_table_row_source_line =
                    byte_offset_to_line(span.start, &self.line_boundaries);
            }
            Tag::TableRow => {
                self.table_row.clear();
                // Capture the source line for body rows so we can map the
                // cursor back to the exact markdown row when entering edit
                // mode or jumping from search.
                self.current_table_row_source_line =
                    byte_offset_to_line(span.start, &self.line_boundaries);
            }
            Tag::TableCell => {}
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.pop_style();
                // Record the anchor before flushing; `self.lines.len()` is the
                // 0-based index of the line we are about to push.
                let anchor = heading_to_anchor(&self.heading_text);
                self.pending_heading_anchors.push(HeadingAnchor {
                    anchor,
                    line: crate::cast::u32_sat(self.lines.len()),
                });
                self.flush_line();
                self.push_blank_line();
                self.in_heading = false;
                self.heading_text.clear();
            }
            TagEnd::Paragraph => {
                self.flush_line();
                self.push_blank_line();
            }
            TagEnd::BlockQuote(_) => {
                self.in_blockquote = false;
                self.pop_style();
                self.push_blank_line();
            }
            TagEnd::CodeBlock => {
                if self.code_block_lang.as_deref() == Some("mermaid") {
                    self.emit_mermaid_block();
                } else {
                    self.render_code_block();
                }
                self.in_code_block = false;
                self.code_block_lang = None;
            }
            TagEnd::List(_) => {
                self.list_depth = self.list_depth.saturating_sub(1);
                self.list_counters.pop();
                if self.list_depth == 0 {
                    self.push_blank_line();
                }
            }
            TagEnd::Item => {
                self.flush_line();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.pop_style();
            }
            TagEnd::Link => {
                self.pop_style();
                if let Some(url) = self.current_link_url.take() {
                    let col_end = self.current_col_width();
                    // Collect the visible text from spans added since link start.
                    let text: String = self
                        .current_spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>()
                        .chars()
                        .skip(self.link_col_start as usize)
                        .collect();
                    self.pending_links.push(LinkInfo {
                        line: crate::cast::u32_sat(self.lines.len()),
                        col_start: self.link_col_start,
                        col_end,
                        url,
                        text,
                    });
                }
            }
            TagEnd::Table => {
                self.emit_table_block();
                self.in_table = false;
            }
            TagEnd::TableHead => {
                self.table_header_row = Some(self.table_row.clone());
                // Record the header row's source line before clearing the flag.
                self.table_row_source_lines
                    .push(self.current_table_row_source_line);
                self.table_header = false;
            }
            TagEnd::TableRow => {
                if !self.table_header {
                    self.table_rows.push(self.table_row.clone());
                    // Record the body row's source line (header is already recorded
                    // in TagEnd::TableHead above).
                    self.table_row_source_lines
                        .push(self.current_table_row_source_line);
                }
            }
            TagEnd::TableCell => {
                let cell_spans: CellSpans = self.current_spans.drain(..).collect();
                self.table_row.push(cell_spans);
            }
            _ => {}
        }
    }

    fn handle_text(&mut self, text: &str) {
        if self.in_code_block {
            for line in text.split('\n') {
                self.code_block_content.push(line.to_string());
            }
            if self.code_block_content.last().is_some_and(String::is_empty) {
                self.code_block_content.pop();
            }
        } else {
            if self.in_heading {
                self.heading_text.push_str(text);
            }
            self.current_spans
                .push(Span::styled(text.to_string(), self.current_style()));
        }
    }

    /// Flush accumulated code lines as a `DocBlock::Mermaid`, preceded by any
    /// pending text lines as a `DocBlock::Text`.
    fn emit_mermaid_block(&mut self) {
        self.flush_text_block();

        let source = self.code_block_content.join("\n");
        self.code_block_content.clear();

        let id = MermaidBlockId(hash_str(&source));
        self.blocks.push(DocBlock::Mermaid {
            id,
            source,
            cell_height: Cell::new(DEFAULT_MERMAID_HEIGHT),
            // The fence line is the canonical source position for the block.
            source_line: self.code_block_start_line,
        });
        // Blank line after the diagram (will open a new Text block).
        self.push_blank_line();
    }

    fn render_code_block(&mut self) {
        let border_style = Style::default().fg(self.code_border);

        // Capture the fence's source line before any mutable borrows below.
        let code_start_line = self.code_block_start_line;

        // Widths are measured in display cells, not bytes, so that lines
        // containing multi-byte characters (em dashes, CJK, emoji, …) align
        // with the box frame drawn around them.
        let max_width = self
            .code_block_content
            .iter()
            .map(|l| UnicodeWidthStr::width(l.as_str()))
            .max()
            .unwrap_or(0)
            .max(20);
        let inner_width = max_width + 1;

        // Join lines with newlines so syntect sees a complete source text.
        // highlight_code returns one TokenLine per source line.
        let source = self.code_block_content.join("\n");
        let token_lines = highlight_code(
            &source,
            self.code_block_lang.as_deref(),
            self.syntax_theme_name,
            self.code_fg,
            self.code_bg,
        );

        // Blank line before the box — maps to whatever was current before the block.
        self.push_blank_line();

        // Top border maps to the fence line.
        self.lines.push(Line::from(Span::styled(
            format!("╭{}╮", "─".repeat(inner_width + 1)),
            border_style,
        )));
        self.current_source_lines.push(code_start_line);

        // One rendered line per source line.
        // Layout per line (matching the original single-span format):
        //   "│ " <highlighted tokens padded to inner_width> "│"
        //
        // The tokens together have `line.len()` visible bytes.  We pad the gap
        // between the last token and the right border with spaces using the
        // same background color, so the box aligns regardless of token count.
        for (i, (src_line, token_line)) in self
            .code_block_content
            .iter()
            .zip(token_lines.iter())
            .enumerate()
        {
            let line_width = UnicodeWidthStr::width(src_line.as_str());
            let pad_len = inner_width.saturating_sub(line_width);

            let mut spans: Vec<Span<'static>> = Vec::with_capacity(token_line.len() + 3);

            // Left border + leading space (border color for `│`, code_bg for
            // the space so it blends with the token background).
            spans.push(Span::styled(
                "│ ".to_string(),
                Style::default().fg(self.code_border).bg(self.code_bg),
            ));

            // Syntax-highlighted token spans.
            for (text, style) in token_line {
                spans.push(Span::styled(text.clone(), *style));
            }

            // Padding to align right border.
            if pad_len > 0 {
                spans.push(Span::styled(
                    " ".repeat(pad_len),
                    Style::default().bg(self.code_bg),
                ));
            }

            // Right border.
            spans.push(Span::styled(
                "│".to_string(),
                Style::default().fg(self.code_border).bg(self.code_bg),
            ));

            self.lines.push(Line::from(spans));
            // Content line i (0-indexed) lives one source line after the fence.
            self.current_source_lines
                .push(code_start_line + 1 + crate::cast::u32_sat(i));
        }

        // Bottom border maps to the line after the last content line.
        let bottom_source_line = code_start_line + 1 + crate::cast::u32_sat(self.code_block_content.len());
        self.lines.push(Line::from(Span::styled(
            format!("╰{}╯", "─".repeat(inner_width + 1)),
            border_style,
        )));
        self.current_source_lines.push(bottom_source_line);

        self.code_block_content.clear();
        self.push_blank_line();
    }

    fn emit_table_block(&mut self) {
        let headers = self.table_header_row.take().unwrap_or_default();
        let rows = std::mem::take(&mut self.table_rows);
        let row_source_lines = std::mem::take(&mut self.table_row_source_lines);
        let alignments = std::mem::take(&mut self.table_alignments);

        let num_cols = headers
            .len()
            .max(rows.iter().map(Vec::len).max().unwrap_or(0));

        if num_cols == 0 {
            return;
        }

        let mut natural_widths = vec![0usize; num_cols];
        for (i, cell) in headers.iter().enumerate() {
            natural_widths[i] = natural_widths[i].max(cell_display_width(cell));
        }
        for row in &rows {
            for (i, cell) in row.iter().enumerate() {
                if i < num_cols {
                    natural_widths[i] = natural_widths[i].max(cell_display_width(cell));
                }
            }
        }
        // Minimum column width of 1 so borders are always valid.
        for w in &mut natural_widths {
            *w = (*w).max(1);
        }

        // Hash the flattened text content for a stable, content-derived id.
        let mut content_bytes = Vec::new();
        for h in &headers {
            content_bytes.extend_from_slice(cell_to_string(h).as_bytes());
        }
        for row in &rows {
            for cell in row {
                content_bytes.extend_from_slice(cell_to_string(cell).as_bytes());
            }
        }
        let id = TableBlockId(hash_bytes(&content_bytes));

        // Pessimistic height: top + header + separator + rows + bottom.
        // layout_table will refine this on first draw; this seeds the scrolling math.
        let rendered_height = (crate::cast::u32_sat(rows.len()) + 3).max(3);

        self.flush_text_block();
        self.blocks.push(DocBlock::Table(TableBlock {
            id,
            headers,
            rows,
            alignments,
            natural_widths,
            rendered_height,
            source_line: self.table_start_line,
            row_source_lines,
        }));
        self.push_blank_line();
    }
}

fn hash_str(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

fn hash_bytes(b: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    b.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    fn default_palette() -> Palette {
        Palette::from_theme(Theme::Default)
    }

    /// Helper: render a fenced code block and extract all rendered lines
    /// (including borders) from the first Text block.
    fn render_code_block_lines(lang: &str, code: &str) -> Vec<Line<'static>> {
        let md = format!("```{lang}\n{code}\n```\n");
        let blocks = render_markdown(&md, &default_palette(), Theme::Default);
        match blocks
            .into_iter()
            .find(|b| matches!(b, DocBlock::Text { .. }))
        {
            Some(DocBlock::Text { text, .. }) => text.lines,
            _ => panic!("expected a Text block"),
        }
    }

    /// A Rust fenced code block must produce content lines that contain more
    /// than one span with distinct foreground colors, confirming that
    /// highlighting was applied.
    #[test]
    fn rust_code_block_spans_have_distinct_colors() {
        let lines = render_code_block_lines("rust", "let x: i32 = 42;");

        // Skip blank lines and border lines; find the first content line.
        let content_line = lines.iter().find(|l| {
            let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
            text.starts_with("│ ") && !text.starts_with("│ ─") && text.contains("let")
        });

        let content_line = content_line.expect("expected a content line containing 'let'");
        assert!(
            content_line.spans.len() > 2,
            "expected more than 2 spans on a highlighted Rust line, got {}",
            content_line.spans.len(),
        );

        let colors: std::collections::HashSet<ratatui::style::Color> = content_line
            .spans
            .iter()
            .filter_map(|s| s.style.fg)
            // Exclude border spans (code_border color).
            .filter(|c| *c != default_palette().code_border)
            .collect();
        assert!(
            colors.len() > 1,
            "expected multiple distinct token colors on a Rust line, got {colors:?}",
        );
    }

    /// A fenced block with no language tag must produce content lines that have
    /// a single foreground color (plain-text fallback).
    #[test]
    fn no_language_code_block_is_single_color() {
        let lines = render_code_block_lines("", "hello world\nsome code");

        let content_lines: Vec<&Line<'static>> = lines
            .iter()
            .filter(|l| {
                let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                // Content lines start with "│ " but are not box borders.
                text.starts_with("│ ") && !text.starts_with("╭") && !text.starts_with("╰")
            })
            .collect();

        assert!(!content_lines.is_empty(), "expected content lines");

        for line in content_lines {
            // Collect token-span colors (excluding border characters).
            let colors: std::collections::HashSet<ratatui::style::Color> = line
                .spans
                .iter()
                .filter(|s| !s.content.contains('│'))
                .filter_map(|s| s.style.fg)
                .collect();
            assert!(
                colors.len() <= 1,
                "expected at most one token color for plain-text fallback, got {colors:?}",
            );
        }
    }

    /// An unknown language tag must not panic and must produce output.
    #[test]
    fn unknown_language_does_not_panic() {
        let lines = render_code_block_lines("notalang", "some code here");
        assert!(
            !lines.is_empty(),
            "expected rendered lines for unknown language",
        );
    }

    /// The right border `│` must be at the same visual column position as it
    /// would be in the old single-span rendering, for a known ASCII input.
    ///
    /// With `max_width = max(len("hello world"), 20) = 20` and
    /// `inner_width = 21`, the full line is:
    ///   "│ " + 21 chars padded + "│"  = 2 + 21 + 1 = 24 chars.
    #[test]
    fn right_border_aligns_at_expected_column() {
        let lines = render_code_block_lines("", "hello world");

        // Find the first content line (not blank, not top/bottom border).
        let content_line = lines.iter().find(|l| {
            let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
            text.starts_with("│ ") && !text.starts_with("╭") && !text.starts_with("╰")
        });

        let content_line = content_line.expect("expected a content line");

        // Concatenate all span text to get the full rendered line.
        let full_text: String = content_line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();

        // inner_width = max(11, 20) + 1 = 21; full line = "│ " + 21 chars + "│"
        let expected_len = 2 + 21 + 1; // = 24
        assert_eq!(
            full_text.chars().count(),
            expected_len,
            "expected line length {expected_len}, got {} for line: {full_text:?}",
            full_text.chars().count(),
        );
        assert!(
            full_text.ends_with('│'),
            "line must end with right border '│': {full_text:?}",
        );
    }

    /// Multi-byte characters (em dash is 3 bytes / 1 display cell) must not
    /// shift the right border: every content line in a mixed-width block must
    /// have the same display width, measured in cells.
    #[test]
    fn right_border_aligns_with_multi_byte_chars() {
        use unicode_width::UnicodeWidthStr;

        // One ASCII line and one em-dash line; the ASCII line is longer in
        // cells so it determines `max_width`.
        let src = "hello world this is a long line\n    /// short — comment";
        let lines = render_code_block_lines("", src);

        let content_lines: Vec<&Line<'static>> = lines
            .iter()
            .filter(|l| {
                let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                text.starts_with("│ ") && !text.starts_with("╭") && !text.starts_with("╰")
            })
            .collect();

        assert!(
            content_lines.len() >= 2,
            "expected at least two content lines, got {}",
            content_lines.len(),
        );

        let widths: Vec<usize> = content_lines
            .iter()
            .map(|l| {
                let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                UnicodeWidthStr::width(text.as_str())
            })
            .collect();

        let first = widths[0];
        for (i, w) in widths.iter().enumerate() {
            assert_eq!(
                *w, first,
                "line {i} has display width {w}, expected {first} (right border misaligned)",
            );
        }
    }

    // ── Phase 1: source-line plumbing tests ──────────────────────────────────

    /// For every `DocBlock::Text`, `source_lines` must be the same length as
    /// `text.lines` — the invariant enforced by `flush_text_block`.
    #[test]
    fn source_lines_parallel_to_text_lines() {
        let md = "Line 1\nLine 2\n\nLine 4\n";
        let blocks = render_markdown(md, &default_palette(), Theme::Default);
        for block in &blocks {
            if let DocBlock::Text {
                text, source_lines, ..
            } = block
            {
                assert_eq!(
                    text.lines.len(),
                    source_lines.len(),
                    "source_lines length {} != text.lines length {}",
                    source_lines.len(),
                    text.lines.len(),
                );
            }
        }
    }

    /// A heading on line 0 should map to source line 0.  A paragraph starting
    /// on line 2 (after blank line) should map to source line 2.
    #[test]
    fn source_lines_map_paragraph_correctly() {
        let md = "# Title\n\nParagraph text\n";
        let blocks = render_markdown(md, &default_palette(), Theme::Default);
        let text_block = blocks
            .iter()
            .find(|b| matches!(b, DocBlock::Text { .. }))
            .expect("expected a Text block");
        let DocBlock::Text { source_lines, .. } = text_block else {
            panic!("expected Text block");
        };
        // The heading is the very first rendered line — source line 0.
        assert_eq!(source_lines[0], 0, "heading should map to source line 0");
        // Find the index of the rendered line containing "Paragraph".
        let DocBlock::Text { text, .. } = text_block else {
            panic!()
        };
        let para_idx = text
            .lines
            .iter()
            .position(|l| l.spans.iter().any(|s| s.content.contains("Paragraph")))
            .expect("expected a 'Paragraph' line");
        // Paragraph starts after "# Title\n\n", i.e., on source line 2.
        assert_eq!(
            source_lines[para_idx], 2,
            "paragraph should map to source line 2"
        );
    }

    /// The top border of a code block maps to the fence line (0), each content
    /// line maps to the fence line + 1 + its 0-based index, and the bottom
    /// border maps to the line after the last content line.
    #[test]
    fn code_block_borders_map_to_fence() {
        // Source layout:
        //   line 0: ```rust
        //   line 1: let x = 1;
        //   line 2: let y = 2;
        //   line 3: ```
        let md = "```rust\nlet x = 1;\nlet y = 2;\n```\n";
        let blocks = render_markdown(md, &default_palette(), Theme::Default);
        let text_block = blocks
            .iter()
            .find(|b| matches!(b, DocBlock::Text { .. }))
            .expect("expected a Text block");
        let DocBlock::Text {
            text, source_lines, ..
        } = text_block
        else {
            panic!("expected Text block");
        };

        // Find the top border line (starts with '╭').
        let top_idx = text
            .lines
            .iter()
            .position(|l| l.spans.iter().any(|s| s.content.starts_with('╭')))
            .expect("top border not found");
        assert_eq!(
            source_lines[top_idx], 0,
            "top border should map to fence line (0)"
        );

        // Content lines immediately follow; their source lines are 1 and 2.
        assert_eq!(
            source_lines[top_idx + 1],
            1,
            "first content line should map to source line 1"
        );
        assert_eq!(
            source_lines[top_idx + 2],
            2,
            "second content line should map to source line 2"
        );

        // Bottom border.
        let bot_idx = text
            .lines
            .iter()
            .position(|l| l.spans.iter().any(|s| s.content.starts_with('╰')))
            .expect("bottom border not found");
        assert_eq!(
            source_lines[bot_idx], 3,
            "bottom border should map to source line 3"
        );
    }

    /// A table block's `source_line` should be 0 when the table starts at the
    /// beginning of the document.
    #[test]
    fn table_captures_start_line() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n";
        let blocks = render_markdown(md, &default_palette(), Theme::Default);
        let table = blocks
            .iter()
            .find(|b| matches!(b, DocBlock::Table(_)))
            .expect("expected a Table block");
        let DocBlock::Table(t) = table else { panic!() };
        assert_eq!(t.source_line, 0, "table source_line should be 0");
    }

    /// A mermaid block's `source_line` should be 0 when the fence starts at
    /// the beginning of the document.
    #[test]
    fn mermaid_captures_start_line() {
        let md = "```mermaid\ngraph LR\nA-->B\n```\n";
        let blocks = render_markdown(md, &default_palette(), Theme::Default);
        let mermaid = blocks
            .iter()
            .find(|b| matches!(b, DocBlock::Mermaid { .. }))
            .expect("expected a Mermaid block");
        let DocBlock::Mermaid { source_line, .. } = mermaid else {
            panic!()
        };
        assert_eq!(*source_line, 0, "mermaid source_line should be 0");
    }

    /// Text before a code block keeps its own source lines; the code block
    /// content lines report source lines relative to the fence opening.
    #[test]
    fn text_before_code_block() {
        // Source layout:
        //   line 0: Intro
        //   line 1: (blank)
        //   line 2: ```rust
        //   line 3: fn main() {}
        //   line 4: ```
        let md = "Intro\n\n```rust\nfn main() {}\n```\n";
        let blocks = render_markdown(md, &default_palette(), Theme::Default);

        // There should be exactly one Text block containing both the intro and
        // the rendered code box (they are in the same text run).
        let text_block = blocks
            .iter()
            .find(|b| matches!(b, DocBlock::Text { .. }))
            .expect("expected a Text block");
        let DocBlock::Text {
            text, source_lines, ..
        } = text_block
        else {
            panic!("expected Text block");
        };

        // The first rendered line is "Intro" — source line 0.
        let intro_idx = text
            .lines
            .iter()
            .position(|l| l.spans.iter().any(|s| s.content.contains("Intro")))
            .expect("intro line not found");
        assert_eq!(
            source_lines[intro_idx], 0,
            "intro should map to source line 0"
        );

        // Find the first content line inside the code box (not a border).
        // Content lines start with "│ " and contain the source code.
        let content_idx = text
            .lines
            .iter()
            .position(|l| {
                let joined: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                joined.contains("fn main") || joined.contains("fn")
            })
            .expect("code content line not found");
        // Content line 0 inside the box → source line 3 (fence=2, content=2+1=3).
        assert_eq!(
            source_lines[content_idx], 3,
            "first code content line should map to source line 3"
        );
    }

    // ── table row_source_lines ───────────────────────────────────────────────

    /// Rendering a 2-column table with a header and two body rows must produce
    /// `row_source_lines` of length 3 (header + 2 body rows) and correctly
    /// map each row to its markdown source line.
    ///
    /// Markdown input (0-indexed lines):
    ///   0: | A | B |
    ///   1: |---|---|
    ///   2: | 1 | 2 |
    ///   3: | 3 | 4 |
    #[test]
    fn table_captures_row_source_lines() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n";
        let p = default_palette();
        let blocks = render_markdown(md, &p, crate::theme::Theme::Default);
        let table = blocks
            .iter()
            .find_map(|b| {
                if let DocBlock::Table(t) = b {
                    Some(t)
                } else {
                    None
                }
            })
            .expect("expected a Table block");

        // Header is on source line 0; body rows on lines 2 and 3.
        // (line 1 is the `|---|---|` separator, which is not a data row.)
        assert_eq!(
            table.row_source_lines,
            vec![0, 2, 3],
            "row_source_lines mismatch: {:#?}",
            table.row_source_lines
        );
    }

    /// A header-only table (no body rows) must produce exactly one entry in
    /// `row_source_lines`.
    #[test]
    fn table_header_source_line_captured() {
        let md = "| A | B |\n|---|---|\n";
        let p = default_palette();
        let blocks = render_markdown(md, &p, crate::theme::Theme::Default);
        let table = blocks
            .iter()
            .find_map(|b| {
                if let DocBlock::Table(t) = b {
                    Some(t)
                } else {
                    None
                }
            })
            .expect("expected a Table block");

        assert_eq!(
            table.row_source_lines,
            vec![0],
            "header-only table must have exactly one entry"
        );
    }

    /// Regression test for a header-row tracking bug: when a table was
    /// preceded by other content, the header's source line was recorded as
    /// 0 instead of the header line's real source position. The root cause
    /// was that pulldown-cmark does NOT emit `Tag::TableRow` for the
    /// header — the header's cells live directly inside `Tag::TableHead` —
    /// so the header's `current_table_row_source_line` was never updated
    /// from its initial zero.
    #[test]
    fn table_header_source_line_not_anchored_to_zero_when_preceded_by_text() {
        // Source layout:
        //   0: # Title
        //   1: (blank)
        //   2: Some intro paragraph.
        //   3: (blank)
        //   4: | A | B |
        //   5: |---|---|
        //   6: | 1 | 2 |
        let md = "# Title\n\nSome intro paragraph.\n\n| A | B |\n|---|---|\n| 1 | 2 |\n";
        let p = default_palette();
        let blocks = render_markdown(md, &p, crate::theme::Theme::Default);
        let table = blocks
            .iter()
            .find_map(|b| {
                if let DocBlock::Table(t) = b {
                    Some(t)
                } else {
                    None
                }
            })
            .expect("expected a Table block");

        assert_eq!(
            table.row_source_lines,
            vec![4, 6],
            "header must be on source line 4 (not 0); body row on 6",
        );
    }

    // ── mermaid source_line_at precision ────────────────────────────────────

    /// `source_line_at` must map each cursor row inside a mermaid block to
    /// the corresponding source line (fence + 1 + `row_offset`), clamped to the
    /// last content line.
    ///
    /// Markdown input (0-indexed lines):
    ///   0: ```mermaid
    ///   1: graph LR
    ///   2: A-->B
    ///   3: C-->D
    ///   4: ```
    ///   5: (blank after fence)
    #[test]
    fn mermaid_source_line_precise_per_row() {
        use crate::markdown::source_line_at;
        use crate::mermaid::DEFAULT_MERMAID_HEIGHT;
        use std::cell::Cell;

        // Construct the block manually; the renderer collapses the content
        // into a single `source` string.
        let blocks = vec![DocBlock::Mermaid {
            id: crate::markdown::MermaidBlockId(0),
            source: "graph LR\nA-->B\nC-->D".to_string(), // 3 content lines
            cell_height: Cell::new(DEFAULT_MERMAID_HEIGHT),
            source_line: 0, // fence is on line 0
        }];

        // local == 0 → fence line
        assert_eq!(source_line_at(&blocks, 0), 0, "fence row");
        // local == 1 → first content line: fence + 1 + 0 = 1
        assert_eq!(source_line_at(&blocks, 1), 1, "content[0]");
        // local == 2 → second content line: fence + 1 + 1 = 2
        assert_eq!(source_line_at(&blocks, 2), 2, "content[1]");
        // local == 3 → third content line: fence + 1 + 2 = 3
        assert_eq!(source_line_at(&blocks, 3), 3, "content[2]");
        // local == 4 → clamped to last content (index 2): fence + 1 + 2 = 3
        assert_eq!(source_line_at(&blocks, 4), 3, "clamped past last content");
    }
}
