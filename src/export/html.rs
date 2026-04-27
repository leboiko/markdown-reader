//! Convert a markdown string to a self-contained HTML document.
//!
//! The output is a complete HTML file that renders correctly in any modern
//! browser without external CSS or JavaScript. Mermaid diagrams are rendered
//! as Unicode text via `mermaid-text` (no browser-side Mermaid.js required),
//! LaTeX math is converted to Unicode, and fenced code blocks are
//! syntax-highlighted with inline `<span style="…">` attributes via `syntect`.

use pulldown_cmark::{CodeBlockKind, CowStr, Event, Options, Parser, Tag, TagEnd};
use syntect::html::{IncludeBackground, append_highlighted_html_for_styled_line, start_highlighted_html_snippet};
use syntect::easy::HighlightLines;
use syntect::util::LinesWithEndings;

use crate::markdown::highlight::{SYNTAX_SET, THEME_SET};
use crate::markdown::math::latex_to_unicode;
use crate::theme::Theme;

// ── CSS ───────────────────────────────────────────────────────────────────────

/// Minimal GitHub-light-ish stylesheet inlined into the document.
///
/// Deliberately kept under ~150 lines: decent typography, table borders,
/// code font, and enough spacing to look like a rendered README.
const INLINE_CSS: &str = r#"
*, *::before, *::after { box-sizing: border-box; }

body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial,
                 sans-serif, "Apple Color Emoji";
    font-size: 16px;
    line-height: 1.6;
    color: #24292f;
    background: #ffffff;
    max-width: 860px;
    margin: 0 auto;
    padding: 2rem 1.5rem 4rem;
}

h1, h2, h3, h4, h5, h6 {
    margin-top: 1.5rem;
    margin-bottom: 0.5rem;
    font-weight: 600;
    line-height: 1.25;
    color: #24292f;
}
h1 { font-size: 2em;   padding-bottom: 0.3em; border-bottom: 1px solid #d0d7de; }
h2 { font-size: 1.5em; padding-bottom: 0.3em; border-bottom: 1px solid #d0d7de; }
h3 { font-size: 1.25em; }

p { margin-top: 0; margin-bottom: 1rem; }

a { color: #0969da; text-decoration: none; }
a:hover { text-decoration: underline; }

code {
    font-family: "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace;
    font-size: 87.5%;
    background: #f6f8fa;
    border-radius: 3px;
    padding: 0.2em 0.4em;
    color: #24292f;
}

pre {
    font-family: "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace;
    font-size: 87.5%;
    line-height: 1.45;
    background: #f6f8fa;
    border-radius: 6px;
    padding: 1rem;
    overflow-x: auto;
    margin-bottom: 1rem;
}
pre code { background: transparent; padding: 0; font-size: inherit; }

pre.mermaid-text {
    background: #f0f3f7;
    border: 1px solid #d0d7de;
    color: #24292f;
    white-space: pre;
}

table {
    border-collapse: collapse;
    width: 100%;
    margin-bottom: 1rem;
    display: block;
    overflow-x: auto;
}
th, td {
    border: 1px solid #d0d7de;
    padding: 0.4em 0.8em;
    text-align: left;
}
thead tr { background: #f6f8fa; }
tbody tr:nth-child(even) { background: #f6f8fa; }

blockquote {
    margin: 0 0 1rem;
    padding: 0 1em;
    color: #57606a;
    border-left: 4px solid #d0d7de;
}

ul, ol { margin-top: 0; margin-bottom: 1rem; padding-left: 2em; }
li { margin-bottom: 0.2em; }
li > ul, li > ol { margin-bottom: 0; }

span.math, div.math {
    font-family: "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace;
    background: #f6f8fa;
    border-radius: 3px;
    padding: 0.1em 0.3em;
    color: #24292f;
}
div.math {
    display: block;
    padding: 0.6em 1em;
    margin-bottom: 1rem;
    overflow-x: auto;
    white-space: pre;
}

hr { border: none; border-top: 1px solid #d0d7de; margin: 1.5rem 0; }

img { max-width: 100%; height: auto; }

del { color: #57606a; }

input[type="checkbox"] { margin-right: 0.4em; }
"#;

// ── Public API ────────────────────────────────────────────────────────────────

/// Render `markdown_content` to a standalone HTML document string.
///
/// The output starts with `<!DOCTYPE html>` and contains an inlined `<style>`
/// block — no external resources are needed to display it in a browser.
///
/// # Arguments
///
/// * `content` — raw markdown source (UTF-8).
/// * `title`   — value used in the HTML `<title>` element.
/// * `theme`   — active TUI theme; selects the matching syntect colour scheme
///   for fenced code block syntax highlighting.
pub fn render_to_html(content: &str, title: &str, theme: Theme) -> String {
    let body = render_body(content, theme);
    // Escape the title so injected filenames with `<` etc. don't break HTML.
    let escaped_title = html_escape(title);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{escaped_title}</title>
<style>{INLINE_CSS}</style>
</head>
<body>
{body}
</body>
</html>
"#
    )
}

// ── Body renderer ─────────────────────────────────────────────────────────────

/// Walk the pulldown-cmark event stream and emit the HTML body.
///
/// We intercept:
/// - `mermaid` fenced code blocks → `<pre class="mermaid-text">` with Unicode
///   output from the `mermaid-text` crate.
/// - Other fenced/indented code blocks → syntect-highlighted `<pre>` blocks.
/// - `InlineMath` / `DisplayMath` events → Unicode via `latex_to_unicode`.
///
/// Everything else is handled by feeding events back to pulldown-cmark's own
/// `push_html` writer one event at a time.
fn render_body(content: &str, theme: Theme) -> String {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH;

    let events: Vec<Event<'_>> = Parser::new_ext(content, opts).collect();
    let mut out = String::with_capacity(content.len() * 2);
    let mut i = 0;

    while i < events.len() {
        match &events[i] {
            // ── Fenced code block ────────────────────────────────────────────
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => lang_token(info),
                    CodeBlockKind::Indented => None,
                };
                // Collect the Text events that form the code body.
                i += 1;
                let mut code = String::new();
                while i < events.len() {
                    match &events[i] {
                        Event::Text(t) => {
                            code.push_str(t);
                            i += 1;
                        }
                        Event::End(TagEnd::CodeBlock) => {
                            i += 1;
                            break;
                        }
                        _ => {
                            i += 1;
                        }
                    }
                }

                if lang == Some("mermaid") {
                    out.push_str(&render_mermaid_block(&code));
                } else {
                    out.push_str(&render_code_block(&code, lang, theme));
                }
            }

            // ── Inline math: $…$ ─────────────────────────────────────────────
            Event::InlineMath(latex) => {
                let unicode = latex_to_unicode(latex);
                out.push_str(r#"<span class="math">"#);
                out.push_str(&html_escape(&unicode));
                out.push_str("</span>");
                i += 1;
            }

            // ── Display math: $$…$$ ──────────────────────────────────────────
            Event::DisplayMath(latex) => {
                let unicode = latex_to_unicode(latex);
                out.push_str(r#"<div class="math">"#);
                out.push_str(&html_escape(&unicode));
                out.push_str("</div>");
                i += 1;
            }

            // ── Everything else: delegate to pulldown-cmark's HTML writer ────
            other => {
                // Clone the single event so pulldown_cmark::html::push_html can
                // own it; the source slice lives long enough via `events`.
                let single = std::iter::once(other.clone());
                pulldown_cmark::html::push_html(&mut out, single);
                i += 1;
            }
        }
    }

    out
}

// ── Mermaid rendering ─────────────────────────────────────────────────────────

/// Render a Mermaid diagram source string to a `<pre class="mermaid-text">` block.
///
/// Falls back to displaying the raw source in a plain `<pre>` when
/// `mermaid-text` cannot parse the diagram (unsupported type, syntax error, etc.).
fn render_mermaid_block(source: &str) -> String {
    let rendered = mermaid_text::render(source)
        .unwrap_or_else(|_| source.to_owned());
    format!(
        "<pre class=\"mermaid-text\">{}</pre>\n",
        html_escape(&rendered)
    )
}

// ── Syntax highlighting ───────────────────────────────────────────────────────

/// Render a fenced code block as a syntax-highlighted HTML `<pre>` snippet.
///
/// Uses syntect's inline-style HTML output so the result is self-contained.
/// Falls back to a plain `<pre><code>` block when the language is unknown or
/// highlighting fails.
fn render_code_block(source: &str, lang: Option<&str>, theme: Theme) -> String {
    let syntax_set = &*SYNTAX_SET;
    let theme_set = &*THEME_SET;

    let syntax = lang
        .and_then(|t| syntax_set.find_syntax_by_token(t))
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text());

    let syntect_theme_name = theme.syntax_theme_name();
    let Some(syntect_theme) = theme_set.themes.get(syntect_theme_name) else {
        // Should never happen for bundled themes; fall back to plain.
        return plain_code_block(source);
    };

    let mut highlighter = HighlightLines::new(syntax, syntect_theme);
    let (mut html_pre, bg) = start_highlighted_html_snippet(syntect_theme);
    // `start_highlighted_html_snippet` opens `<pre style="…">` but does NOT
    // emit `<code>` — that's fine for our usage.

    for line in LinesWithEndings::from(source) {
        match highlighter.highlight_line(line, syntax_set) {
            Ok(regions) => {
                // Append each line's spans; `IfDifferent(bg)` avoids setting
                // background-color on every token when it matches the <pre> bg.
                append_highlighted_html_for_styled_line(
                    &regions,
                    IncludeBackground::IfDifferent(bg),
                    &mut html_pre,
                )
                // Only fails on fmt::Write errors which can't happen for String.
                .unwrap();
            }
            Err(_) => {
                // Highlighting failed for this line; emit it plain.
                html_pre.push_str(&html_escape(line.trim_end_matches('\n')));
                html_pre.push('\n');
            }
        }
    }

    html_pre.push_str("</pre>\n");
    html_pre
}

/// Emit a plain (unhighlighted) `<pre><code>` block.
fn plain_code_block(source: &str) -> String {
    format!("<pre><code>{}</code></pre>\n", html_escape(source))
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Extract the primary language token from a fenced code block info string.
///
/// Pulldown-cmark puts the whole info string (e.g. `"rust,no_run"`) into
/// the `CowStr`. We only need the first word for syntax lookup.
fn lang_token<'a>(info: &'a CowStr<'_>) -> Option<&'a str> {
    let first = info.split(|c: char| c.is_whitespace() || c == ',').next()?;
    if first.is_empty() { None } else { Some(first) }
}

/// HTML-escape the five characters that are significant inside element content.
///
/// Covers `&`, `<`, `>`, `"`, and `'` (the latter two matter inside attribute
/// values but are harmless to escape in text nodes too).
fn html_escape(s: &str) -> String {
    // Pre-allocate with the original length; most strings won't grow.
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&'  => out.push_str("&amp;"),
            '<'  => out.push_str("&lt;"),
            '>'  => out.push_str("&gt;"),
            '"'  => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _    => out.push(ch),
        }
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_self_contained_html() {
        let html = render_to_html("# Hello", "test", Theme::Default);
        assert!(
            html.starts_with("<!DOCTYPE html>"),
            "output must start with DOCTYPE"
        );
        assert!(html.contains("<style>"), "output must contain inline <style>");
        assert!(
            html.contains("</html>"),
            "output must be a complete HTML document"
        );
    }

    #[test]
    fn renders_basic_paragraph() {
        let html = render_to_html("hello world", "t", Theme::Default);
        assert!(
            html.contains("<p>hello world</p>"),
            "expected paragraph tag: {html}"
        );
    }

    #[test]
    fn renders_heading() {
        let html = render_to_html("# Title", "t", Theme::Default);
        assert!(
            html.contains("<h1>Title</h1>"),
            "expected h1 tag: {html}"
        );
    }

    #[test]
    fn renders_fenced_code_with_syntax_highlight() {
        let md = "```rust\nlet x = 42;\n```";
        let html = render_to_html(md, "t", Theme::Default);
        assert!(html.contains("<pre"), "expected <pre> tag: {html}");
        assert!(
            html.contains(r#"<span style="#),
            "expected at least one styled span from syntect: {html}"
        );
    }

    #[test]
    fn renders_mermaid_block_as_text_pre() {
        let md = "```mermaid\ngraph LR\nA-->B\n```";
        let html = render_to_html(md, "t", Theme::Default);
        assert!(
            html.contains(r#"<pre class="mermaid-text""#),
            "expected mermaid-text pre: {html}"
        );
        // The rendered diagram must contain both node labels.
        assert!(html.contains('A'), "expected node A in output: {html}");
        assert!(html.contains('B'), "expected node B in output: {html}");
    }

    #[test]
    fn renders_inline_math() {
        // $\alpha + \beta$ should produce Unicode Greek letters.
        let md = r"Hello $\alpha + \beta$";
        let html = render_to_html(md, "t", Theme::Default);
        assert!(
            html.contains(r#"<span class="math">"#),
            "expected math span: {html}"
        );
        // latex_to_unicode converts \alpha → α, \beta → β.
        assert!(html.contains('α'), "expected alpha: {html}");
        assert!(html.contains('β'), "expected beta: {html}");
    }

    #[test]
    fn renders_display_math() {
        let md = "$$E = mc^2$$";
        let html = render_to_html(md, "t", Theme::Default);
        assert!(
            html.contains(r#"<div class="math">"#),
            "expected display math div: {html}"
        );
        // 2 → ² via to_superscript.
        assert!(html.contains('²'), "expected superscript 2: {html}");
    }

    #[test]
    fn html_escape_encodes_special_chars() {
        let s = r#"<script>alert('xss & "fun"')</script>"#;
        let escaped = html_escape(s);
        assert!(!escaped.contains('<'), "< should be escaped");
        assert!(!escaped.contains('>'), "> should be escaped");
        assert!(escaped.contains("&amp;"), "& should be escaped as &amp;");
    }

    #[test]
    fn title_is_in_document() {
        let html = render_to_html("text", "My Great Doc", Theme::GithubLight);
        assert!(
            html.contains("<title>My Great Doc</title>"),
            "title not found: {html}"
        );
    }

    #[test]
    fn mermaid_fallback_on_invalid_source() {
        // An empty or unrecognised diagram type must not panic — it falls back
        // to displaying the raw source.
        let md = "```mermaid\nnot a valid diagram at all\n```";
        let html = render_to_html(md, "t", Theme::Default);
        assert!(
            html.contains(r#"<pre class="mermaid-text""#),
            "should still emit mermaid-text pre on parse error: {html}"
        );
    }
}
