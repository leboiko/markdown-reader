use mermaid_text::{render, render_ascii};

fn center(output: &str) -> usize {
    let cells: Vec<char> = output.lines().nth(1).unwrap().chars().collect();
    let name = cells.iter().position(|&ch| ch == 'B').unwrap();
    let left = (0..name)
        .rfind(|&col| matches!(cells[col], '│' | '|'))
        .unwrap();
    let right = (name..cells.len())
        .find(|&col| matches!(cells[col], '│' | '|'))
        .unwrap();
    left + (right - left).div_ceil(2)
}

fn pair(output: &str, row: usize, col: usize) -> [char; 2] {
    let cells: Vec<char> = output.lines().nth(row).unwrap().chars().collect();
    [
        cells.get(col).copied().unwrap_or(' '),
        cells.get(col + 1).copied().unwrap_or(' '),
    ]
}

#[test]
fn explicit_activation_covers_spacer_and_note_rows() {
    for anchor in ["right of B", "left of B", "over A"] {
        let source = format!(
            "sequenceDiagram\nparticipant A\nparticipant B\nA->>B: start\nactivate B\nNote {anchor}: thinking\nB-->>A: reply\ndeactivate B"
        );
        for (output, bar) in [
            (render(&source).unwrap(), '█'),
            (render_ascii(&source).unwrap(), '#'),
        ] {
            let rows: Vec<Vec<char>> = output.lines().map(|line| line.chars().collect()).collect();
            let b = center(&output);
            let start = output
                .lines()
                .position(|line| line.contains("start"))
                .unwrap()
                + 2;
            let end = output
                .lines()
                .position(|line| line.contains("reply"))
                .unwrap()
                + 1;
            assert_eq!(output.matches("thinking").count(), 1, "{output}");
            for (row, cells) in rows.iter().enumerate().take(end + 1).skip(start) {
                assert_eq!(
                    [
                        cells.get(b).copied().unwrap_or(' '),
                        cells.get(b + 1).copied().unwrap_or(' ')
                    ],
                    [bar, bar],
                    "activation missing at row {row}, column {b}, anchor {anchor}:\n{output}"
                );
            }
            assert_ne!(
                rows[end + 1][b],
                bar,
                "activation must end at reply:\n{output}"
            );
        }
    }
}

#[test]
fn notes_before_activation_do_not_gain_a_bar() {
    for activation in ["activate B\nB-->>A: reply", "B-->>+A: reply"] {
        let source = format!(
            "sequenceDiagram\nA->>B: start\nNote right of B: earlier\n{activation}\ndeactivate {}",
            if activation.starts_with("activate") {
                "B"
            } else {
                "A"
            }
        );
        for (output, bar) in [
            (render(&source).unwrap(), '█'),
            (render_ascii(&source).unwrap(), '#'),
        ] {
            let b = center(&output);
            let note = output
                .lines()
                .position(|line| line.contains("earlier"))
                .unwrap();
            for row in note - 1..=note + 1 {
                assert_ne!(
                    pair(&output, row, b),
                    [bar, bar],
                    "earlier note gained activation:\n{output}"
                );
            }
            let reply = output
                .lines()
                .position(|line| line.contains("reply"))
                .unwrap();
            assert_eq!(output.matches("reply").count(), 1, "{output}");
            if activation.starts_with("activate") {
                assert_eq!(pair(&output, reply, b), [bar, bar], "{output}");
            } else {
                let a = output
                    .lines()
                    .nth(reply)
                    .unwrap()
                    .chars()
                    .position(|ch| ch == bar)
                    .unwrap();
                assert_eq!(pair(&output, reply + 1, a), [bar, bar], "{output}");
                for row in note - 1..=note + 1 {
                    assert_ne!(
                        pair(&output, row, a),
                        [bar, bar],
                        "inline activation reached backward:\n{output}"
                    );
                }
            }
        }
    }
}

#[test]
fn nested_explicit_activation_spans_multiline_and_consecutive_notes() {
    let source = "sequenceDiagram\nA->>+B: outer\nNote right of B: before inner\nactivate B\nNote right of B: first<br/>second\nNote left of A: third\nB-->>A: inner reply\ndeactivate B\nB-->>A: outer reply\ndeactivate B";
    for (output, bar) in [
        (render(source).unwrap(), '█'),
        (render_ascii(source).unwrap(), '#'),
    ] {
        let b = center(&output);
        let before = output
            .lines()
            .position(|line| line.contains("before inner"))
            .unwrap();
        assert_eq!(pair(&output, before, b), [bar, bar], "{output}");
        assert_ne!(pair(&output, before, b + 3), [bar, bar], "{output}");
        let first = output
            .lines()
            .position(|line| line.contains("first"))
            .unwrap();
        let end = output
            .lines()
            .position(|line| line.contains("inner reply"))
            .unwrap()
            + 1;
        for row in first - 2..=end {
            for col in [b, b + 3] {
                assert_eq!(
                    pair(&output, row, col),
                    [bar, bar],
                    "row {row} col {col}:\n{output}"
                );
            }
        }
        let outer_reply = output
            .lines()
            .position(|line| line.contains("outer reply"))
            .unwrap();
        assert_eq!(pair(&output, outer_reply, b), [bar, bar], "{output}");
        assert_ne!(pair(&output, outer_reply, b + 3), [bar, bar], "{output}");
        for text in [
            "before inner",
            "first",
            "second",
            "third",
            "inner reply",
            "outer reply",
        ] {
            assert_eq!(output.matches(text).count(), 1, "{output}");
        }
    }
}
