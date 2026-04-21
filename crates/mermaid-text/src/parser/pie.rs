//! Parser for Mermaid `pie` charts.
//!
//! Accepted syntax:
//!
//! ```text
//! pie [showData] [title <text>]
//!     "Label1" : 386
//!     "Label2" : 85
//!     ...
//! ```
//!
//! - The `pie` keyword is required as the first non-blank token.
//! - `showData` is optional (case-insensitive); when present the renderer
//!   includes the raw value alongside the percentage.
//! - `title <text>` is optional; the title text is everything after
//!   `title ` on the header line, trimmed.
//! - Each subsequent non-blank, non-comment line must be a slice
//!   declaration: a quoted label, a `:`, then a positive numeric value.
//! - `%%` line comments and blank lines are silently skipped.
//!
//! # Examples
//!
//! ```
//! use mermaid_text::parser::pie::parse;
//!
//! let chart = parse("pie title Pets\n\"Dogs\" : 386\n\"Cats\" : 85").unwrap();
//! assert_eq!(chart.title.as_deref(), Some("Pets"));
//! assert_eq!(chart.slices.len(), 2);
//! assert_eq!(chart.slices[0].label, "Dogs");
//! assert_eq!(chart.slices[0].value, 386.0);
//! ```

use crate::Error;
use crate::parser::common::strip_inline_comment;
use crate::pie::{PieChart, PieSlice};

pub fn parse(src: &str) -> Result<PieChart, Error> {
    let mut chart = PieChart::default();
    let mut header_seen = false;

    for raw in src.lines() {
        let line = strip_inline_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        if !header_seen {
            parse_header(line, &mut chart)?;
            header_seen = true;
            continue;
        }

        let slice = parse_slice_line(line)?;
        chart.slices.push(slice);
    }

    if !header_seen {
        return Err(Error::ParseError(
            "missing `pie` header line".to_string(),
        ));
    }
    if chart.slices.is_empty() {
        return Err(Error::ParseError(
            "pie chart must have at least one slice".to_string(),
        ));
    }
    if chart.total() <= 0.0 {
        return Err(Error::ParseError(
            "pie chart total must be greater than zero".to_string(),
        ));
    }
    Ok(chart)
}

fn parse_header(line: &str, chart: &mut PieChart) -> Result<(), Error> {
    // First whitespace-delimited token must be `pie` (case-insensitive).
    let mut rest = line.trim_start();
    let (head, tail) = split_first_word(rest);
    if !head.eq_ignore_ascii_case("pie") {
        return Err(Error::ParseError(format!(
            "expected `pie` header, got {head:?}"
        )));
    }
    rest = tail.trim_start();

    // Optional `showData` flag (Mermaid uses camelCase).
    let (next, after) = split_first_word(rest);
    if next.eq_ignore_ascii_case("showData") {
        chart.show_data = true;
        rest = after.trim_start();
    }

    // Optional `title <text>`.
    let (next, after) = split_first_word(rest);
    if next.eq_ignore_ascii_case("title") {
        let title = after.trim();
        if !title.is_empty() {
            chart.title = Some(title.to_string());
        }
    } else if !next.is_empty() {
        // Anything else after the optional flag is unexpected — surface it
        // so a typo'd flag (e.g. `pie ShowData …`) doesn't silently hide.
        return Err(Error::ParseError(format!(
            "unexpected token after `pie` header: {next:?}"
        )));
    }
    Ok(())
}

fn parse_slice_line(line: &str) -> Result<PieSlice, Error> {
    // Locate the first quoted label: "<label>".
    let bytes = line.as_bytes();
    if bytes.first() != Some(&b'"') {
        return Err(Error::ParseError(format!(
            "pie slice must start with a quoted label: {line:?}"
        )));
    }
    let close = line[1..].find('"').ok_or_else(|| {
        Error::ParseError(format!(
            "pie slice label is missing closing quote: {line:?}"
        ))
    })?;
    let label = line[1..1 + close].to_string();

    // After the closing quote: optional whitespace, `:`, optional whitespace,
    // then the numeric value.
    let after = line[1 + close + 1..].trim_start();
    let after = after.strip_prefix(':').ok_or_else(|| {
        Error::ParseError(format!(
            "pie slice missing `:` between label and value: {line:?}"
        ))
    })?;
    let value_str = after.trim();
    let value: f64 = value_str.parse().map_err(|_| {
        Error::ParseError(format!(
            "pie slice value is not numeric: {value_str:?} in {line:?}"
        ))
    })?;
    if !value.is_finite() || value <= 0.0 {
        return Err(Error::ParseError(format!(
            "pie slice value must be a positive finite number: {value} in {line:?}"
        )));
    }
    Ok(PieSlice { label, value })
}

/// Split off the first whitespace-delimited word, returning
/// `(first_word, remainder)`. Empty input yields two empty strings.
fn split_first_word(s: &str) -> (&str, &str) {
    let s = s.trim_start();
    match s.find(char::is_whitespace) {
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_pie() {
        let c = parse("pie\n\"A\" : 1").unwrap();
        assert_eq!(c.title, None);
        assert!(!c.show_data);
        assert_eq!(c.slices.len(), 1);
        assert_eq!(c.slices[0].label, "A");
        assert_eq!(c.slices[0].value, 1.0);
    }

    #[test]
    fn parse_with_title() {
        let c = parse("pie title Pet Counts\n\"Dogs\" : 386\n\"Cats\" : 85").unwrap();
        assert_eq!(c.title.as_deref(), Some("Pet Counts"));
        assert_eq!(c.slices.len(), 2);
        assert_eq!(c.slices[1].value, 85.0);
    }

    #[test]
    fn parse_with_show_data_and_title() {
        let c = parse(
            "pie showData title Browser Share\n\
             \"Chrome\" : 60\n\
             \"Firefox\" : 25\n\
             \"Safari\" : 15",
        )
        .unwrap();
        assert!(c.show_data);
        assert_eq!(c.title.as_deref(), Some("Browser Share"));
        assert_eq!(c.slices.len(), 3);
    }

    #[test]
    fn parse_show_data_without_title() {
        let c = parse("pie showData\n\"A\" : 1").unwrap();
        assert!(c.show_data);
        assert_eq!(c.title, None);
    }

    #[test]
    fn parse_float_values_supported() {
        let c = parse("pie\n\"A\" : 12.5\n\"B\" : 7.5").unwrap();
        assert_eq!(c.slices[0].value, 12.5);
        assert_eq!(c.slices[1].value, 7.5);
        assert_eq!(c.total(), 20.0);
    }

    #[test]
    fn parse_skips_comments_and_blanks() {
        let c = parse(
            "%% top comment\n\
             pie title X\n\
             \n\
             \"A\" : 1\n\
             %% mid comment\n\
             \"B\" : 2",
        )
        .unwrap();
        assert_eq!(c.slices.len(), 2);
    }

    #[test]
    fn parse_rejects_negative_value() {
        let err = parse("pie\n\"A\" : -1").expect_err("negative must error");
        assert!(err.to_string().contains("positive"));
    }

    #[test]
    fn parse_rejects_zero_total() {
        // Single zero slice → total is 0, which the parser already rejects
        // via the per-slice positivity check before reaching the total
        // check. Use a value the per-slice check accepts (positive) but
        // arrange a degenerate case... actually positivity rules out a
        // zero total entirely. Verify the per-slice check fires.
        let err = parse("pie\n\"A\" : 0").expect_err("zero must error");
        assert!(err.to_string().contains("positive"));
    }

    #[test]
    fn parse_rejects_missing_header() {
        let err = parse("\"A\" : 1").expect_err("no header must error");
        assert!(err.to_string().contains("pie"));
    }

    #[test]
    fn parse_rejects_no_slices() {
        let err = parse("pie title Empty").expect_err("zero slices must error");
        assert!(err.to_string().contains("at least one slice"));
    }

    #[test]
    fn parse_rejects_unquoted_label() {
        let err = parse("pie\nA : 1").expect_err("missing quote must error");
        assert!(err.to_string().contains("quoted"));
    }

    #[test]
    fn parse_rejects_missing_colon() {
        let err = parse("pie\n\"A\" 1").expect_err("missing colon must error");
        assert!(err.to_string().contains("`:`"));
    }

    #[test]
    fn parse_rejects_non_numeric_value() {
        let err = parse("pie\n\"A\" : abc").expect_err("non-numeric must error");
        assert!(err.to_string().contains("not numeric"));
    }
}
