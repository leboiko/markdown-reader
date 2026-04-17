//! Core types shared across parsing, layout, and rendering.

/// The direction in which a flowchart flows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Left-to-right (`LR`).
    LeftToRight,
    /// Top-to-bottom (`TD` or `TB`).
    TopToBottom,
    /// Right-to-left (`RL`).
    RightToLeft,
    /// Bottom-to-top (`BT`).
    BottomToTop,
}

impl Direction {
    /// Parse a direction keyword, case-insensitive.
    ///
    /// # Returns
    ///
    /// `Some(Direction)` if the keyword is recognised, `None` otherwise.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "LR" => Some(Self::LeftToRight),
            "TD" | "TB" => Some(Self::TopToBottom),
            "RL" => Some(Self::RightToLeft),
            "BT" => Some(Self::BottomToTop),
            _ => None,
        }
    }

    /// Returns `true` if the primary flow axis is horizontal (LR or RL).
    pub fn is_horizontal(self) -> bool {
        matches!(self, Self::LeftToRight | Self::RightToLeft)
    }
}

/// The visual shape used to render a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NodeShape {
    /// Square corners: `┌──┐ │  │ └──┘`
    #[default]
    Rectangle,
    /// Rounded corners: `╭──╮ │  │ ╰──╯`
    Rounded,
    /// Diamond / decision box rendered with `/` and `\` corners.
    Diamond,
    /// Circle rendered as a rounded box with parenthesis markers.
    Circle,
}

/// A single node in the diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Unique identifier used in edge definitions (e.g. `A`).
    pub id: String,
    /// Human-readable label displayed inside the node box.
    pub label: String,
    /// Visual shape of the node.
    pub shape: NodeShape,
}

impl Node {
    /// Construct a new node.
    pub fn new(id: impl Into<String>, label: impl Into<String>, shape: NodeShape) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            shape,
        }
    }
}

/// A directed connection between two nodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// ID of the source node.
    pub from: String,
    /// ID of the destination node.
    pub to: String,
    /// Optional label placed along the edge.
    pub label: Option<String>,
}

impl Edge {
    /// Construct a new edge.
    pub fn new(from: impl Into<String>, to: impl Into<String>, label: Option<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            label,
        }
    }
}

/// A parsed flowchart graph ready for layout and rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graph {
    /// The overall flow direction.
    pub direction: Direction,
    /// All nodes in declaration order.
    pub nodes: Vec<Node>,
    /// All edges in declaration order.
    pub edges: Vec<Edge>,
}

impl Graph {
    /// Construct a new empty graph with the given direction.
    pub fn new(direction: Direction) -> Self {
        Self {
            direction,
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// Look up a node by its ID, returning a reference if found.
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Return `true` if a node with `id` already exists.
    pub fn has_node(&self, id: &str) -> bool {
        self.nodes.iter().any(|n| n.id == id)
    }

    /// Insert a node, or update its label/shape if the ID already exists and
    /// the existing entry was auto-created as a bare-id placeholder.
    pub fn upsert_node(&mut self, node: Node) {
        if let Some(existing) = self.nodes.iter_mut().find(|n| n.id == node.id) {
            // Only promote a bare placeholder (label == id) to a richer definition.
            if existing.label == existing.id
                && (existing.shape == NodeShape::Rectangle)
                && (node.label != node.id || node.shape != NodeShape::Rectangle)
            {
                *existing = node;
            }
        } else {
            self.nodes.push(node);
        }
    }
}
