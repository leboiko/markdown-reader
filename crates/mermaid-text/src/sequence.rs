//! Types for Mermaid sequence diagrams.
//!
//! These types are populated by [`crate::parser::sequence::parse`] and
//! consumed by [`crate::render::sequence::render`].

/// The visual style of a sequence-diagram message arrow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStyle {
    /// Solid line with an arrowhead: `->>`.
    SolidArrow,
    /// Dashed line with an arrowhead: `-->>`.
    DashedArrow,
    /// Solid line without arrowhead: `->`.
    SolidLine,
    /// Dashed line without arrowhead: `-->`.
    DashedLine,
}

impl MessageStyle {
    /// Returns `true` when the line should be rendered with a dashed glyph.
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::sequence::MessageStyle;
    ///
    /// assert!(MessageStyle::DashedArrow.is_dashed());
    /// assert!(MessageStyle::DashedLine.is_dashed());
    /// assert!(!MessageStyle::SolidArrow.is_dashed());
    /// assert!(!MessageStyle::SolidLine.is_dashed());
    /// ```
    pub fn is_dashed(self) -> bool {
        matches!(self, Self::DashedArrow | Self::DashedLine)
    }

    /// Returns `true` when an arrowhead should be drawn at the target end.
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::sequence::MessageStyle;
    ///
    /// assert!(MessageStyle::SolidArrow.has_arrow());
    /// assert!(MessageStyle::DashedArrow.has_arrow());
    /// assert!(!MessageStyle::SolidLine.has_arrow());
    /// assert!(!MessageStyle::DashedLine.has_arrow());
    /// ```
    pub fn has_arrow(self) -> bool {
        matches!(self, Self::SolidArrow | Self::DashedArrow)
    }
}

/// A participant (or actor) in a sequence diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    /// The identifier used in message lines (e.g. `A`).
    pub id: String,
    /// The display label shown in the participant box (defaults to `id` when
    /// no `as <alias>` clause is given).
    pub label: String,
}

impl Participant {
    /// Construct a participant whose label equals its id.
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::sequence::Participant;
    ///
    /// let p = Participant::new("A");
    /// assert_eq!(p.id, "A");
    /// assert_eq!(p.label, "A");
    /// ```
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        let label = id.clone();
        Self { id, label }
    }

    /// Construct a participant with an explicit display label.
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::sequence::Participant;
    ///
    /// let p = Participant::with_label("W", "Worker");
    /// assert_eq!(p.id, "W");
    /// assert_eq!(p.label, "Worker");
    /// ```
    pub fn with_label(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// A message arrow between two participants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Sender participant ID.
    pub from: String,
    /// Receiver participant ID (may equal `from` for self-messages).
    pub to: String,
    /// Optional label displayed above the arrow.
    pub text: String,
    /// Visual style of the arrow.
    pub style: MessageStyle,
}

/// A parsed sequence diagram, ready for rendering.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SequenceDiagram {
    /// Participants in declaration order.  Participants that appear only in
    /// message lines (never declared explicitly) are appended in first-mention
    /// order.
    pub participants: Vec<Participant>,
    /// Messages in source order (top-to-bottom).
    pub messages: Vec<Message>,
}

impl SequenceDiagram {
    /// Return the index of the participant with the given ID, or `None`.
    pub fn participant_index(&self, id: &str) -> Option<usize> {
        self.participants.iter().position(|p| p.id == id)
    }

    /// Ensure a participant with `id` exists, inserting a bare-id entry at
    /// the end if absent.
    pub fn ensure_participant(&mut self, id: &str) {
        if self.participant_index(id).is_none() {
            self.participants.push(Participant::new(id));
        }
    }
}
