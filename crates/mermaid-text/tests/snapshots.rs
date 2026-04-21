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
// 18. ANSI color regression guard — running through `render_with_options`
//     with `color: false` must produce the exact same bytes as `render`.
//     This is the structural promise that ANSI is opt-in.
// ---------------------------------------------------------------------------
#[test]
fn color_disabled_is_byte_identical() {
    let src = "graph LR\nA[Start] --> B[End]\nstyle A fill:#336,stroke:#fff,color:#fff";
    let plain = mermaid_text::render(src).unwrap();
    let opts = mermaid_text::RenderOptions::default();
    let via_options = mermaid_text::render_with_options(src, &opts).unwrap();
    assert_eq!(plain, via_options, "color=false path must be byte-identical");
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
    assert!(out.contains("\x1b[48;2;34;51;68m"), "datastore fill SGR present");
    assert!(out.contains("\x1b[38;2;153;204;255m"), "datastore stroke SGR present");
    assert_snapshot!("classdef_and_class_directives", out);
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
    assert!(out.contains("\x1b[38;2;153;204;255m"), "accent stroke present");
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
    assert!(out.contains('◇'), "missing diamond marker for <<choice>>");
    assert!(
        out.contains('┃'),
        "missing vertical bar glyph for <<fork>>/<<join>> in default LR layout"
    );
    assert_snapshot!("state_diagram_special_shapes", out);
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
        out.contains('━'),
        "missing horizontal bar glyph for <<fork>> in TB layout"
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
    assert!(out.contains("\x1b[48;2;51;51;102m"), "fill SGR survives ascii");
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
    assert!(out.contains("[loop: forever]"));
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
    assert!(out.contains("[alt: success]"));
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
    assert!(out.contains("[opt: cache hit]"));
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
    assert!(out.contains("[loop: outer]"));
    assert!(out.contains("[alt: branch a]"));
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
    assert!(out.contains("[par: first]"));
    assert!(out.contains("[second]"));
    assert!(out.contains("[critical: primary]"));
    assert!(out.contains("[failure]"));
    assert_snapshot!("sequence_with_par_and_critical_blocks", out);
}

// ---------------------------------------------------------------------------
// Pie charts (0.9.4) — first full diagram-type addition since sequence in
// 0.9.0. Renders as a horizontal bar chart in monospace text.
// ---------------------------------------------------------------------------

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
    let src = "pie showData title Browser Share\n\"Chrome\" : 60\n\"Firefox\" : 25\n\"Safari\" : 15";
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
fn sequence_end_note_returns_helpful_error() {
    // Mermaid's sequence grammar has no `end note` form (state diagrams
    // do; sequence uses `<br>`). Make sure the parser flags this with a
    // pointer to the right syntax instead of silently misparsing.
    let src = "sequenceDiagram
participant U
end note";
    let err = mermaid_text::render(src).unwrap_err().to_string();
    assert!(err.contains("<br>"), "error should point at <br> syntax: {err}");
}
