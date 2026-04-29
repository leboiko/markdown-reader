//! Data model for Mermaid `mindmap` diagrams.
//!
//! A mindmap is an indent-based hierarchical outline with a single root node
//! and arbitrarily nested children. The nesting level is determined purely by
//! indentation in the source: deeper indentation means a child of the previous
//! less-indented node.
//!
//! Example source:
//!
//! ```text
//! mindmap
//!   root((mindmap))
//!     Origins
//!       Long history
//!       Popularisation
//!         British popular psychology author Tony Buzan
//!     Research
//!       On effectiveness and features
//!     Tools
//!       Pen and paper
//!       Mermaid
//! ```
//!
//! Constructed by [`crate::parser::mindmap::parse`] and consumed by
//! [`crate::render::mindmap::render`].
//!
//! ## Phase 1 limitations
//!
//! - Node shape variants (`((text))`, `(text)`, `{{text}}`, `))text((`,
//!   `)text(`) are all stripped to their inner text; all nodes render as
//!   plain text in the tree.
//! - `::icon(...)` directives are silently ignored.

/// A single node in a [`Mindmap`] tree.
///
/// `text` is the display label (stripped of any shape bracket syntax);
/// `children` is the ordered list of immediate child nodes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MindmapNode {
    pub text: String,
    pub children: Vec<MindmapNode>,
}

impl MindmapNode {
    /// Create a new leaf node with the given text.
    pub fn new(text: impl Into<String>) -> Self {
        MindmapNode {
            text: text.into(),
            children: Vec::new(),
        }
    }

    /// Total number of nodes in this subtree, including `self`.
    pub fn node_count(&self) -> usize {
        1 + self.children.iter().map(MindmapNode::node_count).sum::<usize>()
    }
}

/// A parsed `mindmap` diagram.
///
/// Constructed by [`crate::parser::mindmap::parse`] and consumed by
/// [`crate::render::mindmap::render`]. The diagram has exactly one root node;
/// all other nodes are children (or descendants) of that root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mindmap {
    pub root: MindmapNode,
}

impl Default for Mindmap {
    fn default() -> Self {
        Mindmap {
            root: MindmapNode::new("root"),
        }
    }
}

impl Mindmap {
    /// Total number of nodes in the diagram (root + all descendants).
    pub fn node_count(&self) -> usize {
        self.root.node_count()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mindmap_has_one_node() {
        let m = Mindmap::default();
        assert_eq!(m.node_count(), 1);
        assert_eq!(m.root.text, "root");
        assert!(m.root.children.is_empty());
    }

    #[test]
    fn node_count_counts_all_descendants() {
        let diag = Mindmap {
            root: MindmapNode {
                text: "root".to_string(),
                children: vec![
                    MindmapNode {
                        text: "A".to_string(),
                        children: vec![MindmapNode::new("A1"), MindmapNode::new("A2")],
                    },
                    MindmapNode::new("B"),
                ],
            },
        };
        // root(1) + A(1) + A1(1) + A2(1) + B(1) = 5
        assert_eq!(diag.node_count(), 5);
    }

    #[test]
    fn equality_holds_for_identical_trees() {
        let a = Mindmap {
            root: MindmapNode {
                text: "root".to_string(),
                children: vec![MindmapNode::new("child")],
            },
        };
        let b = a.clone();
        assert_eq!(a, b);

        let c = Mindmap {
            root: MindmapNode {
                text: "root".to_string(),
                children: vec![MindmapNode::new("other")],
            },
        };
        assert_ne!(a, c);
    }
}
