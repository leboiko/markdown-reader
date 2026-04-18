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
}

impl Graph {
    /// Construct a new empty graph with the given direction.
    pub fn new(direction: Direction) -> Self {
        Self {
            direction,
            nodes: Vec::new(),
            edges: Vec::new(),
            subgraphs: Vec::new(),
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

    /// Build a flat map from node ID → the ID of the **innermost** subgraph
    /// that contains it (only direct `node_ids` members, not transitive).
    ///
    /// The map is computed on demand and not cached — call this once per
    /// render pass and keep the result locally.
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
