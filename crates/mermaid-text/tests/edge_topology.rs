use mermaid_text::{RenderOptions, render, render_with_options};

const SELF_LOOP: &str = "flowchart LR\n    A[Retry] --> A\n";
const BACK_EDGES: &str = "flowchart TD\n\
  E[EMPTY] --> P[PUBLISHED]\n\
  P --> T[TAKEN]\n\
  T --> D[DONE]\n\
  P --> E\n\
  T --> E\n\
  D --> E\n";

fn render_ascii(source: &str) -> String {
    render_with_options(
        source,
        &RenderOptions {
            ascii: true,
            ..RenderOptions::default()
        },
    )
    .expect("fixture should render as ASCII")
}

const UP: u8 = 1;
const RIGHT: u8 = 2;
const DOWN: u8 = 4;
const LEFT: u8 = 8;

fn connector_arms(glyph: char) -> u8 {
    match glyph {
        '─' => LEFT | RIGHT,
        '│' => UP | DOWN,
        '┌' => RIGHT | DOWN,
        '┐' => LEFT | DOWN,
        '└' => UP | RIGHT,
        '┘' => UP | LEFT,
        '├' => UP | RIGHT | DOWN,
        '┤' => UP | DOWN | LEFT,
        '┬' => LEFT | RIGHT | DOWN,
        '┴' => UP | LEFT | RIGHT,
        '┼' => UP | RIGHT | DOWN | LEFT,
        // Arrow masks describe their tail, not the direction of the tip.
        '▴' => DOWN,
        '▾' => UP,
        '▸' => LEFT,
        '◂' => RIGHT,
        _ => 0,
    }
}

fn assert_no_exposed_connector_arms(output: &str) {
    let rows: Vec<Vec<char>> = output.lines().map(|line| line.chars().collect()).collect();
    let neighbours = [
        (-1isize, 0isize, UP, DOWN),
        (0, 1, RIGHT, LEFT),
        (1, 0, DOWN, UP),
        (0, -1, LEFT, RIGHT),
    ];

    for (row, line) in rows.iter().enumerate() {
        for (col, &glyph) in line.iter().enumerate() {
            let arms = connector_arms(glyph);
            for &(dr, dc, arm, _reciprocal) in &neighbours {
                if arms & arm == 0 {
                    continue;
                }
                let neighbour = row
                    .checked_add_signed(dr)
                    .and_then(|r| rows.get(r))
                    .and_then(|line| col.checked_add_signed(dc).and_then(|c| line.get(c)))
                    .copied()
                    .unwrap_or(' ');
                // An arrow tip replaces the node-border cell it lands on. The
                // arrow's own mask describes only its incoming tail, so the
                // two border segments beside that endpoint intentionally do
                // not see a reciprocal arm in the arrow glyph.
                if matches!(neighbour, '▴' | '▾' | '▸' | '◂') {
                    continue;
                }
                assert!(
                    !neighbour.is_whitespace(),
                    "connector {glyph:?} at (col={col}, row={row}) exposes an arm \
                     into blank space:\n{output}"
                );
            }
        }
    }
}

fn assert_back_edges_have_complete_endpoints(
    output: &str,
    left_arrow: char,
    source_junction: char,
) {
    assert_eq!(
        output.chars().filter(|&glyph| glyph == left_arrow).count(),
        3,
        "all three back edges must retain a distinct arrow tip at EMPTY:\n{output}"
    );

    for label in ["PUBLISHED", "TAKEN", "DONE"] {
        let line = output
            .lines()
            .find(|line| line.contains(label))
            .unwrap_or_else(|| panic!("{label} node should be rendered"));
        let chars: Vec<char> = line.chars().collect();
        let label_end = line
            .find(label)
            .map(|byte| line[..byte + label.len()].chars().count())
            .expect("label byte index should map to a character column");
        let junction_col = chars[label_end..]
            .iter()
            .position(|&glyph| glyph == source_junction)
            .map(|offset| label_end + offset)
            .unwrap_or_else(|| {
                panic!("{label} must expose a back-edge source junction:\n{output}")
            });
        assert!(
            chars
                .get(junction_col + 1)
                .is_some_and(|glyph| !glyph.is_whitespace()),
            "{label}'s back-edge source junction must connect to a routed cell \
             on its right, not terminate at the node border:\n{output}"
        );
    }
}

fn assert_self_loop_source_join(output: &str, junction: char) {
    let line = output
        .lines()
        .find(|line| line.contains("Retry"))
        .expect("Retry node should be rendered");
    let chars: Vec<char> = line.chars().collect();
    let label_end = line
        .find("Retry")
        .map(|byte| line[..byte + "Retry".len()].chars().count())
        .expect("label byte index should map to a character column");
    let junction_col = chars[label_end..]
        .iter()
        .position(|&glyph| glyph == junction)
        .map(|offset| label_end + offset)
        .unwrap_or_else(|| panic!("Retry must expose a self-loop source junction:\n{output}"));
    assert!(
        chars
            .get(junction_col + 1)
            .is_some_and(|glyph| !glyph.is_whitespace()),
        "Retry's source junction must connect to the loop route:\n{output}"
    );
}

fn assert_reverse_self_loop_preserves_box(
    output: &str,
    corner: char,
    expected_corners: usize,
    border: char,
) {
    let label_line = output
        .lines()
        .find(|line| line.contains("Retry"))
        .expect("Retry node should be rendered")
        .trim();
    assert!(
        label_line.starts_with(border) && label_line.ends_with(border),
        "reverse self-loop must preserve both side borders:\n{output}"
    );
    assert_eq!(
        output.chars().filter(|&glyph| glyph == corner).count(),
        expected_corners,
        "reverse self-loop must not split a horizontal box border:\n{output}"
    );
}

#[test]
fn unicode_self_loop_leaves_and_returns_to_distinct_ports() {
    let output = render(SELF_LOOP).expect("fixture should render");
    assert_eq!(output.matches("Retry").count(), 1, "{output}");
    assert_eq!(
        output.chars().filter(|&glyph| glyph == '▴').count(),
        1,
        "{output}"
    );
    assert_self_loop_source_join(&output, '├');
    assert_no_exposed_connector_arms(&output);
}

#[test]
fn ascii_self_loop_leaves_and_returns_to_distinct_ports() {
    let unicode = render(SELF_LOOP).expect("fixture should render");
    assert_no_exposed_connector_arms(&unicode);
    let output = render_ascii(SELF_LOOP);
    assert_eq!(output, mermaid_text::to_ascii(&unicode));
    assert_eq!(output.matches("Retry").count(), 1, "{output}");
    assert_eq!(
        output.chars().filter(|&glyph| glyph == '^').count(),
        1,
        "{output}"
    );
    assert_self_loop_source_join(&output, '+');
    assert!(
        output.lines().skip(3).any(|line| line.contains('-')),
        "ASCII self-loop must contain a horizontal return leg:\n{output}"
    );
}

#[test]
fn unicode_reverse_self_loops_preserve_rectangle_borders() {
    for direction in ["RL", "BT"] {
        let output = render(&format!("flowchart {direction}\nA[Retry] --> A\n"))
            .expect("fixture should render");
        assert_reverse_self_loop_preserves_box(&output, '┌', 1, '│');
    }
}

#[test]
fn ascii_reverse_self_loops_preserve_rectangle_borders() {
    for direction in ["RL", "BT"] {
        let output = render_ascii(&format!("flowchart {direction}\nA[Retry] --> A\n"));
        assert_reverse_self_loop_preserves_box(&output, '+', 4, '|');
    }
}

#[test]
fn unicode_back_edges_keep_all_tips_and_source_connectors() {
    let output = render(BACK_EDGES).expect("fixture should render");
    assert_eq!(
        output.chars().filter(|&glyph| glyph == '▾').count(),
        3,
        "{output}"
    );
    assert_back_edges_have_complete_endpoints(&output, '◂', '├');
    assert_no_exposed_connector_arms(&output);
}

#[test]
fn ascii_back_edges_keep_all_tips_and_source_connectors() {
    let unicode = render(BACK_EDGES).expect("fixture should render");
    assert_no_exposed_connector_arms(&unicode);
    let output = render_ascii(BACK_EDGES);
    assert_eq!(output, mermaid_text::to_ascii(&unicode));
    assert_eq!(
        output.chars().filter(|&glyph| glyph == 'v').count(),
        3,
        "{output}"
    );
    assert_back_edges_have_complete_endpoints(&output, '<', '+');
}
