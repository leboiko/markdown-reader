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

/// Where a `note` is anchored relative to its target participant(s).
///
/// Mermaid's grammar accepts a single anchor for `left of` and `right of`,
/// and an optional comma-separated pair for `over`. The pair span widens
/// the rendered note to cover both participants' columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteAnchor {
    /// `note left of <Id>` — the box sits to the left of the target's lifeline.
    LeftOf(String),
    /// `note right of <Id>` — the box sits to the right of the target's lifeline.
    RightOf(String),
    /// `note over <Id>` — the box is centred on the target's lifeline.
    Over(String),
    /// `note over <Id1>,<Id2>` — the box spans columns from `Id1` to `Id2`.
    OverPair(String, String),
}

/// A note attached to the message stream at a specific source position.
///
/// Notes are inserted between messages when rendered: `after_message: 0`
/// places the note before any message; `after_message: N` places it after
/// the Nth message in `SequenceDiagram::messages`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteEvent {
    pub anchor: NoteAnchor,
    /// Note text. Mermaid's `<br>` and `<br/>` line-break tags are
    /// converted to literal `\n` characters at parse time so the
    /// renderer can split on `\n` like any other multi-line label.
    pub text: String,
    /// Insertion position. `0` = before the first message;
    /// `messages.len()` = after the last message.
    pub after_message: usize,
}

/// A lifeline activation span — a region where a participant is "active"
/// (handling a request). Renders as a thick vertical bar overlaid on
/// the participant's lifeline between the start and end message rows.
///
/// Created from explicit `activate <Id>` / `deactivate <Id>` directives
/// or the inline `A->>+B` (activates B) / `A-->>-B` (deactivates the
/// SOURCE A — per Mermaid's spec) shorthand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activation {
    pub participant: String,
    /// Index of the message at which the activation begins.
    pub start_message: usize,
    /// Index of the message at which the activation ends. For an
    /// unmatched `activate` (no later `deactivate`), this is set to
    /// the last message index at parse-time finalisation so the bar
    /// extends to the end of the diagram.
    pub end_message: usize,
}

/// A control-flow block wrapping a contiguous range of messages.
///
/// Mermaid sequence diagrams support `loop`, `alt`/`else`, `opt`,
/// `par`/`and`, `critical`/`option`, and `break` (plus `rect` for
/// background highlight, which is out of scope for v0.9.0). Each
/// block has 1+ branches; multi-branch blocks (alt, par, critical)
/// carry per-branch labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// At least one branch (single-branch blocks like `loop` have
    /// exactly one). Branches store their per-section label.
    pub branches: Vec<BlockBranch>,
    /// First contained message index (inclusive).
    pub start_message: usize,
    /// Last contained message index (inclusive). When the block
    /// contains zero messages this equals `start_message - 1` and
    /// callers should treat the block as empty.
    pub end_message: usize,
}

/// A single branch within a [`Block`]. For single-branch blocks
/// (`loop`, `opt`, `break`) the block has exactly one branch carrying
/// the opener's label. For multi-branch blocks (`alt`, `par`,
/// `critical`) each continuation keyword (`else`, `and`, `option`)
/// opens a new branch with its own label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockBranch {
    pub label: String,
    pub start_message: usize,
    pub end_message: usize,
}

/// The kind of control-flow block, controlling its visible label and
/// which continuation keyword (if any) opens additional branches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockKind {
    /// `loop <label>` — single branch.
    Loop,
    /// `alt <label>` / `else <label>` — multi-branch.
    Alt,
    /// `opt <label>` — single branch.
    Opt,
    /// `par <label>` / `and <label>` — multi-branch.
    Par,
    /// `critical <label>` / `option <label>` — multi-branch.
    Critical,
    /// `break <label>` — single branch.
    Break,
    /// `rect rgb(R, G, B)` / `rect rgba(R, G, B, A)` — borderless background
    /// fill block.  Rendered as a shade-glyph fill keyed by luminance; no
    /// border, no label tag.
    Rect {
        /// Base colour of the background fill.
        rgb: crate::types::Rgb,
        /// Alpha channel, normalised to 0..=255.  `None` means fully opaque
        /// (equivalent to `rgba(..., 255)` but encoded from `rgb(...)`).
        alpha: Option<u8>,
    },
}

/// State of the `autonumber` directive at a particular point in the
/// message stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutonumberState {
    /// Numbering is on; the next message numbered will use `next_value`.
    On { next_value: u32 },
    /// Numbering is off (either never enabled, or explicitly disabled
    /// via `autonumber off`).
    Off,
}

/// A change in the `autonumber` state taking effect at a specific
/// message position. The renderer walks these in lockstep with the
/// message loop, applying the most recent state to each message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutonumberChange {
    /// Index of the first message that this state applies to.
    pub at_message: usize,
    pub state: AutonumberState,
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
    /// Notes anchored to participants, positioned between messages
    /// in source order. Empty for diagrams with no `note …` directives.
    pub notes: Vec<NoteEvent>,
    /// Lifeline activation spans, paired at parse time. An unmatched
    /// `activate` extends to the last message index. Empty for
    /// diagrams without `activate`/`deactivate`/inline `+`/`-`.
    pub activations: Vec<Activation>,
    /// Control-flow blocks (loop / alt / opt / par / critical / break)
    /// wrapping contiguous message ranges. Empty for diagrams that
    /// don't use block statements.
    pub blocks: Vec<Block>,
    /// `autonumber` state changes ordered by `at_message`. Empty
    /// when the directive is never used.
    pub autonumber_changes: Vec<AutonumberChange>,
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
