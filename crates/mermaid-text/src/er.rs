//! Data model for Mermaid `erDiagram` (entity-relationship) charts.
//!
//! Mermaid's erDiagram describes entities (tables / record types)
//! with attribute lists, joined by relationships that carry
//! crow's-foot cardinality glyphs at each end.
//!
//! Example:
//!
//! ```text
//! erDiagram
//!     CUSTOMER ||--o{ ORDER : places
//!     CUSTOMER {
//!         string name
//!         string email PK
//!     }
//!     ORDER ||--|{ LINE-ITEM : contains
//! ```
//!
//! The cardinality halves `||`, `}|`, `}o`, `o|` map to
//! [`Cardinality::ExactlyOne`], [`OneOrMany`], [`ZeroOrMany`],
//! [`ZeroOrOne`]. The connector between them — `--` or `..` — picks
//! [`LineStyle::Identifying`] or [`LineStyle::NonIdentifying`].
//!
//! [`OneOrMany`]: Cardinality::OneOrMany
//! [`ZeroOrMany`]: Cardinality::ZeroOrMany
//! [`ZeroOrOne`]: Cardinality::ZeroOrOne

/// One column of an [`Entity`]'s attribute table.
///
/// `type_name` and `name` are required; `keys` is the list of
/// recognised modifiers in source order; `comment` is the optional
/// trailing quoted string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub type_name: String,
    pub name: String,
    pub keys: Vec<AttributeKey>,
    pub comment: Option<String>,
}

/// Recognised key modifiers on an [`Attribute`]. Mermaid's grammar
/// admits exactly these three; arbitrary other modifiers are rejected
/// at parse time so typos surface instead of silently disappearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeKey {
    /// Primary key (`PK`).
    PrimaryKey,
    /// Foreign key (`FK`).
    ForeignKey,
    /// Unique key (`UK`).
    UniqueKey,
}

/// One row in an [`ErDiagram`] — a named entity with an attribute
/// list. The list may be empty (entities mentioned only in
/// relationships and never declared with a body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub name: String,
    pub attributes: Vec<Attribute>,
}

impl Entity {
    /// Construct an entity with no attributes — used by the parser
    /// when an entity name first appears as a relationship endpoint
    /// before its `{ ... }` block (or when no block is ever supplied).
    pub fn bare(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            attributes: Vec::new(),
        }
    }
}

/// One end of a [`Relationship`]'s cardinality. Each Mermaid
/// crow's-foot half (`||`, `}|`, `}o`, `o|`) maps to one of these
/// four discrete categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    /// `||` — required, exactly one (mandatory single).
    ExactlyOne,
    /// `o|` (or `|o`) — optional, at most one.
    ZeroOrOne,
    /// `}|` (or `|{`) — required, one or more.
    OneOrMany,
    /// `}o` (or `o{`) — optional, zero or more.
    ZeroOrMany,
}

/// Connector style between two cardinality halves of a relationship.
/// Mermaid distinguishes `--` (identifying — solid line, child cannot
/// exist without parent) from `..` (non-identifying — dashed line,
/// looser association).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineStyle {
    /// `--` — solid line. Child entity's identity depends on parent's.
    Identifying,
    /// `..` — dashed line. Looser association.
    NonIdentifying,
}

impl LineStyle {
    /// True for `..` (dashed) — used by the renderer to pick `┄`
    /// over `─` for the relationship line glyph.
    pub fn is_dashed(self) -> bool {
        matches!(self, LineStyle::NonIdentifying)
    }
}

/// One labelled relationship between two entities.
///
/// `from` and `to` reference [`Entity::name`]s. The two cardinality
/// fields describe each end's "how many" semantics; together they
/// reconstruct Mermaid's `||--o{` style notation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    pub from: String,
    pub to: String,
    pub from_cardinality: Cardinality,
    pub to_cardinality: Cardinality,
    pub line_style: LineStyle,
    pub label: Option<String>,
}

/// A parsed `erDiagram` chart.
///
/// Constructed by [`crate::parser::er::parse`] and consumed by
/// [`crate::render::er::render`]. Entities are listed in declaration
/// order; relationships in the order they appear in the source.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ErDiagram {
    pub entities: Vec<Entity>,
    pub relationships: Vec<Relationship>,
}

impl ErDiagram {
    /// Find an entity by name (case-sensitive — Mermaid treats entity
    /// names as opaque identifiers). Returns the entity's index in
    /// `entities` so callers can update it in place.
    pub fn entity_index(&self, name: &str) -> Option<usize> {
        self.entities.iter().position(|e| e.name == name)
    }

    /// Insert an entity if its name isn't already present, returning
    /// its index either way. Used by the parser when a relationship
    /// mentions an entity before its `{ … }` body has been declared.
    pub fn ensure_entity(&mut self, name: &str) -> usize {
        if let Some(idx) = self.entity_index(name) {
            return idx;
        }
        self.entities.push(Entity::bare(name));
        self.entities.len() - 1
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_style_is_dashed_distinguishes_identifying_from_non() {
        assert!(LineStyle::NonIdentifying.is_dashed());
        assert!(!LineStyle::Identifying.is_dashed());
    }

    #[test]
    fn entity_bare_starts_with_no_attributes() {
        let e = Entity::bare("CUSTOMER");
        assert_eq!(e.name, "CUSTOMER");
        assert!(e.attributes.is_empty());
    }

    #[test]
    fn ensure_entity_inserts_then_reuses() {
        let mut diag = ErDiagram::default();
        let first = diag.ensure_entity("A");
        let second = diag.ensure_entity("A");
        let third = diag.ensure_entity("B");
        assert_eq!(first, 0);
        assert_eq!(second, 0); // reuse existing
        assert_eq!(third, 1);
        assert_eq!(diag.entities.len(), 2);
    }

    #[test]
    fn entity_index_returns_none_for_unknown() {
        let diag = ErDiagram::default();
        assert_eq!(diag.entity_index("X"), None);
    }
}
