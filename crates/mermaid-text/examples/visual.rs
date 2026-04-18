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
    dump(
        "TD diamond",
        "graph TD; A[Start] --> B{Ok?}; B -->|Yes| C[Go]; B -->|No| D[Stop]",
    );
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
    dump(
        "subgraph LR",
        r#"graph LR
    subgraph Supervisor
        F[Factory] --> W[Worker]
    end"#,
    );
    dump(
        "subgraph with external edge LR",
        r#"graph LR
    subgraph S
        F[Factory] --> W[Worker]
    end
    W --> HB[Heartbeat]"#,
    );
    dump(
        "nested subgraphs TD",
        r#"graph TD
    subgraph Outer
        subgraph Inner
            A[A]
        end
        B[B]
    end"#,
    );
    dump(
        "real-world with subgraph",
        r#"graph LR
    subgraph Supervisor
        direction TB
        F[Factory] -->|creates| W[Worker]
        W -->|panics/exits| F
    end
    W -->|beat| HB[Heartbeat]
    HB --> WD[Watchdog]
    W --> CB{Circuit Breaker}
    CB -->|CLOSED| DB[(Database)]"#,
    );
}
