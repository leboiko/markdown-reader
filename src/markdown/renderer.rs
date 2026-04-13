use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};
use std::cell::Cell;

use crate::markdown::{
    CellSpans, DocBlock, HeadingAnchor, LinkInfo, MermaidBlockId, TableBlock, TableBlockId,
    cell_display_width, cell_to_string, heading_to_anchor,
};
use crate::mermaid::DEFAULT_MERMAID_HEIGHT;
use crate::theme::Palette;

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
pub fn render_markdown(content: &str, palette: &Palette) -> Vec<DocBlock> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(content, opts);
    let renderer = MdRenderer::new(palette);
    renderer.render(parser)
}

// ── Internal renderer ────────────────────────────────────────────────────────

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
}

impl MdRenderer {
    fn new(palette: &Palette) -> Self {
        Self {
            lines: Vec::new(),
            blocks: Vec::new(),
            current_spans: Vec::new(),
            style_stack: vec![Style::default()],
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
    }

    fn push_blank_line(&mut self) {
        if self.in_table {
            return;
        }
        self.lines.push(Line::from(""));
    }

    /// Drain `self.lines` into a `DocBlock::Text` if there are any pending lines.
    ///
    /// Any links and heading anchors accumulated are moved into the block;
    /// their `line` fields are already relative to this block's start.
    fn flush_text_block(&mut self) {
        if !self.lines.is_empty() {
            let lines = std::mem::take(&mut self.lines);
            let links = std::mem::take(&mut self.pending_links);
            let heading_anchors = std::mem::take(&mut self.pending_heading_anchors);
            self.blocks.push(DocBlock::Text {
                text: Text::from(lines),
                links,
                heading_anchors,
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
            .map(|s| s.content.chars().count() as u16)
            .sum()
    }

    fn render(mut self, parser: Parser) -> Vec<DocBlock> {
        for event in parser {
            match event {
                Event::Start(tag) => self.start_tag(tag),
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
                    self.push_blank_line();
                }
                Event::TaskListMarker(checked) => {
                    let marker = if checked { "☑ " } else { "☐ " };
                    self.current_spans.push(Span::styled(
                        marker.to_string(),
                        Style::default().fg(self.task_marker),
                    ));
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

    fn start_tag(&mut self, tag: Tag) {
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
                self.flush_line();
            }
            Tag::TableHead => {
                self.table_header = true;
                self.table_row.clear();
            }
            Tag::TableRow => {
                self.table_row.clear();
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
                    line: self.lines.len() as u32,
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
                        line: self.lines.len() as u32,
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
                self.table_header = false;
            }
            TagEnd::TableRow => {
                if !self.table_header {
                    self.table_rows.push(self.table_row.clone());
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
            if self.code_block_content.last().is_some_and(|l| l.is_empty()) {
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
        });
        // Blank line after the diagram (will open a new Text block).
        self.push_blank_line();
    }

    fn render_code_block(&mut self) {
        let code_style = Style::default().fg(self.code_fg).bg(self.code_bg);
        let border_style = Style::default().fg(self.code_border);

        let max_width = self
            .code_block_content
            .iter()
            .map(|l| l.len())
            .max()
            .unwrap_or(0)
            .max(20);
        let inner_width = max_width + 1;

        self.lines.push(Line::from(Span::styled(
            format!("╭{}╮", "─".repeat(inner_width + 1)),
            border_style,
        )));

        for line in &self.code_block_content {
            self.lines.push(Line::from(Span::styled(
                format!("│ {:<inner_width$}│", line),
                code_style,
            )));
        }

        self.lines.push(Line::from(Span::styled(
            format!("╰{}╯", "─".repeat(inner_width + 1)),
            border_style,
        )));

        self.code_block_content.clear();
        self.push_blank_line();
    }

    fn emit_table_block(&mut self) {
        let headers = self.table_header_row.take().unwrap_or_default();
        let rows = std::mem::take(&mut self.table_rows);
        let alignments = std::mem::take(&mut self.table_alignments);

        let num_cols = headers
            .len()
            .max(rows.iter().map(|r| r.len()).max().unwrap_or(0));

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
        let rendered_height = (rows.len() as u32 + 3).max(3);

        self.flush_text_block();
        self.blocks.push(DocBlock::Table(TableBlock {
            id,
            headers,
            rows,
            alignments,
            natural_widths,
            rendered_height,
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
