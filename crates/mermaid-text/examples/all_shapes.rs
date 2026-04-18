//! All-shapes example for `mermaid-text`.
//!
//! Renders every supported node shape in a single diagram.  Useful as a
//! visual regression test and as documentation showing what each shape looks
//! like in the terminal.
//!
//! Run with:
//! ```text
//! cargo run -p mermaid-text --example all_shapes
//! ```

fn main() {
    // Each node uses a different shape syntax; all twelve supported shapes are
    // represented here.  The labels intentionally include the Mermaid syntax
    // so the rendered output is self-documenting.
    let source = r#"graph TD
    R[Rectangle]
    Ro(Rounded)
    D{Diamond}
    C((Circle))
    St([Stadium])
    Sub[[Subroutine]]
    Cy[(Cylinder)]
    H{{Hexagon}}
    As>Asymmetric]
    P[/Parallelogram/]
    T[/Trapezoid\]
    DC(((DoubleCircle)))"#;

    println!("All supported node shapes:");
    println!();
    let output = mermaid_text::render(source).expect("valid Mermaid input");
    println!("{output}");
}
