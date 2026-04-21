fn main() {
    let blocks = [
        ("Block 1: state machine", r#"graph TD
    Idle -->|event| Running
    Running -->|done| Done
    Running -->|error| Failed
    Failed -->|retry| Idle"#),
        ("Block 2: Supervisor", r#"graph LR
    subgraph Supervisor
        direction TB
        F[Factory] -->|creates| W[Worker]
        W -->|panics| F
    end
    W -->|beat| HB[Heartbeat]
    HB --> WD[Watchdog]"#),
    ];
    for (name, src) in blocks {
        println!("--- {name} ---");
        println!("Image render:");
        match mermaid_rs_renderer::render(src) {
            Ok(svg) => println!("  OK ({} bytes svg)", svg.len()),
            Err(e) => println!("  ERROR — {e}"),
        }
        println!("Text render (figurehead/mermaid-text):");
        match mermaid_text::render_with_width(src, Some(80)) {
            Ok(out) => {
                println!("  OK ({} bytes):", out.len());
                for line in out.lines().take(20) {
                    println!("    {line}");
                }
            }
            Err(e) => println!("  ERROR — {e}"),
        }
        println!();
    }
}
