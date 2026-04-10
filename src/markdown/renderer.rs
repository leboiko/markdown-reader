use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};

use crate::theme::Palette;

/// Render a markdown string into a ratatui [`Text`] value ready for display.
///
/// All colors are sourced from `palette` so the output reflects the active theme.
pub fn render_markdown(content: &str, palette: &Palette) -> Text<'static> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(content, opts);
    let mut renderer = MdRenderer::new(palette);
    renderer.render(parser);
    Text::from(renderer.lines)
}

struct MdRenderer {
    lines: Vec<Line<'static>>,
    current_spans: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    list_depth: usize,
    list_counters: Vec<Option<u64>>,
    in_code_block: bool,
    code_block_content: Vec<String>,
    in_heading: bool,
    heading_level: u8,
    in_blockquote: bool,
    in_table: bool,
    table_alignments: Vec<pulldown_cmark::Alignment>,
    table_row: Vec<String>,
    table_rows: Vec<Vec<String>>,
    table_header_row: Option<Vec<String>>,
    table_header: bool,
    // Captured palette colors — stored as Color to avoid a lifetime on MdRenderer.
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
    table_header_color: Color,
    table_border: Color,
    dim: Color,
}

impl MdRenderer {
    fn new(palette: &Palette) -> Self {
        Self {
            lines: Vec::new(),
            current_spans: Vec::new(),
            style_stack: vec![Style::default()],
            list_depth: 0,
            list_counters: Vec::new(),
            in_code_block: false,
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
            table_header_color: palette.table_header,
            table_border: palette.table_border,
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

    fn render(&mut self, parser: Parser) {
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
    }

    fn start_tag(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { level, .. } => {
                self.in_heading = true;
                self.heading_level = level as u8;
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
            Tag::CodeBlock(_) => {
                self.in_code_block = true;
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
            Tag::Link { .. } => {
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
                self.flush_line();
                self.push_blank_line();
                self.in_heading = false;
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
                self.render_code_block();
                self.in_code_block = false;
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
            }
            TagEnd::Table => {
                self.render_table();
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
                let cell_text: String = self
                    .current_spans
                    .drain(..)
                    .map(|s| s.content.to_string())
                    .collect();
                self.table_row.push(cell_text);
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
            self.current_spans
                .push(Span::styled(text.to_string(), self.current_style()));
        }
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

    fn render_table(&mut self) {
        let border_style = Style::default().fg(self.table_border);
        let header_style = Style::default()
            .fg(self.table_header_color)
            .add_modifier(Modifier::BOLD);
        let cell_style = Style::default().fg(self.heading_other);

        let header = self.table_header_row.take().unwrap_or_default();
        let num_cols = header
            .len()
            .max(self.table_rows.iter().map(|r| r.len()).max().unwrap_or(0));

        if num_cols == 0 {
            return;
        }

        let mut col_widths = vec![0usize; num_cols];
        for (i, cell) in header.iter().enumerate() {
            col_widths[i] = col_widths[i].max(cell.len());
        }
        for row in &self.table_rows {
            for (i, cell) in row.iter().enumerate() {
                if i < num_cols {
                    col_widths[i] = col_widths[i].max(cell.len());
                }
            }
        }
        for w in &mut col_widths {
            *w = (*w).max(3);
        }

        let top: String = col_widths
            .iter()
            .map(|w| "─".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("┬");
        self.lines
            .push(Line::from(Span::styled(format!("┌{top}┐"), border_style)));

        let mut spans = Vec::new();
        spans.push(Span::styled("│".to_string(), border_style));
        for (i, w) in col_widths.iter().enumerate() {
            let cell = header.get(i).map(|s| s.as_str()).unwrap_or("");
            spans.push(Span::styled(format!(" {:<w$} ", cell), header_style));
            spans.push(Span::styled("│".to_string(), border_style));
        }
        self.lines.push(Line::from(spans));

        let sep: String = col_widths
            .iter()
            .map(|w| "─".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("┼");
        self.lines
            .push(Line::from(Span::styled(format!("├{sep}┤"), border_style)));

        for row in &self.table_rows {
            let mut spans = Vec::new();
            spans.push(Span::styled("│".to_string(), border_style));
            for (i, w) in col_widths.iter().enumerate() {
                let cell = row.get(i).map(|s| s.as_str()).unwrap_or("");
                spans.push(Span::styled(format!(" {:<w$} ", cell), cell_style));
                spans.push(Span::styled("│".to_string(), border_style));
            }
            self.lines.push(Line::from(spans));
        }

        let bottom: String = col_widths
            .iter()
            .map(|w| "─".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("┴");
        self.lines.push(Line::from(Span::styled(
            format!("└{bottom}┘"),
            border_style,
        )));

        self.table_rows.clear();
        self.push_blank_line();
    }
}
