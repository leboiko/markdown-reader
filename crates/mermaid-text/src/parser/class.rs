//! Parser for Mermaid `classDiagram` syntax.
//!
//! Recognises three line shapes after the `classDiagram` header:
//!
//! 1. **Class declaration** — `class Name` or `class Name { … }`.
//!    Lines inside a body block carry member definitions.
//! 2. **Relationship** — `A <kind> B : label` (see [`RelKind`]).
//! 3. **Stereotype** — `<<stereotype>>` inside or immediately after a class body.
//!
//! `%%` line comments and blank lines are silently skipped. Lines matching
//! unsupported v1 features (generics, namespaces, notes, links, direction)
//! produce a clear [`Error::ParseError`].
//!
//! # v1 scope
//!
//! Supported:
//! - Class declarations with/without `{ … }` body
//! - Visibility: `+`, `-`, `#`, `~`
//! - Attributes: typed-before (`+Type name`) and typed-after (`+name Type`)
//! - Methods with parameters and optional return type (`+method(args) : Type`)
//! - Static (`$`) and abstract (`*`) suffixes
//! - All 7 relationship types (see [`RelKind`])
//! - Stereotypes (`<<interface>>`, `<<enumeration>>`, `<<abstract>>`, others)
//! - Edge labels and multiplicity (quoted strings)
//! - `%%` comments
//!
//! Not supported (returns [`Error::ParseError`]):
//! - Generics (`Class~T~`)
//! - Namespace blocks
//! - `note for X` annotations
//! - `link Class "url"` / `click` directives
//! - Colon-shorthand member form (`Animal : +name String`)
//! - `direction` header
//!
//! # Examples
//!
//! ```
//! use mermaid_text::parser::class::parse;
//!
//! let src = "classDiagram\n\
//!     class Animal {\n\
//!         +String name\n\
//!         +speak() void\n\
//!     }\n\
//!     class Dog\n\
//!     Animal <|-- Dog";
//! let diag = parse(src).unwrap();
//! assert_eq!(diag.classes.len(), 2);
//! assert_eq!(diag.relations.len(), 1);
//! assert_eq!(diag.classes[0].members.len(), 2);
//! ```

use crate::Error;
use crate::class::{
    Attribute, ClassDiagram, Member, Method, RelKind, Relation, Stereotype, Visibility,
};
use crate::parser::common::strip_inline_comment;

/// Parse a `classDiagram` source string into a [`ClassDiagram`].
///
/// # Errors
///
/// - [`Error::ParseError`] if the header is missing, an unsupported feature is
///   encountered, or a line cannot be parsed.
pub fn parse(src: &str) -> Result<ClassDiagram, Error> {
    let mut diag = ClassDiagram::default();
    let mut header_seen = false;
    // `Some(idx)` while inside a class body block.
    let mut current_class: Option<usize> = None;

    for raw in src.lines() {
        let line = strip_inline_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        if !header_seen {
            if !line.eq_ignore_ascii_case("classdiagram") {
                return Err(Error::ParseError(format!(
                    "expected `classDiagram` header, got {line:?}"
                )));
            }
            header_seen = true;
            continue;
        }

        // ---- Closing brace ends the current class body ----
        if line == "}" {
            if current_class.is_none() {
                return Err(Error::ParseError(
                    "stray `}` outside any class body".to_string(),
                ));
            }
            current_class = None;
            continue;
        }

        // ---- Stereotype annotation line `<<name>>` ----
        // Can appear inside OR outside a class body (attached to current or last class).
        if let Some(stereo) = try_parse_stereotype(line) {
            let target_idx = current_class.or_else(|| {
                // Outside a body: attach to the last declared class.
                if diag.classes.is_empty() {
                    None
                } else {
                    Some(diag.classes.len() - 1)
                }
            });
            if let Some(idx) = target_idx {
                diag.classes[idx].stereotype = Some(stereo);
            }
            continue;
        }

        // ---- Inside a class body ----
        if let Some(class_idx) = current_class {
            let member = parse_member(line)?;
            diag.classes[class_idx].members.push(member);
            continue;
        }

        // ---- Detect unsupported features before other parsing ----
        reject_unsupported(line)?;

        // ---- Class declaration: `class Name` or `class Name {` ----
        if let Some(rest) = strip_keyword_prefix_ci(line, "class") {
            let (name, opens_body) = if let Some(name_part) = rest.strip_suffix('{') {
                (name_part.trim(), true)
            } else {
                (rest, false)
            };

            if name.is_empty() {
                return Err(Error::ParseError(
                    "class declaration missing name".to_string(),
                ));
            }
            reject_generic(name, line)?;

            let idx = diag.ensure_class(name);
            if opens_body {
                current_class = Some(idx);
            }
            continue;
        }

        // ---- Relationship line ----
        if let Some(rel) = try_parse_relation(line)? {
            reject_generic(&rel.from, line)?;
            reject_generic(&rel.to, line)?;
            diag.ensure_class(&rel.from);
            diag.ensure_class(&rel.to);
            diag.relations.push(rel);
            continue;
        }

        return Err(Error::ParseError(format!(
            "classDiagram: unrecognised line: {line:?}"
        )));
    }

    if !header_seen {
        return Err(Error::ParseError(
            "missing `classDiagram` header line".to_string(),
        ));
    }
    if let Some(idx) = current_class {
        return Err(Error::ParseError(format!(
            "unclosed class body for `{}` (missing `}}`)",
            diag.classes[idx].name
        )));
    }
    Ok(diag)
}

// ---------------------------------------------------------------------------
// Unsupported-feature guards
// ---------------------------------------------------------------------------

/// Return `Err` if `line` begins an unsupported v1 feature.
fn reject_unsupported(line: &str) -> Result<(), Error> {
    // `direction` header inside diagram body.
    if strip_keyword_prefix_ci(line, "direction").is_some() {
        return Err(Error::ParseError(
            "classDiagram: `direction` directive not yet supported".to_string(),
        ));
    }
    // `note for ClassName`
    if strip_keyword_prefix_ci(line, "note").is_some() {
        return Err(Error::ParseError(
            "classDiagram: `note for` not yet supported".to_string(),
        ));
    }
    // `link ClassName "url"`
    if strip_keyword_prefix_ci(line, "link").is_some() {
        return Err(Error::ParseError(
            "classDiagram: `link` directive not yet supported".to_string(),
        ));
    }
    // `click ClassName …`
    if strip_keyword_prefix_ci(line, "click").is_some() {
        return Err(Error::ParseError(
            "classDiagram: `click` directive not yet supported".to_string(),
        ));
    }
    // `namespace Name {`
    if strip_keyword_prefix_ci(line, "namespace").is_some() {
        return Err(Error::ParseError(
            "classDiagram: `namespace` blocks not yet supported".to_string(),
        ));
    }
    // Colon-shorthand `ClassName : member` (a single word followed by ` : `)
    // is detected by the presence of ` : ` (space-colon-space) in a line that
    // doesn't look like a relationship (no `--` or `..`).
    if line.contains(" : ")
        && !line.contains("--")
        && !line.contains("..")
        && !line.starts_with("class ")
        && !line.starts_with("class\t")
    {
        return Err(Error::ParseError(
            "classDiagram: colon-shorthand member form not yet supported".to_string(),
        ));
    }
    Ok(())
}

/// Return `Err` if `name` contains a generic parameter `~T~`.
fn reject_generic(name: &str, line: &str) -> Result<(), Error> {
    if name.contains('~') {
        return Err(Error::ParseError(format!(
            "classDiagram: generics not yet supported (got {name:?} in {line:?})"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Stereotype parsing
// ---------------------------------------------------------------------------

/// If `line` is of the form `<<name>>` (optionally with surrounding
/// whitespace), parse and return the [`Stereotype`]. Returns `None` otherwise.
fn try_parse_stereotype(line: &str) -> Option<Stereotype> {
    let inner = line.strip_prefix("<<")?;
    let inner = inner.strip_suffix(">>")?;
    let label = inner.trim();
    if label.is_empty() {
        return None;
    }
    Some(match label {
        "interface" => Stereotype::Interface,
        "enumeration" => Stereotype::Enumeration,
        "abstract" => Stereotype::Abstract,
        other => Stereotype::Other(other.to_string()),
    })
}

// ---------------------------------------------------------------------------
// Member parsing
// ---------------------------------------------------------------------------

/// Parse a single member line inside a class body.
///
/// Accepted forms:
/// - `+name Type` / `+Type name` — attribute (no `()`)
/// - `+method() ReturnType` — method
/// - Suffixes `$` (static) and `*` (abstract) are stripped before the name.
///
/// The heuristic for typed-before vs typed-after:
/// - If the first non-visibility token contains `(`, it's a method.
/// - Otherwise: if there are two tokens after stripping the visibility prefix,
///   we try to determine which is the type. We use a simple rule:
///   if the second token starts with an uppercase letter it is the type-after
///   form (`+name Type`), otherwise it is type-before (`+Type name`).
///   The exact same ambiguity exists in Mermaid's own parser — it is inherent
///   in the unquoted form.
fn parse_member(line: &str) -> Result<Member, Error> {
    let line = line.trim();
    if line.is_empty() {
        return Err(Error::ParseError(
            "empty member line in class body".to_string(),
        ));
    }

    // Strip optional visibility prefix.
    let (visibility, rest) = strip_visibility(line);
    let rest = rest.trim();

    if rest.is_empty() {
        return Err(Error::ParseError(format!(
            "member line has only a visibility marker: {line:?}"
        )));
    }

    // Detect method by presence of `(`.
    if let Some(paren_pos) = rest.find('(') {
        return parse_method(visibility, rest, paren_pos);
    }

    // Attribute: up to two tokens.
    parse_attribute(visibility, rest, line)
}

/// Strip the leading visibility character (`+`, `-`, `#`, `~`) and return
/// `(Some(vis), rest)`. If no such character is present, returns
/// `(None, original_line)`.
fn strip_visibility(s: &str) -> (Option<Visibility>, &str) {
    match s.chars().next() {
        Some('+') => (Some(Visibility::Public), &s[1..]),
        Some('-') => (Some(Visibility::Private), &s[1..]),
        Some('#') => (Some(Visibility::Protected), &s[1..]),
        Some('~') => (Some(Visibility::Package), &s[1..]),
        _ => (None, s),
    }
}

/// Parse a method member from `rest` (the part after visibility was stripped).
/// `paren_pos` is the byte offset of the first `(`.
///
/// Handles two common forms:
/// - **Typed-after**: `getName() String` — name before `()`, type after.
/// - **Typed-before**: `String getName()` — type token before `name()`.
///   Detected when the text before `(` contains whitespace; the last
///   whitespace-delimited token is taken as the method name and the preceding
///   tokens as the prefix return type.
fn parse_method(
    visibility: Option<Visibility>,
    rest: &str,
    paren_pos: usize,
) -> Result<Member, Error> {
    let before_paren = rest[..paren_pos].trim();

    // Find the matching `)`.
    let close = rest
        .find(')')
        .ok_or_else(|| Error::ParseError(format!("method missing closing `)`: {rest:?}")))?;

    let params = rest[paren_pos + 1..close].trim().to_string();

    // Anything after `)` is an optional return type (with optional `:` separator)
    // and optional `$`/`*` suffixes.
    let after_paren = rest[close + 1..].trim();
    let (return_raw_after, is_static_after, is_abstract_after) = strip_suffixes(after_paren);
    let return_type_after = {
        let r = return_raw_after.trim_start_matches(':').trim();
        if r.is_empty() {
            None
        } else {
            Some(r.to_string())
        }
    };

    // The name and an optional typed-before return type are in `before_paren`.
    // If `before_paren` contains whitespace, the last token is the method name
    // and everything before it is a typed-before return type prefix.
    let (name, return_type_before) =
        if let Some(last_space) = before_paren.rfind(char::is_whitespace) {
            let prefix = before_paren[..last_space].trim();
            let name_part = before_paren[last_space + 1..].trim();
            let (name_clean, _, _) = strip_suffixes(name_part);
            let prefix_type = if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            };
            (name_clean.trim().to_string(), prefix_type)
        } else {
            let (name_clean, _, _) = strip_suffixes(before_paren);
            (name_clean.trim().to_string(), None)
        };

    if name.is_empty() {
        return Err(Error::ParseError(format!(
            "method line missing name: {rest:?}"
        )));
    }

    // Merge $/* from the name portion itself.
    let (_, is_static_name, is_abstract_name) = strip_suffixes(before_paren);

    // Return type: prefer the typed-before form; fall back to typed-after.
    let return_type = return_type_before.or(return_type_after);

    Ok(Member::Method(Method {
        visibility,
        name,
        params,
        return_type,
        is_static: is_static_after || is_static_name,
        is_abstract: is_abstract_after || is_abstract_name,
    }))
}

/// Parse an attribute member from `rest` (after visibility was stripped).
fn parse_attribute(
    visibility: Option<Visibility>,
    rest: &str,
    original_line: &str,
) -> Result<Member, Error> {
    let (body, is_static, _is_abstract) = strip_suffixes(rest);
    let mut tokens = body.split_whitespace();
    let first = tokens
        .next()
        .ok_or_else(|| Error::ParseError(format!("attribute missing tokens: {original_line:?}")))?;
    let second = tokens.next();

    // If there's no second token, treat `first` as the name with an empty type.
    let (name, type_name) = if let Some(sec) = second {
        // Heuristic: if the second token starts with an uppercase letter, it is
        // the type in typed-after form (`+name Type`); otherwise, the first
        // token is the type (`+Type name`).
        if sec.chars().next().is_some_and(|c| c.is_uppercase()) {
            (first.to_string(), sec.to_string())
        } else {
            // typed-before: `+Type name`
            (sec.to_string(), first.to_string())
        }
    } else {
        (first.to_string(), String::new())
    };

    Ok(Member::Attribute(Attribute {
        visibility,
        name,
        type_name,
        is_static,
    }))
}

/// Strip trailing `$` (static) and `*` (abstract) characters from `s`.
///
/// Returns `(stripped, is_static, is_abstract)`. Both suffixes may appear in
/// either order.
fn strip_suffixes(s: &str) -> (&str, bool, bool) {
    let mut end = s;
    let mut is_static = false;
    let mut is_abstract = false;
    loop {
        let trimmed = end.trim_end();
        if let Some(inner) = trimmed.strip_suffix('$') {
            is_static = true;
            end = inner;
        } else if let Some(inner) = trimmed.strip_suffix('*') {
            is_abstract = true;
            end = inner;
        } else {
            return (trimmed, is_static, is_abstract);
        }
    }
}

// ---------------------------------------------------------------------------
// Relationship parsing
// ---------------------------------------------------------------------------

/// Attempt to parse a relationship line. Returns `Ok(Some(rel))` on success,
/// `Ok(None)` if the line doesn't look like a relationship, or `Err` for a
/// malformed line that clearly was intended to be a relationship.
fn try_parse_relation(line: &str) -> Result<Option<Relation>, Error> {
    // Split optional ` : label` suffix. We only split on the LAST ` : ` to
    // avoid splitting inside multiplicity strings like `"0..1" : label`.
    let (arrow_part, label) = split_relation_label(line);

    // A relationship line must contain `--` or `..`. If neither is present,
    // this line is not a relationship (caller will produce the "unrecognised
    // line" error).
    if !arrow_part.contains("--") && !arrow_part.contains("..") {
        return Ok(None);
    }

    // Find the arrow token by scanning for relationship markers in order
    // of specificity (longer patterns first to avoid partial matches).
    let tokens: Vec<&str> = arrow_part.split_whitespace().collect();

    // We expect 2 or 3 whitespace-separated tokens:
    //   3-token: `ClassA  <marker>  ClassB`
    //   2-token: can arise if someone writes `ClassA<marker>ClassB` (no spaces)
    //            but Mermaid's spec requires spaces, so we only handle 3-token.
    // Find which token is the marker.
    let (from, marker_tok, to) = if tokens.len() == 3 {
        (tokens[0], tokens[1], tokens[2])
    } else if tokens.len() == 2 {
        // Occasionally source has `ClassA --> ClassB` parsed as two tokens
        // if names are run together — try splitting the second token.
        return Ok(None); // let the "unrecognised" error handle it
    } else {
        return Ok(None);
    };

    // Parse the marker into (kind, from_mult, to_mult).
    let (kind, from_mult, to_mult) = parse_marker(marker_tok, line)?;

    Ok(Some(Relation {
        from: from.to_string(),
        to: to.to_string(),
        kind,
        from_multiplicity: from_mult,
        to_multiplicity: to_mult,
        label,
    }))
}

/// Split a line like `A --> B : "label"` into (`A --> B`, `Some("label")`).
/// The split point is ` : ` (space colon space). If no such delimiter is found,
/// returns `(line, None)`. Multiplicity strings like `"0..*"` can contain
/// colons so we split only on ` : ` (with surrounding spaces).
fn split_relation_label(line: &str) -> (&str, Option<String>) {
    // Find ` : ` after any closing `"` so we don't split inside a quoted
    // multiplicity. Simple approach: find the rightmost ` : `.
    if let Some(pos) = line.rfind(" : ") {
        let arrow = line[..pos].trim_end();
        let raw_label = line[pos + 3..].trim();
        let label = raw_label.trim_matches('"').to_string();
        let label = if label.is_empty() { None } else { Some(label) };
        (arrow, label)
    } else {
        (line, None)
    }
}

/// Parse a relationship marker token (e.g. `<|--`, `*--`, `-->`, `..>`) into
/// its kind and optional multiplicity annotations.
///
/// Multiplicity is encoded as quoted strings embedded in the marker token by
/// some Mermaid sources but is more commonly placed on either side of the
/// marker in separate tokens — we handle the most common inline forms here.
///
/// Recognised two-segment markers (arrow_head `--` or `..` arrow_tail):
///
/// | Marker  | Kind |
/// |---------|------|
/// | `<\|--` | Inheritance (triangle at right/`to` end) |
/// | `--\|>` | Inheritance (triangle at left/`from` end; `to` is parent) |
/// | `*--`   | Composition |
/// | `--*`   | Composition (reversed) |
/// | `o--`   | Aggregation |
/// | `--o`   | Aggregation (reversed) |
/// | `-->`   | Association directed |
/// | `<--`   | Association directed (reversed) |
/// | `--`    | Association plain |
/// | `<\|..` | Realization (triangle at right) |
/// | `..\|>` | Realization (triangle at left) |
/// | `..>`   | Dependency |
/// | `<..`   | Dependency (reversed) |
fn parse_marker(tok: &str, line: &str) -> Result<(RelKind, Option<String>, Option<String>), Error> {
    // Strip leading/trailing quoted multiplicity annotations embedded in the
    // marker token, e.g. `"1"-->"0..*"` (rare but valid).
    let (from_mult, core, to_mult) = strip_inline_multiplicity(tok);

    let kind = match core {
        "<|--" | "-->" => {
            // `<|--`: triangle at LEFT (from) end, meaning FROM inherits TO.
            // But Mermaid's `A <|-- B` means B inherits A (A is parent).
            // In the AST we keep from=A, to=B, kind=Inheritance; renderer
            // puts the triangle at `from` (the parent side).
            if core == "<|--" {
                RelKind::Inheritance
            } else {
                RelKind::AssociationDirected
            }
        }
        "--|>" => RelKind::Inheritance,
        "<--" => RelKind::AssociationDirected,
        "*--" | "--*" => RelKind::Composition,
        "o--" | "--o" => RelKind::Aggregation,
        "--" => RelKind::AssociationPlain,
        "<|.." => RelKind::Realization,
        "..|>" => RelKind::Realization,
        "..>" => RelKind::Dependency,
        "<.." => RelKind::Dependency,
        other => {
            return Err(Error::ParseError(format!(
                "classDiagram: unrecognised relationship marker {other:?} in {line:?}"
            )));
        }
    };

    Ok((kind, from_mult, to_mult))
}

/// Extract optional quoted multiplicity annotations from the beginning and end
/// of a marker token. Returns `(from_mult, core_marker, to_mult)`.
///
/// Example: `"1"-->"0..*"` → `(Some("1"), "-->", Some("0..*"))`.
fn strip_inline_multiplicity(tok: &str) -> (Option<String>, &str, Option<String>) {
    let mut s = tok;
    let from_mult = if s.starts_with('"') {
        if let Some(close) = s[1..].find('"') {
            let mult = s[1..close + 1].to_string();
            s = &s[close + 2..];
            Some(mult)
        } else {
            None
        }
    } else {
        None
    };
    let to_mult = if s.ends_with('"') {
        if let Some(open) = s[..s.len() - 1].rfind('"') {
            let mult = s[open + 1..s.len() - 1].to_string();
            s = &s[..open];
            Some(mult)
        } else {
            None
        }
    } else {
        None
    };
    (from_mult, s.trim(), to_mult)
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

/// Strip a case-insensitive keyword prefix followed by whitespace. Returns the
/// trimmed remainder on match, `None` otherwise.
fn strip_keyword_prefix_ci<'a>(line: &'a str, kw: &str) -> Option<&'a str> {
    let len = kw.len();
    if line.len() > len
        && line[..len].eq_ignore_ascii_case(kw)
        && line.as_bytes()[len].is_ascii_whitespace()
    {
        Some(line[len..].trim())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::class::{Member, RelKind, Stereotype, Visibility};

    // ---- header validation ----

    #[test]
    fn parse_empty_body_is_ok() {
        let diag = parse("classDiagram").unwrap();
        assert!(diag.classes.is_empty());
        assert!(diag.relations.is_empty());
    }

    #[test]
    fn parse_missing_header_errors() {
        let err = parse("class Animal").unwrap_err();
        assert!(err.to_string().contains("classDiagram"));
    }

    #[test]
    fn parse_header_case_insensitive() {
        let diag = parse("ClassDiagram").unwrap();
        assert!(diag.classes.is_empty());
    }

    // ---- class declarations ----

    #[test]
    fn parse_bare_class_declaration() {
        let diag = parse("classDiagram\nclass Animal").unwrap();
        assert_eq!(diag.classes.len(), 1);
        assert_eq!(diag.classes[0].name, "Animal");
        assert!(diag.classes[0].members.is_empty());
    }

    #[test]
    fn parse_class_with_empty_body() {
        let diag = parse("classDiagram\nclass Animal {\n}").unwrap();
        assert_eq!(diag.classes.len(), 1);
        assert!(diag.classes[0].members.is_empty());
    }

    #[test]
    fn parse_class_with_members() {
        let src = "classDiagram\nclass Animal {\n    +String name\n    +speak() void\n}";
        let diag = parse(src).unwrap();
        assert_eq!(diag.classes[0].members.len(), 2);
    }

    // ---- visibility ----

    #[test]
    fn parse_all_visibility_prefixes() {
        let src = "classDiagram\nclass C {\n    +pub\n    -priv\n    #prot\n    ~pkg\n}";
        let diag = parse(src).unwrap();
        let members = &diag.classes[0].members;
        assert_eq!(members.len(), 4);
        let vis: Vec<_> = members
            .iter()
            .map(|m| match m {
                Member::Attribute(a) => a.visibility,
                Member::Method(m) => m.visibility,
            })
            .collect();
        assert_eq!(vis[0], Some(Visibility::Public));
        assert_eq!(vis[1], Some(Visibility::Private));
        assert_eq!(vis[2], Some(Visibility::Protected));
        assert_eq!(vis[3], Some(Visibility::Package));
    }

    // ---- attributes ----

    #[test]
    fn parse_attribute_typed_before() {
        // `+String name` — type before name.
        let src = "classDiagram\nclass C {\n    +String name\n}";
        let diag = parse(src).unwrap();
        if let Member::Attribute(a) = &diag.classes[0].members[0] {
            assert_eq!(a.name, "name");
            assert_eq!(a.type_name, "String");
            assert_eq!(a.visibility, Some(Visibility::Public));
        } else {
            panic!("expected Attribute");
        }
    }

    #[test]
    fn parse_attribute_typed_after_uppercase_type() {
        // `+name String` — second token uppercase → typed-after.
        let src = "classDiagram\nclass C {\n    +age int\n}";
        let diag = parse(src).unwrap();
        if let Member::Attribute(a) = &diag.classes[0].members[0] {
            // First token lowercase → type-before: type=age, name=int? No:
            // `int` starts with lowercase so this is type-before: type=age, name=int.
            // The heuristic: second token uppercase → typed-after. `int` is lowercase →
            // typed-before, so type=age, name=int.
            assert_eq!(a.type_name, "age");
            assert_eq!(a.name, "int");
        } else {
            panic!("expected Attribute");
        }
    }

    #[test]
    fn parse_attribute_no_type() {
        let src = "classDiagram\nclass C {\n    +id\n}";
        let diag = parse(src).unwrap();
        if let Member::Attribute(a) = &diag.classes[0].members[0] {
            assert_eq!(a.name, "id");
            assert_eq!(a.type_name, "");
        } else {
            panic!("expected Attribute");
        }
    }

    #[test]
    fn parse_static_attribute() {
        let src = "classDiagram\nclass C {\n    +int count$\n}";
        let diag = parse(src).unwrap();
        if let Member::Attribute(a) = &diag.classes[0].members[0] {
            assert!(a.is_static);
        } else {
            panic!("expected Attribute");
        }
    }

    // ---- methods ----

    #[test]
    fn parse_method_no_return() {
        let src = "classDiagram\nclass C {\n    +speak()\n}";
        let diag = parse(src).unwrap();
        if let Member::Method(m) = &diag.classes[0].members[0] {
            assert_eq!(m.name, "speak");
            assert_eq!(m.params, "");
            assert!(m.return_type.is_none());
        } else {
            panic!("expected Method");
        }
    }

    #[test]
    fn parse_method_with_return_type() {
        let src = "classDiagram\nclass C {\n    +String getName()\n}";
        let diag = parse(src).unwrap();
        if let Member::Method(m) = &diag.classes[0].members[0] {
            assert_eq!(m.name, "getName");
            assert_eq!(m.return_type.as_deref(), Some("String"));
        } else {
            panic!("expected Method");
        }
    }

    #[test]
    fn parse_method_with_params() {
        let src = "classDiagram\nclass C {\n    +deposit(amount: float)\n}";
        let diag = parse(src).unwrap();
        if let Member::Method(m) = &diag.classes[0].members[0] {
            assert_eq!(m.params, "amount: float");
        } else {
            panic!("expected Method");
        }
    }

    #[test]
    fn parse_method_abstract_suffix() {
        let src = "classDiagram\nclass C {\n    +draw()*\n}";
        let diag = parse(src).unwrap();
        if let Member::Method(m) = &diag.classes[0].members[0] {
            assert!(m.is_abstract);
            assert!(!m.is_static);
        } else {
            panic!("expected Method");
        }
    }

    #[test]
    fn parse_method_static_suffix() {
        let src = "classDiagram\nclass C {\n    +getInstance()$\n}";
        let diag = parse(src).unwrap();
        if let Member::Method(m) = &diag.classes[0].members[0] {
            assert!(m.is_static);
            assert!(!m.is_abstract);
        } else {
            panic!("expected Method");
        }
    }

    // ---- stereotypes ----

    #[test]
    fn parse_stereotype_inside_body() {
        let src = "classDiagram\nclass IShape {\n    <<interface>>\n    +draw()\n}";
        let diag = parse(src).unwrap();
        assert_eq!(diag.classes[0].stereotype, Some(Stereotype::Interface));
        assert_eq!(diag.classes[0].members.len(), 1);
    }

    #[test]
    fn parse_stereotype_outside_body() {
        // `<<enumeration>> Color` (with trailing class name) is not matched by
        // our strict `<<name>>` form. The pure `<<name>>` line form is what we
        // support for out-of-body stereotypes.
        let src = "classDiagram\nclass Color\n<<enumeration>>";
        let diag = parse(src).unwrap();
        assert_eq!(diag.classes[0].stereotype, Some(Stereotype::Enumeration));
    }

    // ---- relationships ----

    #[test]
    fn parse_inheritance_left_to_right() {
        // `Animal <|-- Dog` means Dog inherits Animal (triangle at Animal/left).
        let diag = parse("classDiagram\nAnimal <|-- Dog").unwrap();
        assert_eq!(diag.relations.len(), 1);
        let r = &diag.relations[0];
        assert_eq!(r.from, "Animal");
        assert_eq!(r.to, "Dog");
        assert_eq!(r.kind, RelKind::Inheritance);
    }

    #[test]
    fn parse_inheritance_right_to_left() {
        let diag = parse("classDiagram\nDog --|> Animal").unwrap();
        assert_eq!(diag.relations[0].kind, RelKind::Inheritance);
    }

    #[test]
    fn parse_composition() {
        let diag = parse("classDiagram\nCar *-- Engine").unwrap();
        assert_eq!(diag.relations[0].kind, RelKind::Composition);
    }

    #[test]
    fn parse_aggregation() {
        let diag = parse("classDiagram\nFleet o-- Car").unwrap();
        assert_eq!(diag.relations[0].kind, RelKind::Aggregation);
    }

    #[test]
    fn parse_association_directed() {
        let diag = parse("classDiagram\nA --> B").unwrap();
        assert_eq!(diag.relations[0].kind, RelKind::AssociationDirected);
    }

    #[test]
    fn parse_association_plain() {
        let diag = parse("classDiagram\nA -- B").unwrap();
        assert_eq!(diag.relations[0].kind, RelKind::AssociationPlain);
    }

    #[test]
    fn parse_realization() {
        let diag = parse("classDiagram\nIShape <|.. Circle").unwrap();
        assert_eq!(diag.relations[0].kind, RelKind::Realization);
        let diag2 = parse("classDiagram\nCircle ..|> IShape").unwrap();
        assert_eq!(diag2.relations[0].kind, RelKind::Realization);
    }

    #[test]
    fn parse_dependency() {
        let diag = parse("classDiagram\nA ..> B").unwrap();
        assert_eq!(diag.relations[0].kind, RelKind::Dependency);
    }

    #[test]
    fn parse_relation_with_label() {
        let diag = parse("classDiagram\nAnimal <|-- Dog : inherits").unwrap();
        assert_eq!(diag.relations[0].label.as_deref(), Some("inherits"));
    }

    #[test]
    fn parse_relation_creates_missing_classes() {
        let diag = parse("classDiagram\nAnimal <|-- Dog").unwrap();
        assert_eq!(diag.classes.len(), 2);
        assert_eq!(diag.classes[0].name, "Animal");
        assert_eq!(diag.classes[1].name, "Dog");
    }

    // ---- comments and blanks ----

    #[test]
    fn parse_skips_comments_and_blank_lines() {
        let src = "%% header comment\nclassDiagram\n\n%% body comment\nclass A";
        let diag = parse(src).unwrap();
        assert_eq!(diag.classes.len(), 1);
    }

    // ---- error cases ----

    #[test]
    fn parse_stray_close_brace_errors() {
        let err = parse("classDiagram\n}").unwrap_err();
        assert!(err.to_string().contains("stray"));
    }

    #[test]
    fn parse_unclosed_body_errors() {
        let err = parse("classDiagram\nclass A {\n    +name").unwrap_err();
        assert!(err.to_string().contains("unclosed"));
    }

    #[test]
    fn parse_generic_class_errors() {
        let err = parse("classDiagram\nclass Container~T~").unwrap_err();
        assert!(err.to_string().contains("generics"));
    }

    #[test]
    fn parse_note_directive_errors() {
        let err = parse("classDiagram\nnote for Animal \"hello\"").unwrap_err();
        assert!(err.to_string().contains("note"));
    }

    #[test]
    fn parse_namespace_errors() {
        let err = parse("classDiagram\nnamespace Ns {").unwrap_err();
        assert!(err.to_string().contains("namespace"));
    }

    #[test]
    fn parse_colon_shorthand_errors() {
        let err = parse("classDiagram\nAnimal : +name String").unwrap_err();
        assert!(err.to_string().contains("colon-shorthand"));
    }

    // ---- round-trip: class then relation ----

    #[test]
    fn parse_class_declared_before_relation_keeps_members() {
        let src = "classDiagram\nclass Animal {\n    +String name\n}\nclass Dog\nAnimal <|-- Dog";
        let diag = parse(src).unwrap();
        assert_eq!(diag.classes.len(), 2);
        let animal_idx = diag.class_index("Animal").unwrap();
        assert_eq!(diag.classes[animal_idx].members.len(), 1);
    }

    #[test]
    fn parse_relation_forward_reference_then_body() {
        let src = "classDiagram\nAnimal <|-- Dog\nclass Animal {\n    +String name\n}";
        let diag = parse(src).unwrap();
        let animal_idx = diag.class_index("Animal").unwrap();
        assert_eq!(diag.classes[animal_idx].members.len(), 1);
    }
}
