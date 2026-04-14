//! Syntax highlighting for fenced code blocks.
//!
//! Uses [`syntect`] with its bundled grammars and themes. The [`SyntaxSet`] and
//! [`ThemeSet`] are loaded exactly once via [`std::sync::LazyLock`] — this
//! avoids re-loading ~2 MB of grammar data on every render pass while keeping
//! the API synchronous (highlighting a code block takes well under a
//! millisecond with `fancy-regex`).

use std::sync::LazyLock;

use ratatui::style::{Color, Modifier, Style};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

// ── Lazy globals ─────────────────────────────────────────────────────────────

/// The bundled [`SyntaxSet`], loaded once on first access.
///
/// `load_defaults_newlines()` is required (not `load_defaults_nonewlines()`)
/// because `syntect::easy::HighlightLines` expects lines that end with `\n`.
/// Failure to load the embedded data is a compile-time-embedded binary defect,
/// not a runtime condition, so the `LazyLock` initialiser panics on error.
pub static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

/// The bundled [`ThemeSet`], loaded once on first access.
///
/// Failure to load the embedded data is a binary defect; panic is intentional.
pub static THEME_SET: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);

// ── Public API ───────────────────────────────────────────────────────────────

/// A sequence of `(text, ratatui_style)` tokens representing one highlighted
/// line of source code.
///
/// The token strings together form the complete source line (without a trailing
/// newline). Each token carries the foreground color and font modifiers derived
/// from the active syntect theme.
pub type TokenLine = Vec<(String, Style)>;

/// Highlight `source` as the given language, returning one [`TokenLine`] per
/// source line.
///
/// # Arguments
///
/// * `source` – raw source text of the code block (may be multi-line).
/// * `lang_token` – optional language identifier (e.g. `"rust"`, `"python"`).
///   Matched via [`SyntaxSet::find_syntax_by_token`]; `None`
///   or an unrecognised token falls back to plain text.
/// * `theme_name` – name of a bundled syntect theme (e.g. `"base16-ocean.dark"`).
///   If the theme is not found the function falls back to
///   plain text styled with `fallback_fg`.
/// * `fallback_fg` – foreground color used when no highlighting is applied
///   (plain-text fallback and unknown language).
/// * `bg` – background color applied to every token span; syntect's
///   own theme background is ignored to keep the UI palette consistent.
///
/// # Returns
///
/// A `Vec<TokenLine>` with one entry per source line. Trailing empty lines
/// that pulldown-cmark strips from the code block are not present in `source`;
/// the caller is responsible for normalization before calling this function.
///
/// # Errors
///
/// This function never returns `Err`. All syntect errors (missing syntax,
/// missing theme, highlighting failures) are handled by falling back to plain
/// text so that rendering always succeeds.
pub fn highlight_code(
    source: &str,
    lang_token: Option<&str>,
    theme_name: &str,
    fallback_fg: Color,
    bg: Color,
) -> Vec<TokenLine> {
    let syntax_set = &*SYNTAX_SET;
    let theme_set = &*THEME_SET;

    // Resolve syntax definition: try the language token, fall back to plain text.
    let syntax = lang_token
        .and_then(|t| syntax_set.find_syntax_by_token(t))
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text());

    // Resolve theme: fall back to plain-text rendering if the name is missing.
    let Some(theme) = theme_set.themes.get(theme_name) else {
        return plain_text_lines(source, fallback_fg, bg);
    };

    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut result = Vec::new();

    // `LinesWithEndings` preserves the trailing `\n` on each line, which
    // syntect's incremental state machine requires for correct tokenisation.
    for raw_line in LinesWithEndings::from(source) {
        // Strip the trailing newline before building spans — we do not want
        // literal `\n` characters in the rendered output.
        let line_text = raw_line.trim_end_matches('\n');

        match highlighter.highlight_line(raw_line, syntax_set) {
            Ok(tokens) => {
                let token_line: TokenLine = tokens
                    .iter()
                    .map(|(style, fragment)| {
                        // Each fragment still has the raw bytes from `raw_line`;
                        // trim any trailing newline from the last token.
                        let text = fragment.trim_end_matches('\n').to_string();
                        (text, syntect_to_ratatui(*style, bg))
                    })
                    // Drop zero-length tokens produced by some grammars.
                    .filter(|(text, _)| !text.is_empty())
                    .collect();

                // If highlighting produced no tokens (shouldn't happen but be
                // defensive), emit the whole line as plain text.
                if token_line.is_empty() && !line_text.is_empty() {
                    result.push(vec![(
                        line_text.to_string(),
                        Style::default().fg(fallback_fg).bg(bg),
                    )]);
                } else {
                    result.push(token_line);
                }
            }
            Err(_) => {
                // On any highlighting error fall back to plain text for this
                // line rather than panicking or skipping the line.
                result.push(vec![(
                    line_text.to_string(),
                    Style::default().fg(fallback_fg).bg(bg),
                )]);
            }
        }
    }

    result
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Render `source` as unstyled lines, each consisting of a single token with
/// `fallback_fg` foreground and `bg` background.
fn plain_text_lines(source: &str, fallback_fg: Color, bg: Color) -> Vec<TokenLine> {
    let style = Style::default().fg(fallback_fg).bg(bg);
    source
        .split('\n')
        // pulldown-cmark always ends code block text with a trailing newline,
        // which after split produces a final empty string we want to skip here
        // to stay consistent with the highlighting path.
        .filter(|line| !line.is_empty() || source.trim_end_matches('\n') != source.trim())
        .map(|line| vec![(line.to_string(), style)])
        .collect()
}

/// Convert a [`syntect::highlighting::Style`] to a ratatui [`Style`].
///
/// * Foreground color is mapped to `Color::Rgb(r, g, b)`; alpha is discarded.
/// * Syntect's theme background is **ignored**; the caller supplies `bg` so
///   that the TUI palette remains consistent regardless of the syntect theme.
/// * `FontStyle::BOLD`, `FontStyle::ITALIC`, and `FontStyle::UNDERLINE` are
///   translated to the corresponding ratatui [`Modifier`]s.
pub fn syntect_to_ratatui(style: syntect::highlighting::Style, bg: Color) -> Style {
    let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);

    let mut ratatui_style = Style::default().fg(fg).bg(bg);

    if style.font_style.contains(FontStyle::BOLD) {
        ratatui_style = ratatui_style.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        ratatui_style = ratatui_style.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        ratatui_style = ratatui_style.add_modifier(Modifier::UNDERLINED);
    }

    ratatui_style
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;

    /// Every `Theme::ALL` variant must map to a bundled syntect theme name that
    /// can actually be resolved at runtime — no panics on first use.
    #[test]
    fn all_theme_variants_resolve_syntect_theme() {
        let theme_set = &*THEME_SET;
        for theme in Theme::ALL {
            let name = theme.syntax_theme_name();
            assert!(
                theme_set.themes.contains_key(name),
                "Theme::{theme:?} maps to '{name}' which is not in the bundled ThemeSet",
            );
        }
    }

    /// A Rust code block must produce more than one span, and those spans must
    /// include at least two distinct foreground colors — i.e., actual syntax
    /// highlighting occurred.
    #[test]
    fn rust_code_block_has_multiple_distinct_colors() {
        // `let` is a keyword, `x` is an identifier, `42` is a literal — three
        // token types that any sane Rust grammar assigns different colors.
        let source = "let x: i32 = 42;\n";
        let lines = highlight_code(
            source,
            Some("rust"),
            "base16-ocean.dark",
            Color::White,
            Color::Black,
        );
        assert!(!lines.is_empty(), "expected at least one line");
        let all_tokens: Vec<&(String, Style)> = lines.iter().flatten().collect();
        assert!(
            all_tokens.len() > 1,
            "expected multiple tokens, got {}",
            all_tokens.len(),
        );
        let colors: std::collections::HashSet<Color> =
            all_tokens.iter().filter_map(|(_, s)| s.fg).collect();
        assert!(
            colors.len() > 1,
            "expected multiple distinct foreground colors, got {colors:?}",
        );
    }

    /// A code block with no language tag must render with a single foreground
    /// color (plain-text fallback — same as the old behaviour).
    #[test]
    fn no_language_fallback_is_single_color() {
        let source = "hello world\nsome code\n";
        let lines = highlight_code(
            source,
            None,
            "base16-ocean.dark",
            Color::White,
            Color::Black,
        );
        let colors: std::collections::HashSet<Color> =
            lines.iter().flatten().filter_map(|(_, s)| s.fg).collect();
        // Plain-text syntax assigns every token the same foreground color from
        // the theme, so there should be exactly one distinct fg color.
        assert_eq!(
            colors.len(),
            1,
            "expected one foreground color for plain-text fallback, got {colors:?}",
        );
    }

    /// An unknown language token must NOT panic and must produce output (falling
    /// back to plain text).
    #[test]
    fn unknown_language_falls_back_without_panic() {
        let source = "some unknown code\n";
        let lines = highlight_code(
            source,
            Some("notalang"),
            "base16-ocean.dark",
            Color::White,
            Color::Black,
        );
        assert!(!lines.is_empty(), "expected output for unknown language");
        // Verify it is truly plain-text: a single token per line.
        for line in &lines {
            assert_eq!(
                line.len(),
                1,
                "expected single token per line for unknown language fallback",
            );
        }
    }
}
