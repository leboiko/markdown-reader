//! Core types shared across parsing, layout, and rendering.

use std::collections::HashMap;

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
    /// # Arguments
    ///
    /// * `s` — a direction token such as `"LR"`, `"TD"`, `"TB"`, `"RL"`, or `"BT"`.
    ///
    /// # Returns
    ///
    /// `Some(Direction)` if the keyword is recognised, `None` otherwise.
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::Direction;
    ///
    /// assert_eq!(Direction::parse("LR"), Some(Direction::LeftToRight));
    /// assert_eq!(Direction::parse("td"), Some(Direction::TopToBottom)); // case-insensitive
    /// assert_eq!(Direction::parse("TB"), Some(Direction::TopToBottom));
    /// assert_eq!(Direction::parse("RL"), Some(Direction::RightToLeft));
    /// assert_eq!(Direction::parse("BT"), Some(Direction::BottomToTop));
    /// assert_eq!(Direction::parse("XX"), None);
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::Direction;
    ///
    /// assert!(Direction::LeftToRight.is_horizontal());
    /// assert!(Direction::RightToLeft.is_horizontal());
    /// assert!(!Direction::TopToBottom.is_horizontal());
    /// assert!(!Direction::BottomToTop.is_horizontal());
    /// ```
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
    /// Stadium / pill: rounded box with `(` / `)` markers at vertical midpoints.
    ///
    /// Mermaid syntax: `([label])`
    Stadium,
    /// Subroutine: rectangle with an extra inner vertical bar on each side.
    ///
    /// Mermaid syntax: `[[label]]`
    Subroutine,
    /// Cylinder (database): rectangle with arc markers at top and bottom centres.
    ///
    /// Mermaid syntax: `[(label)]`
    Cylinder,
    /// Hexagon: rectangle with `<` / `>` markers at vertical midpoints of left/right edges.
    ///
    /// Mermaid syntax: `{{label}}`
    Hexagon,
    /// Asymmetric flag: rectangle with a `⟩` marker at the right vertical midpoint.
    ///
    /// Mermaid syntax: `>label]`
    Asymmetric,
    /// Parallelogram (lean-right): rectangle with `/` markers at top-left / bottom-right corners.
    ///
    /// Mermaid syntax: `[/label/]`
    Parallelogram,
    /// Trapezoid (wider top): rectangle with `/` at top-left and `\` at top-right corners.
    ///
    /// Mermaid syntax: `[/label\]`
    Trapezoid,
    /// Double circle: two concentric rounded boxes, one cell inside the other.
    ///
    /// Mermaid syntax: `(((label)))`
    DoubleCircle,
    /// UML synchronisation bar — a single line used as a fork (one
    /// incoming, many outgoing) or join (many incoming, one
    /// outgoing) point in parallel-flow state machines.
    ///
    /// The orientation is **perpendicular to the flow direction**
    /// (so edges fan in/out across its long axis):
    ///
    /// - In LR/RL flow: [`BarOrientation::Vertical`] — a column of `┃`
    ///   glyphs.
    /// - In TD/BT flow: [`BarOrientation::Horizontal`] — a row of `━`
    ///   glyphs.
    ///
    /// Fork and join are visually identical (only the semantic role
    /// differs); both use this single shape variant. The renderer
    /// skips drawing the node label for `Bar(_)` shapes — bars are
    /// connection points, not labelled states.
    ///
    /// Mermaid syntax: `state X <<fork>>` / `state X <<join>>`
    /// (and the `[[…]]` alternative spellings).
    Bar(BarOrientation),
}

/// Orientation for a [`NodeShape::Bar`] (fork/join synchronisation bar).
///
/// Resolved at parse time from the graph's flow direction so the
/// renderer doesn't need direction context — the bar is always
/// perpendicular to flow:
///
/// - `Horizontal` for TD/BT-flow diagrams (row of `━`).
/// - `Vertical` for LR/RL-flow diagrams (column of `┃`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarOrientation {
    /// `━━━` — used for fork/join in TD/BT-flow diagrams.
    Horizontal,
    /// `┃` stacked — used for fork/join in LR/RL-flow diagrams.
    Vertical,
}

/// A 24-bit RGB color, used for ANSI truecolor SGR sequences.
///
/// Parsed from Mermaid `style` / `linkStyle` directives like
/// `fill:#336`, `stroke:#ffffff`, `color:#fff`. Both `#RGB` and
/// `#RRGGBB` forms are accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parse a Mermaid hex color of the form `#RGB` or `#RRGGBB`
    /// (case-insensitive). The leading `#` is required.
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::types::Rgb;
    ///
    /// assert_eq!(Rgb::parse_hex("#336"), Some(Rgb(0x33, 0x33, 0x66)));
    /// assert_eq!(Rgb::parse_hex("#FFFFFF"), Some(Rgb(0xff, 0xff, 0xff)));
    /// assert_eq!(Rgb::parse_hex("336"), None); // missing '#'
    /// assert_eq!(Rgb::parse_hex("#GG0000"), None); // not hex
    /// ```
    pub fn parse_hex(s: &str) -> Option<Self> {
        let body = s.strip_prefix('#')?;
        match body.len() {
            3 => {
                let r = u8::from_str_radix(&body[0..1], 16).ok()?;
                let g = u8::from_str_radix(&body[1..2], 16).ok()?;
                let b = u8::from_str_radix(&body[2..3], 16).ok()?;
                Some(Self(r * 0x11, g * 0x11, b * 0x11))
            }
            6 => {
                let r = u8::from_str_radix(&body[0..2], 16).ok()?;
                let g = u8::from_str_radix(&body[2..4], 16).ok()?;
                let b = u8::from_str_radix(&body[4..6], 16).ok()?;
                Some(Self(r, g, b))
            }
            _ => None,
        }
    }
}

/// Per-node style attributes parsed from `style <id> ...` directives.
///
/// Only color-related attributes are tracked. Unrecognised keys
/// (e.g. `font-size`) are silently ignored at parse time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NodeStyle {
    /// Background color for the node interior cells (`fill:#…`).
    pub fill: Option<Rgb>,
    /// Foreground color for the node border glyphs (`stroke:#…`).
    pub stroke: Option<Rgb>,
    /// Foreground color for the node label text (`color:#…`).
    pub color: Option<Rgb>,
}

/// Per-edge color attributes parsed from `linkStyle <index> ...` directives.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EdgeStyleColors {
    /// Foreground color for the edge glyphs (`stroke:#…`).
    pub stroke: Option<Rgb>,
    /// Foreground color for the edge label text (`color:#…`).
    pub color: Option<Rgb>,
}

/// The visual style of an edge line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeStyle {
    /// Solid line (default). Characters: `─` / `│`.
    #[default]
    Solid,
    /// Dotted line. Characters: `┄` / `┆`.
    Dotted,
    /// Thick / bold line. Characters: `━` / `┃`.
    Thick,
}

/// The kind of endpoint drawn at each end of an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeEndpoint {
    /// An arrow tip pointing in the direction of travel.
    #[default]
    Arrow,
    /// No arrow tip — just the line reaching the node border.
    None,
    /// A circle endpoint (`○`).
    Circle,
    /// A cross endpoint (`×`).
    Cross,
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
    ///
    /// # Arguments
    ///
    /// * `id`    — unique identifier used in edge definitions
    /// * `label` — human-readable text displayed inside the node box
    /// * `shape` — visual shape of the node
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::{Node, NodeShape};
    ///
    /// let node = Node::new("A", "Start", NodeShape::Rounded);
    /// assert_eq!(node.id, "A");
    /// assert_eq!(node.label, "Start");
    /// assert_eq!(node.shape, NodeShape::Rounded);
    /// ```
    pub fn new(id: impl Into<String>, label: impl Into<String>, shape: NodeShape) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            shape,
        }
    }

    /// Return the widest line of the label, measured in terminal cells.
    ///
    /// Labels may contain `\n` line breaks (inserted by the parser when
    /// converting `<br/>` tags or when soft-wrapping long lines). The
    /// renderer sizes node boxes by the widest single line rather than by
    /// the whole label string, so the parser-inserted breaks actually
    /// narrow the box.
    ///
    /// Returns `0` for an empty label.
    pub fn label_width(&self) -> usize {
        use unicode_width::UnicodeWidthStr;
        self.label
            .lines()
            .map(UnicodeWidthStr::width)
            .max()
            .unwrap_or(0)
    }

    /// Return the number of rendered text rows this node's label occupies.
    ///
    /// Always at least 1, even for empty labels, so node boxes retain their
    /// minimum height.
    pub fn label_line_count(&self) -> usize {
        let n = self.label.lines().count();
        n.max(1)
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
    /// Visual style of the edge line (solid, dotted, or thick).
    pub style: EdgeStyle,
    /// Endpoint drawn at the **destination** end.
    pub end: EdgeEndpoint,
    /// Endpoint drawn at the **source** end (for bidirectional edges).
    pub start: EdgeEndpoint,
}

impl Edge {
    /// Construct a new solid arrow edge (the most common case).
    ///
    /// Equivalent to `new_styled` with [`EdgeStyle::Solid`], [`EdgeEndpoint::None`]
    /// at the source, and [`EdgeEndpoint::Arrow`] at the destination.
    ///
    /// # Arguments
    ///
    /// * `from`  — source node ID
    /// * `to`    — destination node ID
    /// * `label` — optional label placed along the edge
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::{Edge, EdgeEndpoint, EdgeStyle};
    ///
    /// let e = Edge::new("A", "B", Some("ok".to_string()));
    /// assert_eq!(e.from, "A");
    /// assert_eq!(e.to, "B");
    /// assert_eq!(e.label.as_deref(), Some("ok"));
    /// assert_eq!(e.style, EdgeStyle::Solid);
    /// assert_eq!(e.end, EdgeEndpoint::Arrow);
    /// assert_eq!(e.start, EdgeEndpoint::None);
    /// ```
    pub fn new(from: impl Into<String>, to: impl Into<String>, label: Option<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            label,
            style: EdgeStyle::Solid,
            end: EdgeEndpoint::Arrow,
            start: EdgeEndpoint::None,
        }
    }

    /// Construct an edge with explicit style and endpoint kinds.
    ///
    /// # Arguments
    ///
    /// * `from`  — source node ID
    /// * `to`    — destination node ID
    /// * `label` — optional label placed along the edge
    /// * `style` — line style (solid, dotted, thick)
    /// * `start` — endpoint at the source end
    /// * `end`   — endpoint at the destination end
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::{Edge, EdgeEndpoint, EdgeStyle};
    ///
    /// // A bidirectional thick edge with a label
    /// let e = Edge::new_styled(
    ///     "A", "B",
    ///     Some("sync".to_string()),
    ///     EdgeStyle::Thick,
    ///     EdgeEndpoint::Arrow,
    ///     EdgeEndpoint::Arrow,
    /// );
    /// assert_eq!(e.style, EdgeStyle::Thick);
    /// assert_eq!(e.start, EdgeEndpoint::Arrow);
    /// assert_eq!(e.end, EdgeEndpoint::Arrow);
    /// ```
    pub fn new_styled(
        from: impl Into<String>,
        to: impl Into<String>,
        label: Option<String>,
        style: EdgeStyle,
        start: EdgeEndpoint,
        end: EdgeEndpoint,
    ) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            label,
            style,
            end,
            start,
        }
    }
}

/// A named cluster of nodes (and optionally nested subgraphs).
///
/// Subgraphs are rendered as a rounded rectangle that encloses all their
/// direct and indirect member nodes. Edges may freely cross subgraph
/// boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subgraph {
    /// Unique identifier (the `id` token after `subgraph`).
    pub id: String,
    /// Human-readable label displayed at the top of the border. Falls back
    /// to `id` when not explicitly specified.
    pub label: String,
    /// Optional per-subgraph flow direction override.
    ///
    /// Currently preserved on the model for future use; the renderer
    /// always uses the parent graph direction.
    pub direction: Option<Direction>,
    /// IDs of **direct** child nodes (not recursively nested ones).
    pub node_ids: Vec<String>,
    /// IDs of **direct** child subgraphs.
    pub subgraph_ids: Vec<String>,
}

impl Subgraph {
    /// Construct a new subgraph with the given id and label.
    ///
    /// Both `node_ids` and `subgraph_ids` start empty; the parser fills them
    /// as it processes the subgraph body. `direction` defaults to `None`
    /// (inherits from the parent graph).
    ///
    /// # Arguments
    ///
    /// * `id`    — unique identifier (the token after `subgraph`)
    /// * `label` — display label at the top of the border
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::types::Subgraph;
    ///
    /// let sg = Subgraph::new("S1", "My Cluster");
    /// assert_eq!(sg.id, "S1");
    /// assert_eq!(sg.label, "My Cluster");
    /// assert!(sg.node_ids.is_empty());
    /// assert!(sg.direction.is_none());
    /// ```
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        let id = id.into();
        let label = label.into();
        Self {
            id,
            label,
            direction: None,
            node_ids: Vec::new(),
            subgraph_ids: Vec::new(),
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
    /// All top-level subgraphs in declaration order.
    ///
    /// Subgraphs may nest: a subgraph's `subgraph_ids` list references the
    /// IDs of its immediate children. Use [`Graph::node_to_subgraph`] for
    /// efficient node→subgraph lookups.
    pub subgraphs: Vec<Subgraph>,
    /// Per-node color overrides parsed from `style <id> ...` directives.
    ///
    /// Empty by default; populated only when the source contains `style`
    /// directives. Used by the renderer when ANSI color output is enabled.
    pub node_styles: HashMap<String, NodeStyle>,
    /// Per-edge color overrides parsed from `linkStyle <index> ...` directives.
    ///
    /// Keyed by the edge's positional index (0-based, in declaration order).
    /// Empty by default.
    pub edge_styles: HashMap<usize, EdgeStyleColors>,
    /// Named style classes from `classDef name fill:#…,stroke:#…,color:#…`
    /// directives. Acts as the palette that `class A foo` and `A:::foo`
    /// look up at end-of-parse to populate `node_styles` /
    /// `subgraph_styles`. Multiple `classDef` entries with the same name
    /// are last-wins (matches Mermaid).
    pub class_defs: HashMap<String, NodeStyle>,
    /// Per-subgraph color overrides — populated when `class CompositeId
    /// styleName` is applied to a known composite/subgraph id. The
    /// renderer paints the rounded border with `stroke`. `fill` and
    /// `color` are accepted in the schema for consistency with
    /// `node_styles` but only `stroke` is honoured today (filling a
    /// composite's interior would conflict with inner node fills).
    pub subgraph_styles: HashMap<String, NodeStyle>,
}

impl Graph {
    /// Construct a new empty graph with the given direction.
    ///
    /// # Arguments
    ///
    /// * `direction` — the overall flow direction for this graph
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::{Graph, Direction};
    ///
    /// let g = Graph::new(Direction::LeftToRight);
    /// assert_eq!(g.direction, Direction::LeftToRight);
    /// assert!(g.nodes.is_empty());
    /// assert!(g.edges.is_empty());
    /// ```
    pub fn new(direction: Direction) -> Self {
        Self {
            direction,
            nodes: Vec::new(),
            edges: Vec::new(),
            subgraphs: Vec::new(),
            node_styles: HashMap::new(),
            edge_styles: HashMap::new(),
            class_defs: HashMap::new(),
            subgraph_styles: HashMap::new(),
        }
    }

    /// Look up a node by its ID, returning a reference if found.
    ///
    /// # Arguments
    ///
    /// * `id` — the node identifier to search for
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::{Graph, Node, NodeShape, Direction};
    ///
    /// let mut g = Graph::new(Direction::LeftToRight);
    /// g.nodes.push(Node::new("A", "Start", NodeShape::Rectangle));
    /// assert_eq!(g.node("A").map(|n| n.label.as_str()), Some("Start"));
    /// assert!(g.node("Z").is_none());
    /// ```
    pub fn node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Return `true` if a node with `id` already exists.
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::{Graph, Node, NodeShape, Direction};
    ///
    /// let mut g = Graph::new(Direction::TopToBottom);
    /// g.nodes.push(Node::new("A", "A", NodeShape::Rectangle));
    /// assert!(g.has_node("A"));
    /// assert!(!g.has_node("B"));
    /// ```
    pub fn has_node(&self, id: &str) -> bool {
        self.nodes.iter().any(|n| n.id == id)
    }

    /// Insert a node, or update its label/shape if the ID already exists and
    /// the existing entry was auto-created as a bare-id placeholder.
    ///
    /// A "bare-id placeholder" is a node whose `label == id` and `shape == Rectangle`
    /// (the default produced when a node is first seen in an edge definition
    /// without an explicit shape). If such a placeholder already exists and the
    /// incoming `node` has a richer definition (different label or non-default shape),
    /// the placeholder is promoted to the richer definition.
    ///
    /// # Examples
    ///
    /// ```
    /// use mermaid_text::{Graph, Node, NodeShape, Direction};
    ///
    /// let mut g = Graph::new(Direction::LeftToRight);
    /// // Insert a bare-id placeholder.
    /// g.upsert_node(Node::new("A", "A", NodeShape::Rectangle));
    /// // Promote it to a richer definition.
    /// g.upsert_node(Node::new("A", "Start", NodeShape::Rounded));
    /// assert_eq!(g.node("A").unwrap().label, "Start");
    /// assert_eq!(g.node("A").unwrap().shape, NodeShape::Rounded);
    /// // If neither condition holds, the existing entry is kept.
    /// g.upsert_node(Node::new("A", "Other", NodeShape::Diamond));
    /// assert_eq!(g.node("A").unwrap().label, "Start"); // unchanged
    /// ```
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

    /// Build a flat map from node ID → the ID of the **innermost** subgraph
    /// that contains it (only direct `node_ids` members, not transitive).
    ///
    /// The map is computed on demand and not cached — call this once per
    /// render pass and keep the result locally.
    ///
    /// # Examples
    ///
    /// ```
    /// let graph = mermaid_text::parser::parse(
    ///     "graph LR\nsubgraph S\nA-->B\nend\nC",
    /// ).unwrap();
    /// let map = graph.node_to_subgraph();
    /// assert_eq!(map.get("A").map(String::as_str), Some("S"));
    /// assert_eq!(map.get("B").map(String::as_str), Some("S"));
    /// assert!(map.get("C").is_none());
    /// ```
    pub fn node_to_subgraph(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        // Walk all subgraphs (including nested ones reachable via subgraph_ids)
        // depth-first so that inner subgraphs overwrite outer ones for direct members.
        for sg in &self.subgraphs {
            self.collect_node_subgraph_map(sg, &mut map);
        }
        map
    }

    /// Recursive helper: walk `sg` and all its descendants, inserting
    /// node_id → sg.id for **direct** children (children of a child subgraph
    /// are overwritten by that child's own recursive call).
    fn collect_node_subgraph_map(&self, sg: &Subgraph, map: &mut HashMap<String, String>) {
        // Register direct node members first.
        for nid in &sg.node_ids {
            map.insert(nid.clone(), sg.id.clone());
        }
        // Recurse into nested subgraphs — their entries overwrite ours for
        // any nodes that appear in both (Mermaid allows implicit membership
        // through nesting).
        for child_id in &sg.subgraph_ids {
            if let Some(child) = self.find_subgraph(child_id) {
                self.collect_node_subgraph_map(child, map);
            }
        }
    }

    /// Find a subgraph by ID, searching recursively through all nesting levels.
    ///
    /// # Arguments
    ///
    /// * `id` — the subgraph identifier to search for
    ///
    /// # Examples
    ///
    /// ```
    /// let graph = mermaid_text::parser::parse(
    ///     "graph TD\nsubgraph Outer\nsubgraph Inner\nA\nend\nend",
    /// ).unwrap();
    /// assert!(graph.find_subgraph("Outer").is_some());
    /// assert!(graph.find_subgraph("Inner").is_some());
    /// assert!(graph.find_subgraph("Missing").is_none());
    /// ```
    pub fn find_subgraph(&self, id: &str) -> Option<&Subgraph> {
        fn search<'a>(sgs: &'a [Subgraph], all: &'a [Subgraph], id: &str) -> Option<&'a Subgraph> {
            for sg in sgs {
                if sg.id == id {
                    return Some(sg);
                }
                // Search in nested subgraphs by looking up their IDs.
                for child_id in &sg.subgraph_ids {
                    if let Some(found) = all.iter().find(|s| &s.id == child_id)
                        && let Some(result) = search(std::slice::from_ref(found), all, id)
                    {
                        return Some(result);
                    }
                }
            }
            None
        }
        search(&self.subgraphs, &self.subgraphs, id)
    }

    /// Collect all node IDs that belong to `sg` or any of its nested subgraphs.
    ///
    /// This is a deep traversal: nodes in nested subgraphs within `sg` are
    /// included in the result, not just direct `sg.node_ids` members.
    ///
    /// # Arguments
    ///
    /// * `sg` — the subgraph to collect nodes from (including descendants)
    ///
    /// # Examples
    ///
    /// ```
    /// let graph = mermaid_text::parser::parse(
    ///     "graph TD\nsubgraph Outer\nsubgraph Inner\nA\nend\nB\nend",
    /// ).unwrap();
    /// let outer = graph.find_subgraph("Outer").unwrap();
    /// let nodes = graph.all_nodes_in_subgraph(outer);
    /// assert!(nodes.contains(&"A".to_string()));
    /// assert!(nodes.contains(&"B".to_string()));
    /// ```
    pub fn all_nodes_in_subgraph(&self, sg: &Subgraph) -> Vec<String> {
        let mut result = sg.node_ids.clone();
        for child_id in &sg.subgraph_ids {
            if let Some(child) = self.find_subgraph(child_id) {
                result.extend(self.all_nodes_in_subgraph(child));
            }
        }
        result
    }
}
