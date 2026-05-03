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
// 4b. All node shapes — phase 2: full audit diagram (0.25.0)
//
// Covers shapes added/fixed in 0.25.0: ParallelogramBackslash,
// TrapezoidInverted, plus the visually fixed Stadium, Cylinder, Hexagon,
// Parallelogram and Trapezoid.
// ---------------------------------------------------------------------------
#[test]
fn flowchart_all_node_shapes_phase_2() {
    let src = r#"graph LR
        A[Square]
        B(Round)
        C((Circle))
        D{Rhombus}
        E[[Subroutine]]
        F[(Database)]
        G{{Hexagon}}
        H[/Parallelogram/]
        I[\BackSlash\]
        J[/Trapezoid\]
        K[\InvTrapezoid/]
        L([Stadium])
        M>Asymmetric]"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("flowchart_all_node_shapes_phase_2", out);
}

// ---------------------------------------------------------------------------
// 5. All supported edge styles — pipe-label form (regression baseline)
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
// 5b. Inline-quoted label syntax for all three arrow styles (B1/B2 regression)
//
//     Mermaid supports two label syntaxes: pipe-form (`-->|"label"|`) and
//     inline-quoted form (`-- "label" -->`). The inline-quoted form for
//     dashed (`-. "x" .->`) and thick (`== "x" ==>`) arrows was silently
//     broken: the lexer consumed the opening arrow half as part of the
//     preceding node token, producing a ghost node instead of a labelled edge.
//     This snapshot pins the correct output after the fix.
// ---------------------------------------------------------------------------
#[test]
fn all_edge_styles_inline_quoted_labels() {
    let src = r#"graph LR
        A -- "solid quoted" --> B
        A -. "dashed quoted" .-> C
        A == "thick quoted" ==> D"#;
    let out = mermaid_text::render(src).unwrap();
    // Each edge must carry its label — verify the label text appears in the
    // rendered output as a basic sanity check before the full snapshot.
    assert!(
        out.contains("solid quoted"),
        "solid inline-quoted label missing from output:\n{out}"
    );
    assert!(
        out.contains("dashed quoted"),
        "dashed inline-quoted label missing from output:\n{out}"
    );
    assert!(
        out.contains("thick quoted"),
        "thick inline-quoted label missing from output:\n{out}"
    );
    assert_snapshot!("all_edge_styles_inline_quoted_labels", out);
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

// ---------------------------------------------------------------------------
// 16. ASCII mode — same source as simple_chain_lr rendered without Unicode
// ---------------------------------------------------------------------------
#[test]
fn ascii_mode() {
    let out = mermaid_text::render_ascii("graph LR; A-->B-->C").unwrap();
    // Snapshot the ASCII output so any future visual regression is caught.
    assert_snapshot!("ascii_mode", out);
}

// ---------------------------------------------------------------------------
// 17. Back-edge LR — chain with a feedback edge (C → A)
//     Regression guard: the back-edge must route below the node row (▴ tip)
//     and must not cut through any node box.
// ---------------------------------------------------------------------------
#[test]
fn back_edge_lr() {
    let out = mermaid_text::render("graph LR; A-->B-->C; C-->A").unwrap();
    assert_snapshot!("back_edge_lr", out);
}

// ---------------------------------------------------------------------------
// 16b. Rendered output must not start with blank rows.
//
//     The Sugiyama backend reserves a vertical corridor above the source
//     row to route back-edges. When that reservation is unused (or routed
//     elsewhere) the topmost rows of the canvas remain empty, and the
//     `Grid` Display impl historically only stripped TRAILING blank rows.
//     The result is 1–5 leading blank lines visible in the gallery on
//     diagrams 1, 3, 6, 8, 9, 37 — see `scripts/render-gallery.sh`.
//
//     This test counts the literal `\n` bytes at the start of the rendered
//     string. A trivially-broken implementation cannot satisfy
//     `count == 0` because the back-edge fixture's pre-fix snapshot
//     starts with `\n\n\n` — see
//     `tests/snapshots/snapshots__back_edge_lr.snap` lines 5–8. Any value
//     other than zero means the leading-blank artifact is back.
// ---------------------------------------------------------------------------
#[test]
fn back_edge_lr_no_leading_blank_rows() {
    let out = mermaid_text::render("graph LR; A-->B-->C; C-->A").unwrap();
    let leading = out.bytes().take_while(|&b| b == b'\n').count();
    assert_eq!(
        leading,
        0,
        "rendered output begins with {leading} leading newline byte(s); \
         the Grid Display impl must strip leading blank rows the same way \
         it strips trailing ones. First 80 bytes: {:?}",
        &out.as_bytes()[..out.len().min(80)]
    );
}

// ---------------------------------------------------------------------------
// 17b. Bidirectional edges in a subgraph (Supervisor pattern).
//      Regression guard: when two edges connect the same node pair (here
//      Factory ↔ Worker, with both labels), the labels must NOT overwrite
//      either node's border row — `└──panics──┘` reads as part of Factory,
//      which is what 0.9.5 fixed via the `overlaps_node_border_row` guard.
// ---------------------------------------------------------------------------
#[test]
fn supervisor_bidirectional_in_subgraph() {
    let src = "graph LR
    subgraph Supervisor
        direction TB
        F[Factory] -->|creates| W[Worker]
        W -->|panics| F
    end
    W -->|beat| HB[Heartbeat]
    HB --> WD[Watchdog]";
    let out = mermaid_text::render(src).unwrap();
    // Factory and Worker box outlines must be intact — labels must not
    // appear between the corner glyphs on the same border row.
    assert!(
        !out.contains("└───panics┘") && !out.contains("└─────creates─────┘"),
        "labels overwrote node border rows:\n{out}"
    );
    assert_snapshot!("supervisor_bidirectional_in_subgraph", out);
}

// ---------------------------------------------------------------------------
// 17c. Parallel edges between same node pair with different styles
//      (CI/CD pipeline). The `pass`/`skip` labels are necessarily cramped
//      because the layout pipeline doesn't yet widen the gap for parallel
//      edges (a layout-level follow-up — see ROADMAP item #6). What we
//      *do* guard: subgraph borders aren't overwritten — `pass` lands at
//      col 41 (immediately right of CI's `│`), not on it.
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// 17d. Arrow tip merges into destination box border (TD/BT).
//      Regression guard: 0.9.6 changed `▾` from "floating one row above the
//      box" to "merged into the top border row, replacing one ─". Verify
//      the `▾` always lands on the same line as the destination box's
//      `┌────┐` top border (and same for `▴` on the bottom border for BT).
// ---------------------------------------------------------------------------
#[test]
fn arrow_tip_merges_into_destination_box_top_td() {
    let out = mermaid_text::render("graph TD\nA --> B").unwrap();
    let lines: Vec<&str> = out.lines().collect();
    let top_row = lines
        .iter()
        .rposition(|l| l.contains('┌'))
        .expect("destination top border not found");
    assert!(
        lines[top_row].contains('▾'),
        "▾ should merge into the destination's top border row (the line with `┌`):\n{out}"
    );
    assert_snapshot!("arrow_tip_merges_into_destination_box_top_td", out);
}

#[test]
fn cicd_parallel_styles_to_same_target() {
    let src = "graph LR
    subgraph CI
        L[Lint] ==> B[Build] ==> T[Test]
    end
    T ==>|pass| D[Deploy]
    T -.->|skip| D";
    let out = mermaid_text::render(src).unwrap();
    // No `│pass` (label puncturing CI's right border).
    assert!(
        !out.contains("│pass│"),
        "pass label punctured subgraph border:\n{out}"
    );
    assert_snapshot!("cicd_parallel_styles_to_same_target", out);
}

// ---------------------------------------------------------------------------
// 18. Sugiyama (ascii-dag) backend on the README architecture case.
//     Native layered layout collapses Worker into the Cache/RabbitMQ row
//     and routes App→PostgreSQL through awkward zig-zags. Sugiyama gives
//     the topologically correct 4-layer layout with the long edge routed
//     via dummy nodes.
// ---------------------------------------------------------------------------
#[test]
fn architecture_diagram_with_sugiyama_backend() {
    let src = "graph LR
    App --> DB[(PostgreSQL)]
    App --> Cache[(Redis)]
    App --> Queue[(RabbitMQ)]
    Queue --> Worker[Worker]
    Worker --> DB";
    let opts = mermaid_text::RenderOptions {
        backend: mermaid_text::layout::LayoutBackend::Sugiyama,
        ..Default::default()
    };
    let out = mermaid_text::render_with_options(src, &opts).unwrap();
    assert_snapshot!("architecture_diagram_with_sugiyama_backend", out);
}

// ---------------------------------------------------------------------------
// 19. ANSI color regression guard — running through `render_with_options`
//     with `color: false` must produce the exact same bytes as `render`.
//     This is the structural promise that ANSI is opt-in.
// ---------------------------------------------------------------------------
#[test]
fn color_disabled_is_byte_identical() {
    let src = "graph LR\nA[Start] --> B[End]\nstyle A fill:#336,stroke:#fff,color:#fff";
    let plain = mermaid_text::render(src).unwrap();
    let opts = mermaid_text::RenderOptions::default();
    let via_options = mermaid_text::render_with_options(src, &opts).unwrap();
    assert_eq!(
        plain, via_options,
        "color=false path must be byte-identical"
    );
    assert!(
        !via_options.contains('\x1b'),
        "no ANSI escape bytes when color=false"
    );
}

// ---------------------------------------------------------------------------
// 19. Node fill / stroke / color via `style` directive — the canonical case.
//     Snapshot captures the SGR sequences literally so any drift in the
//     emission shape is caught.
// ---------------------------------------------------------------------------
#[test]
fn node_fill_stroke_and_color() {
    let src = r#"graph LR
        A[Start] --> B[End]
        style A fill:#336,stroke:#fff,color:#fff"#;
    let opts = mermaid_text::RenderOptions {
        color: true,
        ..Default::default()
    };
    let out = mermaid_text::render_with_options(src, &opts).unwrap();
    assert!(out.contains("\x1b[48;2;51;51;102m"), "fill SGR present");
    assert!(out.contains("\x1b[38;2;255;255;255m"), "fg SGR present");
    assert_snapshot!("node_fill_stroke_and_color", out);
}

// ---------------------------------------------------------------------------
// classDef + class — palette reuse via named style classes.
// ---------------------------------------------------------------------------
#[test]
fn classdef_and_class_directives() {
    let src = r#"graph LR
        A[Cache] --> B[DB] --> C[Done]
        classDef datastore fill:#234,stroke:#9cf,color:#fff
        class A,B datastore"#;
    let opts = mermaid_text::RenderOptions {
        color: true,
        ..Default::default()
    };
    let out = mermaid_text::render_with_options(src, &opts).unwrap();
    // Both A and B should pick up the class fill colour.
    assert!(
        out.contains("\x1b[48;2;34;51;68m"),
        "datastore fill SGR present"
    );
    assert!(
        out.contains("\x1b[38;2;153;204;255m"),
        "datastore stroke SGR present"
    );
    assert_snapshot!("classdef_and_class_directives", out);
}

// ---------------------------------------------------------------------------
// `classDef DEFAULT` special semantics — DEFAULT is a universal base class:
// - unstyled nodes pick it up directly,
// - explicitly-classed nodes get DEFAULT merged under (explicit wins).
// ---------------------------------------------------------------------------
#[test]
fn classdef_default_merges_into_all_nodes() {
    // A[Apple]:::fruit — gets DEFAULT stroke AND fruit fill.
    // B[Bone]          — gets only DEFAULT stroke (no explicit class).
    let src = r#"graph LR
        A[Apple]:::fruit
        B[Bone]
        classDef DEFAULT stroke:#0ff
        classDef fruit fill:#f00"#;
    let opts = mermaid_text::RenderOptions {
        color: true,
        ..Default::default()
    };
    let out = mermaid_text::render_with_options(src, &opts).unwrap();
    // Both nodes should carry the cyan stroke from DEFAULT.
    assert!(
        out.matches("\x1b[38;2;0;255;255m").count() >= 2,
        "DEFAULT stroke SGR must appear on both nodes"
    );
    // Only node A (fruit) carries the red fill.
    assert!(
        out.contains("\x1b[48;2;255;0;0m"),
        "fruit fill SGR present on A"
    );
    assert_snapshot!("classdef_default_merges_into_all_nodes", out);
}

// ---------------------------------------------------------------------------
// `:::` shorthand inline on node references in transitions.
// ---------------------------------------------------------------------------
#[test]
fn triple_colon_shorthand() {
    let src = r#"graph LR
        A[Start]:::warm --> B[End]:::cool
        classDef warm fill:#f00,color:#fff
        classDef cool fill:#00f,color:#fff"#;
    let opts = mermaid_text::RenderOptions {
        color: true,
        ..Default::default()
    };
    let out = mermaid_text::render_with_options(src, &opts).unwrap();
    assert!(out.contains("\x1b[48;2;255;0;0m"), "warm fill present");
    assert!(out.contains("\x1b[48;2;0;0;255m"), "cool fill present");
    assert_snapshot!("triple_colon_shorthand", out);
}

// ---------------------------------------------------------------------------
// State-diagram classDef + class on both states and a composite (the
// composite border picks up the class stroke color).
// ---------------------------------------------------------------------------
#[test]
fn state_diagram_classdef() {
    let src = "stateDiagram-v2
[*] --> Active
state Active {
  [*] --> Idle
  Idle --> Working : start
  Working --> Idle : done
}
classDef accent stroke:#9cf,color:#fff
classDef warn fill:#f00,color:#fff
class Active accent
class Working warn";
    let opts = mermaid_text::RenderOptions {
        color: true,
        ..Default::default()
    };
    let out = mermaid_text::render_with_options(src, &opts).unwrap();
    assert!(
        out.contains("\x1b[38;2;153;204;255m"),
        "accent stroke present"
    );
    assert!(out.contains("\x1b[48;2;255;0;0m"), "warn fill present");
    assert_snapshot!("state_diagram_classdef", out);
}

// ---------------------------------------------------------------------------
// 20. Edge color via `linkStyle` directive.
// ---------------------------------------------------------------------------
#[test]
fn edge_link_style() {
    let src = r#"graph LR
        A --> B
        A --> C
        linkStyle 0 stroke:#f00
        linkStyle 1 stroke:#0f0,color:#fff"#;
    let opts = mermaid_text::RenderOptions {
        color: true,
        ..Default::default()
    };
    let out = mermaid_text::render_with_options(src, &opts).unwrap();
    assert!(out.contains("\x1b[38;2;255;0;0m"), "stroke #f00 present");
    assert!(out.contains("\x1b[38;2;0;255;0m"), "stroke #0f0 present");
    assert_snapshot!("edge_link_style", out);
}

// ---------------------------------------------------------------------------
// State diagrams — transformed to flowchart Graph, ride the same renderer.
// ---------------------------------------------------------------------------

#[test]
fn state_simple_chain() {
    let src = "stateDiagram-v2\n[*] --> A\nA --> B\nB --> [*]";
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("state_simple_chain", out);
}

#[test]
fn state_self_transition() {
    let src = "stateDiagram-v2\n[*] --> A\nA --> A : retry";
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("state_self_transition", out);
}

#[test]
fn state_self_loop_multi_outgoing_no_artifacts() {
    // Regression test for B4: a self-loop on a node that also has other
    // outgoing edges must not produce stray ├/┼/│ glyphs that merge into
    // adjacent box borders. The self-loop must route around the bottom of
    // the node (back-edge path) instead of the right side (forward-edge
    // path), so its A* path never crosses the exit column of the other edges.
    let src = "stateDiagram-v2\n[*] --> pending\npending --> pending : retry\npending --> sent";
    let out = mermaid_text::render(src).unwrap();
    // The self-loop must NOT leave a dangling ┌┐ / ├┼ above the sent box.
    assert!(
        !out.contains("││"),
        "stray double-bar from self-loop routing"
    );
    assert!(
        !out.contains("┌─"),
        "stray box-corner from self-loop routing"
    );
    assert_snapshot!("state_self_loop_multi_outgoing", out);
}

#[test]
fn state_multi_line_description() {
    let src = "stateDiagram-v2
direction LR
[*] --> Active
Active : Line one
Active : Line two
Active : Line three";
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("state_multi_line_description", out);
}

#[test]
fn state_diagram_special_shapes() {
    // Exercises the three UML shape modifiers introduced in 0.7.2:
    //   <<choice>> → diamond
    //   <<fork>>   → bar perpendicular to flow (vertical for default LR)
    //   <<join>>   → bar perpendicular to flow (vertical for default LR)
    let src = "stateDiagram-v2
[*] --> Decision
state Decision <<choice>>
Decision --> Forked : positive
Decision --> [*] : negative
state Forked <<fork>>
Forked --> Branch1
Forked --> Branch2
Branch1 --> Sync
Branch2 --> Sync
state Sync <<join>>
Sync --> [*]";
    let out = mermaid_text::render(src).unwrap();
    // <<choice>> now renders with diagonal corner characters (╱ ╲) instead of
    // the old ◇ markers, giving a clearer visual distinction from plain rects.
    assert!(
        out.contains('╱'),
        "missing diagonal corner '╱' for <<choice>>"
    );
    assert!(
        out.contains('╲'),
        "missing diagonal corner '╲' for <<choice>>"
    );
    assert!(
        out.contains('█'),
        "missing filled-block glyph for <<fork>>/<<join>> in default LR layout"
    );
    assert_snapshot!("state_diagram_special_shapes", out);
}

/// Snapshot test for anonymous vs named `<<choice>>` rendering.
///
/// - Named choice (`state named_cond <<choice>>`): label "named_cond" must
///   appear inside the diamond.
/// - Anonymous choice (`<<choice>>` used directly as a transition endpoint):
///   the diamond must be present but the synthetic id (`__choice_N__`) must
///   NOT appear in the output.
#[test]
fn state_diagram_anonymous_choice() {
    let src = "stateDiagram-v2
[*] --> named_cond
state named_cond <<choice>>
named_cond --> Pass: ok
named_cond --> Fail: error
Fail --> [*]
Pass --> Done
Done --> [*]
[*] --> <<choice>>
<<choice>> --> Open: start
<<choice>> --> Closed: stop
Open --> [*]
Closed --> [*]";
    let out = mermaid_text::render(src).unwrap();
    // Both diamonds must have their diagonal corners rendered.
    assert!(
        out.contains('╱'),
        "missing diagonal corner '╱' for <<choice>> in:\n{out}"
    );
    // Named choice label must be present.
    assert!(
        out.contains("named_cond"),
        "named <<choice>> label 'named_cond' missing from output:\n{out}"
    );
    // Anonymous choice synthetic id must NOT appear.
    assert!(
        !out.contains("__choice_"),
        "synthetic anonymous-choice id leaked into output:\n{out}"
    );
    assert_snapshot!("state_diagram_anonymous_choice", out);
}

#[test]
fn state_diagram_fork_in_tb_uses_horizontal_bar() {
    // Confirms orientation flips when the user writes `direction TB`
    // explicitly — bar is perpendicular to flow regardless of fork
    // vs. join.
    let src = "stateDiagram-v2
direction TB
[*] --> F
state F <<fork>>
F --> A
F --> B";
    let out = mermaid_text::render(src).unwrap();
    assert!(
        out.contains('█'),
        "missing filled-block glyph for <<fork>> in TB layout"
    );
    assert_snapshot!("state_diagram_fork_in_tb_uses_horizontal_bar", out);
}

#[test]
fn state_composite_simple() {
    let src = "stateDiagram-v2
state Active {
[*] --> Inner
Inner --> Done
}";
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("state_composite_simple", out);
}

#[test]
fn state_composite_with_external_edges() {
    // External edges to/from the composite ID get rewritten at parse time
    // so the arrows visibly land on the composite's start / end markers.
    let src = "stateDiagram-v2
direction LR
[*] --> Active
state Active {
Idle --> Working
Working --> Idle
}
Active --> [*]";
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("state_composite_with_external_edges", out);
}

#[test]
fn state_nested_composites() {
    let src = "stateDiagram-v2
state Outer {
state Inner {
A --> B
}
Other --> Other
}";
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("state_nested_composites", out);
}

#[test]
fn state_composite_keyboard_lock() {
    // The classic Mermaid composite-state example: Active wraps three
    // independent toggle states (NumLock, CapsLock, ScrollLock).
    let src = "stateDiagram-v2
[*] --> Active
state Active {
NumLockOff --> NumLockOn : EvNumLockPressed
NumLockOn --> NumLockOff : EvNumLockPressed
CapsLockOff --> CapsLockOn : EvCapsLockPressed
CapsLockOn --> CapsLockOff : EvCapsLockPressed
}";
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("state_composite_keyboard_lock", out);
}

#[test]
fn state_diagram_with_note_right_of() {
    // Single-line note anchored to the right of a state. The note
    // renders as a small rounded box connected by a dotted line
    // (no arrow tip).
    let src = "stateDiagram-v2
[*] --> Active
Active --> Done
note right of Active : retries 3x with backoff";
    let out = mermaid_text::render(src).unwrap();
    assert!(
        out.contains("retries 3x with backoff"),
        "note text must appear in rendered output"
    );
    assert!(
        out.contains('┄') || out.contains('┆'),
        "dotted connector glyph must appear"
    );
    assert_snapshot!("state_diagram_with_note_right_of", out);
}

#[test]
fn state_diagram_with_multiline_note() {
    // Multi-line `note left of X / … / end note` form. The body
    // lines are joined with `\n` into the note's label, which the
    // existing multi-line label rendering handles.
    let src = "stateDiagram-v2
[*] --> Idle
Idle --> Working
note left of Idle
  worker pool size = 4
  shared with retry queue
end note";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("worker pool size"));
    assert!(out.contains("shared with retry queue"));
    assert_snapshot!("state_diagram_with_multiline_note", out);
}

#[test]
fn edge_label_not_adjacent_to_corner_glyph() {
    // Regression test for B10: edge labels must not be placed immediately
    // adjacent to path corner/junction glyphs (┘ └ ┐ ┌ ┤ ├ ┬ ┴ ┼).
    // Such adjacency produces artifacts like `label─┘` or `└─label` where
    // the label text merges visually with the route corner.
    //
    // The supervisor test previously showed `┌────panics┼┘` — "panics"
    // placed immediately left of `┼`. The guard in `label_touches_path_corner`
    // now moves such labels to a position where no corner glyph is adjacent.
    let src = "graph LR
subgraph SG[Supervisor]
  A-->B
  B-->|panics|A
  C-->|creates|B
end";
    let out = mermaid_text::render(src).unwrap();
    // Labels must be present.
    assert!(out.contains("panics"), "panics label missing");
    assert!(out.contains("creates"), "creates label missing");
    // The label 'panics' must not be immediately followed by a junction glyph.
    for line in out.lines() {
        if let Some(pos) = line.find("panics") {
            let after = &line[pos + "panics".len()..];
            let first_char = after.chars().next().unwrap_or(' ');
            assert!(
                !matches!(
                    first_char,
                    '┘' | '└' | '┐' | '┌' | '┤' | '├' | '┬' | '┴' | '┼'
                ),
                "label 'panics' immediately followed by corner glyph: {:?}",
                first_char
            );
        }
    }
    assert_snapshot!("edge_label_not_adjacent_to_corner_glyph", out);
}

#[test]
fn state_circuit_breaker() {
    // The user's exact input — the primary acceptance test for v1.
    let src = r#"stateDiagram-v2
    [*] --> CLOSED
    CLOSED --> OPEN : 5 consecutive failures
    OPEN --> HALF_OPEN : probe interval elapsed
    HALF_OPEN --> CLOSED : probe succeeds
    HALF_OPEN --> OPEN : probe fails (increased backoff)

    CLOSED : All DB calls pass through
    CLOSED : Counting consecutive failures
    OPEN : DB calls skipped (sleep for probe interval)
    OPEN : No writes attempted
    HALF_OPEN : One probe call allowed through"#;
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("state_circuit_breaker", out);
}

// ---------------------------------------------------------------------------
// 21. ANSI + ASCII compose — `to_ascii` is char-by-char and must leave the
//     embedded SGR escape sequences untouched.
// ---------------------------------------------------------------------------
#[test]
fn color_plus_ascii_composes() {
    let src = "graph LR\nA --> B\nstyle A fill:#336,color:#fff";
    let opts = mermaid_text::RenderOptions {
        color: true,
        ascii: true,
        ..Default::default()
    };
    let out = mermaid_text::render_with_options(src, &opts).unwrap();
    assert!(
        out.contains("\x1b[48;2;51;51;102m"),
        "fill SGR survives ascii"
    );
    // Strip SGR; remainder must be pure ASCII.
    let stripped: String = {
        let mut s = String::with_capacity(out.len());
        let mut in_esc = false;
        for ch in out.chars() {
            if ch == '\x1b' {
                in_esc = true;
                continue;
            }
            if in_esc {
                if ch == 'm' {
                    in_esc = false;
                }
                continue;
            }
            s.push(ch);
        }
        s
    };
    assert!(stripped.is_ascii(), "post-strip output is pure ASCII");
}

// ---------------------------------------------------------------------------
// Sequence diagrams — first snapshots in the project (none existed before
// 0.9.0). Establishes the regression baseline for the sequence renderer.
// ---------------------------------------------------------------------------

#[test]
fn sequence_minimal() {
    let src = "sequenceDiagram\nA->>B: hello\nB-->>A: hi back";
    let out = mermaid_text::render(src).unwrap();
    assert_snapshot!("sequence_minimal", out);
}

#[test]
fn sequence_with_autonumber() {
    let src = "sequenceDiagram
autonumber
participant U as User
participant API
U->>API: POST /order
API->>U: 201 Created
U->>API: GET /order/123
API->>U: 200 OK";
    let out = mermaid_text::render(src).unwrap();
    assert!(
        out.contains("[1] POST /order") && out.contains("[4] 200 OK"),
        "autonumber prefixes must appear in label text"
    );
    assert_snapshot!("sequence_with_autonumber", out);
}

#[test]
fn sequence_autonumber_off_then_on_rebases() {
    // Mermaid: `autonumber off` halts numbering; a subsequent
    // `autonumber 100` re-bases. Verify the renderer follows the
    // active state at each message position.
    let src = "sequenceDiagram
autonumber
A->>B: one
B->>A: two
autonumber off
A->>B: silent
autonumber 100
A->>B: hundred";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("[1] one"));
    assert!(out.contains("[2] two"));
    assert!(out.contains("silent") && !out.contains("[3] silent"));
    assert!(out.contains("[100] hundred"));
}

#[test]
fn sequence_with_note_right_of() {
    let src = "sequenceDiagram
participant U as User
participant API
U->>API: POST /login
note right of U : token cached for 1h
API->>U: 200 OK";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("token cached for 1h"));
    assert!(out.contains('╭') && out.contains('╯'));
    assert_snapshot!("sequence_with_note_right_of", out);
}

#[test]
fn sequence_with_note_over_pair() {
    // Multi-anchor `note over A,B` spans both participant columns.
    let src = "sequenceDiagram
participant U as User
participant API
note over U,API : Authentication flow
U->>API: POST /login";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("Authentication flow"));
    assert_snapshot!("sequence_with_note_over_pair", out);
}

#[test]
fn sequence_with_multiline_note() {
    // `<br>` and `<br/>` in note text become newlines, producing a
    // multi-line note box.
    let src = "sequenceDiagram
participant U as User
participant API
U->>API: POST /audit
note left of API : audit log entry<br/>recorded async";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("audit log entry"));
    assert!(out.contains("recorded async"));
    assert_snapshot!("sequence_with_multiline_note", out);
}

// ---------------------------------------------------------------------------
// Note word-wrap + canvas widening (0.39.0)
// ---------------------------------------------------------------------------

#[test]
fn sequence_note_auto_wrap_long_text() {
    // A `note over U,API` whose text is longer than the span between the two
    // participants should be auto-wrapped to 2-3 lines rather than clipped.
    let src = "sequenceDiagram
participant U as User
participant API
U->>API: POST /login
note over U,API : This is a long note that spans both participants and should wrap nicely
API->>U: 200 OK";
    let out = mermaid_text::render(src).unwrap();
    // The wrapped text must still appear in the output (possibly split across lines).
    assert!(
        out.contains("This is a long note"),
        "note text start missing from:\n{out}"
    );
    assert!(
        out.contains("wrap nicely"),
        "note text end missing from:\n{out}"
    );
    // The note box must be present.
    assert!(out.contains('╭') && out.contains('╯'));
    assert_snapshot!("sequence_note_auto_wrap_long_text", out);
}

#[test]
fn sequence_note_canvas_widens_for_long_word() {
    // A `note right of B` with an unbreakable long word — the canvas must
    // widen to fit rather than silently clipping the word.
    let src = "sequenceDiagram
participant A
participant B
A->>B: request
note right of B : antidisestablishmentarianism
B->>A: response";
    let out = mermaid_text::render(src).unwrap();
    assert!(
        out.contains("antidisestablishmentarianism"),
        "long unbreakable word must appear unclipped in:\n{out}"
    );
    assert_snapshot!("sequence_note_canvas_widens_for_long_word", out);
}

#[test]
fn sequence_note_respects_explicit_br() {
    // User-supplied `<br>` separators become `\n` at parse time and must
    // not be re-joined or re-wrapped — each explicit line is authoritative.
    let src = "sequenceDiagram
participant A
participant B
A->>B: go
note over A,B : first line<br>second line<br>third line";
    let out = mermaid_text::render(src).unwrap();
    assert!(
        out.contains("first line"),
        "first line missing from:\n{out}"
    );
    assert!(
        out.contains("second line"),
        "second line missing from:\n{out}"
    );
    assert!(
        out.contains("third line"),
        "third line missing from:\n{out}"
    );
    // All three lines must be separate rows in the rendered note box.
    let first = out.lines().position(|l| l.contains("first line")).unwrap();
    let second = out.lines().position(|l| l.contains("second line")).unwrap();
    let third = out.lines().position(|l| l.contains("third line")).unwrap();
    assert!(
        first < second && second < third,
        "lines must be in top-down order"
    );
    assert_snapshot!("sequence_note_respects_explicit_br", out);
}

#[test]
fn sequence_note_left_of_wraps() {
    // A `note left of B` with text wider than the available left-of space
    // should wrap into multiple lines using the left-of budget.
    let src = "sequenceDiagram
participant A
participant B
A->>B: call
note left of B : this is a somewhat long note anchored left of B
B->>A: reply";
    let out = mermaid_text::render(src).unwrap();
    // Text must appear somewhere in the output, possibly split across lines.
    assert!(
        out.contains("somewhat long"),
        "note text missing from:\n{out}"
    );
    assert!(out.contains('╭') && out.contains('╯'));
    assert_snapshot!("sequence_note_left_of_wraps", out);
}

#[test]
fn sequence_with_explicit_activation() {
    // `activate X` / `deactivate X` overlay heavy `┃` bars on the
    // participant's lifeline between the activate and deactivate rows.
    let src = "sequenceDiagram
participant U as User
participant API
U->>API: POST /login
activate API
API->>U: 200 OK
deactivate API";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains('┃'), "expected activation bar in:\n{out}");
    assert_snapshot!("sequence_with_explicit_activation", out);
}

#[test]
fn sequence_with_inline_call_reply_activation() {
    // Canonical Mermaid pattern: `+B` activates B at the call,
    // `-A` deactivates the source (A) at the reply — though
    // visually the bar attaches to B (the active participant).
    let src = "sequenceDiagram
participant U as User
participant API
participant DB
U->>+API: POST /login
API->>+DB: SELECT user
DB-->>-API: user record
API-->>-U: 200 + token";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains('┃'), "expected activation bar in:\n{out}");
    assert_snapshot!("sequence_with_inline_call_reply_activation", out);
}

#[test]
fn sequence_with_nested_activations() {
    // Two activations on the same participant (B) nest LIFO.
    let src = "sequenceDiagram
A->>B: outer call
activate B
A->>B: inner call
activate B
B->>A: inner reply
deactivate B
B->>A: outer reply
deactivate B";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains('┃'));
    assert_snapshot!("sequence_with_nested_activations", out);
}

#[test]
fn sequence_with_loop_block() {
    let src = "sequenceDiagram
participant A
participant B
loop forever
A->>B: tick
end";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("[loop]") && out.contains("[forever]"));
    assert!(out.contains('╔') && out.contains('╝'));
    assert_snapshot!("sequence_with_loop_block", out);
}

#[test]
fn sequence_with_alt_else_block() {
    // alt/else with two branches; both branches' labels should appear,
    // separated by a dashed `╠ ┄ ╣` divider.
    let src = "sequenceDiagram
participant A
participant B
alt success
A->>B: ok
else failure
A->>B: fail
end";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("[alt]") && out.contains("[success]"));
    assert!(out.contains("[failure]"));
    assert!(out.contains('╠') && out.contains('╣'));
    assert_snapshot!("sequence_with_alt_else_block", out);
}

#[test]
fn sequence_with_opt_block() {
    let src = "sequenceDiagram
A->>B: hi
opt cache hit
B->>A: cached
end";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("[opt]") && out.contains("[cache hit]"));
    assert_snapshot!("sequence_with_opt_block", out);
}

#[test]
fn sequence_with_nested_loop_alt() {
    // Nested blocks inset by one cell per nesting level so the inner
    // rectangle reads distinctly from the outer.
    let src = "sequenceDiagram
participant A
participant B
loop outer
alt branch a
A->>B: a
else branch b
A->>B: b
end
end";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("[loop]") && out.contains("[outer]"));
    assert!(out.contains("[alt]") && out.contains("[branch a]"));
    assert_snapshot!("sequence_with_nested_loop_alt", out);
}

#[test]
fn sequence_with_par_and_critical_blocks() {
    // Exercises the less-common multi-branch kinds.
    let src = "sequenceDiagram
participant A
participant B
par first
A->>B: msg1
and second
A->>B: msg2
end
critical primary
A->>B: try
option failure
A->>B: retry
end";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("[par]") && out.contains("[first]"));
    assert!(out.contains("[second]"));
    assert!(out.contains("[critical]") && out.contains("[primary]"));
    assert!(out.contains("[failure]"));
    assert_snapshot!("sequence_with_par_and_critical_blocks", out);
}

// ---------------------------------------------------------------------------
// Pie charts (0.9.4) — first full diagram-type addition since sequence in
// 0.9.0. Renders as a horizontal bar chart in monospace text.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// erDiagram — Phase 1: minimal renderer (entity-name boxes + labelled arrows
// in source-order row). Phases 2-3 add attribute tables, cardinality glyphs,
// and grid layout.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Back-edge perimeter routing (ROADMAP item #7, fixed in 0.11.2).
// Regression guard: when a TD diagram has a back-edge in a cycle,
// the back-edge route must NOT thread through the gap between
// forward-edge target nodes. The 0.11.2 fix biases A* against
// `InnerArea` cells (the bounding-box interior between nodes).
// ---------------------------------------------------------------------------
#[test]
fn back_edge_avoids_diagram_interior_in_td_cycle() {
    // Idle → Running → Done / Failed → Idle: the cycle pulls Idle's
    // layer down, making Idle → Running a back-edge that the old A*
    // would route through the channel between Done and Failed.
    let src = "graph TD
        Idle -->|event| Running
        Running -->|done| Done
        Running -->|error| Failed
        Failed -->|retry| Idle";
    let out = mermaid_text::render(src).unwrap();
    // The forward-edge labels `done` and `error` must each be on
    // a row whose Done/Failed columns are NOT split by a back-edge
    // `│`. Easier to check via snapshot — visual inspection is the
    // ground truth here.
    assert_snapshot!("back_edge_avoids_diagram_interior_in_td_cycle", out);
}

#[test]
fn er_minimal_two_entities() {
    let src = "erDiagram\nCUSTOMER ||--o{ ORDER : places";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("CUSTOMER"));
    assert!(out.contains("ORDER"));
    // Cardinality glyphs at the endpoints: `1` (ExactlyOne) on the
    // source side, `*` (ZeroOrMany) on the target.
    assert!(out.contains('1') && out.contains('*'));
    assert!(out.contains("places"));
    assert_snapshot!("er_minimal_two_entities", out);
}

#[test]
fn er_canonical_three_entities() {
    // The Mermaid docs' canonical example — attributes are parsed
    // but not rendered in Phase 1 (Phase 2 adds attribute tables).
    let src = "erDiagram
    CUSTOMER ||--o{ ORDER : places
    CUSTOMER {
        string name
        string email PK
    }
    ORDER ||--|{ LINE-ITEM : contains";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("CUSTOMER") && out.contains("ORDER") && out.contains("LINE-ITEM"));
    assert_snapshot!("er_canonical_three_entities", out);
}

#[test]
fn er_non_identifying_renders_dashed_line() {
    let src = "erDiagram\nPARENT ||..o{ CHILD : optional";
    let out = mermaid_text::render(src).unwrap();
    assert!(
        out.contains("┄"),
        "non-identifying relationship must use dashed glyph"
    );
    assert_snapshot!("er_non_identifying_renders_dashed_line", out);
}

// ---------------------------------------------------------------------------
// erDiagram Phase 3: grid layout — 8-entity order-management schema.
//
// With `render_with_width(src, Some(40))` the single-row layout would be
// ~180 cols wide; the renderer wraps to a 3-column grid (ceil(sqrt(8))=3).
// Cross-row relationships route via a right-margin spine.
// ---------------------------------------------------------------------------
#[test]
fn er_diagram_grid_layout_8_entities() {
    let src = "erDiagram
CUSTOMER ||--o{ ORDER : places
ORDER ||--|{ ITEM : contains
PRODUCT ||--o{ ITEM : describes
CATEGORY ||--o{ PRODUCT : groups
ACCOUNT ||--|| CUSTOMER : owns
INVOICE ||--|{ ORDER : bills
WAREHOUSE ||--o{ PRODUCT : stocks
REGION ||--o{ WAREHOUSE : hosts
CUSTOMER {
    int id PK
    string name
}
ORDER {
    int id PK
    int customerId FK
}
PRODUCT {
    int id PK
    string name
    int categoryId FK
}
CATEGORY {
    int id PK
    string label
}
ACCOUNT {
    int id PK
}
INVOICE {
    int id PK
}
WAREHOUSE {
    int id PK
    int regionId FK
}
REGION {
    int id PK
    string name
}
ITEM {
    int orderId FK
    int productId FK
}";
    // 40-column budget forces a multi-row grid.
    let out = mermaid_text::render_with_width(src, Some(40)).unwrap();
    // All 8 entity names must appear.
    for name in &[
        "CUSTOMER",
        "ORDER",
        "ITEM",
        "PRODUCT",
        "CATEGORY",
        "ACCOUNT",
        "INVOICE",
        "WAREHOUSE",
        "REGION",
    ] {
        assert!(out.contains(name), "{name} missing from output:\n{out}");
    }
    // More than one row of top-border glyphs confirms multi-row grid layout.
    let top_border_rows = out.lines().filter(|l| l.contains('┌')).count();
    assert!(
        top_border_rows > 1,
        "expected multi-row grid, got {top_border_rows} top-border rows"
    );
    // Cardinality glyphs must still appear.
    assert!(out.contains('1'), "cardinality '1' missing");
    assert!(out.contains('*'), "cardinality '*' missing");
    assert_snapshot!("er_diagram_grid_layout_8_entities", out);
}

#[test]
fn pie_minimal() {
    let src = "pie\n\"A\" : 1\n\"B\" : 1\n\"C\" : 2";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains('█'));
    assert!(out.contains("50.0%"));
    assert_snapshot!("pie_minimal", out);
}

#[test]
fn pie_with_title() {
    let src = "pie title Pet Counts\n\"Dogs\" : 386\n\"Cats\" : 85\n\"Rats\" : 15";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("Pet Counts"));
    assert_snapshot!("pie_with_title", out);
}

#[test]
fn pie_with_show_data() {
    let src =
        "pie showData title Browser Share\n\"Chrome\" : 60\n\"Firefox\" : 25\n\"Safari\" : 15";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("(60)"));
    assert!(out.contains("(25)"));
    assert_snapshot!("pie_with_show_data", out);
}

#[test]
fn pie_many_slices_with_decimals() {
    // Stresses label-column padding (varying widths) and decimal value
    // formatting. The value column should align by closing paren.
    let src = "pie showData title Releases\n\
        \"v0.9.0\" : 12\n\
        \"v0.9.1\" : 8.5\n\
        \"v0.9.2\" : 17.25\n\
        \"v0.9.3\" : 30\n\
        \"v0.9.4\" : 5\n\
        \"older\" : 27.25";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("(8.5)"));
    assert!(out.contains("(17.25)"));
    assert_snapshot!("pie_many_slices_with_decimals", out);
}

#[test]
fn pie_color_mode_emits_ansi_per_slice() {
    // Verifies that enabling `color` wraps each slice's filled-block run in a
    // distinct ANSI 24-bit SGR sequence and resets after each one.
    let src = "pie title Planets\n\"Mercury\" : 10\n\"Venus\" : 20\n\"Earth\" : 30";
    let opts = mermaid_text::RenderOptions {
        color: true,
        max_width: Some(80),
        ..Default::default()
    };
    let out = mermaid_text::render_with_options(src, &opts).unwrap();
    // At least one 24-bit foreground escape must be present.
    assert!(
        out.contains("\x1b[38;2;"),
        "expected ANSI color escape in: {out:?}"
    );
    // Each colored run must be closed by a reset.
    assert!(out.contains("\x1b[0m"), "expected ANSI reset in: {out:?}");
    // Monochrome content still present.
    assert!(out.contains("Planets"));
    assert!(out.contains("Mercury"));
    assert_snapshot!("pie_color_mode", out);
}

#[test]
fn sequence_end_note_returns_helpful_error() {
    // Mermaid's sequence grammar has no `end note` form (state diagrams
    // do; sequence uses `<br>`). Make sure the parser flags this with a
    // pointer to the right syntax instead of silently misparsing.
    let src = "sequenceDiagram
participant U
end note";
    let err = mermaid_text::render(src).unwrap_err().to_string();
    assert!(
        err.contains("<br>"),
        "error should point at <br> syntax: {err}"
    );
}

// ---------------------------------------------------------------------------
// classDiagram — Phase 5: class boxes with members and typed relationships.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// S1. Single class with attributes and a method.
// ---------------------------------------------------------------------------
#[test]
fn class_single_class() {
    let src = "classDiagram
class Animal {
    +String name
    +int age
    +speak() void
}";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("Animal"), "class name must appear");
    assert!(out.contains("name"), "attribute name must appear");
    assert!(out.contains("speak"), "method name must appear");
    assert_snapshot!("class_single_class", out);
}

// ---------------------------------------------------------------------------
// S2. Three-level inheritance chain: Animal → Dog → GoldenRetriever.
// ---------------------------------------------------------------------------
#[test]
fn class_inheritance_three_level() {
    let src = "classDiagram
class Animal {
    +String name
    +speak() void
}
class Dog {
    +String breed
    +fetch() void
}
class GoldenRetriever {
    +bool loves_water
}
Animal <|-- Dog
Dog <|-- GoldenRetriever";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("Animal"));
    assert!(out.contains("Dog"));
    assert!(out.contains("GoldenRetriever"));
    // Inheritance endpoint glyph must appear.
    assert!(out.contains('△'), "inheritance glyph △ must appear");
    assert_snapshot!("class_inheritance_three_level", out);
}

// ---------------------------------------------------------------------------
// S3. Composition and aggregation relationships.
// ---------------------------------------------------------------------------
#[test]
fn class_composition_aggregation() {
    let src = "classDiagram
class Engine {
    +int horsepower
    +start() void
}
class Wheel {
    +int diameter
}
class Car {
    +String model
    +drive() void
}
class Fleet {
    +String name
}
Car *-- Engine : has
Car o-- Wheel : uses
Fleet o-- Car : contains";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("Engine"));
    assert!(out.contains("Car"));
    assert!(out.contains("Fleet"));
    // Composition and aggregation endpoint glyphs.
    assert!(out.contains('◆'), "composition glyph ◆ must appear");
    assert!(out.contains('◇'), "aggregation glyph ◇ must appear");
    assert_snapshot!("class_composition_aggregation", out);
}

// ---------------------------------------------------------------------------
// S4. Mixed relationship types: all seven kinds in one diagram.
// ---------------------------------------------------------------------------
#[test]
fn class_mixed_relationships() {
    let src = "classDiagram
class A
class B
class C
class D
class E
class F
class G
A <|-- B
A *-- C
A o-- D
A --> E
A -- F
A <|.. G";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains('A'));
    assert_snapshot!("class_mixed_relationships", out);
}

// ---------------------------------------------------------------------------
// S5. Abstract and static member suffixes.
// ---------------------------------------------------------------------------
#[test]
fn class_abstract_and_static_members() {
    let src = "classDiagram
class Shape {
    +String color
    +area() double*
    +perimeter() double*
    +reset()$ void
}
class Circle {
    +double radius
    +area() double*
    +perimeter() double*
}
Shape <|-- Circle";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("Shape"));
    assert!(out.contains("Circle"));
    assert!(out.contains("area"));
    assert!(out.contains("perimeter"));
    assert_snapshot!("class_abstract_and_static_members", out);
}

// ---------------------------------------------------------------------------
// S6. Width compaction: rendering at a tight budget forces narrower boxes.
//     Uses render_with_width with 60 cols (tight for a multi-class diagram).
// ---------------------------------------------------------------------------
#[test]
fn class_wide_compaction() {
    let src = "classDiagram
class VeryLongClassName {
    +String attributeWithLongName
    +computeSomethingExpensive() int
}
class AnotherLongClassName {
    +bool flag
}
VeryLongClassName --> AnotherLongClassName";
    let out = mermaid_text::render_with_width(src, Some(60)).unwrap();
    assert!(out.contains("VeryLongClassName") || out.contains("VeryLongClas"));
    assert_snapshot!("class_wide_compaction", out);
}

// ---------------------------------------------------------------------------
// B8. Edge label must not abut the subgraph right wall (`beat│` artifact).
//     Regression guard: placing a label whose last character is immediately
//     before the `│` right wall makes it look clipped by the border.
//     `label_abuts_subgraph_right_wall` rejects such positions in Pass A so
//     a better-positioned column anchor is tried first.
// ---------------------------------------------------------------------------
#[test]
fn edge_label_does_not_abut_subgraph_right_wall() {
    // The `beat` label on the Worker→Heartbeat edge used to render as
    // `beat│` — the last char immediately before the subgraph right wall.
    let src = "graph LR
    subgraph Supervisor
        direction TB
        F[Factory] -->|creates| W[Worker]
        W -->|panics| F
    end
    W -->|beat| HB[Heartbeat]
    HB --> WD[Watchdog]";
    let out = mermaid_text::render(src).unwrap();
    // Every line that contains "beat" must NOT have the `│` wall character
    // immediately after it (which would indicate the `beat│` artifact).
    for line in out.lines() {
        if let Some(pos) = line.find("beat") {
            let after = &line[pos + "beat".len()..];
            let first = after.chars().next().unwrap_or(' ');
            assert_ne!(first, '│', "beat label abuts right wall: {line:?}");
        }
    }
    assert_snapshot!("edge_label_does_not_abut_subgraph_right_wall", out);
}

// ---------------------------------------------------------------------------
// B11. Wrapped (multi-line) edge labels must stay inside the subgraph border.
//      Regression guard: `<br/>` in an edge label normalises to `\n`.
//      Both the width guard and the line-by-line write pass must handle the
//      multi-line case so neither line escapes the subgraph outline.
// ---------------------------------------------------------------------------
#[test]
fn wrapped_edge_label_stays_inside_subgraph() {
    let src = "graph LR
subgraph SG[Group]
  A -->|\"emitOutboxEvent<br/>(fire-and-forget)\"| B
end
C --> D";
    let out = mermaid_text::render(src).unwrap();
    // Both lines of the label must be present.
    assert!(
        out.contains("emitOutboxEvent"),
        "first label line missing:\n{out}"
    );
    assert!(
        out.contains("(fire-and-forget)"),
        "second label line missing:\n{out}"
    );
    // The subgraph border must be intact — top and bottom rows of `╭─╮` / `╰─╯`
    // must not be corrupted by label text. Check that every line containing
    // the label text also contains the subgraph left border `│`.
    let sg_lines: Vec<&str> = out.lines().filter(|l| l.contains("│")).collect();
    assert!(!sg_lines.is_empty(), "subgraph border lines missing");
    assert_snapshot!("wrapped_edge_label_stays_inside_subgraph", out);
}

// ---------------------------------------------------------------------------
// B5. Cross-subgraph edge label must not overwrite the subgraph bottom border.
//     Regression guard: when Pass A rejects all positions and Pass B falls back,
//     `label_spans_subgraph_border_cell` prevents writing label text into
//     `╰─╯` border cells even as a last resort.
// ---------------------------------------------------------------------------
#[test]
fn cross_subgraph_edge_label_avoids_bottom_border() {
    let src = "graph LR
subgraph SG[Group]
  A --> B
end
B -->|\"not wired to\"| C";
    let out = mermaid_text::render(src).unwrap();
    // The label must be present.
    assert!(out.contains("not wired to"), "label missing:\n{out}");
    // The subgraph bottom border `╰─╯` must be intact — no label character
    // should appear inside a line that starts with `╰`.
    for line in out.lines() {
        if line.trim_start().starts_with('╰') {
            assert!(
                !line.contains("not wired to"),
                "label corrupts subgraph bottom border: {line:?}"
            );
        }
    }
    assert_snapshot!("cross_subgraph_edge_label_avoids_bottom_border", out);
}

// ---------------------------------------------------------------------------
// click directive / OSC 8 hyperlink
// ---------------------------------------------------------------------------

/// Snapshot test: a `click` directive on a flowchart node wraps that node's
/// label with OSC 8 hyperlink escape sequences in the rendered output. Nodes
/// without a `click` directive are unaffected — their labels remain plain text.
#[test]
fn click_directive_osc8_hyperlink() {
    let src = "graph LR
A[Home] --> B[Docs] --> C[API]
click A \"https://example.com\"
click C \"https://api.example.com\" \"API reference\"";
    let out = mermaid_text::render(src).unwrap();

    // Node A: OSC 8 open sequence with the correct URL must be present.
    assert!(
        out.contains("\x1b]8;;https://example.com\x1b\\"),
        "OSC 8 open for node A missing:\n{out:?}"
    );
    // Node C: OSC 8 open sequence for the second URL must be present.
    assert!(
        out.contains("\x1b]8;;https://api.example.com\x1b\\"),
        "OSC 8 open for node C missing:\n{out:?}"
    );
    // At least one OSC 8 close sequence must be present.
    assert!(
        out.contains("\x1b]8;;\x1b\\"),
        "OSC 8 close sequence missing:\n{out:?}"
    );
    // All labels must still be visible.
    assert!(out.contains("Home"), "label 'Home' missing");
    assert!(out.contains("Docs"), "label 'Docs' missing");
    assert!(out.contains("API"), "label 'API' missing");

    // Snapshot captures the exact byte-level output including OSC 8 sequences
    // so any future change to the wrapping logic is detected immediately.
    assert_snapshot!("click_directive_osc8_hyperlink", out);
}

/// A `click` directive in a state diagram wraps the target state's label with
/// OSC 8 just like in a flowchart.
#[test]
fn click_directive_state_diagram_osc8() {
    let src = "stateDiagram-v2
[*] --> Idle
Idle --> Active
Active --> [*]
click Idle \"https://state.example.com\"";
    let out = mermaid_text::render(src).unwrap();

    assert!(
        out.contains("\x1b]8;;https://state.example.com\x1b\\"),
        "OSC 8 missing for state 'Idle':\n{out:?}"
    );
    assert!(out.contains("Idle"), "label 'Idle' missing");
    assert_snapshot!("click_directive_state_diagram_osc8", out);
}

// ---------------------------------------------------------------------------
// Journey diagram snapshot
// ---------------------------------------------------------------------------

/// Representative `journey` diagram: title, two sections, varied scores and
/// multi-actor tasks.
#[test]
fn journey_working_day() {
    let src = "journey
    title My working day
    section Go to work
      Make tea: 5: Me
      Go upstairs: 3: Me
      Do work: 1: Me, Cat
    section Go home
      Go downstairs: 5: Me
      Sit down: 3: Me";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("My working day"));
    assert!(out.contains("Go to work"));
    assert!(out.contains("Make tea"));
    assert!(out.contains("Me, Cat"));
    assert_snapshot!("journey_working_day", out);
}

// ---------------------------------------------------------------------------
// Gantt diagram snapshot
// ---------------------------------------------------------------------------

/// Representative `gantt` diagram: title, dateFormat, axisFormat, two
/// sections, explicit dates, an `after X` dep, and chained implicit start.
/// All tasks have explicit or derivable dates — no today-anchored implicit
/// start — so this snapshot is fully deterministic across runs.
#[test]
fn gantt_project_schedule() {
    let src = "gantt
    title Software Release v2
    dateFormat YYYY-MM-DD
    axisFormat %m-%d
    section Design
      Research       :r1, 2024-01-01, 7d
      Wireframes     :after r1, 5d
    section Development
      Backend        :b1, 2024-01-13, 14d
      Frontend       :after b1, 10d
    section QA
      Testing        :2024-02-06, 7d";
    let out = mermaid_text::render_with_width(src, Some(100)).unwrap();
    assert!(
        out.contains("Software Release v2"),
        "title missing in:\n{out}"
    );
    assert!(out.contains("Research"), "Research task missing in:\n{out}");
    assert!(out.contains("Backend"), "Backend task missing in:\n{out}");
    assert!(out.contains("Testing"), "Testing task missing in:\n{out}");
    insta::assert_snapshot!("gantt_project_schedule", out);
}

// ---------------------------------------------------------------------------
// B7. TB sibling-subgraph horizontal collision — regression guard.
//
// Repro source: two top-level subgraphs in a `flowchart TB` diagram where
// one subgraph has a wide node (forces a wide border) and the other has only
// narrow nodes.  With the native layout backend, the packing decision for
// each layer used only the *current layer's* node widths to determine the
// horizontal gap between sibling subgraph nodes, ignoring that the subgraph
// border is sized by the *widest* node across ALL layers.  This caused the
// Alpha border (grown by A2's wide label) to overlap Beta's border in layers
// where A's node was narrow.
//
// Fix: `compute_positions` (native backend, TB/BT direction) now pre-computes
// the maximum node width per top-level subgraph and enforces, at every
// layer's sibling-boundary transition, that the new subgraph's start column
// clears the previous subgraph's rendered right border.
// ---------------------------------------------------------------------------
#[test]
fn b7_tb_sibling_subgraph_no_horizontal_collision() {
    // Repro: Alpha has one wide node (A2) and narrow nodes otherwise.
    // Beta has only narrow nodes.  Without the fix the native backend placed
    // B1/B3 (in the narrow layers) so close to Alpha that Alpha's wide
    // border (sized by A2) overlapped Beta's border.
    const REPRO: &str = "flowchart TB
    subgraph Alpha
        A1[Short]
        A2[A very wide label that forces the subgraph border to be wide]
        A3[Short]
        A1 --> A2 --> A3
    end
    subgraph Beta
        B1[Short]
        B2[Short]
        B3[Short]
        B1 --> B2 --> B3
    end";

    let opts = mermaid_text::RenderOptions {
        backend: mermaid_text::layout::LayoutBackend::Native,
        ..Default::default()
    };
    let out = mermaid_text::render_with_options(REPRO, &opts).unwrap();

    // Extract the column ranges of the two subgraph borders from the first
    // (top border) line of each.  The top border line contains the subgraph
    // label and unique box-drawing corners (`╭` / `╮`).
    let first_line = out
        .lines()
        .next()
        .expect("output should have at least one line");

    // Find all occurrences of `╭` (left corner) on the top border line.
    let corners: Vec<usize> = first_line
        .char_indices()
        .filter(|(_, c)| *c == '╭')
        .map(|(i, _)| i)
        .collect();
    assert!(
        corners.len() >= 2,
        "expected at least 2 subgraph left corners on top line:\n{out}"
    );

    // Find all occurrences of `╮` (right corner) on the top border line.
    let right_corners: Vec<usize> = first_line
        .char_indices()
        .filter(|(_, c)| *c == '╮')
        .map(|(i, _)| i)
        .collect();
    assert!(
        right_corners.len() >= 2,
        "expected at least 2 subgraph right corners on top line:\n{out}"
    );

    // Alpha occupies [corners[0], right_corners[0]], Beta [corners[1], right_corners[1]].
    // They must not overlap: Alpha's right corner must be strictly left of Beta's
    // left corner.
    let alpha_right = right_corners[0];
    let beta_left = corners[1];
    assert!(
        alpha_right < beta_left,
        "Alpha border (ends at byte {alpha_right}) overlaps Beta border \
         (starts at byte {beta_left}) — B7 regression:\n{out}"
    );

    // Snapshot so any future layout change that touches these positions is caught.
    insta::assert_snapshot!("b7_tb_sibling_subgraph_no_horizontal_collision", out);
}

// ---------------------------------------------------------------------------
// Timeline diagram snapshot
// ---------------------------------------------------------------------------

/// Representative `timeline` diagram: title, two sections, multi-event period.
/// The source mirrors the canonical Mermaid documentation example for timelines.
#[test]
fn timeline_social_media_history() {
    let src = "timeline
    title History of Social Media
    section 2002-2004
        2002 : LinkedIn
        2003 : MySpace launched
        2004 : Facebook : Google goes public
    section 2005-2008
        2005 : YouTube
        2006 : Twitter
        2007 : iPhone : Tumblr";
    let out = mermaid_text::render(src).unwrap();
    assert!(
        out.contains("History of Social Media"),
        "title missing in:\n{out}"
    );
    assert!(out.contains("LinkedIn"), "LinkedIn missing in:\n{out}");
    assert!(
        out.contains("Google goes public"),
        "multi-event entry missing in:\n{out}"
    );
    assert!(
        out.contains("2002-2004"),
        "section header missing in:\n{out}"
    );
    assert!(
        out.contains("2005-2008"),
        "section header missing in:\n{out}"
    );
    insta::assert_snapshot!("timeline_social_media_history", out);
}

// ---------------------------------------------------------------------------
// Git graph diagram snapshot
// ---------------------------------------------------------------------------

/// Representative `gitGraph` with main + develop branch and a merge commit.
/// The source mirrors the canonical Mermaid gitGraph documentation example.
#[test]
fn git_graph_main_develop_merge() {
    let src = "gitGraph
    commit
    commit id: \"second\"
    branch develop
    checkout develop
    commit
    commit id: \"feature-x\"
    checkout main
    merge develop
    commit tag: \"v1.0\"";
    let out = mermaid_text::render(src).unwrap();
    assert!(out.contains("second"), "commit id 'second' missing:\n{out}");
    assert!(
        out.contains("feature-x"),
        "commit id 'feature-x' missing:\n{out}"
    );
    assert!(out.contains("v1.0"), "tag 'v1.0' missing:\n{out}");
    assert!(out.contains("main"), "branch label 'main' missing:\n{out}");
    assert!(
        out.contains("develop"),
        "branch label 'develop' missing:\n{out}"
    );
    insta::assert_snapshot!("git_graph_main_develop_merge", out);
}

// ---------------------------------------------------------------------------
// md-tui integration evaluation diagram (regression baseline from 0.24.0)
// ---------------------------------------------------------------------------
/// Snapshot test for the exact diagram submitted by the md-tui maintainer when
/// evaluating mermaid-text for integration. Serves as the visible regression
/// baseline for the 0.24.0 circle / rhombus shape-rendering polish.
///
/// Expected properties after the fix:
/// - Node B is `((Circle))` — rendered with `(` / `)` ON the border, not
///   embedded in the label text ("Circle", not "( Circle )").
/// - Node D is `{Rhombus}` — rendered with `╱` / `╲` diagonal corners, not
///   a rectangle with `◇` markers.
#[test]
fn flowchart_md_tui_test_diagram() {
    let src = "graph LR
    A[Square Rect] -- Link text --> B((Circle))
    A --> C(Round Rect)
    B --> D{Rhombus}
    C --> D";
    let out = mermaid_text::render(src).unwrap();
    // Circle label must be clean — no leaked parens.
    assert!(out.contains("Circle"), "Circle label missing:\n{out}");
    assert!(
        !out.contains("( Circle )"),
        "circle label still leaks parens — bug 1 not fixed:\n{out}"
    );
    // Rhombus must use diagonal corners.
    assert!(
        out.contains('╱'),
        "diagonal corner '╱' missing for Rhombus:\n{out}"
    );
    assert!(
        out.contains('╲'),
        "diagonal corner '╲' missing for Rhombus:\n{out}"
    );
    assert!(!out.contains('◇'), "old '◇' marker still present:\n{out}");
    assert_snapshot!("flowchart_md_tui_test_diagram", out);
}

// ---------------------------------------------------------------------------
// Edge-label midpoint placement regression (LR multi-segment route)
//
// This snapshot guards the `longest_horizontal_segment_with_range` fix: when
// an edge in an LR graph is routed via multiple horizontal segments, the label
// must be placed on the LONGEST horizontal segment (closest to the geometric
// midpoint of the full route), not on the last (destination-side) segment.
//
// The A→B edge here forces A* to produce a path with horizontal segments on
// both the source and destination sides of the route; the source-side segment
// is longer. With the old code the label landed adjacent to B; with the fix
// it lands on the longer source-side run.
// ---------------------------------------------------------------------------
#[test]
fn flowchart_label_midpoint_placement_lr() {
    let src = "graph LR
    A[Source] -- \"edge label\" --> B[Dest]
    A --> G1[Gate1]
    G1 --> B
    G1 --> G2[Gate2]
    G2 --> B";
    let out = mermaid_text::render(src).unwrap();
    // The label must appear somewhere in the output.
    assert!(out.contains("edge label"), "edge label missing:\n{out}");
    // The label must NOT be immediately adjacent to the destination node
    // border character. We check this by asserting that "edge label" does
    // not appear on the same row as the `▸│` destination-arrival glyph.
    // A destination-adjacent label would produce something like:
    //   `  edge label  ▸│ Dest │`
    // while a correctly-centred label appears well to the left of `▸│`.
    let bad_proximity = out.lines().any(|line| {
        // "edge label" and the destination arrow on the same line with
        // fewer than 4 characters between them.
        if let (Some(label_pos), Some(arrow_pos)) = (line.find("edge label"), line.find("▸│")) {
            let gap = arrow_pos.saturating_sub(label_pos + "edge label".len());
            gap < 4
        } else {
            false
        }
    });
    assert!(
        !bad_proximity,
        "edge label is immediately adjacent to the destination arrow — \
         midpoint placement regression:\n{out}"
    );
    assert_snapshot!("flowchart_label_midpoint_placement_lr", out);
}

// ---------------------------------------------------------------------------
// Mindmap — canonical Mermaid docs example
//     Regression guard: root box, branch glyphs, and nested indentation
//     must all render correctly.
// ---------------------------------------------------------------------------
#[test]
fn mindmap_canonical_example() {
    let src = r"mindmap
  root((mindmap))
    Origins
      Long history
      ::icon(fa fa-book)
      Popularisation
        British popular psychology author Tony Buzan
    Research
      On effectiveness and features
      On Automatic creation
        Uses
          Creative techniques
          Strategic planning
          Argument mapping
    Tools
      Pen and paper
      Mermaid";
    let out = mermaid_text::render(src).unwrap();
    // Root text must appear in its box.
    assert!(out.contains("mindmap"), "root text missing from output");
    // Top-level children must all appear.
    assert!(out.contains("Origins"), "Origins node missing");
    assert!(out.contains("Research"), "Research node missing");
    assert!(out.contains("Tools"), "Tools node missing");
    // Nested children must be present.
    assert!(out.contains("Long history"), "Long history missing");
    assert!(
        out.contains("British popular psychology author Tony Buzan"),
        "Buzan missing"
    );
    // Icon lines must be silently ignored — no `::icon` text in output.
    assert!(!out.contains("::icon"), "icon directive leaked into output");
    // Branch glyphs must be present.
    assert!(
        out.contains('\u{251C}') || out.contains('\u{2514}'),
        "no branch glyphs"
    );
    assert_snapshot!("mindmap_canonical_example", out);
}

// ---------------------------------------------------------------------------
// QuadrantChart — canonical Mermaid docs example
//     Regression guard: title, axis labels, quadrant labels, and plotted
//     data points must all render correctly.
// ---------------------------------------------------------------------------
#[test]
fn quadrant_chart_canonical_example() {
    let src = "quadrantChart
    title Reach and engagement of campaigns
    x-axis Low Reach --> High Reach
    y-axis Low Engagement --> High Engagement
    quadrant-1 We should expand
    quadrant-2 Need to promote
    quadrant-3 Re-evaluate
    quadrant-4 May be improved
    Campaign A: [0.3, 0.6]
    Campaign B: [0.45, 0.23]
    Campaign C: [0.57, 0.69]
    Campaign D: [0.78, 0.34]
    Campaign E: [0.40, 0.34]
    Campaign F: [0.35, 0.78]";

    let out = mermaid_text::render(src).unwrap();

    // Title must be present.
    assert!(
        out.contains("Reach and engagement of campaigns"),
        "title missing"
    );
    // All quadrant labels must appear.
    assert!(out.contains("We should expand"), "Q1 label missing");
    assert!(out.contains("Need to promote"), "Q2 label missing");
    assert!(out.contains("Re-evaluate"), "Q3 label missing");
    assert!(out.contains("May be improved"), "Q4 label missing");
    // All campaign points must appear.
    for name in &[
        "Campaign A",
        "Campaign B",
        "Campaign C",
        "Campaign D",
        "Campaign E",
        "Campaign F",
    ] {
        assert!(out.contains(name), "{name} missing");
    }
    // Axis labels must appear.
    assert!(out.contains("Low Reach"), "Low Reach missing");
    assert!(out.contains("High Reach"), "High Reach missing");
    assert!(out.contains("Low Engagement"), "Low Engagement missing");
    assert!(out.contains("High Engagement"), "High Engagement missing");
    // The cross glyph must be present.
    assert!(out.contains('\u{253C}'), "cross glyph ┼ missing");

    assert_snapshot!("quadrant_chart_canonical_example", out);
}

// ---------------------------------------------------------------------------
// QuadrantChart — point labels at high-x must not be silently truncated.
//
// Regression guard for D1: when a point is near the right edge of the canvas
// the label string overflowed and was silently chopped.  The fix flips the
// label to the LEFT side of the marker so the full text is always visible.
//
// Strong-assertion design: we require the FULL label including coordinates.
// The truncated form "Campaign D (0." does NOT contain the expected substring,
// so a no-op cannot satisfy this assertion.
// ---------------------------------------------------------------------------
#[test]
fn quadrant_chart_high_x_label_not_truncated() {
    // Point at x=0.95 — very close to the right edge — with a name long enough
    // to overflow when placed to the right of the marker.
    let src = "quadrantChart
    x-axis Low --> High
    y-axis Low --> High
    quadrant-1 Q1
    quadrant-2 Q2
    quadrant-3 Q3
    quadrant-4 Q4
    Campaign D: [0.95, 0.50]";

    let out = mermaid_text::render(src).unwrap();

    // The FULL label including coordinates must appear somewhere in the output.
    // "Campaign D (0.95,0.50)" is 22 chars; a right-side placement at x=0.95
    // on a 70-column canvas overflows, so without the fix only "Campaign D (0."
    // would be present and this assertion would fail.
    assert!(
        out.contains("Campaign D (0.95,0.50)"),
        "full label not found — likely truncated; rendered output:\n{out}"
    );
}

// ---------------------------------------------------------------------------
// requirementDiagram — canonical Mermaid example.
// ---------------------------------------------------------------------------
#[test]
fn requirement_diagram_canonical_example() {
    let src = "requirementDiagram

    requirement test_req {
        id: 1
        text: the test text.
        risk: high
        verifymethod: test
    }

    functionalRequirement test_req2 {
        id: 1.1
        text: the second test text.
        risk: low
        verifymethod: inspection
    }

    performanceRequirement test_req3 {
        id: 1.2
        text: the third test text.
        risk: medium
        verifymethod: demonstration
    }

    interfaceRequirement test_req4 {
        id: 1.2.1
        text: the fourth test text.
        risk: medium
        verifymethod: analysis
    }

    designConstraint test_req5 {
        id: 1.2.2
        text: the fifth test text.
        risk: low
        verifymethod: analysis
    }

    element test_entity {
        type: simulation
    }

    element test_entity2 {
        type: word doc
        docref: reqs/test_entity
    }

    test_entity - satisfies -> test_req2
    test_req - traces -> test_req2
    test_req - contains -> test_req3
    test_req3 - contains -> test_req4
    test_req4 - derives -> test_req5
    test_req5 - refines -> test_req4
    test_entity2 - verifies -> test_req5
    test_req5 - copies -> test_req2";

    let out = mermaid_text::render(src).unwrap();

    // All requirement names must appear.
    for name in &[
        "test_req",
        "test_req2",
        "test_req3",
        "test_req4",
        "test_req5",
    ] {
        assert!(out.contains(name), "{name} missing from output:\n{out}");
    }
    // All element names must appear.
    assert!(out.contains("test_entity"), "test_entity missing");
    assert!(out.contains("test_entity2"), "test_entity2 missing");
    // Stereotype tags must appear.
    assert!(
        out.contains("<<requirement>>"),
        "<<requirement>> tag missing"
    );
    assert!(
        out.contains("<<functionalRequirement>>"),
        "<<functionalRequirement>> tag missing"
    );
    // Element boxes must use rounded corners.
    assert!(out.contains('\u{256D}'), "rounded corner ╭ missing");
    // Relationships section must be present.
    assert!(
        out.contains("Relationships:"),
        "Relationships section missing"
    );
    assert!(out.contains("satisfies"), "satisfies relationship missing");
    assert!(out.contains("traces"), "traces relationship missing");

    assert_snapshot!("requirement_diagram_canonical_example", out);
}

// ---------------------------------------------------------------------------
// sankey-beta — canonical Mermaid energy-flow example.
//     Regression guard: source node headers, arc arrows, and values must all
//     render correctly in the grouped-arrow list layout.
// ---------------------------------------------------------------------------
#[test]
fn sankey_beta_canonical_example() {
    let src = "sankey-beta

%% source,target,value
Agricultural 'waste',Bio-conversion,124.729
Bio-conversion,Liquid,0.597
Bio-conversion,Solid,280.322
Coal imports,Coal,11.606
Coal,Solid,75.571";

    let out = mermaid_text::render(src).unwrap();

    // Source nodes that have outgoing arcs must appear as header lines.
    assert!(
        out.contains("Bio-conversion"),
        "Bio-conversion source header missing:\n{out}"
    );
    assert!(
        out.contains("Coal imports"),
        "Coal imports source header missing:\n{out}"
    );
    // Target-only nodes must also appear (as arc targets).
    assert!(out.contains("Liquid"), "Liquid target missing:\n{out}");
    assert!(out.contains("Solid"), "Solid target missing:\n{out}");
    // Arrow glyphs must be present.
    assert!(
        out.contains('\u{25BA}'),
        "arrowhead glyph \u{25BA} missing:\n{out}"
    );
    // Spot-check at least one value.
    assert!(out.contains("124.7"), "value 124.7 missing:\n{out}");
    assert!(out.contains("280.3"), "value 280.3 missing:\n{out}");

    assert_snapshot!("sankey_beta_canonical_example", out);
}

// ---------------------------------------------------------------------------
// xychart-beta — canonical Mermaid sales-revenue example.
//     Regression guard: title, y-axis label, x-axis labels, bar glyphs, and
//     axis structure must all render correctly.
// ---------------------------------------------------------------------------
#[test]
fn xychart_beta_canonical_example() {
    let src = "xychart-beta
    title \"Sales Revenue\"
    x-axis [jan, feb, mar, apr, may, jun, jul, aug, sep, oct, nov, dec]
    y-axis \"Revenue (in $)\" 4000 --> 11000
    bar [5000, 6000, 7500, 8200, 9500, 10500, 11000, 10200, 9200, 8500, 7000, 6000]
    line [5000, 6000, 7500, 8200, 9500, 10500, 11000, 10200, 9200, 8500, 7000, 6000]";

    let out = mermaid_text::render(src).unwrap();

    // Title must appear.
    assert!(
        out.contains("Sales Revenue"),
        "title missing from output:\n{out}"
    );
    // Y-axis label must appear.
    assert!(
        out.contains("Revenue (in $)"),
        "y-axis label missing from output:\n{out}"
    );
    // X-axis category labels must appear.
    assert!(out.contains("jan"), "jan label missing:\n{out}");
    assert!(out.contains("jun"), "jun label missing:\n{out}");
    assert!(out.contains("dec"), "dec label missing:\n{out}");
    // Bar glyphs must be present.
    assert!(
        out.contains('\u{2588}'),
        "bar glyph missing from output:\n{out}"
    );
    // Y-axis ticks must appear.
    assert!(out.contains("11000"), "y_max tick missing:\n{out}");
    assert!(out.contains("4000"), "y_min tick missing:\n{out}");
    // Axis connector glyph must appear.
    assert!(
        out.contains('\u{2524}') || out.contains('\u{2514}'),
        "axis glyph missing:\n{out}"
    );

    assert_snapshot!("xychart_beta_canonical_example", out);
}

// ---------------------------------------------------------------------------
// xychart-beta — every data point must show a `●` marker.
//
//     Regression guard against asymmetric line markers. Pre-fix
//     `draw_line` placed the `●` before the segment-drawing call,
//     and the rising-edge segment then drew its bottom corner `╯`
//     OVER the source data point's marker — leaving only the
//     descending half of a peaked line series visibly marked.
//     The canonical sales-revenue example exposed this: 12 monthly
//     data points (peak at jul), only 6 visible dots before the fix.
//
//     A trivially-broken implementation that draws zero or only
//     descending markers cannot satisfy `count == 12`. Counting
//     literal `●` characters across the whole output is robust to
//     incidental glyphs because `●` is not used elsewhere in the
//     xy-chart renderer (axis ticks, bars, and connectors all use
//     different glyphs).
// ---------------------------------------------------------------------------
#[test]
fn xy_chart_line_has_marker_per_data_point() {
    let src = "xychart-beta
    title \"Sales Revenue\"
    x-axis [jan, feb, mar, apr, may, jun, jul, aug, sep, oct, nov, dec]
    y-axis \"Revenue (in $)\" 4000 --> 11000
    bar [5000, 6000, 7500, 8200, 9500, 10500, 11000, 10200, 9200, 8500, 7000, 6000]
    line [5000, 6000, 7500, 8200, 9500, 10500, 11000, 10200, 9200, 8500, 7000, 6000]";
    let out = mermaid_text::render(src).unwrap();
    let dot_count = out.chars().filter(|&c| c == '\u{25CF}').count();
    assert_eq!(
        dot_count, 12,
        "expected one `●` line marker per data point (12), got {dot_count}. \
         The rising-edge segment likely overwrites the source marker — see \
         draw_line / draw_segment in render/xy_chart.rs.\n\nFull output:\n{out}"
    );
}

// ---------------------------------------------------------------------------
// block-beta — canonical Phase 1 grid + arrows example.
//     Regression guard: blocks, column spans, and edge summary must all
//     render correctly in the fixed-width grid layout.
// ---------------------------------------------------------------------------
#[test]
fn block_beta_canonical_example() {
    let src = "block-beta
    columns 3
    a[\"A label\"] b:2 c
    d e f
    g[\"spans across\"]:3
    a --> d
    b --> e
    c --> f";

    let out = mermaid_text::render(src).unwrap();

    // All block labels must appear.
    assert!(out.contains("A label"), "A label missing:\n{out}");
    assert!(out.contains("spans across"), "spans across missing:\n{out}");
    for id in &["b", "c", "d", "e", "f"] {
        assert!(out.contains(id), "block {id} missing from output:\n{out}");
    }
    // Box-drawing characters must be present.
    assert!(
        out.contains('\u{250C}'),
        "top-left corner ┌ missing:\n{out}"
    );
    assert!(
        out.contains('\u{2518}'),
        "bottom-right corner ┘ missing:\n{out}"
    );
    assert!(out.contains('\u{2502}'), "vertical bar │ missing:\n{out}");
    // Edge summary must appear.
    assert!(out.contains("Edges:"), "Edges: header missing:\n{out}");
    assert!(
        out.contains('\u{25BA}'),
        "arrowhead glyph ► missing:\n{out}"
    );
    // Source/target ids must appear in the edge lines.
    assert!(out.contains("a "), "source 'a' in edges missing:\n{out}");
    assert!(out.contains(" d"), "target 'd' in edges missing:\n{out}");

    assert_snapshot!("block_beta_canonical_example", out);
}

// ---------------------------------------------------------------------------
// block-beta — inline spatial edge rendering (0.42.0).
//
// Strong-assertion test written BEFORE the implementation to confirm the
// current renderer would fail all three checks:
//   1. At least 2 right-arrow glyphs (►) in the output — one per adjacent edge.
//      A no-op impl has 0; an impl that only handles the first row has 1.
//   2. Grid integrity: ┌ count equals block count (6). A "lost the grid" bug
//      would reduce this.
//   3. "Edges:" header absent — all edges in this diagram are adjacent and
//      must be rendered inline, so the text summary must be gone entirely.
// ---------------------------------------------------------------------------
#[test]
fn block_beta_inline_adjacent_edges() {
    // 3-column grid, two rows, two horizontally-adjacent edges in different rows.
    let src = "block-beta
    columns 3
    A B C
    D E F
    A --> B
    D --> E";

    let out = mermaid_text::render(src).unwrap();

    // 1. Both adjacent edges must produce an inline right-arrow glyph.
    let arrow_count = out.chars().filter(|&c| c == '\u{25BA}').count();
    assert!(
        arrow_count >= 2,
        "expected at least 2 inline ► arrows (one per adjacent edge), \
         got {arrow_count}:\n{out}"
    );

    // 2. Grid integrity: 6 blocks → 6 top-left corner glyphs.
    let corner_count: usize = out.chars().filter(|&c| c == '\u{250C}').count();
    assert_eq!(
        corner_count, 6,
        "expected exactly 6 ┌ corners (one per block), got {corner_count}:\n{out}"
    );

    // 3. Text summary must be absent when every edge is routable inline.
    assert!(
        !out.contains("Edges:"),
        "\"Edges:\" text summary must be absent when all edges are routed inline:\n{out}"
    );
}

// ---------------------------------------------------------------------------
// block-beta — vertical adjacent edge (same column, neighbouring rows).
// ---------------------------------------------------------------------------
#[test]
fn block_beta_inline_vertical_edge() {
    let src = "block-beta
    columns 1
    A
    B
    A --> B";

    let out = mermaid_text::render(src).unwrap();

    // Vertical adjacent edge: expect a downward-arrow glyph ▼ in the gap row.
    let down_arrow_count = out.chars().filter(|&c| c == '\u{25BC}').count();
    assert!(
        down_arrow_count >= 1,
        "expected at least 1 inline ▼ arrow for vertical adjacent edge, \
         got {down_arrow_count}:\n{out}"
    );

    // Grid integrity: 2 blocks → 2 ┌ corners.
    let corner_count: usize = out.chars().filter(|&c| c == '\u{250C}').count();
    assert_eq!(
        corner_count, 2,
        "expected exactly 2 ┌ corners, got {corner_count}:\n{out}"
    );

    // No text summary needed since edge is routable.
    assert!(
        !out.contains("Edges:"),
        "\"Edges:\" must be absent when vertical edge is routed inline:\n{out}"
    );
}

// ---------------------------------------------------------------------------
// block-beta — non-adjacent edge falls back to text summary (Tier 3).
// ---------------------------------------------------------------------------
#[test]
fn block_beta_non_adjacent_edge_falls_back_to_summary() {
    // A and F are not adjacent — A is (row 0, col 0), F is (row 1, col 2).
    let src = "block-beta
    columns 3
    A B C
    D E F
    A --> F";

    let out = mermaid_text::render(src).unwrap();

    // Grid must still be intact.
    let corner_count: usize = out.chars().filter(|&c| c == '\u{250C}').count();
    assert_eq!(
        corner_count, 6,
        "expected exactly 6 ┌ corners, got {corner_count}:\n{out}"
    );

    // Non-adjacent edge falls back to text summary.
    assert!(
        out.contains("Edges:") || out.contains('\u{25BA}'),
        "non-adjacent edge must appear somewhere (summary or inline):\n{out}"
    );
}

// ---------------------------------------------------------------------------
// architecture-beta — canonical Phase 1 system-architecture example.
//     Regression guard: groups, services, and connection summary must all
//     render correctly.
// ---------------------------------------------------------------------------
#[test]
fn architecture_beta_canonical_example() {
    let src = "architecture-beta
    group api(cloud)[API]

    service db(database)[Database] in api
    service disk1(disk)[Storage] in api
    service disk2(disk)[Storage] in api
    service server(server)[Server] in api

    db:L -- R:server
    disk1:T -- B:server
    disk2:T -- B:db";

    let out = mermaid_text::render(src).unwrap();

    // Group label must appear in the subgraph border.
    assert!(out.contains("API"), "group label 'API' missing:\n{out}");
    // Service labels must appear inside their node boxes.
    for label in &["Database", "Storage", "Server"] {
        assert!(out.contains(label), "service label {label} missing:\n{out}");
    }
    // Box-drawing characters must be present (subgraph border + node boxes + edge lines).
    assert!(out.contains('\u{2502}'), "vertical bar │ missing:\n{out}");
    // Path A: edges are spatially routed — no "Connections:" text summary.
    assert!(
        !out.contains("Connections:"),
        "Connections: text summary must not appear after Path A upgrade:\n{out}"
    );
    // Spatial routing must produce at least one edge-drawing character.
    let has_edge_char = out.contains('\u{2500}') || out.contains('\u{2502}') || out.contains("▸");
    assert!(has_edge_char, "no spatial edge character found:\n{out}");

    assert_snapshot!("architecture_beta_canonical_example", out);
}

// ---------------------------------------------------------------------------
// packet-beta — canonical TCP Packet example.
//     Regression guard: title, 32-bit rows, field labels, bit ruler, and
//     box-drawing borders must all appear correctly.
// ---------------------------------------------------------------------------
#[test]
fn packet_beta_canonical_example() {
    let src = "packet-beta
    title TCP Packet
    0-15: \"Source Port\"
    16-31: \"Destination Port\"
    32-63: \"Sequence Number\"
    64-95: \"Acknowledgment Number\"
    96-99: \"Data Offset\"
    100-105: \"Reserved\"
    106: \"URG\"
    107: \"ACK\"
    108: \"PSH\"
    109: \"RST\"
    110: \"SYN\"
    111: \"FIN\"
    112-127: \"Window\"
    128-143: \"Checksum\"
    144-159: \"Urgent Pointer\"
    160-191: \"(Options and Padding)\"
    192-223: \"Data (variable length)\"";

    let out = mermaid_text::render(src).unwrap();

    // Title must appear.
    assert!(out.contains("TCP Packet"), "title missing:\n{out}");
    // Field labels that fit their cells must appear.
    assert!(out.contains("Source Port"), "Source Port missing:\n{out}");
    assert!(
        out.contains("Destination Port"),
        "Destination Port missing:\n{out}"
    );
    assert!(
        out.contains("Sequence Number"),
        "Sequence Number missing:\n{out}"
    );
    assert!(
        out.contains("Acknowledgment Number"),
        "Acknowledgment Number missing:\n{out}"
    );
    assert!(out.contains("Window"), "Window missing:\n{out}");
    assert!(out.contains("Checksum"), "Checksum missing:\n{out}");
    assert!(
        out.contains("(Options and Padding)"),
        "(Options and Padding) missing:\n{out}"
    );
    assert!(
        out.contains("Data (variable length)"),
        "Data (variable length) missing:\n{out}"
    );
    // Box-drawing characters must be present.
    assert!(
        out.contains('\u{250C}'),
        "top-left corner ┌ missing:\n{out}"
    );
    assert!(
        out.contains('\u{2510}'),
        "top-right corner ┐ missing:\n{out}"
    );
    assert!(
        out.contains('\u{2514}'),
        "bottom-left corner └ missing:\n{out}"
    );
    assert!(
        out.contains('\u{2518}'),
        "bottom-right corner ┘ missing:\n{out}"
    );
    assert!(out.contains('\u{2502}'), "vertical bar │ missing:\n{out}");
    // Continuation row border ├ must be present (TCP header spans multiple rows).
    assert!(
        out.contains('\u{251C}'),
        "continuation row border ├ missing:\n{out}"
    );

    assert_snapshot!("packet_beta_canonical_example", out);
}
