//! Helpers shared by the flowchart and state-diagram parsers.
//!
//! Centralises the small text-processing primitives (comment stripping,
//! keyword matching, `key:value,…` payload parsing, `:::className`
//! shorthand extraction, NodeStyle merging) so the two parsers don't
//! drift their own copies. Each helper is `pub(crate)` — these are
//! parser-internal building blocks, not public crate API.

use crate::types::{EdgeStyleColors, Graph, NodeStyle, Rgb};

/// Strip a trailing `%% comment` if present, but only if the `%%` is
/// outside a `"…"` quoted string (state diagrams put quoted display
/// names inside `state "…" as Id` and we don't want to truncate one of
/// those if the user includes `%%` literally).
pub(crate) fn strip_inline_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_quote = false;
    let mut i = 0;
    while i + 1 < bytes.len() {
        let c = bytes[i];
        if c == b'"' {
            in_quote = !in_quote;
        } else if !in_quote && c == b'%' && bytes[i + 1] == b'%' {
            return &line[..i];
        }
        i += 1;
    }
    line
}

/// Returns true if `stmt` starts with `keyword` followed by whitespace,
/// a colon, or end-of-string. Used by the silent-skip dispatch in both
/// parsers to recognise directives like `accTitle:` / `classDef foo`.
pub(crate) fn matches_keyword(stmt: &str, keyword: &str) -> bool {
    if let Some(rest) = stmt.strip_prefix(keyword) {
        rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with(':')
    } else {
        false
    }
}

/// Walk a `key:value,key:value,...` payload (the right-hand side of a
/// Mermaid `style` / `linkStyle` / `classDef` directive) and invoke `f`
/// for each pair. Whitespace around keys, values, and the comma
/// separator is trimmed; pairs without a `:` are silently skipped.
///
/// This is the low-level primitive — for the common case of extracting
/// recognised colour attributes use [`parse_node_style_payload`] or
/// [`parse_edge_color_payload`] instead.
pub(crate) fn apply_color_pairs(payload: &str, mut f: impl FnMut(&str, &str)) {
    for pair in payload.split(',') {
        let pair = pair.trim();
        let Some((key, value)) = pair.split_once(':') else {
            continue;
        };
        f(key.trim(), value.trim());
    }
}

/// Parse the `fill` / `stroke` / `color` attributes from a
/// `key:value,…` payload into a [`NodeStyle`]. Unknown keys and
/// unparseable hex values are silently ignored.
///
/// Used by both `style <id> …` (per-id) and `classDef name …` (named
/// reusable class) directive handlers — they take the same payload
/// shape, so the parsing is identical.
pub(crate) fn parse_node_style_payload(payload: &str) -> NodeStyle {
    let mut style = NodeStyle::default();
    apply_color_pairs(payload, |key, value| match key {
        "fill" => style.fill = Rgb::parse_hex(value),
        "stroke" => style.stroke = Rgb::parse_hex(value),
        "color" => style.color = Rgb::parse_hex(value),
        _ => {}
    });
    style
}

/// Parse the `stroke` / `color` attributes from a `key:value,…`
/// payload into an [`EdgeStyleColors`]. Edges only have these two
/// colour attributes (no fill — there's no interior to fill).
pub(crate) fn parse_edge_color_payload(payload: &str) -> EdgeStyleColors {
    let mut colors = EdgeStyleColors::default();
    apply_color_pairs(payload, |key, value| match key {
        "stroke" => colors.stroke = Rgb::parse_hex(value),
        "color" => colors.color = Rgb::parse_hex(value),
        _ => {}
    });
    colors
}

/// Strip a trailing `:::className` shorthand (or chain like
/// `:::a:::b:::c`) from `token`. Returns the cleaned token plus the
/// list of class names in source order.
///
/// Used at every node-id extraction point so the shorthand works in
/// transitions (`A:::cache --> B:::warn`), declarations
/// (`state X:::important`), and shape-bracket expressions
/// (`A[Label]:::cache`). The helper is allocation-free when the token
/// has no modifier — the cleaned id is borrowed from the input.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(extract_class_modifier("A"),
///            ("A".to_string(), vec![]));
/// assert_eq!(extract_class_modifier("A:::cache"),
///            ("A".to_string(), vec!["cache".to_string()]));
/// assert_eq!(extract_class_modifier("A:::a:::b"),
///            ("A".to_string(), vec!["a".to_string(), "b".to_string()]));
/// assert_eq!(extract_class_modifier("A[Label]:::cache"),
///            ("A[Label]".to_string(), vec!["cache".to_string()]));
/// // [*] markers are preserved verbatim so the caller can still
/// // mangle them per scope.
/// assert_eq!(extract_class_modifier("[*]:::started"),
///            ("[*]".to_string(), vec!["started".to_string()]));
/// ```
pub(crate) fn extract_class_modifier(token: &str) -> (String, Vec<String>) {
    // Walk from the end peeling off `:::name` segments. We split on
    // `:::` (three colons) — Mermaid uses this exact separator and it
    // doesn't collide with single colons inside labels.
    let mut classes: Vec<String> = Vec::new();
    let mut remainder = token;
    while let Some(idx) = remainder.rfind(":::") {
        let after = &remainder[idx + 3..];
        // Class names are alphanumeric plus underscores. If the chunk
        // after `:::` contains whitespace or other separators, this
        // isn't a class modifier — bail.
        if after.is_empty()
            || !after
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            break;
        }
        classes.push(after.to_string());
        remainder = &remainder[..idx];
    }
    classes.reverse(); // restore source order (we pushed back-to-front)
    (remainder.to_string(), classes)
}

/// Merge `overlay` on top of `base` — for any field where `overlay`
/// has a value, that value wins; otherwise `base` is preserved.
///
/// Used by the class-application resolver to stack multiple
/// `:::class1:::class2` shorthands and to layer per-id `style`
/// directives over class-derived styles.
pub(crate) fn merge_node_style(base: NodeStyle, overlay: NodeStyle) -> NodeStyle {
    NodeStyle {
        fill: overlay.fill.or(base.fill),
        stroke: overlay.stroke.or(base.stroke),
        color: overlay.color.or(base.color),
    }
}

// ---------------------------------------------------------------------------
// Style / class directive parsers — shared by both the flowchart and
// state-diagram parsers. Mermaid's directive syntax is the same in
// both; the only diagram-specific concern is who owns the dispatching
// (each parser's statement loop) and where pending applications get
// collected during the walk.
// ---------------------------------------------------------------------------

/// Parse a `style <id> key:value,key:value,...` directive and merge the
/// recognised color attributes into `graph.node_styles[id]`. Unknown
/// keys and unparseable hex values are silently ignored so a stray
/// attribute can never break otherwise-valid input.
pub(crate) fn parse_style_directive(stmt: &str, graph: &mut Graph) {
    let mut parts = stmt.splitn(3, char::is_whitespace);
    let _ = parts.next(); // "style"
    let Some(id) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    let rest = parts.next().unwrap_or("");
    let overlay = parse_node_style_payload(rest);
    let base = graph.node_styles.get(id).copied().unwrap_or_default();
    graph
        .node_styles
        .insert(id.to_string(), merge_node_style(base, overlay));
}

/// Parse a `linkStyle <indexes> key:value,...` directive and merge the
/// recognised colors into `graph.edge_styles` for each listed edge.
///
/// `indexes` may be a comma-separated list of integers or the keyword
/// `default`, which we interpret as "apply to every edge that exists at
/// the time the directive is processed."
pub(crate) fn parse_link_style_directive(stmt: &str, graph: &mut Graph) {
    let mut parts = stmt.splitn(3, char::is_whitespace);
    let _ = parts.next(); // "linkStyle"
    let Some(indexes) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    let rest = parts.next().unwrap_or("");

    let target_indexes: Vec<usize> = if indexes == "default" {
        (0..graph.edges.len()).collect()
    } else {
        indexes
            .split(',')
            .filter_map(|s| s.trim().parse::<usize>().ok())
            .collect()
    };
    if target_indexes.is_empty() {
        return;
    }

    let delta = parse_edge_color_payload(rest);
    for idx in target_indexes {
        let entry = graph.edge_styles.entry(idx).or_default();
        if delta.stroke.is_some() {
            entry.stroke = delta.stroke;
        }
        if delta.color.is_some() {
            entry.color = delta.color;
        }
    }
}

/// Parse a `classDef name fill:#…,stroke:#…,color:#…` directive,
/// inserting the parsed [`NodeStyle`] into `graph.class_defs`.
/// Last-wins on duplicate names, matching Mermaid.
pub(crate) fn parse_class_def_directive(stmt: &str, graph: &mut Graph) {
    let mut parts = stmt.splitn(3, char::is_whitespace);
    let _ = parts.next(); // "classDef"
    let Some(name) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    let payload = parts.next().unwrap_or("");
    let style = parse_node_style_payload(payload);
    graph.class_defs.insert(name.to_string(), style);
}

/// Parse a `class id1,id2,id3 className` directive, pushing one
/// `(id, class_name)` pair per listed id onto `pending_classes`. The
/// pending list is resolved at end-of-parse via
/// [`apply_pending_classes`] so forward references (`class A foo`
/// before `classDef foo …`) work.
pub(crate) fn parse_class_directive(stmt: &str, pending_classes: &mut Vec<(String, String)>) {
    let mut parts = stmt.splitn(3, char::is_whitespace);
    let _ = parts.next(); // "class"
    let Some(ids_part) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    let Some(class_name) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    for id in ids_part.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        pending_classes.push((id.to_string(), class_name.to_string()));
    }
}

/// Resolve `(target_id, class_name)` pairs into concrete style entries
/// on `graph.node_styles` or `graph.subgraph_styles`. Multiple classes
/// per target stack via [`merge_node_style`] in source order. Class
/// names without a matching `classDef` are silently ignored.
pub(crate) fn apply_pending_classes(graph: &mut Graph, pending: &[(String, String)]) {
    let subgraph_ids: std::collections::HashSet<String> =
        graph.subgraphs.iter().map(|s| s.id.clone()).collect();
    for (target, class_name) in pending {
        let Some(overlay) = graph.class_defs.get(class_name).copied() else {
            continue;
        };
        let target_map = if subgraph_ids.contains(target) {
            &mut graph.subgraph_styles
        } else {
            &mut graph.node_styles
        };
        let base = target_map.get(target).copied().unwrap_or_default();
        target_map.insert(target.clone(), merge_node_style(base, overlay));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_inline_comment_outside_quotes() {
        assert_eq!(strip_inline_comment("foo %% bar"), "foo ");
        assert_eq!(strip_inline_comment("foo"), "foo");
    }

    #[test]
    fn strip_inline_comment_preserves_quoted_percent() {
        assert_eq!(
            strip_inline_comment(r#"state "A %% B" as X"#),
            r#"state "A %% B" as X"#
        );
    }

    #[test]
    fn matches_keyword_recognises_word_followed_by_space_or_colon() {
        assert!(matches_keyword("classDef foo …", "classDef"));
        assert!(matches_keyword("accTitle: hi", "accTitle"));
        assert!(matches_keyword("end note", "end"));
        assert!(!matches_keyword("classDeffoo", "classDef"));
    }

    #[test]
    fn parse_node_style_payload_recognised_keys() {
        let s = parse_node_style_payload("fill:#336,stroke:#fff,color:#000");
        assert_eq!(s.fill, Some(Rgb(0x33, 0x33, 0x66)));
        assert_eq!(s.stroke, Some(Rgb(0xff, 0xff, 0xff)));
        assert_eq!(s.color, Some(Rgb(0, 0, 0)));
    }

    #[test]
    fn parse_node_style_payload_ignores_unknown_keys_and_bad_hex() {
        let s = parse_node_style_payload("font-size:14,fill:#zzz,stroke:#abc");
        assert_eq!(s.fill, None);
        assert_eq!(s.stroke, Some(Rgb(0xaa, 0xbb, 0xcc)));
    }

    #[test]
    fn parse_edge_color_payload_only_picks_edge_keys() {
        let c = parse_edge_color_payload("stroke:#f00,color:#fff,fill:#000");
        assert_eq!(c.stroke, Some(Rgb(0xff, 0, 0)));
        assert_eq!(c.color, Some(Rgb(0xff, 0xff, 0xff)));
        // fill is silently dropped — edges have no interior to fill.
    }

    #[test]
    fn extract_class_modifier_no_modifier() {
        let (id, classes) = extract_class_modifier("A");
        assert_eq!(id, "A");
        assert!(classes.is_empty());
    }

    #[test]
    fn extract_class_modifier_single_class() {
        let (id, classes) = extract_class_modifier("A:::cache");
        assert_eq!(id, "A");
        assert_eq!(classes, vec!["cache"]);
    }

    #[test]
    fn extract_class_modifier_multiple_classes_preserve_order() {
        let (id, classes) = extract_class_modifier("A:::first:::second:::third");
        assert_eq!(id, "A");
        assert_eq!(classes, vec!["first", "second", "third"]);
    }

    #[test]
    fn extract_class_modifier_keeps_shape_brackets() {
        let (id, classes) = extract_class_modifier("A[Label]:::cache");
        assert_eq!(id, "A[Label]");
        assert_eq!(classes, vec!["cache"]);
    }

    #[test]
    fn extract_class_modifier_handles_star_marker() {
        // `[*]` is the start/end marker; the modifier strips off but
        // the marker is preserved verbatim for the caller to mangle.
        let (id, classes) = extract_class_modifier("[*]:::started");
        assert_eq!(id, "[*]");
        assert_eq!(classes, vec!["started"]);
    }

    #[test]
    fn extract_class_modifier_invalid_suffix_is_ignored() {
        // A `:::` followed by a space or punctuation isn't a class
        // shorthand — leave the token alone.
        let (id, classes) = extract_class_modifier("A:::not a class");
        assert_eq!(id, "A:::not a class");
        assert!(classes.is_empty());
    }

    #[test]
    fn merge_node_style_overlay_wins_on_present_fields() {
        let base = NodeStyle {
            fill: Some(Rgb(1, 1, 1)),
            stroke: Some(Rgb(2, 2, 2)),
            color: None,
        };
        let overlay = NodeStyle {
            fill: Some(Rgb(9, 9, 9)),
            stroke: None,
            color: Some(Rgb(5, 5, 5)),
        };
        let merged = merge_node_style(base, overlay);
        assert_eq!(merged.fill, Some(Rgb(9, 9, 9))); // overlay wins
        assert_eq!(merged.stroke, Some(Rgb(2, 2, 2))); // base preserved
        assert_eq!(merged.color, Some(Rgb(5, 5, 5))); // overlay supplies
    }

    #[test]
    fn merge_node_style_default_overlay_preserves_base() {
        let base = NodeStyle {
            fill: Some(Rgb(1, 2, 3)),
            stroke: None,
            color: None,
        };
        let merged = merge_node_style(base, NodeStyle::default());
        assert_eq!(merged.fill, Some(Rgb(1, 2, 3)));
    }
}
