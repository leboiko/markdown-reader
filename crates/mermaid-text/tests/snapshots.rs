//! Snapshot tests for canonical rendered Mermaid diagrams.
//!
//! Each test renders a fixed Mermaid source string and compares the output
//! against a committed `.snap` file. Any silent visual regression (layout
//! change, character substitution, line collapse) will cause the test to fail
//! and show a diff.
//!
//! To regenerate snapshots after an intentional rendering change:
//!   INSTA_UPDATE=always cargo test -p mermaid-text --test snapshots
//! then commit the updated `.snap` files.

// Snapshot tests render real output and insta manages the `.snap` files, so
// unused-variable warnings on the rendered string are not meaningful here.
#![allow(clippy::items_after_test_module)]

use insta::assert_snapshot;

// ---------------------------------------------------------------------------
// 1. Simple left-to-right chain
// ---------------------------------------------------------------------------
#[test]
fn simple_chain_lr() {
    let out = mermaid_text::render("graph LR; A-->B-->C").unwrap();
    assert_snapshot!("simple_chain_lr", out);
}

// ---------------------------------------------------------------------------
// 2. Simple top-down chain
// ---------------------------------------------------------------------------
#[test]
fn simple_chain_td() {
    let out = mermaid_text::render("graph TD; A-->B-->C").unwrap();
    assert_snapshot!("simple_chain_td", out);
}

// ---------------------------------------------------------------------------
// 3. Diamond with labelled branches (yes/no decision)
// ---------------------------------------------------------------------------
#[test]
fn diamond_with_branches() {
    let src = r#"graph TD
        A[Start]-->B{Ok?}
        B-->|Yes|C[Go]
        B-->|No|D[Stop]"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("diamond_with_branches", out);
}

// ---------------------------------------------------------------------------
// 4. All supported node shapes in one diagram
// ---------------------------------------------------------------------------
#[test]
fn all_node_shapes() {
    let src = r#"graph TD
        R[Rectangle]
        Ro(Rounded)
        Di{Diamond}
        Ci((Circle))
        St([Stadium])
        Su[[Subroutine]]
        Cy[(Cylinder)]
        Hx{{Hexagon}}
        As>Asymmetric]
        Pa[/Parallelogram/]
        Tr[/Trapezoid\]
        Dc(((DoubleCircle)))"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("all_node_shapes", out);
}

// ---------------------------------------------------------------------------
// 5. All supported edge styles
// ---------------------------------------------------------------------------
#[test]
fn all_edge_styles() {
    let src = r#"graph LR
        A-->B
        A-.->C
        A==>D
        A---E
        A<-->F
        A--oG
        A--xH"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("all_edge_styles", out);
}

// ---------------------------------------------------------------------------
// 6. Single subgraph, left-to-right
// ---------------------------------------------------------------------------
#[test]
fn single_subgraph_lr() {
    let src = r#"graph LR
        subgraph SG[My Group]
            A-->B
        end
        B-->C"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("single_subgraph_lr", out);
}

// ---------------------------------------------------------------------------
// 7. Nested subgraphs, top-down
// ---------------------------------------------------------------------------
#[test]
fn nested_subgraphs_td() {
    let src = r#"graph TD
        subgraph Outer
            subgraph Inner
                A-->B
            end
            B-->C
        end
        C-->D"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("nested_subgraphs_td", out);
}

// ---------------------------------------------------------------------------
// 8. Three sibling subgraphs LR — regression for v0.2.2 overlap bug
// ---------------------------------------------------------------------------
#[test]
fn three_sibling_subgraphs_lr() {
    let src = r#"graph LR
        subgraph Alpha
            A1-->A2
        end
        subgraph Beta
            B1-->B2
        end
        subgraph Gamma
            G1-->G2
        end
        A2-->B1
        B2-->G1"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("three_sibling_subgraphs_lr", out);
}

// ---------------------------------------------------------------------------
// 9. Subgraph with perpendicular direction override — regression for v0.2.3
// ---------------------------------------------------------------------------
#[test]
fn perpendicular_subgraph_direction() {
    let src = r#"graph LR
        subgraph Sub
            direction TD
            X-->Y-->Z
        end
        A-->Sub"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("perpendicular_subgraph_direction", out);
}

// ---------------------------------------------------------------------------
// 10. Multi-line label via <br/> — regression for v0.2.3 flattening bug
// ---------------------------------------------------------------------------
#[test]
fn multiline_label_br() {
    let src = r#"graph TD
        A["Line one<br/>Line two<br/>Line three"]-->B[End]"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("multiline_label_br", out);
}

// ---------------------------------------------------------------------------
// 11. Long label that requires soft-wrapping — regression for v0.2.3
// ---------------------------------------------------------------------------
#[test]
fn long_label_soft_wrapped() {
    let src = r#"graph TD
        A["This is a very long label that should be soft-wrapped by the renderer"]-->B[Done]"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("long_label_soft_wrapped", out);
}

// ---------------------------------------------------------------------------
// 12. Cylinder node inside a flow — regression for v0.2.4 cylinder redesign
// ---------------------------------------------------------------------------
#[test]
fn cylinder_in_flow() {
    let src = r#"graph LR
        A[App]-->DB[(Database)]-->B[Cache]-->C[Output]"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("cylinder_in_flow", out);
}

// ---------------------------------------------------------------------------
// 13. Edge crossing subgraph boundary — regression for v0.2.5 A* fallback bug
//     Multi-source, multi-target deployment scenario similar to intuition-v2.
// ---------------------------------------------------------------------------
#[test]
fn edge_crosses_subgraph_boundary() {
    let src = r#"graph LR
        subgraph Infra
            DB[(Postgres)]
            Cache[(Redis)]
        end
        subgraph Services
            API[API Server]
            Worker[Worker]
        end
        API-->DB
        API-->Cache
        Worker-->DB
        Worker-->Cache
        LB[Load Balancer]-->API
        LB-->Worker"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("edge_crosses_subgraph_boundary", out);
}

// ---------------------------------------------------------------------------
// 14. Width-constrained rendering — compaction under tight budget
// ---------------------------------------------------------------------------
#[test]
fn width_constrained_rendering() {
    let src = r#"graph LR
        A[Alpha]-->B[Bravo]-->C[Charlie]-->D[Delta]-->E[Echo]"#;
    // 40 columns is tight enough to force compaction on most configurations.
    let out = mermaid_text::render_with_width(src, Some(40)).unwrap();
    assert_snapshot!("width_constrained_rendering", out);
}

// ---------------------------------------------------------------------------
// 15. Crossing edges that should produce cross-junction characters (┼)
// ---------------------------------------------------------------------------
#[test]
fn crossing_edges_with_cross_junction() {
    let src = r#"graph LR
        A-->C
        B-->D
        A-->D
        B-->C"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("crossing_edges_with_cross_junction", out);
}
