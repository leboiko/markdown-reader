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

    // Regression: sibling subgraphs must not overlap each other.
    dump(
        "sibling subgraphs LR",
        r#"graph LR
    subgraph projections-pg [projections-pg :9092]
        PG_W[event_log, account_registry]
    end
    subgraph projections-surreal [projections-surreal :9093]
        S_W[atom, triple, deposit]
    end
    subgraph projections-core [projections-core-entities :9094]
        CE_W[core_entities dual-write]
    end
    PG_W --> PG[(PostgreSQL)]
    S_W --> PG
    S_W --> SDB[(SurrealDB)]
    CE_W --> PG
    CE_W --> SDB"#,
    );

    // --- Perpendicular-direction subgraph ---
    dump(
        "perpendicular: LR subgraph inside TD parent",
        r#"graph TD
    subgraph Pipeline
        direction LR
        A[Input] --> B[Process] --> C[Output]
    end
    C --> D[Finish]"#,
    );
    dump(
        "perpendicular: TB subgraph inside LR parent",
        r#"graph LR
    subgraph Supervisor
        direction TB
        F[Factory] --> W[Worker]
        W --> F
    end
    W --> HB[Heartbeat]"#,
    );

    // --- Part A: New node shapes ---
    dump("stadium node", "graph LR; A([Stadium]) --> B[End]");
    dump("subroutine node", "graph LR; A[[Subroutine]] --> B[End]");
    dump(
        "cylinder (database) node",
        "graph LR; A[(Database)] --> B[End]",
    );
    dump("hexagon node", "graph LR; A{{Hexagon}} --> B[End]");
    dump("asymmetric node", "graph LR; A>Flag] --> B[End]");
    dump("parallelogram node", "graph LR; A[/Lean/] --> B[End]");
    dump("trapezoid node", "graph LR; A[/Trap\\] --> B[End]");
    dump("double circle node", "graph LR; A(((Double))) --> B[End]");
    dump(
        "all new shapes",
        r#"graph TD
    S([Stadium])
    R[[Subroutine]]
    D[(Database)]
    H{{Hexagon}}
    F>Flag]
    P[/Parallel/]
    T[/Trap\]
    DC(((DblCircle)))
    S --> R
    R --> D
    D --> H
    H --> F
    F --> P
    P --> T
    T --> DC"#,
    );

    // --- Part B: Edge styles ---
    dump("dotted edge", "graph LR; A-.->B");
    dump("thick edge", "graph LR; A==>B");
    dump("bidirectional edge", "graph LR; A<-->B");
    dump("plain line (no arrow)", "graph LR; A---B");
    dump("circle endpoint", "graph LR; A--oB");
    dump("cross endpoint", "graph LR; A--xB");
    dump(
        "mixed edge styles",
        r#"graph LR
    A[Solid] --> B[Dotted]
    B -.-> C[Thick]
    C ==> D[BiDir]
    D <--> E[Plain]
    E --- F[Circle]
    F --o G[Cross]
    G --x A"#,
    );
}
