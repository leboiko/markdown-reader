//! Hand-rolled parser for Mermaid `stateDiagram` / `stateDiagram-v2` syntax.
//!
//! State diagrams are transformed into the existing flowchart [`Graph`] type,
//! so the rest of the rendering pipeline (layered layout, A* edge routing,
//! ANSI color, ASCII fallback, width compaction) is reused unchanged. The
//! mapping is direct:
//!
//! - Each named state → a [`Node`] with [`NodeShape::Rounded`] (Mermaid's
//!   default state shape, visually distinct from flowcharts' default
//!   rectangles).
//! - `[*]` start (appears as transition source) → a synthesised node with
//!   id `__start__` (top-level) or `__start__<ancestor_path>` (inside a
//!   composite), shape [`NodeShape::Circle`], label `●`.
//! - `[*]` end (appears as transition destination) → analogous,
//!   id `__end__<…>`, shape [`NodeShape::DoubleCircle`].
//! - `state X { … }` blocks → a [`Subgraph`]. Recursive nesting supported.
//! - Edges referencing a composite ID are rewritten at parse time to point
//!   at the composite's synthesised `[*]` start (incoming) or end
//!   (outgoing), so they land visibly inside the composite border.
//! - Each transition → a solid arrow [`Edge`].
//! - `STATE : description` lines accumulate into a multi-line label.
//! - `state "Display" as Id` overrides the generated label.
//!
//! Out-of-scope features (concurrent regions `--`, fork / join / choice
//! shapes, notes, classDef / class / style / click, cross-composite
//! transitions) are silently skipped.

use std::collections::{HashMap, HashSet};

use crate::{
    Error,
    parser::common::{
        apply_pending_classes, extract_class_modifier, matches_keyword,
        parse_class_def_directive, parse_class_directive, parse_link_style_directive,
        parse_style_directive, strip_inline_comment,
    },
    types::{BarOrientation, Direction, Edge, Graph, Node, NodeShape, Subgraph},
};

const START_PREFIX: &str = "__start__";
const END_PREFIX: &str = "__end__";
const PATH_SEP: &str = "__";
const MARKER_LABEL: &str = "●";

/// Parse a Mermaid `stateDiagram` / `stateDiagram-v2` source string into a
/// [`Graph`] ready for the standard flowchart rendering pipeline.
///
/// # Errors
///
/// Returns [`Error::ParseError`] if the header is missing, a composite
/// state body is unterminated, a stray `}` appears at the top level, or a
/// non-blank line cannot be classified.
///
/// # Examples
///
/// ```
/// use mermaid_text::parser::state;
///
/// let graph = state::parse(
///     "stateDiagram-v2\n[*] --> Idle\nIdle --> Done\nDone --> [*]"
/// ).unwrap();
/// assert!(graph.has_node("Idle"));
/// assert!(graph.has_node("Done"));
/// // Synthesised top-level start and end markers:
/// assert!(graph.has_node("__start__"));
/// assert!(graph.has_node("__end__"));
/// assert_eq!(graph.edges.len(), 3);
/// ```
pub fn parse(input: &str) -> Result<Graph, Error> {
    let stmts = tokenise(input)?;

    // ---- Header -----------------------------------------------------------
    if stmts.is_empty() {
        return Err(Error::ParseError(
            "no 'stateDiagram' header found".to_string(),
        ));
    }
    let header = &stmts[0].1;
    let first_word = header.split_whitespace().next().unwrap_or("");
    let lower = first_word.to_lowercase();
    if lower != "statediagram" && lower != "statediagram-v2" {
        return Err(Error::ParseError(format!(
            "expected 'stateDiagram' or 'stateDiagram-v2' header, got '{first_word}'"
        )));
    }

    let mut walker = Walker::default();
    walker.parse_block(&stmts, 1, &[])?;
    walker.materialise()
}

// ---------------------------------------------------------------------------
// Tokenisation
// ---------------------------------------------------------------------------

/// Pre-tokenise the input into `(lineno, statement)` pairs. Strips inline
/// `%% comments`, drops blank lines and full-line comments, and consumes
/// multi-line `note … end note` blocks entirely.
fn tokenise(input: &str) -> Result<Vec<(usize, String)>, Error> {
    let lines: Vec<&str> = input.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let stripped = strip_inline_comment(raw).trim();
        let lineno = i + 1;
        i += 1;
        if stripped.is_empty() || stripped.starts_with("%%") {
            continue;
        }

        // Multi-line note: `note left of X` / `note right of X` (no colon)
        // → skip until `end note`.
        if let Some(rest) = stripped.strip_prefix("note ")
            && !rest.contains(':')
        {
            while i < lines.len() {
                let body = lines[i].trim();
                i += 1;
                if body == "end note" {
                    break;
                }
            }
            continue;
        }

        out.push((lineno, stripped.to_string()));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------------

struct Walker {
    seen: HashSet<String>,
    seen_order: Vec<String>,
    descriptions: HashMap<String, Vec<String>>,
    explicit_labels: HashMap<String, String>,
    shapes: HashMap<String, NodeShape>,
    edges: Vec<Edge>,
    direction: Direction,
    composite_ids: HashSet<String>,
    composite_order: Vec<String>,
    /// Map from composite id → its full ancestor path (root → composite, inclusive).
    composite_path: HashMap<String, Vec<String>>,
    /// Direct (non-recursive) node members per composite.
    composite_members: HashMap<String, Vec<String>>,
    /// Direct nested composite ids per composite.
    composite_children: HashMap<String, Vec<String>>,
    /// Per-composite direction override.
    composite_directions: HashMap<String, Direction>,
    /// State ids that received a `<<fork>>` or `<<join>>` modifier.
    /// Stored separately from `shapes` because the visual `Bar`
    /// orientation depends on the graph's flow direction, which we
    /// resolve at materialise time. Both fork and join collapse to the
    /// same `NodeShape::Bar` — they're visually identical and only the
    /// semantic role differs.
    pending_bar_kinds: HashSet<String>,
    /// `(target_id, class_name)` pairs from `class …` directives and
    /// inline `:::className` shorthands. Resolved at end-of-parse via
    /// the shared [`apply_pending_classes`] helper so a `class A foo`
    /// before `classDef foo …` works.
    pending_classes: Vec<(String, String)>,
    /// Scratch [`Graph`] populated incrementally by the shared style /
    /// classDef / linkStyle directive helpers (which need `&mut Graph`
    /// to write into the style registries). At [`materialise`] we
    /// consume its `node_styles` / `edge_styles` / `class_defs` /
    /// `subgraph_styles` into the final graph. Direction is set
    /// here too once it's known so the helpers don't need separate
    /// state.
    style_scratch: Graph,
}

impl Default for Walker {
    fn default() -> Self {
        Self {
            seen: HashSet::new(),
            seen_order: Vec::new(),
            descriptions: HashMap::new(),
            explicit_labels: HashMap::new(),
            shapes: HashMap::new(),
            edges: Vec::new(),
            // Mermaid's browser renderer defaults state diagrams to TB, but
            // in a text canvas TB blows up vertically: each layered-layout
            // "layer" inserts `layer_gap` (default 6) blank rows between
            // rows of nodes, so a 5-state chain balloons into 40+ rows of
            // mostly-empty space. LR keeps the layers horizontal so the
            // only vertical cost is the tallest node's label height.
            // Users who want the Mermaid default can still write
            // `direction TB` explicitly.
            direction: Direction::LeftToRight,
            composite_ids: HashSet::new(),
            composite_order: Vec::new(),
            composite_path: HashMap::new(),
            composite_members: HashMap::new(),
            composite_children: HashMap::new(),
            composite_directions: HashMap::new(),
            pending_bar_kinds: HashSet::new(),
            pending_classes: Vec::new(),
            // Direction will be overwritten in materialise; the
            // helpers don't read it, so any value works as the seed.
            style_scratch: Graph::new(Direction::LeftToRight),
        }
    }
}

impl Walker {
    /// Parse statements starting at `start`, returning the index of the
    /// statement *after* the matching `}` (or `stmts.len()` at top level).
    fn parse_block(
        &mut self,
        stmts: &[(usize, String)],
        start: usize,
        path: &[String],
    ) -> Result<usize, Error> {
        let mut i = start;
        while i < stmts.len() {
            let (lineno, stmt) = &stmts[i];

            // Closing brace returns control to the caller — but only if
            // we're inside a composite. At the top level it's a stray.
            if stmt == "}" {
                if path.is_empty() {
                    return Err(Error::ParseError(format!(
                        "line {lineno}: unexpected '}}' at top level"
                    )));
                }
                return Ok(i + 1);
            }

            // `direction LR/TB/BT/RL`
            if let Some(rest) = stmt.strip_prefix("direction ").map(str::trim) {
                if let Some(dir) = Direction::parse(rest) {
                    if let Some(parent) = path.last() {
                        self.composite_directions.insert(parent.clone(), dir);
                    } else {
                        self.direction = dir;
                    }
                }
                i += 1;
                continue;
            }

            // `note … :` single-line — silently skip. (Multi-line was already
            // consumed at tokenisation time.)
            if stmt.starts_with("note ") {
                i += 1;
                continue;
            }

            // Style / class directives — recognised. (style + linkStyle
            // were silently skipped before 0.8.0; they now apply to
            // state diagrams the same way they do for flowcharts.)
            if matches_keyword(stmt, "classDef") {
                parse_class_def_directive(stmt, &mut self.style_scratch);
                i += 1;
                continue;
            }
            if matches_keyword(stmt, "class") {
                parse_class_directive(stmt, &mut self.pending_classes);
                i += 1;
                continue;
            }
            if matches_keyword(stmt, "style") {
                parse_style_directive(stmt, &mut self.style_scratch);
                i += 1;
                continue;
            }
            if matches_keyword(stmt, "linkStyle") {
                parse_link_style_directive(stmt, &mut self.style_scratch);
                i += 1;
                continue;
            }

            // Other directives still silently skipped — orthogonal to color.
            if matches_keyword(stmt, "click")
                || matches_keyword(stmt, "accTitle")
                || matches_keyword(stmt, "accDescr")
                || matches_keyword(stmt, "scale")
                || stmt == "hide empty description"
            {
                i += 1;
                continue;
            }

            // `state Id {` (composite opener) or plain `state Id …`.
            if let Some(rest) = stmt.strip_prefix("state ") {
                let body = rest.trim();
                if let Some(header_body) = body.strip_suffix('{') {
                    // Composite opener. `header_body` is the part before `{`.
                    let header_body = header_body.trim();
                    let (composite_id, composite_label) =
                        parse_composite_header(header_body, *lineno)?;
                    self.open_composite(composite_id.clone(), composite_label, path);
                    let mut child_path = path.to_vec();
                    child_path.push(composite_id);
                    let after = self.parse_block(stmts, i + 1, &child_path)?;
                    if after == stmts.len() && (after == 0 || stmts[after - 1].1 != "}") {
                        return Err(Error::ParseError(format!(
                            "line {lineno}: composite state opened with `{{` is missing its closing `}}`"
                        )));
                    }
                    i = after;
                    continue;
                }
                // Plain `state …` declaration.
                self.handle_state_decl(body, path);
                i += 1;
                continue;
            }

            // Transition. Peel `:::cls1:::cls2` shorthand off each
            // endpoint BEFORE resolving (so `[*]:::started` → resolve
            // `[*]` then attach `started` to the mangled marker id).
            if let Some((from, to, label)) = split_transition(stmt) {
                let (from_clean, from_classes) = extract_class_modifier(&from);
                let (to_clean, to_classes) = extract_class_modifier(&to);
                let from_id = self.resolve_endpoint(&from_clean, EndpointSide::Source, path);
                let to_id = self.resolve_endpoint(&to_clean, EndpointSide::Destination, path);
                for c in from_classes {
                    self.pending_classes.push((from_id.clone(), c));
                }
                for c in to_classes {
                    self.pending_classes.push((to_id.clone(), c));
                }
                self.edges.push(Edge::new(from_id, to_id, label));
                i += 1;
                continue;
            }

            // `STATE : description`. The description form doesn't take
            // a class modifier on the id (Mermaid syntax doesn't allow
            // `A:::cls : desc`), so no extraction needed here.
            if let Some((id, desc)) = split_description(stmt) {
                self.register_node(&id, path);
                self.descriptions.entry(id).or_default().push(desc);
                i += 1;
                continue;
            }

            return Err(Error::ParseError(format!(
                "line {lineno}: unrecognised statement: '{stmt}'"
            )));
        }

        // Reached EOF. If we're inside a composite (path non-empty), that's
        // an unterminated block.
        if !path.is_empty() {
            return Err(Error::ParseError(format!(
                "composite state '{}' is missing its closing `}}`",
                path.last().unwrap()
            )));
        }
        Ok(i)
    }

    fn open_composite(&mut self, id: String, label: String, parent_path: &[String]) {
        if !self.composite_ids.insert(id.clone()) {
            // Duplicate composite opener for the same id — keep the first
            // declaration's metadata; subsequent body still extends it.
            return;
        }
        self.composite_order.push(id.clone());
        self.explicit_labels
            .entry(id.clone())
            .or_insert_with(|| label.clone());
        let mut full_path = parent_path.to_vec();
        full_path.push(id.clone());
        self.composite_path.insert(id.clone(), full_path);
        self.composite_members.entry(id.clone()).or_default();
        self.composite_children.entry(id.clone()).or_default();
        // Register the composite as a child of its parent.
        if let Some(parent_id) = parent_path.last() {
            self.composite_children
                .entry(parent_id.clone())
                .or_default()
                .push(id);
        }
    }

    fn register_node(&mut self, id: &str, path: &[String]) {
        if self.seen.insert(id.to_string()) {
            self.seen_order.push(id.to_string());
            if let Some(parent) = path.last() {
                self.composite_members
                    .entry(parent.clone())
                    .or_default()
                    .push(id.to_string());
            }
        }
    }

    fn resolve_endpoint(&mut self, raw: &str, side: EndpointSide, path: &[String]) -> String {
        if raw == "[*]" {
            let prefix = match side {
                EndpointSide::Source => START_PREFIX,
                EndpointSide::Destination => END_PREFIX,
            };
            let shape = match side {
                EndpointSide::Source => NodeShape::Circle,
                EndpointSide::Destination => NodeShape::DoubleCircle,
            };
            let id = mangle_marker(prefix, path);
            self.register_node(&id, path);
            self.shapes.entry(id.clone()).or_insert(shape);
            return id;
        }
        self.register_node(raw, path);
        raw.to_string()
    }

    fn handle_state_decl(&mut self, body: &str, path: &[String]) {
        // `"Display" as Id`
        if body.starts_with('"')
            && let Some(close_quote) = body[1..].find('"').map(|p| p + 1)
        {
            let display_raw = &body[1..close_quote];
            let after = body[close_quote + 1..].trim_start();
            if let Some(rest) = strip_keyword_prefix(after, "as") {
                let id = rest.split_whitespace().next().unwrap_or("").to_string();
                if !id.is_empty() {
                    self.register_node(&id, path);
                    let display = display_raw.replace("\\n", "\n");
                    self.explicit_labels.insert(id, display);
                    return;
                }
            }
        }

        // Plain `Id[:::cls] [modifier]` — split id and rest, then peel
        // any trailing `:::cls1:::cls2` shorthand off the id token.
        let mut parts = body.splitn(2, char::is_whitespace);
        let raw_id = parts.next().unwrap_or("").trim().to_string();
        if raw_id.is_empty() {
            return;
        }
        let (id, classes) = extract_class_modifier(&raw_id);
        self.register_node(&id, path);
        for c in classes {
            self.pending_classes.push((id.clone(), c));
        }
        let rest = parts.next().unwrap_or("");
        if let Some(kind) = parse_shape_modifier(rest) {
            match kind {
                ShapeKind::Choice => {
                    // Choice = decision diamond. Reuse the existing shape;
                    // no orientation resolution needed.
                    self.shapes.insert(id, NodeShape::Diamond);
                }
                ShapeKind::ForkOrJoin => {
                    // Defer to materialise-time so we know self.direction.
                    self.pending_bar_kinds.insert(id);
                }
            }
        }
    }

    /// Drop synthesised `[*]` marker nodes that are not connected (in the
    /// undirected graph) to any real user-declared state.
    ///
    /// This cleans up cases like `Active --> [*]` on a composite whose
    /// inner flow never reaches an end marker: the rewrite produces an
    /// `__end__Active --> __end__` pair that has no incoming edge from
    /// inside the composite, leaving two floating double-circles in the
    /// rendered output. GC removes them so the diagram only shows markers
    /// that visibly connect to real content.
    ///
    /// Real states (those not starting with `__start__` / `__end__`) are
    /// always kept even if fully disconnected — they're the user's model.
    fn gc_orphan_markers(&mut self) {
        // Build an undirected adjacency list over all seen ids.
        let mut neighbours: HashMap<String, Vec<String>> = HashMap::new();
        for id in &self.seen_order {
            neighbours.entry(id.clone()).or_default();
        }
        for edge in &self.edges {
            neighbours
                .entry(edge.from.clone())
                .or_default()
                .push(edge.to.clone());
            neighbours
                .entry(edge.to.clone())
                .or_default()
                .push(edge.from.clone());
        }

        // Seed the reachable set with every real (user-declared) node.
        let mut reachable: HashSet<String> = HashSet::new();
        let mut stack: Vec<String> = Vec::new();
        for id in &self.seen_order {
            if !is_marker_id(id) {
                reachable.insert(id.clone());
                stack.push(id.clone());
            }
        }
        // Also keep composite ids themselves reachable (they anchor the
        // subgraph). They're not in `seen_order` as nodes but they're
        // referenced by members.
        for id in &self.composite_order {
            reachable.insert(id.clone());
        }

        // Flood-fill undirected reachability from real nodes.
        while let Some(id) = stack.pop() {
            if let Some(adj) = neighbours.get(&id) {
                for n in adj.clone() {
                    if reachable.insert(n.clone()) {
                        stack.push(n);
                    }
                }
            }
        }

        // Drop markers that aren't reachable.
        let dropped: HashSet<String> = self
            .seen_order
            .iter()
            .filter(|id| is_marker_id(id) && !reachable.contains(id.as_str()))
            .cloned()
            .collect();
        if dropped.is_empty() {
            return;
        }
        self.seen_order.retain(|id| !dropped.contains(id));
        self.seen.retain(|id| !dropped.contains(id));
        self.shapes.retain(|id, _| !dropped.contains(id));
        self.descriptions.retain(|id, _| !dropped.contains(id));
        self.explicit_labels.retain(|id, _| !dropped.contains(id));
        for members in self.composite_members.values_mut() {
            members.retain(|id| !dropped.contains(id));
        }
        self.edges
            .retain(|e| !dropped.contains(&e.from) && !dropped.contains(&e.to));
    }

    /// Rewrite edges whose endpoints are composite IDs and synthesise the
    /// target start/end nodes inside the composite if they don't already
    /// exist.
    fn rewrite_composite_edges(&mut self) {
        let composite_ids = self.composite_ids.clone();
        let composite_paths = self.composite_path.clone();
        let edges = std::mem::take(&mut self.edges);
        let mut rewritten = Vec::with_capacity(edges.len());
        for edge in edges {
            let mut new_from = edge.from;
            let mut new_to = edge.to;

            if composite_ids.contains(&new_from) {
                let path = composite_paths
                    .get(&new_from)
                    .cloned()
                    .unwrap_or_else(|| vec![new_from.clone()]);
                let id = mangle_marker(END_PREFIX, &path);
                self.register_node(&id, &path);
                self.shapes
                    .entry(id.clone())
                    .or_insert(NodeShape::DoubleCircle);
                new_from = id;
            }
            if composite_ids.contains(&new_to) {
                let path = composite_paths
                    .get(&new_to)
                    .cloned()
                    .unwrap_or_else(|| vec![new_to.clone()]);
                let id = mangle_marker(START_PREFIX, &path);
                self.register_node(&id, &path);
                self.shapes.entry(id.clone()).or_insert(NodeShape::Circle);
                new_to = id;
            }

            rewritten.push(Edge::new(new_from, new_to, edge.label));
        }
        self.edges = rewritten;
    }

    /// Resolve `<<fork>>` / `<<join>>` modifiers to concrete `Bar`
    /// shapes now that the graph's flow direction is known. Bars are
    /// drawn perpendicular to flow (matching UML / Mermaid convention),
    /// so the orientation is derived from `self.direction`.
    fn resolve_pending_bars(&mut self) {
        if self.pending_bar_kinds.is_empty() {
            return;
        }
        let orientation = match self.direction {
            Direction::LeftToRight | Direction::RightToLeft => BarOrientation::Vertical,
            Direction::TopToBottom | Direction::BottomToTop => BarOrientation::Horizontal,
        };
        for id in self.pending_bar_kinds.drain() {
            self.shapes.insert(id, NodeShape::Bar(orientation));
        }
    }

    /// Build the final Graph.
    fn materialise(mut self) -> Result<Graph, Error> {
        self.resolve_pending_bars();
        self.rewrite_composite_edges();
        self.gc_orphan_markers();

        let mut graph = Graph::new(self.direction);
        for id in &self.seen_order {
            // Composite IDs themselves are subgraphs, not nodes.
            if self.composite_ids.contains(id) {
                continue;
            }
            let shape = self
                .shapes
                .get(id)
                .copied()
                .unwrap_or(NodeShape::Rounded);
            let label = if self.is_marker(id) {
                MARKER_LABEL.to_string()
            } else if let Some(explicit) = self.explicit_labels.get(id) {
                explicit.clone()
            } else if let Some(lines) = self.descriptions.get(id) {
                lines.join("\n")
            } else {
                id.clone()
            };
            graph.nodes.push(Node::new(id.clone(), label, shape));
        }
        for sg_id in &self.composite_order {
            let label = self
                .explicit_labels
                .get(sg_id)
                .cloned()
                .unwrap_or_else(|| sg_id.clone());
            let mut sg = Subgraph::new(sg_id.clone(), label);
            sg.direction = self.composite_directions.get(sg_id).copied();
            sg.node_ids = self
                .composite_members
                .get(sg_id)
                .cloned()
                .unwrap_or_default();
            sg.subgraph_ids = self
                .composite_children
                .get(sg_id)
                .cloned()
                .unwrap_or_default();
            graph.subgraphs.push(sg);
        }
        graph.edges = self.edges;

        // Move the style registries collected during the walk by the
        // shared directive helpers into the final graph, then resolve
        // pending class applications now that subgraphs exist.
        graph.node_styles = self.style_scratch.node_styles;
        graph.edge_styles = self.style_scratch.edge_styles;
        graph.class_defs = self.style_scratch.class_defs;
        graph.subgraph_styles = self.style_scratch.subgraph_styles;
        apply_pending_classes(&mut graph, &self.pending_classes);

        Ok(graph)
    }

    fn is_marker(&self, id: &str) -> bool {
        is_marker_id(id)
    }
}

/// Standalone helper so it can be used outside `impl Walker` (notably inside
/// closures that also borrow `self`).
fn is_marker_id(id: &str) -> bool {
    id.starts_with(START_PREFIX) || id.starts_with(END_PREFIX)
}

// ---------------------------------------------------------------------------
// Stateless helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum EndpointSide {
    Source,
    Destination,
}

/// Build the synthesised marker node id for a `[*]` reference at the given
/// composite path. Top-level (empty path) gives the bare prefix; nested
/// scopes append each composite id with `__` between.
/// Recognised UML shape modifiers that may follow a `state Id` declaration.
///
/// Two variants because choice maps directly to an existing `NodeShape`
/// while fork / join collapse to a single `NodeShape::Bar` whose
/// orientation depends on the graph's flow direction (resolved at
/// materialise time, not here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShapeKind {
    /// `<<choice>>` / `[[choice]]` → decision diamond.
    Choice,
    /// `<<fork>>` / `<<join>>` (and `[[…]]` variants) — both render as
    /// the same `NodeShape::Bar`. The semantic difference between fork
    /// (one in, many out) and join (many in, one out) is not visible
    /// in the rendered output.
    ForkOrJoin,
}

/// Detect a trailing `<<choice>>` / `<<fork>>` / `<<join>>` (or `[[…]]`)
/// shape modifier on a `state Id …` declaration. Returns `None` when no
/// recognised modifier is present (so plain `state Id` declarations
/// continue to use the default `Rounded` shape).
fn parse_shape_modifier(rest: &str) -> Option<ShapeKind> {
    match rest.trim() {
        "<<choice>>" | "[[choice]]" => Some(ShapeKind::Choice),
        "<<fork>>" | "[[fork]]" | "<<join>>" | "[[join]]" => Some(ShapeKind::ForkOrJoin),
        _ => None,
    }
}

fn mangle_marker(prefix: &str, path: &[String]) -> String {
    if path.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}{}", path.join(PATH_SEP))
    }
}

fn split_transition(stmt: &str) -> Option<(String, String, Option<String>)> {
    let arrow_pos = stmt.find("-->")?;
    let from = stmt[..arrow_pos].trim().to_string();
    let after = &stmt[arrow_pos + 3..];
    // Find the label-separator colon — the FIRST `:` that is NOT part of
    // a `:::className` shorthand. Walk past `:::` triples (3 consecutive
    // colons) and only stop on a lone `:` (which marks the start of the
    // edge label per Mermaid's `A --> B : label` syntax). Without this
    // the destination `B:::cls` would be split into `dest=B`, `label=:cls`.
    let label_colon = find_label_colon(after);
    let (dest_raw, label) = if let Some(colon_pos) = label_colon {
        (
            after[..colon_pos].trim().to_string(),
            Some(after[colon_pos + 1..].trim().to_string()),
        )
    } else {
        (after.trim().to_string(), None)
    };
    if from.is_empty() || dest_raw.is_empty() {
        return None;
    }
    Some((from, dest_raw, label.filter(|s| !s.is_empty())))
}

/// Find the byte index of the first `:` in `s` that is not part of a
/// `:::` triple (i.e. the label-separator colon). Returns `None` if no
/// such standalone colon exists.
fn find_label_colon(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' {
            // Triple-colon `:::`? Skip the whole triple.
            if i + 2 < bytes.len() && bytes[i + 1] == b':' && bytes[i + 2] == b':' {
                i += 3;
                continue;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

fn split_description(stmt: &str) -> Option<(String, String)> {
    let colon_pos = stmt.find(':')?;
    let id = stmt[..colon_pos].trim();
    let desc = stmt[colon_pos + 1..].trim();
    if id.is_empty() || desc.is_empty() || id.contains(char::is_whitespace) {
        return None;
    }
    Some((id.to_string(), desc.to_string()))
}

/// Parse the composite header body — the part between `state` and `{`. Returns
/// `(id, label)`.
///
/// Forms:
/// - `Id`
/// - `"Display Name" as Id`
fn parse_composite_header(body: &str, lineno: usize) -> Result<(String, String), Error> {
    if body.starts_with('"')
        && let Some(close_quote) = body[1..].find('"').map(|p| p + 1)
    {
        let display = body[1..close_quote].replace("\\n", "\n");
        let after = body[close_quote + 1..].trim_start();
        if let Some(rest) = strip_keyword_prefix(after, "as") {
            let id = rest.split_whitespace().next().unwrap_or("").to_string();
            if !id.is_empty() {
                return Ok((id, display));
            }
        }
        return Err(Error::ParseError(format!(
            "line {lineno}: composite header has a quoted display but no `as <Id>` follows"
        )));
    }
    let id = body.split_whitespace().next().unwrap_or("").to_string();
    if id.is_empty() {
        return Err(Error::ParseError(format!(
            "line {lineno}: composite header is missing an id"
        )));
    }
    let label = id.clone();
    Ok((id, label))
}

fn strip_keyword_prefix<'a>(s: &'a str, kw: &str) -> Option<&'a str> {
    let lower = s.to_lowercase();
    let lower_kw = kw.to_lowercase();
    if lower.starts_with(&lower_kw) {
        let rest = &s[kw.len()..];
        if rest.starts_with(char::is_whitespace) {
            Some(rest.trim_start())
        } else {
            None
        }
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

    // ---- Header --------------------------------------------------------

    #[test]
    fn header_required() {
        assert!(parse("").is_err());
        assert!(parse("[*] --> A").is_err());
    }

    #[test]
    fn accepts_both_keyword_variants() {
        assert!(parse("stateDiagram\n[*] --> A").is_ok());
        assert!(parse("stateDiagram-v2\n[*] --> A").is_ok());
        assert!(parse("StateDiagram-V2\n[*] --> A").is_ok());
    }

    // ---- Top-level [*] (regression guard for 0.5.0 byte-identical output)

    #[test]
    fn synthesises_start_node_for_left_star() {
        let g = parse("stateDiagram-v2\n[*] --> A").unwrap();
        let start = g.node("__start__").unwrap();
        assert_eq!(start.label, MARKER_LABEL);
        assert_eq!(start.shape, NodeShape::Circle);
        assert!(g.has_node("A"));
    }

    #[test]
    fn synthesises_end_node_for_right_star() {
        let g = parse("stateDiagram-v2\nA --> [*]").unwrap();
        let end = g.node("__end__").unwrap();
        assert_eq!(end.label, MARKER_LABEL);
        assert_eq!(end.shape, NodeShape::DoubleCircle);
    }

    #[test]
    fn start_and_end_can_coexist() {
        let g = parse("stateDiagram-v2\n[*] --> A\nA --> [*]").unwrap();
        assert!(g.has_node("__start__"));
        assert!(g.has_node("__end__"));
        assert_eq!(g.edges.len(), 2);
    }

    #[test]
    fn top_level_marker_ids_unchanged_regression_guard() {
        // 0.5.0 used "__start__" / "__end__" for top-level. v1.1 must keep
        // this exact id so existing snapshots don't regress.
        let g = parse("stateDiagram-v2\n[*] --> A\nA --> [*]").unwrap();
        let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"__start__"));
        assert!(ids.contains(&"__end__"));
    }

    #[test]
    fn self_transition() {
        let g = parse("stateDiagram-v2\nA --> A : retry").unwrap();
        assert_eq!(g.edges.len(), 1);
        assert_eq!(g.edges[0].from, "A");
        assert_eq!(g.edges[0].to, "A");
        assert_eq!(g.edges[0].label.as_deref(), Some("retry"));
    }

    // ---- Descriptions, labels, direction --------------------------------

    #[test]
    fn description_lines_accumulate() {
        let src = "stateDiagram-v2\nA : line one\nA : line two\nA : line three";
        let g = parse(src).unwrap();
        assert_eq!(g.node("A").unwrap().label, "line one\nline two\nline three");
    }

    #[test]
    fn explicit_label_form() {
        let g = parse("stateDiagram-v2\nstate \"Hello World\" as A").unwrap();
        assert_eq!(g.node("A").unwrap().label, "Hello World");
    }

    #[test]
    fn explicit_label_with_quoted_newline() {
        let g = parse("stateDiagram-v2\nstate \"Top\\nBottom\" as A").unwrap();
        assert_eq!(g.node("A").unwrap().label, "Top\nBottom");
    }

    #[test]
    fn explicit_label_overrides_descriptions() {
        let src =
            "stateDiagram-v2\nA : description that should be ignored\nstate \"Real Label\" as A";
        let g = parse(src).unwrap();
        assert_eq!(g.node("A").unwrap().label, "Real Label");
    }

    #[test]
    fn colons_in_labels_preserved() {
        let g = parse("stateDiagram-v2\nA --> B : key: value").unwrap();
        assert_eq!(g.edges[0].label.as_deref(), Some("key: value"));
    }

    #[test]
    fn colons_in_descriptions_preserved() {
        let g = parse("stateDiagram-v2\nA : status: active").unwrap();
        assert_eq!(g.node("A").unwrap().label, "status: active");
    }

    #[test]
    fn direction_override() {
        let g = parse("stateDiagram-v2\ndirection LR\n[*] --> A").unwrap();
        assert_eq!(g.direction, Direction::LeftToRight);
    }

    #[test]
    fn default_direction_is_left_to_right() {
        // mermaid-text intentionally defaults state diagrams to LR (vs.
        // Mermaid's TB) because TB balloons vertically in text output.
        // Users can still write `direction TB` for the Mermaid default.
        let g = parse("stateDiagram-v2\n[*] --> A").unwrap();
        assert_eq!(g.direction, Direction::LeftToRight);
    }

    #[test]
    fn explicit_direction_tb_still_honoured() {
        let g = parse("stateDiagram-v2\ndirection TB\n[*] --> A").unwrap();
        assert_eq!(g.direction, Direction::TopToBottom);
    }

    // ---- Comments / silent skips ---------------------------------------

    #[test]
    fn comments_skipped() {
        let src = "stateDiagram-v2\n%% this is a comment\nA --> B %% inline\n%% another";
        let g = parse(src).unwrap();
        assert_eq!(g.edges.len(), 1);
    }

    #[test]
    fn single_line_note_silently_skipped() {
        let g = parse("stateDiagram-v2\nA --> B\nnote right of A : hello").unwrap();
        assert_eq!(g.edges.len(), 1);
    }

    #[test]
    fn multi_line_note_silently_skipped() {
        let src =
            "stateDiagram-v2\nA --> B\nnote right of A\n  some text\n  more text\nend note\nB --> C";
        let g = parse(src).unwrap();
        assert_eq!(g.edges.len(), 2);
    }

    #[test]
    fn classdef_and_style_silently_skipped() {
        let src = "stateDiagram-v2\nclassDef foo fill:#f00\nclass A foo\nstyle A fill:#0f0\nA --> B";
        let g = parse(src).unwrap();
        assert_eq!(g.edges.len(), 1);
    }

    #[test]
    fn choice_modifier_assigns_diamond_shape() {
        let g = parse("stateDiagram-v2\nstate D <<choice>>\n[*] --> D").unwrap();
        assert_eq!(g.node("D").unwrap().shape, NodeShape::Diamond);
    }

    #[test]
    fn fork_modifier_assigns_bar_perpendicular_to_flow() {
        // Default is LR (per Walker::Default), so fork → vertical bar.
        let g = parse("stateDiagram-v2\nstate F <<fork>>\n[*] --> F").unwrap();
        assert_eq!(
            g.node("F").unwrap().shape,
            NodeShape::Bar(BarOrientation::Vertical)
        );
        // Explicit TB → horizontal bar.
        let g = parse("stateDiagram-v2\ndirection TB\nstate F <<fork>>\n[*] --> F").unwrap();
        assert_eq!(
            g.node("F").unwrap().shape,
            NodeShape::Bar(BarOrientation::Horizontal)
        );
    }

    #[test]
    fn join_modifier_uses_same_shape_as_fork() {
        // Both fork and join collapse to the same NodeShape::Bar — the
        // visual is identical; only the semantic role differs.
        let g = parse("stateDiagram-v2\nstate J <<join>>\n[*] --> J").unwrap();
        assert_eq!(
            g.node("J").unwrap().shape,
            NodeShape::Bar(BarOrientation::Vertical)
        );
    }

    #[test]
    fn double_bracket_shape_modifier_variants_accepted() {
        let g = parse("stateDiagram-v2\nstate D [[choice]]\n[*] --> D").unwrap();
        assert_eq!(g.node("D").unwrap().shape, NodeShape::Diamond);
        let g = parse("stateDiagram-v2\nstate F [[fork]]\n[*] --> F").unwrap();
        assert_eq!(
            g.node("F").unwrap().shape,
            NodeShape::Bar(BarOrientation::Vertical)
        );
        let g = parse("stateDiagram-v2\nstate J [[join]]\n[*] --> J").unwrap();
        assert_eq!(
            g.node("J").unwrap().shape,
            NodeShape::Bar(BarOrientation::Vertical)
        );
    }

    #[test]
    fn unrecognised_modifier_falls_through_to_default_shape() {
        // Defensive: a typo or unsupported `<<…>>` modifier shouldn't
        // crash or pick a wrong shape — the state stays default Rounded.
        let g = parse("stateDiagram-v2\nstate X <<typo>>\n[*] --> X").unwrap();
        assert_eq!(g.node("X").unwrap().shape, NodeShape::Rounded);
    }

    #[test]
    fn states_appear_in_source_order() {
        let g = parse("stateDiagram-v2\n[*] --> CLOSED\nCLOSED --> OPEN").unwrap();
        let ids: Vec<&str> = g.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["__start__", "CLOSED", "OPEN"]);
    }

    // ---- classDef / class / ::: ---------------------------------------
    // These mirror the flowchart-side tests but verify the same shared
    // helpers work for the state-diagram parser too.

    #[test]
    fn state_diagram_classdef_records_palette() {
        let src = "stateDiagram-v2
A --> B
classDef cache fill:#234,stroke:#9cf";
        let g = parse(src).unwrap();
        let style = g.class_defs.get("cache").copied().unwrap();
        assert_eq!(style.fill, Some(crate::types::Rgb(0x22, 0x33, 0x44)));
        assert_eq!(style.stroke, Some(crate::types::Rgb(0x99, 0xcc, 0xff)));
    }

    #[test]
    fn state_diagram_class_directive_applies_to_states() {
        let src = "stateDiagram-v2
A --> B
classDef hot fill:#f00
class A,B hot";
        let g = parse(src).unwrap();
        assert_eq!(
            g.node_styles.get("A").and_then(|s| s.fill),
            Some(crate::types::Rgb(0xff, 0, 0))
        );
        assert_eq!(
            g.node_styles.get("B").and_then(|s| s.fill),
            Some(crate::types::Rgb(0xff, 0, 0))
        );
    }

    #[test]
    fn state_diagram_triple_colon_inline_on_transition_endpoint() {
        let src = "stateDiagram-v2
A:::warm --> B:::cold
classDef warm fill:#f00
classDef cold fill:#00f";
        let g = parse(src).unwrap();
        assert_eq!(
            g.node_styles.get("A").and_then(|s| s.fill),
            Some(crate::types::Rgb(0xff, 0, 0))
        );
        assert_eq!(
            g.node_styles.get("B").and_then(|s| s.fill),
            Some(crate::types::Rgb(0, 0, 0xff))
        );
    }

    #[test]
    fn state_diagram_class_on_composite_lands_in_subgraph_styles() {
        let src = "stateDiagram-v2
state Active {
  Inner --> Inner
}
classDef accent stroke:#abc
class Active accent";
        let g = parse(src).unwrap();
        // Composite ID gets routed to subgraph_styles, not node_styles.
        assert_eq!(
            g.subgraph_styles.get("Active").and_then(|s| s.stroke),
            Some(crate::types::Rgb(0xaa, 0xbb, 0xcc))
        );
        assert!(g.node_styles.get("Active").is_none());
    }

    #[test]
    fn state_diagram_triple_colon_on_star_marker_attaches_to_mangled_id() {
        // `[*]:::started` — the marker is mangled to `__start__` so the
        // class application targets `__start__`, not `[*]`.
        let src = "stateDiagram-v2
[*]:::started --> A
classDef started fill:#0f0";
        let g = parse(src).unwrap();
        assert_eq!(
            g.node_styles.get("__start__").and_then(|s| s.fill),
            Some(crate::types::Rgb(0, 0xff, 0))
        );
    }

    #[test]
    fn state_diagram_style_directive_no_longer_silently_skipped() {
        // Pre-0.8.0 the state parser silently swallowed `style …`.
        // It now applies the same way it does in flowcharts.
        let src = "stateDiagram-v2
[*] --> A
style A fill:#abc";
        let g = parse(src).unwrap();
        assert_eq!(
            g.node_styles.get("A").and_then(|s| s.fill),
            Some(crate::types::Rgb(0xaa, 0xbb, 0xcc))
        );
    }

    // ---- Composite states (v1.1) ---------------------------------------

    #[test]
    fn simple_composite() {
        let src = "stateDiagram-v2
state X {
Inner1 --> Inner2
}";
        let g = parse(src).unwrap();
        assert_eq!(g.subgraphs.len(), 1);
        let sg = &g.subgraphs[0];
        assert_eq!(sg.id, "X");
        assert_eq!(sg.label, "X");
        assert_eq!(sg.node_ids, vec!["Inner1", "Inner2"]);
        assert!(g.has_node("Inner1"));
        assert!(g.has_node("Inner2"));
        assert_eq!(g.edges.len(), 1);
        // Composite id itself is not a node.
        assert!(g.node("X").is_none());
    }

    #[test]
    fn composite_with_internal_star_uses_scoped_marker() {
        let src = "stateDiagram-v2
state X {
[*] --> Inner
Inner --> [*]
}";
        let g = parse(src).unwrap();
        // Top-level markers must NOT appear (no top-level [*] in the source).
        assert!(!g.has_node("__start__"));
        assert!(!g.has_node("__end__"));
        // Scoped markers do appear:
        assert!(g.has_node("__start__X"));
        assert!(g.has_node("__end__X"));
        let sg = &g.subgraphs[0];
        assert!(sg.node_ids.contains(&"__start__X".to_string()));
        assert!(sg.node_ids.contains(&"__end__X".to_string()));
        assert!(sg.node_ids.contains(&"Inner".to_string()));
    }

    #[test]
    fn external_edge_into_composite_rewrites_to_scoped_start() {
        // Active must have an inner edge out of its own `[*]` — otherwise
        // the orphan-marker GC would (correctly) drop the synthesised
        // `__start__Active` as disconnected from any real state.
        let src = "stateDiagram-v2
[*] --> Active
state Active {
[*] --> Inner
Inner --> Inner
}";
        let g = parse(src).unwrap();
        // The external edge should now point to __start__Active, not Active.
        let edge = g.edges.iter().find(|e| e.from == "__start__").unwrap();
        assert_eq!(edge.to, "__start__Active");
        // The synthesised __start__Active is inside Active's subgraph.
        let sg = g.subgraphs.iter().find(|s| s.id == "Active").unwrap();
        assert!(sg.node_ids.contains(&"__start__Active".to_string()));
    }

    #[test]
    fn orphan_markers_are_dropped_by_gc() {
        // `Active --> [*]` on a composite whose inner flow never reaches an
        // end marker must not produce a floating `__end__Active` /
        // `__end__` pair in the rendered output.
        let src = "stateDiagram-v2
state Active {
[*] --> Idle
Idle --> Idle
}
Active --> [*]";
        let g = parse(src).unwrap();
        assert!(
            g.node("__end__Active").is_none(),
            "orphan __end__Active should be dropped"
        );
        assert!(
            g.node("__end__").is_none(),
            "orphan __end__ should be dropped"
        );
        // Entry path is still connected, so these survive.
        assert!(g.has_node("__start__Active"));
        assert!(g.has_node("Idle"));
    }

    #[test]
    fn external_edge_out_of_composite_rewrites_to_scoped_end() {
        let src = "stateDiagram-v2
state Active {
Inner --> Inner
}
Active --> Done";
        let g = parse(src).unwrap();
        let edge = g.edges.iter().find(|e| e.to == "Done").unwrap();
        assert_eq!(edge.from, "__end__Active");
        let sg = g.subgraphs.iter().find(|s| s.id == "Active").unwrap();
        assert!(sg.node_ids.contains(&"__end__Active".to_string()));
    }

    #[test]
    fn nested_composites() {
        let src = "stateDiagram-v2
state Outer {
state Inner {
Leaf --> Leaf
}
}";
        let g = parse(src).unwrap();
        assert_eq!(g.subgraphs.len(), 2);
        let outer = g.subgraphs.iter().find(|s| s.id == "Outer").unwrap();
        let inner = g.subgraphs.iter().find(|s| s.id == "Inner").unwrap();
        assert_eq!(outer.subgraph_ids, vec!["Inner"]);
        assert_eq!(inner.node_ids, vec!["Leaf"]);
    }

    #[test]
    fn nested_composite_marker_id_uses_full_path() {
        let src = "stateDiagram-v2
state Outer {
state Inner {
[*] --> Leaf
}
}";
        let g = parse(src).unwrap();
        assert!(g.has_node("__start__Outer__Inner"));
    }

    #[test]
    fn per_composite_direction_override() {
        let src = "stateDiagram-v2
direction TB
state X {
direction LR
A --> B
}";
        let g = parse(src).unwrap();
        assert_eq!(g.direction, Direction::TopToBottom);
        let sg = g.subgraphs.iter().find(|s| s.id == "X").unwrap();
        assert_eq!(sg.direction, Some(Direction::LeftToRight));
    }

    #[test]
    fn composite_explicit_label_form() {
        let src = "stateDiagram-v2
state \"Display Name\" as X {
A --> B
}";
        let g = parse(src).unwrap();
        let sg = g.subgraphs.iter().find(|s| s.id == "X").unwrap();
        assert_eq!(sg.label, "Display Name");
    }

    #[test]
    fn unterminated_composite_returns_error() {
        let src = "stateDiagram-v2
state X {
A --> B";
        let err = parse(src).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("missing its closing"),
            "expected unterminated-composite error, got: {msg}"
        );
    }

    #[test]
    fn stray_closing_brace_at_top_level_returns_error() {
        let src = "stateDiagram-v2
A --> B
}";
        let err = parse(src).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("unexpected '}'"),
            "expected stray-brace error, got: {msg}"
        );
    }

    #[test]
    fn composite_id_does_not_appear_as_a_node() {
        let src = "stateDiagram-v2
state Active {
Inner --> Inner
}
[*] --> Active";
        let g = parse(src).unwrap();
        assert!(g.node("Active").is_none(), "composite id leaked as a node");
        // It must appear as a subgraph instead.
        assert!(g.subgraphs.iter().any(|s| s.id == "Active"));
    }
}
