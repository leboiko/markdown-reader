//! Data model for Mermaid `journey` (user-journey) diagrams.
//!
//! A user-journey diagram records how satisfied an actor is at each step of
//! a process. Steps are grouped into named sections; each step has a title,
//! a satisfaction score (1–5), and one or more actors.
//!
//! Example source:
//!
//! ```text
//! journey
//!     title My working day
//!     section Go to work
//!       Make tea: 5: Me
//!       Do work: 1: Me, Cat
//!     section Go home
//!       Sit down: 3: Me
//! ```
//!
//! Constructed by [`crate::parser::journey::parse`] and consumed by
//! [`crate::render::journey::render`].

/// A single step in a user-journey diagram.
///
/// `title` is the display name of the step; `score` is the satisfaction
/// rating (1 = lowest, 5 = highest); `actors` is the list of participants
/// involved in this step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub title: String,
    pub score: u8,
    pub actors: Vec<String>,
}

/// A named group of [`Task`]s within a [`JourneyDiagram`].
///
/// `name` is `None` for tasks that appear before any `section` keyword
/// (Mermaid silently accepts these and groups them under an unnamed section).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: Option<String>,
    pub tasks: Vec<Task>,
}

/// A parsed `journey` diagram.
///
/// Constructed by [`crate::parser::journey::parse`] and consumed by
/// [`crate::render::journey::render`]. `title` is the optional diagram
/// title declared with the `title` keyword; `sections` is the ordered
/// list of sections (each containing its tasks).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct JourneyDiagram {
    pub title: Option<String>,
    pub sections: Vec<Section>,
}

impl JourneyDiagram {
    /// Total number of tasks across all sections.
    ///
    /// Useful for validation and summary display. Returns 0 when the diagram
    /// has no sections or all sections are empty.
    pub fn total_tasks(&self) -> usize {
        self.sections.iter().map(|s| s.tasks.len()).sum()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_tasks_across_sections() {
        let diag = JourneyDiagram {
            title: None,
            sections: vec![
                Section {
                    name: Some("A".to_string()),
                    tasks: vec![
                        Task {
                            title: "t1".to_string(),
                            score: 3,
                            actors: vec!["Me".to_string()],
                        },
                        Task {
                            title: "t2".to_string(),
                            score: 5,
                            actors: vec!["Me".to_string()],
                        },
                    ],
                },
                Section {
                    name: Some("B".to_string()),
                    tasks: vec![Task {
                        title: "t3".to_string(),
                        score: 1,
                        actors: vec!["Cat".to_string()],
                    }],
                },
            ],
        };
        assert_eq!(diag.total_tasks(), 3);
    }

    #[test]
    fn total_tasks_empty_diagram() {
        assert_eq!(JourneyDiagram::default().total_tasks(), 0);
    }
}
