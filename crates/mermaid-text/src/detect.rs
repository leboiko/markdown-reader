//! Diagram-type detection from the first non-blank line of Mermaid source.

use crate::Error;

/// The diagram types that this crate can handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagramKind {
    /// `graph <direction>` or `flowchart <direction>` diagrams.
    Flowchart,
    /// `sequenceDiagram` diagrams.
    Sequence,
}

/// Detect the kind of Mermaid diagram described by `input`.
///
/// Only the first non-blank, non-comment line is examined.  Lines beginning
/// with `%%` are treated as comments and skipped.
///
/// # Arguments
///
/// * `input` — the Mermaid source string (need not be fully valid)
///
/// # Returns
///
/// The detected [`DiagramKind`] on success.
///
/// # Errors
///
/// Returns [`Error::EmptyInput`] if `input` contains no non-blank lines.
/// Returns [`Error::UnsupportedDiagram`] if the diagram type is not supported.
///
/// # Examples
///
/// ```
/// use mermaid_text::detect::{detect, DiagramKind};
///
/// assert_eq!(detect("graph LR\nA-->B").unwrap(), DiagramKind::Flowchart);
/// assert_eq!(detect("flowchart TD\nA-->B").unwrap(), DiagramKind::Flowchart);
/// assert_eq!(detect("sequenceDiagram\nA->>B: hi").unwrap(), DiagramKind::Sequence);
/// assert!(detect("").is_err());
/// assert!(detect("pie title Pets").is_err());
/// ```
pub fn detect(input: &str) -> Result<DiagramKind, Error> {
    let first = input
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("%%"))
        .ok_or(Error::EmptyInput)?;

    // The first token (before any whitespace) determines the diagram type.
    let keyword = first.split_whitespace().next().unwrap_or(first);

    match keyword.to_lowercase().as_str() {
        "graph" | "flowchart" => Ok(DiagramKind::Flowchart),
        "sequencediagram" => Ok(DiagramKind::Sequence),
        other => Err(Error::UnsupportedDiagram(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_graph_keyword() {
        assert_eq!(detect("graph LR\nA-->B").unwrap(), DiagramKind::Flowchart);
    }

    #[test]
    fn detects_flowchart_keyword() {
        assert_eq!(
            detect("flowchart TD\nA-->B").unwrap(),
            DiagramKind::Flowchart
        );
    }

    #[test]
    fn empty_returns_error() {
        assert!(matches!(detect(""), Err(Error::EmptyInput)));
        assert!(matches!(detect("   \n  "), Err(Error::EmptyInput)));
    }

    #[test]
    fn unknown_type_returns_error() {
        assert!(matches!(
            detect("pie title Pets"),
            Err(Error::UnsupportedDiagram(_))
        ));
    }

    #[test]
    fn skips_comment_lines() {
        assert_eq!(
            detect("%% This is a comment\ngraph LR\nA-->B").unwrap(),
            DiagramKind::Flowchart
        );
    }
}
