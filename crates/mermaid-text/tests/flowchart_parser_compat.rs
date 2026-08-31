use mermaid_text::parser;
use mermaid_text::{render, render_ascii};
use std::time::{Duration, Instant};

struct Case {
    source: &'static str,
    expected_nodes: &'static [(&'static str, &'static str)],
    expected_edge_label: Option<&'static str>,
}

fn assert_round_trip(case: Case) {
    let graph = parser::parse(case.source).expect("compatibility source should parse");
    let actual_nodes = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.label.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        actual_nodes, case.expected_nodes,
        "node labels changed for source:\n{}",
        case.source
    );
    assert_eq!(graph.edges.len(), 1, "source:\n{}", case.source);
    assert_eq!(
        graph.edges[0].label.as_deref(),
        case.expected_edge_label,
        "edge label changed for source:\n{}",
        case.source
    );

    for output in [
        render(case.source).expect("Unicode render should succeed"),
        render_ascii(case.source).expect("ASCII render should succeed"),
    ] {
        for (_, label) in case.expected_nodes {
            for line in label.lines() {
                let segment = line.trim();
                assert_eq!(
                    output.matches(segment).count(),
                    1,
                    "node label segment {segment:?} was not rendered exactly once:\n{output}"
                );
            }
        }
        if let Some(label) = case.expected_edge_label {
            assert_eq!(
                output.matches(label).count(),
                1,
                "edge label {label:?} was not rendered exactly once:\n{output}"
            );
        }
    }
}

#[test]
fn semicolon_inside_shaped_label_is_not_a_statement_separator() {
    assert_round_trip(Case {
        source: "flowchart LR\n    a[Alpha] --> b[Beta; gamma]\n",
        expected_nodes: &[("a", "Alpha"), ("b", "Beta; gamma")],
        expected_edge_label: None,
    });
}

#[test]
fn quoted_shaped_label_decodes_numeric_quote_entities() {
    assert_round_trip(Case {
        source: "flowchart LR\n    say[\"say #34;hi#34; x\"]\n    end[end]\n    say --> end\n",
        expected_nodes: &[("say", "say \"hi\" x"), ("end", "end")],
        expected_edge_label: None,
    });
}

#[test]
fn newline_inside_shaped_label_is_not_a_statement_separator() {
    assert_round_trip(Case {
        source: "flowchart LR\n    a[Alpha\n    Beta] --> b[Gamma]\n",
        expected_nodes: &[("a", "Alpha\n    Beta"), ("b", "Gamma")],
        expected_edge_label: None,
    });
}

#[test]
fn unmatched_quote_inside_shaped_label_does_not_hide_the_edge() {
    assert_round_trip(Case {
        source: "flowchart LR\n    a[Alpha \"Beta] --> b[\"Gamma\"]\n",
        expected_nodes: &[("a", "Alpha \"Beta"), ("b", "Gamma")],
        expected_edge_label: None,
    });
}

#[test]
fn pipe_inside_pipe_delimited_edge_label_is_label_content() {
    assert_round_trip(Case {
        source: "flowchart LR\n    a -->|one|two| b\n",
        expected_nodes: &[("a", "a"), ("b", "b")],
        expected_edge_label: Some("one|two"),
    });
}

#[test]
fn separate_pipe_labels_in_an_edge_chain_stay_separate() {
    let graph = parser::parse("flowchart LR\n    a-->|first|b-->|second|c\n")
        .expect("labeled edge chain should parse");
    let actual_nodes = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.label.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(actual_nodes, [("a", "a"), ("b", "b"), ("c", "c")]);
    let actual_edges = graph
        .edges
        .iter()
        .map(|edge| (edge.from.as_str(), edge.to.as_str(), edge.label.as_deref()))
        .collect::<Vec<_>>();
    assert_eq!(
        actual_edges,
        [("a", "b", Some("first")), ("b", "c", Some("second"))]
    );
}

#[test]
fn balanced_quoted_labels_protect_structural_characters() {
    assert_round_trip(Case {
        source: "flowchart LR\n    a[\"left [ bracket\"] --> b[End]\n",
        expected_nodes: &[("a", "left [ bracket"), ("b", "End")],
        expected_edge_label: None,
    });
    assert_round_trip(Case {
        source: "flowchart LR\n    a[\"x ] & y\"] --> b[End]\n",
        expected_nodes: &[("a", "x ] & y"), ("b", "End")],
        expected_edge_label: None,
    });
}

#[test]
fn comment_delimiters_cannot_suppress_later_statements() {
    assert_round_trip(Case {
        source: "flowchart LR\n%% TODO: support [later\nA[Start] --> B[End]\n",
        expected_nodes: &[("A", "Start"), ("B", "End")],
        expected_edge_label: None,
    });
}

#[test]
fn unmatched_quote_cannot_borrow_from_a_later_statement() {
    let source = "flowchart LR\na[Alpha \"Beta]\nb[\"Gamma\"]\nc[End]\n";
    let graph = parser::parse(source).expect("malformed label should stay isolated");
    let actual_nodes = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node.label.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        actual_nodes,
        [("a", "Alpha \"Beta"), ("b", "Gamma"), ("c", "End")]
    );
    assert!(graph.edges.is_empty());
}

#[test]
fn unclosed_shape_cannot_consume_later_valid_edges() {
    let source = "flowchart LR\nA[bad\nB-->C\nD-->E\n";
    let graph = parser::parse(source).expect("later statements should remain parseable");
    let actual_edges = graph
        .edges
        .iter()
        .map(|edge| (edge.from.as_str(), edge.to.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(actual_edges, [("B", "C"), ("D", "E")]);
    assert_eq!(graph.nodes.len(), 5, "unexpected nodes: {:?}", graph.nodes);
}

#[test]
fn arrow_like_text_after_an_inner_pipe_stays_in_the_label() {
    assert_round_trip(Case {
        source: "flowchart LR\n    a -->|one|-->two| b\n",
        expected_nodes: &[("a", "a"), ("b", "b")],
        expected_edge_label: Some("one|-->two"),
    });
}

#[test]
fn pipe_rich_label_parses_within_a_linear_time_budget() {
    let label = format!("{}tail", "x|".repeat(32_768));
    let source = format!("flowchart LR\na-->|{label}|b\n");
    let started = Instant::now();
    let graph = parser::parse(&source).expect("large label should parse");
    let elapsed = started.elapsed();

    assert_eq!(graph.edges[0].label.as_deref(), Some(label.as_str()));
    assert!(
        elapsed < Duration::from_millis(500),
        "64 KiB pipe-rich label took {elapsed:?}; expected a linear scan"
    );
}
