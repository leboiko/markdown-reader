//! CI/CD pipeline example for `mermaid-text`.
//!
//! A realistic multi-stage pipeline with subgraphs, dotted edges for optional
//! paths, and thick edges for the critical path.  Demonstrates subgraph
//! borders, edge styles, and edge labels.
//!
//! Run with:
//! ```text
//! cargo run -p mermaid-text --example ci_pipeline
//! ```

fn main() {
    let source = r#"graph LR
    subgraph CI
        direction LR
        L[Lint] ==> B[Build] ==> T[Test]
    end
    subgraph CD
        direction LR
        Pub[Publish] ==> D[Deploy]
    end
    T ==>|pass| Pub
    T -.->|skip| D
    B -.->|cache miss| L
    D -->|notify| Slack[Slack]"#;

    println!("CI/CD pipeline diagram:");
    println!();
    println!("  Thick edges (==>) = critical path");
    println!("  Dotted edges (-.->)  = optional paths");
    println!();

    let output = mermaid_text::render(source).expect("valid Mermaid input");
    println!("{output}");
}
