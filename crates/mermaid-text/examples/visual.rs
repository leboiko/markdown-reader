fn dump(title: &str, src: &str) {
    println!("=== {title} ===");
    println!("{src}\n");
    match mermaid_text::render(src) {
        Ok(out) => println!("{out}"),
        Err(e) => println!("ERROR: {e}"),
    }
    println!();
}

fn main() {
    dump("simple chain LR", "graph LR; A-->B-->C");
    dump("TD diamond", "graph TD; A[Start] --> B{Ok?}; B -->|Yes| C[Go]; B -->|No| D[Stop]");
    dump("crossing edges", "graph LR; A-->C; B-->D; A-->D; B-->C");
    dump(
        "real-world",
        r#"graph LR
    F[Factory] -->|creates| W[Worker]
    W -->|panics/exits| F
    W -->|beat| HB[Heartbeat]
    HB --> WD[Watchdog]
    W --> CB{Circuit Breaker}
    CB -->|CLOSED| DB[(Database)]"#,
    );
}
