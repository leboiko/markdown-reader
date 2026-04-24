//! Data model for Mermaid `classDiagram` charts.
//!
//! Mermaid's `classDiagram` describes object-oriented classes with attributes,
//! methods, visibility modifiers, and inter-class relationships (inheritance,
//! composition, aggregation, association, dependency, and realization).
//!
//! # v1 supported features
//!
//! - Class declarations with optional `{ … }` body
//! - Member visibility: `+` public, `-` private, `#` protected, `~` package
//! - Attributes: `+name Type` or `+Type name` (typed-before and typed-after)
//! - Methods: `+method(args) ReturnType`; `$` static suffix, `*` abstract suffix
//! - Stereotypes: `<<interface>>`, `<<enumeration>>`, `<<abstract>>`
//! - All seven relationship types (see [`RelKind`])
//! - Edge labels and multiplicity (quoted strings)
//! - `%%` line comments
//!
//! # v1 explicitly unsupported
//!
//! The following Mermaid features return a [`crate::Error::ParseError`] when
//! encountered:
//! - Generics (`Class~T~`)
//! - Namespace blocks
//! - `note for X` annotations
//! - `link` / `click` directives
//! - Colon-shorthand member form (`Animal : +name String`)
//! - `direction` header inside the diagram body

/// Visibility modifier on a class member.
///
/// Mermaid's four visibility symbols map directly to OOP conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// `+` — public
    Public,
    /// `-` — private
    Private,
    /// `#` — protected
    Protected,
    /// `~` — package (internal to the package/module)
    Package,
}

impl Visibility {
    /// The single-character source symbol for this visibility level.
    pub fn as_char(self) -> char {
        match self {
            Self::Public => '+',
            Self::Private => '-',
            Self::Protected => '#',
            Self::Package => '~',
        }
    }
}

/// An attribute (field) member inside a class body.
///
/// Mermaid allows the type before or after the name:
/// - `+String name` — typed-before (Mermaid's primary form)
/// - `+name String` — typed-after (also accepted)
///
/// Both forms are normalised to `(visibility, name, type)` here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    /// Visibility modifier (may be `None` when the `+/-/#/~` prefix is absent).
    pub visibility: Option<Visibility>,
    /// Attribute name.
    pub name: String,
    /// Declared type (may be empty if the source omits it).
    pub type_name: String,
    /// `true` when the `$` suffix marks this attribute as static.
    pub is_static: bool,
}

/// A method member inside a class body.
///
/// `+method(args) ReturnType$*` decomposes as:
/// - visibility = `Public`
/// - name = `"method"`
/// - params = `"args"`
/// - return_type = `Some("ReturnType")`
/// - is_static = `false`
/// - is_abstract = `false`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Method {
    /// Visibility modifier.
    pub visibility: Option<Visibility>,
    /// Method name.
    pub name: String,
    /// Raw parameter list (content between parentheses), may be empty.
    pub params: String,
    /// Declared return type, if present.
    pub return_type: Option<String>,
    /// `true` when the `$` suffix marks this method as static.
    pub is_static: bool,
    /// `true` when the `*` suffix marks this method as abstract.
    pub is_abstract: bool,
}

/// A member of a class body — either an attribute or a method.
///
/// Methods are distinguished from attributes by the presence of
/// parentheses in the source (`method()`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Member {
    /// A typed field / property.
    Attribute(Attribute),
    /// A callable method.
    Method(Method),
}

impl Member {
    /// The display name of this member (used for box width calculations).
    pub fn name(&self) -> &str {
        match self {
            Self::Attribute(a) => &a.name,
            Self::Method(m) => &m.name,
        }
    }
}

/// UML stereotype associated with a class.
///
/// Stereotypes appear as `<<name>>` inside or below the class declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stereotype {
    /// `<<interface>>` — class is a pure interface.
    Interface,
    /// `<<enumeration>>` — class is an enumeration.
    Enumeration,
    /// `<<abstract>>` — class is abstract.
    Abstract,
    /// Any other `<<name>>` (stored verbatim for future use).
    Other(String),
}

impl Stereotype {
    /// Returns the raw label string shown in `<<…>>`.
    pub fn label(&self) -> &str {
        match self {
            Self::Interface => "interface",
            Self::Enumeration => "enumeration",
            Self::Abstract => "abstract",
            Self::Other(s) => s,
        }
    }
}

/// One class in a [`ClassDiagram`].
///
/// Classes are identified by name. They may have zero or more members and an
/// optional stereotype. Classes can be forward-declared (mentioned only in a
/// relationship) with an empty `members` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Class {
    /// Unique class name (case-sensitive).
    pub name: String,
    /// Optional UML stereotype (`<<interface>>`, `<<enumeration>>`, etc.).
    pub stereotype: Option<Stereotype>,
    /// Members declared in the `{ … }` body, in source order.
    pub members: Vec<Member>,
}

impl Class {
    /// Construct a bare class with no members or stereotype.
    ///
    /// Used by the parser when a class is first mentioned in a relationship
    /// before any `{ … }` declaration has been seen.
    pub fn bare(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            stereotype: None,
            members: Vec::new(),
        }
    }
}

/// The kind of relationship between two classes.
///
/// Mermaid's class diagram supports seven relationship arrows:
///
/// ```text
/// <|--   Inheritance  (solid line, hollow triangle at parent)
/// --|>   Inheritance  (solid line, hollow triangle at child — reversed)
/// *--    Composition  (solid line, filled diamond at owner)
/// o--    Aggregation  (solid line, hollow diamond at owner)
/// -->    Association  (solid directed arrow)
/// --     Association  (plain, no arrow)
/// <|..   Realization  (dashed line, hollow triangle at interface)
/// ..|>   Realization  (dashed line, hollow triangle — reversed)
/// <..    Dependency   (dashed arrow, reversed)
/// ..>    Dependency   (dashed arrow)
/// ```
///
/// The `from`/`to` fields of [`Relation`] define which class the arrow
/// originates from. The displayed endpoint glyph is at the `to` end unless
/// `rel_kind` is `Inheritance` or `Realization` (where the triangle points
/// at the parent/interface, i.e. the `to` end).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelKind {
    /// Inheritance — solid line with hollow triangle at parent (`--|>` / `<|--`).
    Inheritance,
    /// Composition — solid line with filled diamond at owner (`*--` / `--*`).
    Composition,
    /// Aggregation — solid line with hollow diamond at owner (`o--` / `--o`).
    Aggregation,
    /// Directed association — solid arrow (`-->`).
    AssociationDirected,
    /// Plain association — plain line with no arrow endpoints (`--`).
    AssociationPlain,
    /// Realization — dashed line with hollow triangle (`..|>` / `<|..`).
    Realization,
    /// Dependency — dashed arrow (`..>`).
    Dependency,
}

impl RelKind {
    /// Returns `true` if this relationship uses a dashed (non-identifying) line.
    pub fn is_dashed(self) -> bool {
        matches!(self, Self::Realization | Self::Dependency)
    }
}

/// One directed relationship between two classes.
///
/// `from` and `to` are class names. The arrow's visual meaning depends on
/// [`RelKind`]:
/// - For [`RelKind::Inheritance`] and [`RelKind::Realization`] the hollow
///   triangle points at `to` (the parent / interface).
/// - For [`RelKind::Composition`] and [`RelKind::Aggregation`] the diamond is
///   at the `from` end (the owner).
/// - For [`RelKind::AssociationDirected`] and [`RelKind::Dependency`] the
///   arrowhead points at `to`.
/// - For [`RelKind::AssociationPlain`] there are no endpoint glyphs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    /// Source class name.
    pub from: String,
    /// Target class name.
    pub to: String,
    /// Kind of relationship (determines the endpoint glyph style).
    pub kind: RelKind,
    /// Optional multiplicity at the `from` end (e.g. `"1"`, `"0..*"`).
    pub from_multiplicity: Option<String>,
    /// Optional multiplicity at the `to` end.
    pub to_multiplicity: Option<String>,
    /// Optional label text placed along the line.
    pub label: Option<String>,
}

/// A fully-parsed `classDiagram`.
///
/// Constructed by [`crate::parser::class::parse`] and consumed by
/// [`crate::render::class::render`]. Classes are listed in declaration order
/// (forward references from relationships are inserted when first encountered);
/// relations in source order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClassDiagram {
    /// All classes, in declaration/first-mention order.
    pub classes: Vec<Class>,
    /// All relationships, in source order.
    pub relations: Vec<Relation>,
}

impl ClassDiagram {
    /// Find a class by name. Returns its index in `classes`, or `None` if not
    /// found.
    pub fn class_index(&self, name: &str) -> Option<usize> {
        self.classes.iter().position(|c| c.name == name)
    }

    /// Insert a bare class if its name isn't already present, returning its
    /// index either way. Used by the parser when a relationship mentions a class
    /// before its `{ … }` body is declared.
    pub fn ensure_class(&mut self, name: &str) -> usize {
        if let Some(idx) = self.class_index(name) {
            return idx;
        }
        self.classes.push(Class::bare(name));
        self.classes.len() - 1
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_as_char_round_trips() {
        assert_eq!(Visibility::Public.as_char(), '+');
        assert_eq!(Visibility::Private.as_char(), '-');
        assert_eq!(Visibility::Protected.as_char(), '#');
        assert_eq!(Visibility::Package.as_char(), '~');
    }

    #[test]
    fn class_bare_starts_empty() {
        let c = Class::bare("Animal");
        assert_eq!(c.name, "Animal");
        assert!(c.members.is_empty());
        assert!(c.stereotype.is_none());
    }

    #[test]
    fn ensure_class_inserts_then_reuses() {
        let mut diag = ClassDiagram::default();
        let a0 = diag.ensure_class("A");
        let a1 = diag.ensure_class("A");
        let b0 = diag.ensure_class("B");
        assert_eq!(a0, 0);
        assert_eq!(a1, 0);
        assert_eq!(b0, 1);
        assert_eq!(diag.classes.len(), 2);
    }

    #[test]
    fn class_index_returns_none_for_unknown() {
        let diag = ClassDiagram::default();
        assert_eq!(diag.class_index("X"), None);
    }

    #[test]
    fn rel_kind_is_dashed_for_realization_and_dependency() {
        assert!(RelKind::Realization.is_dashed());
        assert!(RelKind::Dependency.is_dashed());
        assert!(!RelKind::Inheritance.is_dashed());
        assert!(!RelKind::Composition.is_dashed());
        assert!(!RelKind::Aggregation.is_dashed());
        assert!(!RelKind::AssociationDirected.is_dashed());
        assert!(!RelKind::AssociationPlain.is_dashed());
    }

    #[test]
    fn stereotype_label_returns_canonical_strings() {
        assert_eq!(Stereotype::Interface.label(), "interface");
        assert_eq!(Stereotype::Enumeration.label(), "enumeration");
        assert_eq!(Stereotype::Abstract.label(), "abstract");
        assert_eq!(Stereotype::Other("service".to_string()).label(), "service");
    }

    #[test]
    fn member_name_delegates_correctly() {
        let attr = Member::Attribute(Attribute {
            visibility: None,
            name: "count".to_string(),
            type_name: "int".to_string(),
            is_static: false,
        });
        let method = Member::Method(Method {
            visibility: None,
            name: "run".to_string(),
            params: String::new(),
            return_type: None,
            is_static: false,
            is_abstract: false,
        });
        assert_eq!(attr.name(), "count");
        assert_eq!(method.name(), "run");
    }
}
