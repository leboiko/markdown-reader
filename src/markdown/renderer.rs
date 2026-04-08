use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
};

/// Render a markdown string into a ratatui [`Text`] value ready for display.
///
/// Supported markdown features: headings, paragraphs, block-quotes, fenced
/// code blocks, ordered/unordered lists, task lists, tables, emphasis, bold,
/// strikethrough, inline code, links, and horizontal rules.
///
/// # Arguments
///
/// * `content` - Raw markdown text to parse and render.
///
/// # Returns
///
/// A `'static` [`Text`] whose spans carry appropriate styling for each
/// markdown element.
pub fn render_markdown(content: &str) -> Text<'static> {
    // Combine all desired parser extensions with a single bitwise-OR expression.
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;

    let parser = Parser::new_ext(content, opts);
    let mut renderer = MdRenderer::new();
    renderer.render(parser);
    Text::from(renderer.lines)
}

struct MdRenderer {
    lines: Vec<Line<'static>>,
    current_spans: Vec<Span<'static>>,
    style_stack: Vec<Style>,
    list_depth: usize,
    list_counters: Vec<Option<u64>>, // None = unordered, Some(n) = ordered
    in_code_block: bool,
    code_block_lines: Vec<String>,
    in_heading: bool,
    heading_level: u8,
    in_blockquote: bool,
    in_table: bool,
    table_alignments: Vec<pulldown_cmark::Alignment>,
    table_row: Vec<String>,
    table_header: bool,
}

impl MdRenderer {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            current_spans: Vec::new(),
            style_stack: vec![Style::default()],
            list_depth: 0,
            list_counters: Vec::new(),
            in_code_block: false,
            code_block_lines: Vec::new(),
            in_heading: false,
            heading_level: 0,
            in_blockquote: false,
            in_table: false,
            table_alignments: Vec::new(),
            table_row: Vec::new(),
            table_header: false,
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
        let spans = std::mem::take(&mut self.current_spans);
        if self.in_blockquote && !self.in_code_block {
            let mut bq_spans = vec![Span::styled(
                "│ ".to_string(),
                Style::default().fg(Color::DarkGray),
            )];
            bq_spans.extend(spans);
            self.lines.push(Line::from(bq_spans));
        } else {
            self.lines.push(Line::from(spans));
        }
    }

    fn push_blank_line(&mut self) {
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
                        .fg(Color::Green)
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
                        Style::default().fg(Color::DarkGray),
                    )));
                    self.push_blank_line();
                }
                Event::TaskListMarker(checked) => {
                    let marker = if checked { "☑ " } else { "☐ " };
                    self.current_spans.push(Span::styled(
                        marker.to_string(),
                        Style::default().fg(Color::Cyan),
                    ));
                }
                _ => {}
            }
        }
        // Flush any remaining spans
        if !self.current_spans.is_empty() {
            self.flush_line();
        }
    }

    fn start_tag(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { level, .. } => {
                self.in_heading = true;
                self.heading_level = level as u8;
                let style = match level {
                    pulldown_cmark::HeadingLevel::H1 => Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                    pulldown_cmark::HeadingLevel::H2 => Style::default()
                        .fg(Color::Blue)
                        .add_modifier(Modifier::BOLD),
                    pulldown_cmark::HeadingLevel::H3 => Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                    _ => Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                };
                self.push_style(style);
                // Add heading prefix
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
                self.push_style(Style::default().fg(Color::Gray));
            }
            Tag::CodeBlock(_) => {
                self.in_code_block = true;
                self.code_block_lines.clear();
                self.flush_line();
                // Top border
                self.lines.push(Line::from(Span::styled(
                    "╭─────────────────────────────────────────────╮".to_string(),
                    Style::default().fg(Color::DarkGray),
                )));
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
                    .push(Span::styled(bullet, Style::default().fg(Color::Yellow)));
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
                        .fg(Color::Blue)
                        .add_modifier(Modifier::UNDERLINED),
                );
            }
            Tag::Table(alignments) => {
                self.in_table = true;
                self.table_alignments = alignments;
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
                // Bottom border
                self.lines.push(Line::from(Span::styled(
                    "╰─────────────────────────────────────────────╯".to_string(),
                    Style::default().fg(Color::DarkGray),
                )));
                self.push_blank_line();
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
                self.in_table = false;
                self.push_blank_line();
            }
            TagEnd::TableHead => {
                self.render_table_row(true);
                self.table_header = false;
                // Separator line
                let sep: Vec<String> = self.table_row.iter().map(|_| "─".repeat(14)).collect();
                self.lines.push(Line::from(Span::styled(
                    format!("├{}┤", sep.join("┼")),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            TagEnd::TableRow => {
                if !self.table_header {
                    self.render_table_row(false);
                }
            }
            TagEnd::TableCell => {
                // Collect cell text from current spans
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
            let code_style = Style::default()
                .fg(Color::Rgb(180, 200, 180))
                .bg(Color::Rgb(40, 40, 40));
            // Split into lines, skip trailing empty line from pulldown-cmark
            let lines: Vec<&str> = text.split('\n').collect();
            let lines = if lines.last() == Some(&"") {
                &lines[..lines.len() - 1]
            } else {
                &lines
            };
            for line in lines {
                let padded = format!("│ {:<44}│", line);
                self.lines
                    .push(Line::from(Span::styled(padded, code_style)));
            }
        } else {
            self.current_spans
                .push(Span::styled(text.to_string(), self.current_style()));
        }
    }

    fn render_table_row(&mut self, is_header: bool) {
        let style = if is_header {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let mut spans = vec![Span::styled(
            "│ ".to_string(),
            Style::default().fg(Color::DarkGray),
        )];

        for (i, cell) in self.table_row.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(
                    " │ ".to_string(),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            spans.push(Span::styled(format!("{:<12}", cell), style));
        }

        spans.push(Span::styled(
            " │".to_string(),
            Style::default().fg(Color::DarkGray),
        ));

        self.lines.push(Line::from(spans));
    }
}
