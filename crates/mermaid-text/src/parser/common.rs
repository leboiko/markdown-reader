//! Helpers shared by the flowchart and state-diagram parsers.
//!
//! Centralises the small text-processing primitives (comment stripping,
//! keyword matching, `key:value,…` payload parsing, `:::className`
//! shorthand extraction, NodeStyle merging) so the two parsers don't
//! drift their own copies. Each helper is `pub(crate)` — these are
//! parser-internal building blocks, not public crate API.

use crate::sequence::{BlockKind, NoteAnchor};
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

/// Strip a case-insensitive keyword prefix that is followed by at least
/// one whitespace character. Returns the trimmed remainder, or `None`
/// if the prefix doesn't match or there's no whitespace after it.
///
/// Used in both the sequence and state parsers for `participant <Id>`,
/// `actor <Id>`, `as <Id>`, `note left of <Id>`, etc. Lifted into
/// `common` in 0.9.0 to eliminate the duplicate definitions that had
/// drifted slightly (the sequence version used `eq_ignore_ascii_case`,
/// the state version did `to_lowercase().starts_with` which allocates
/// per call). This canonical implementation is the ASCII-fast one.
pub(crate) fn strip_keyword_prefix<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let len = keyword.len();
    if line.len() > len
        && line[..len].eq_ignore_ascii_case(keyword)
        && line.as_bytes()[len].is_ascii_whitespace()
    {
        Some(line[len..].trim())
    } else {
        None
    }
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

/// Position of a state-diagram note relative to its anchor.
///
/// The renderer encodes this as edge direction so the existing
/// layered layout places the note on the appropriate side of the
/// anchor without needing dedicated layout machinery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoteSide {
    /// `note left of X` — note becomes upstream layer (edge: note → X).
    Left,
    /// `note right of X` — note becomes downstream layer (edge: X → note).
    Right,
    /// `note over X` — defaults to the same direction as `Right` for v1
    /// (the layered layout has no concept of "over"; the note sits
    /// adjacent to the anchor on the flow axis).
    Over,
}

/// Parse the anchor specifier of a `note <left|right|over> of <Id>`
/// directive — the part after the `note ` keyword and before the
/// optional `: text`. Returns `Some((side, anchor_id))` on success,
/// `None` for unrecognised forms (including the floating
/// `note "text" as Id` which we don't support).
///
/// Mermaid's grammar uses `note over X` (no `of`) but `note left of X`
/// / `note right of X` (with `of`). Both forms are handled here.
///
/// Defensive: an `over X,Y` multi-anchor form returns the comma-laden
/// id verbatim — the caller should reject it before synthesising
/// edges (no real state will ever have `,` in its id).
pub(crate) fn parse_note_anchor(s: &str) -> Option<(NoteSide, String)> {
    let s = s.trim();
    let (side_word, rest) = s.split_once(char::is_whitespace)?;
    let side = match side_word {
        "left" => NoteSide::Left,
        "right" => NoteSide::Right,
        "over" => NoteSide::Over,
        _ => return None,
    };
    let rest = rest.trim();
    // `left`/`right` require `of <Id>`; `over` accepts either
    // `over <Id>` (Mermaid's actual syntax) or `over of <Id>`.
    let id_str = if let Some(stripped) = rest.strip_prefix("of ") {
        stripped.trim()
    } else if matches!(side, NoteSide::Over) {
        rest
    } else {
        return None;
    };
    if id_str.is_empty() {
        return None;
    }
    Some((side, id_str.to_string()))
}

/// Parse the anchor portion of a sequence-diagram `note` directive
/// — the part between `note ` and the colon. Returns the parsed
/// [`NoteAnchor`] (which can be a single-anchor [`NoteAnchor::Over`]
/// / `LeftOf` / `RightOf` or the multi-anchor [`NoteAnchor::OverPair`]).
///
/// Sister of [`parse_note_anchor`] (state-diagram only); kept separate
/// because state diagrams have no `over X,Y` form and the return
/// types differ. The two helpers share the keyword-recognition
/// pattern but produce different shapes; collapsing them into one
/// would require a flags argument and a tagged-union return that
/// adds more complexity than the modest duplication.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(parse_sequence_note_anchor("left of A"),
///            Some(NoteAnchor::LeftOf("A".to_string())));
/// assert_eq!(parse_sequence_note_anchor("over A,B"),
///            Some(NoteAnchor::OverPair("A".to_string(), "B".to_string())));
/// ```
pub(crate) fn parse_sequence_note_anchor(s: &str) -> Option<NoteAnchor> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("left of ") {
        let id = rest.trim();
        if id.is_empty() {
            return None;
        }
        return Some(NoteAnchor::LeftOf(id.to_string()));
    }
    if let Some(rest) = s.strip_prefix("right of ") {
        let id = rest.trim();
        if id.is_empty() {
            return None;
        }
        return Some(NoteAnchor::RightOf(id.to_string()));
    }
    if let Some(rest) = s.strip_prefix("over ") {
        let body = rest.trim();
        if body.is_empty() {
            return None;
        }
        if let Some((a, b)) = body.split_once(',') {
            let a = a.trim();
            let b = b.trim();
            if a.is_empty() || b.is_empty() {
                return None;
            }
            return Some(NoteAnchor::OverPair(a.to_string(), b.to_string()));
        }
        return Some(NoteAnchor::Over(body.to_string()));
    }
    None
}

/// Peel a leading `+` or `-` from an identifier token. Used by the
/// sequence-diagram parser to recognise the inline activation
/// shorthand on a message target: `A->>+B` (activate target) and
/// `A-->>-B` (deactivate, applied per `Activation`'s docs to the
/// SOURCE — preserves the call/reply pattern `A->>+B; B-->>-A`).
///
/// Returns `(stripped_id, marker)` where `marker` is `Some(true)` for
/// `+`, `Some(false)` for `-`, and `None` for no marker. The stripped
/// id has surrounding whitespace removed so the caller can use it
/// directly as a participant id.
pub(crate) fn strip_activation_marker(token: &str) -> (String, Option<bool>) {
    let t = token.trim_start();
    if let Some(rest) = t.strip_prefix('+') {
        (rest.trim().to_string(), Some(true))
    } else if let Some(rest) = t.strip_prefix('-') {
        (rest.trim().to_string(), Some(false))
    } else {
        (t.trim().to_string(), None)
    }
}

/// Map a sequence-diagram block-opener keyword to its [`BlockKind`].
/// Used by the sequence parser to recognise `loop` / `alt` / `opt` /
/// `par` / `critical` / `break` openers. Returns `None` for any other
/// token so callers can dispatch with `if let Some(kind) = …`.
pub(crate) fn block_kind_from_keyword(s: &str) -> Option<BlockKind> {
    match s {
        "loop" => Some(BlockKind::Loop),
        "alt" => Some(BlockKind::Alt),
        "opt" => Some(BlockKind::Opt),
        "par" => Some(BlockKind::Par),
        "critical" => Some(BlockKind::Critical),
        "break" => Some(BlockKind::Break),
        _ => None,
    }
}

/// The continuation keyword (if any) that opens an additional branch
/// for the given block kind. `Alt → "else"`, `Par → "and"`,
/// `Critical → "option"`. Single-branch kinds (`Loop`, `Opt`, `Break`)
/// return `None` — encountering their continuation keyword inside any
/// such block is a parse error.
pub(crate) fn continuation_keyword_for(kind: BlockKind) -> Option<&'static str> {
    match kind {
        BlockKind::Alt => Some("else"),
        BlockKind::Par => Some("and"),
        BlockKind::Critical => Some("option"),
        BlockKind::Loop | BlockKind::Opt | BlockKind::Break => None,
    }
}

/// Resolve `(target_id, class_name)` pairs into concrete style entries
/// on `graph.node_styles` or `graph.subgraph_styles`. Multiple classes
/// per target stack via [`merge_node_style`] in source order. Class
/// names without a matching `classDef` are silently ignored.
///
/// After all explicit class assignments are resolved, if the diagram defines
/// a `classDef DEFAULT …`, that style is applied as a **base** to every node
/// and subgraph in the graph. Explicit class styles (applied above) are then
/// merged on top of DEFAULT, so they win on any property they define while
/// still inheriting any property DEFAULT supplies that they don't.
///
/// This matches Mermaid's reference behaviour: `DEFAULT` is a special sentinel
/// that acts as a universal base class rather than just another named class.
pub(crate) fn apply_pending_classes(graph: &mut Graph, pending: &[(String, String)]) {
    let subgraph_ids: std::collections::HashSet<String> =
        graph.subgraphs.iter().map(|s| s.id.clone()).collect();

    // Pass 1: apply all explicitly-listed class assignments in source order.
    for (target, class_name) in pending {
        // Skip the special DEFAULT sentinel here — it is handled in Pass 2.
        if class_name == "DEFAULT" {
            continue;
        }
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

    // Pass 2: apply `DEFAULT` as the universal base class.
    //
    // Mermaid treats `classDef DEFAULT` as a baseline that is merged under
    // every other class definition and applied to every node not otherwise
    // styled. We implement this by:
    // 1. For each node/subgraph that already has an explicit style (from
    //    Pass 1 or a `style` directive), merge DEFAULT under it so the
    //    explicit properties still win.
    // 2. For each node/subgraph with no style entry yet, insert DEFAULT's
    //    style directly.
    let Some(default_style) = graph.class_defs.get("DEFAULT").copied() else {
        // No DEFAULT defined — nothing to do (preserves pre-existing behaviour).
        return;
    };

    // Collect the IDs we need to visit so we can mutate the style maps after.
    let node_ids: Vec<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    let sg_ids: Vec<String> = graph.subgraphs.iter().map(|s| s.id.clone()).collect();

    for id in node_ids {
        // Merge DEFAULT under the existing explicit style: `merge_node_style(base, overlay)`
        // returns `overlay.field.or(base.field)`, so existing explicit values win.
        let explicit = graph.node_styles.get(&id).copied().unwrap_or_default();
        // default_style is the base; explicit is the overlay (overlay wins on conflict).
        let merged = merge_node_style(default_style, explicit);
        graph.node_styles.insert(id, merged);
    }

    for id in sg_ids {
        let explicit = graph.subgraph_styles.get(&id).copied().unwrap_or_default();
        let merged = merge_node_style(default_style, explicit);
        graph.subgraph_styles.insert(id, merged);
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
    fn strip_keyword_prefix_basic() {
        assert_eq!(
            strip_keyword_prefix("note left of A", "note"),
            Some("left of A")
        );
        assert_eq!(
            strip_keyword_prefix("Note left of A", "note"),
            Some("left of A")
        );
        assert_eq!(strip_keyword_prefix("note", "note"), None); // no whitespace after
        assert_eq!(strip_keyword_prefix("notes", "note"), None); // not whitespace
        assert_eq!(strip_keyword_prefix("class A foo", "class"), Some("A foo"));
        assert_eq!(strip_keyword_prefix("", "note"), None);
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

    // ---- parse_note_anchor --------------------------------------------

    #[test]
    fn parse_note_anchor_left_of() {
        assert_eq!(
            parse_note_anchor("left of MyState"),
            Some((NoteSide::Left, "MyState".to_string()))
        );
    }

    #[test]
    fn parse_note_anchor_right_of() {
        assert_eq!(
            parse_note_anchor("right of OPEN"),
            Some((NoteSide::Right, "OPEN".to_string()))
        );
    }

    #[test]
    fn parse_note_anchor_over_no_of() {
        // Mermaid's actual syntax is `note over X` (no `of`).
        assert_eq!(
            parse_note_anchor("over Active"),
            Some((NoteSide::Over, "Active".to_string()))
        );
    }

    #[test]
    fn parse_note_anchor_left_without_of_is_rejected() {
        assert_eq!(parse_note_anchor("left X"), None);
    }

    #[test]
    fn parse_note_anchor_floating_note_form_returns_none() {
        // `note "text" as N1` is the floating form — we don't support
        // it. The caller will silently skip when this returns None.
        assert_eq!(parse_note_anchor("\"some text\" as N1"), None);
    }

    #[test]
    fn parse_note_anchor_empty_id_returns_none() {
        assert_eq!(parse_note_anchor("left of "), None);
        assert_eq!(parse_note_anchor("over"), None);
    }

    // ---- parse_sequence_note_anchor (sequence-diagram form) ----------

    #[test]
    fn parse_sequence_note_anchor_left_of() {
        assert_eq!(
            parse_sequence_note_anchor("left of Alice"),
            Some(NoteAnchor::LeftOf("Alice".to_string()))
        );
    }

    #[test]
    fn parse_sequence_note_anchor_right_of() {
        assert_eq!(
            parse_sequence_note_anchor("right of Bob"),
            Some(NoteAnchor::RightOf("Bob".to_string()))
        );
    }

    #[test]
    fn parse_sequence_note_anchor_over_single() {
        assert_eq!(
            parse_sequence_note_anchor("over Alice"),
            Some(NoteAnchor::Over("Alice".to_string()))
        );
    }

    #[test]
    fn parse_sequence_note_anchor_over_pair() {
        assert_eq!(
            parse_sequence_note_anchor("over Alice,Bob"),
            Some(NoteAnchor::OverPair("Alice".to_string(), "Bob".to_string()))
        );
        // Whitespace around the comma is tolerated.
        assert_eq!(
            parse_sequence_note_anchor("over Alice , Bob"),
            Some(NoteAnchor::OverPair("Alice".to_string(), "Bob".to_string()))
        );
    }

    #[test]
    fn parse_sequence_note_anchor_invalid_returns_none() {
        assert_eq!(parse_sequence_note_anchor("left A"), None); // missing `of`
        assert_eq!(parse_sequence_note_anchor("over"), None); // empty body
        assert_eq!(parse_sequence_note_anchor(""), None);
        assert_eq!(parse_sequence_note_anchor("over Alice,"), None); // empty pair half
    }

    // ---- strip_activation_marker (sequence-diagram inline +/- shorthand) -

    #[test]
    fn strip_activation_marker_plus() {
        assert_eq!(strip_activation_marker("+B"), ("B".to_string(), Some(true)));
        // Whitespace inside `+ B` is tolerated; the id trims clean.
        assert_eq!(
            strip_activation_marker("+ B"),
            ("B".to_string(), Some(true))
        );
    }

    #[test]
    fn strip_activation_marker_minus() {
        assert_eq!(
            strip_activation_marker("-Alice"),
            ("Alice".to_string(), Some(false))
        );
    }

    #[test]
    fn strip_activation_marker_no_marker() {
        assert_eq!(strip_activation_marker("B"), ("B".to_string(), None));
        assert_eq!(strip_activation_marker("  B  "), ("B".to_string(), None));
        assert_eq!(strip_activation_marker(""), (String::new(), None));
    }

    // ---- block helpers (sequence-diagram block statements) ------------

    #[test]
    fn block_kind_from_keyword_recognises_all_kinds() {
        assert_eq!(block_kind_from_keyword("loop"), Some(BlockKind::Loop));
        assert_eq!(block_kind_from_keyword("alt"), Some(BlockKind::Alt));
        assert_eq!(block_kind_from_keyword("opt"), Some(BlockKind::Opt));
        assert_eq!(block_kind_from_keyword("par"), Some(BlockKind::Par));
        assert_eq!(
            block_kind_from_keyword("critical"),
            Some(BlockKind::Critical)
        );
        assert_eq!(block_kind_from_keyword("break"), Some(BlockKind::Break));
        assert_eq!(block_kind_from_keyword("else"), None); // continuation, not opener
        assert_eq!(block_kind_from_keyword("end"), None);
        assert_eq!(block_kind_from_keyword(""), None);
    }

    #[test]
    fn continuation_keyword_for_multi_branch_kinds() {
        assert_eq!(continuation_keyword_for(BlockKind::Alt), Some("else"));
        assert_eq!(continuation_keyword_for(BlockKind::Par), Some("and"));
        assert_eq!(
            continuation_keyword_for(BlockKind::Critical),
            Some("option")
        );
        assert_eq!(continuation_keyword_for(BlockKind::Loop), None);
        assert_eq!(continuation_keyword_for(BlockKind::Opt), None);
        assert_eq!(continuation_keyword_for(BlockKind::Break), None);
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

    // ---- classDef DEFAULT special semantics ---------------------------------

    /// A single unstyled node picks up `classDef DEFAULT fill:red`.
    #[test]
    fn default_classdef_merges_into_unstyled_node() {
        // Build a minimal Graph with one node and a DEFAULT class.
        let mut graph = crate::types::Graph::new(crate::types::Direction::LeftToRight);
        graph.nodes.push(crate::types::Node::new(
            "A",
            "A",
            crate::types::NodeShape::Rectangle,
        ));
        graph.class_defs.insert(
            "DEFAULT".to_string(),
            NodeStyle {
                fill: Some(Rgb(0xff, 0, 0)),
                ..Default::default()
            },
        );

        // No explicit class assignments — pending is empty.
        apply_pending_classes(&mut graph, &[]);

        let style = graph.node_styles.get("A").copied().unwrap_or_default();
        assert_eq!(
            style.fill,
            Some(Rgb(0xff, 0, 0)),
            "unstyled node must inherit DEFAULT fill"
        );
    }

    /// An explicitly classed node gets DEFAULT as base, explicit class wins on conflict.
    #[test]
    fn default_classdef_merges_under_explicit_class() {
        // classDef DEFAULT fill:#eee, stroke:#999
        // classDef important fill:#f00
        // node A:::important  => fill:#f00 (important wins), stroke:#999 (DEFAULT inherited)
        let mut graph = crate::types::Graph::new(crate::types::Direction::LeftToRight);
        graph.nodes.push(crate::types::Node::new(
            "A",
            "A",
            crate::types::NodeShape::Rectangle,
        ));
        graph.class_defs.insert(
            "DEFAULT".to_string(),
            NodeStyle {
                fill: Some(Rgb(0xee, 0xee, 0xee)),
                stroke: Some(Rgb(0x99, 0x99, 0x99)),
                color: None,
            },
        );
        graph.class_defs.insert(
            "important".to_string(),
            NodeStyle {
                fill: Some(Rgb(0xff, 0, 0)),
                stroke: None,
                color: None,
            },
        );

        apply_pending_classes(&mut graph, &[("A".to_string(), "important".to_string())]);

        let style = graph.node_styles.get("A").copied().unwrap_or_default();
        assert_eq!(
            style.fill,
            Some(Rgb(0xff, 0, 0)),
            "explicit class fill must win over DEFAULT"
        );
        assert_eq!(
            style.stroke,
            Some(Rgb(0x99, 0x99, 0x99)),
            "DEFAULT stroke must be inherited when explicit class doesn't define it"
        );
    }

    /// When no DEFAULT is defined, unstyled nodes remain unstyled.
    #[test]
    fn default_classdef_does_not_apply_when_absent() {
        let mut graph = crate::types::Graph::new(crate::types::Direction::LeftToRight);
        graph.nodes.push(crate::types::Node::new(
            "A",
            "A",
            crate::types::NodeShape::Rectangle,
        ));
        // Intentionally no DEFAULT in class_defs.

        apply_pending_classes(&mut graph, &[]);

        assert!(
            !graph.node_styles.contains_key("A"),
            "no DEFAULT means no style entry for unstyled node"
        );
    }

    /// State diagrams use the same apply_pending_classes path — verify that
    /// classDef DEFAULT propagates to state-diagram nodes identically.
    #[test]
    fn default_classdef_works_in_state_diagrams() {
        // State diagrams go through the same apply_pending_classes helper.
        // We can test this by simulating what the state-diagram parser does:
        // build a Graph with nodes representing states and a DEFAULT classDef.
        let mut graph = crate::types::Graph::new(crate::types::Direction::TopToBottom);
        graph.nodes.push(crate::types::Node::new(
            "Idle",
            "Idle",
            crate::types::NodeShape::Rounded,
        ));
        graph.nodes.push(crate::types::Node::new(
            "Working",
            "Working",
            crate::types::NodeShape::Rounded,
        ));
        graph.class_defs.insert(
            "DEFAULT".to_string(),
            NodeStyle {
                stroke: Some(Rgb(0x99, 0xcc, 0xff)),
                ..Default::default()
            },
        );

        apply_pending_classes(&mut graph, &[]);

        // Both states must pick up DEFAULT's stroke.
        assert_eq!(
            graph.node_styles.get("Idle").and_then(|s| s.stroke),
            Some(Rgb(0x99, 0xcc, 0xff)),
            "state Idle must inherit DEFAULT stroke"
        );
        assert_eq!(
            graph.node_styles.get("Working").and_then(|s| s.stroke),
            Some(Rgb(0x99, 0xcc, 0xff)),
            "state Working must inherit DEFAULT stroke"
        );
    }
}
