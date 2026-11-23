use super::*;
use crate::markdown::DocBlock;
use std::cell::RefCell;

fn load_document(app: &mut App, name: &str, content: &str, new_tab: bool) {
    let path = PathBuf::from(name);
    app.open_or_focus(path.clone(), new_tab, None);
    app.apply_file_loaded(path.clone(), content.to_owned(), new_tab, None);
    assert_eq!(app.tabs.active_tab().unwrap().view.current_path, Some(path));
    assert_eq!(app.tabs.active_tab().unwrap().view.content, content);
}

fn seed_render_cache(app: &mut App) -> (MermaidBlockId, MathBlockId) {
    let blocks = &app.tabs.active_tab().unwrap().view.rendered;
    let diagram = blocks
        .iter()
        .find_map(|b| match b {
            DocBlock::Mermaid { id, .. } => Some(*id),
            _ => None,
        })
        .expect("diagram parsed");
    let formula = blocks
        .iter()
        .find_map(|b| match b {
            DocBlock::Math { id, .. } => Some(*id),
            _ => None,
        })
        .expect("formula parsed");
    app.mermaid_cache.insert(
        diagram,
        MermaidEntry::AsciiDiagram {
            diagram: "diagram payload\n".repeat(128),
            reason: "memory regression".into(),
            styled_text_cache: RefCell::new(None),
        },
    );
    app.math_cache.insert(
        formula,
        MathEntry::Unicode {
            text: "formula payload\n".repeat(128),
            reason: "memory regression".into(),
        },
    );
    (diagram, formula)
}

#[test]
fn replacement_evicts_unreferenced_render_allocations() {
    crate::config::use_isolated_test_config();
    let root = tempfile::tempdir().unwrap();
    let mut app = App::new(root.path().into(), None, None);
    app.math_mode = crate::config::MathMode::Image;
    load_document(
        &mut app,
        "/fake/old.md",
        "```mermaid\ngraph LR\nA --> B\n```\n\n$$x^2$$",
        false,
    );
    let (diagram, formula) = seed_render_cache(&mut app);
    assert!(app.mermaid_cache.get(diagram).is_some());
    assert!(app.math_cache.get(formula).is_some());
    load_document(&mut app, "/fake/new.md", "# Replacement", false);
    assert_eq!(app.tabs.len(), 1);
    assert!(
        app.mermaid_cache.get(diagram).is_none(),
        "replaced diagram allocation retained"
    );
    assert!(
        app.math_cache.get(formula).is_none(),
        "replaced formula allocation retained"
    );
    assert_eq!(
        app.mermaid_cache.height(diagram, "", 1000),
        DEFAULT_MERMAID_HEIGHT,
        "removed diagram height retained"
    );
}

#[test]
fn reload_evicts_removed_diagram_height_metadata() {
    crate::config::use_isolated_test_config();
    let root = tempfile::tempdir().unwrap();
    let mut app = App::new(root.path().into(), None, None);
    app.math_mode = crate::config::MathMode::Image;
    load_document(
        &mut app,
        "/fake/reload.md",
        "```mermaid\ngraph LR\nA --> B\n```\n\n$$x^2$$",
        false,
    );
    let (diagram, _) = seed_render_cache(&mut app);
    assert_eq!(app.mermaid_cache.height(diagram, "", 1000), 130);
    app.handle_action(Action::FileReloaded {
        path: PathBuf::from("/fake/reload.md"),
        content: "# Reloaded".into(),
    });
    assert!(app.mermaid_cache.get(diagram).is_none());
    assert_eq!(
        app.mermaid_cache.height(diagram, "", 1000),
        DEFAULT_MERMAID_HEIGHT,
        "removed diagram height retained after reload"
    );
}

#[test]
fn closing_tabs_preserves_shared_entries_until_last_reference() {
    crate::config::use_isolated_test_config();
    let root = tempfile::tempdir().unwrap();
    let mut app = App::new(root.path().into(), None, None);
    app.math_mode = crate::config::MathMode::Image;
    let content = "```mermaid\ngraph LR\nA --> B\n```\n\n$$x^2$$";
    load_document(&mut app, "/fake/first.md", content, true);
    let (diagram, formula) = seed_render_cache(&mut app);
    load_document(&mut app, "/fake/second.md", content, true);
    app.focus = Focus::Viewer;
    app.handle_key(KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(app.tabs.len(), 1);
    assert!(
        app.mermaid_cache.get(diagram).is_some(),
        "live shared diagram lost"
    );
    assert!(
        app.math_cache.get(formula).is_some(),
        "live shared formula lost"
    );
    app.handle_key(KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(app.tabs.is_empty());
    assert!(
        app.mermaid_cache.get(diagram).is_none(),
        "closed diagram retained"
    );
    assert!(
        app.math_cache.get(formula).is_none(),
        "closed formula retained"
    );
    app.handle_action(Action::MermaidReady(
        diagram,
        Box::new(MermaidEntry::AsciiDiagram {
            diagram: "late result".into(),
            reason: "late".into(),
            styled_text_cache: RefCell::new(None),
        }),
    ));
    assert!(
        app.mermaid_cache.get(diagram).is_none(),
        "late completion resurrected closed entry"
    );
}

/// Opt-in workload for RSS sampling, native leaks and AddressSanitizer.
#[test]
#[ignore = "release memory soak; run scripts/check-memory.py"]
fn memory_soak_document_lifecycle() {
    use ratatui::{Terminal, backend::TestBackend};
    crate::config::use_isolated_test_config();
    let root = tempfile::tempdir().unwrap();
    let mut app = App::new(root.path().into(), None, None);
    app.math_mode = crate::config::MathMode::Image;
    app.tree_hidden = true;
    app.picker = None;
    app.mermaid_mode = crate::config::MermaidMode::Text;
    let mut terminal = Terminal::new(TestBackend::new(100, 60)).unwrap();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    for iteration in 0..2200 {
        let content = format!(
            "# Document {iteration}\n\n```mermaid\ngraph LR\nA{iteration} --> B\n```\n\n$$x^{{{iteration}}}$$\n\n| A | B |\n|---|---|\n| one | two |\n"
        );
        app.action_tx = None;
        load_document(&mut app, "/fake/soak.md", &content, false);
        app.action_tx = Some(tx.clone());
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        let (diagram, formula) = seed_render_cache(&mut app);
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .any(|cell| cell.symbol() == "D")
        );
        app.action_tx = None;
        load_document(&mut app, "/fake/empty.md", "# Empty", false);
        assert!(app.mermaid_cache.get(diagram).is_none());
        assert!(app.math_cache.get(formula).is_none());
        assert_eq!(
            app.mermaid_cache.height(diagram, "", 1000),
            DEFAULT_MERMAID_HEIGHT
        );
        if iteration % 100 == 0 {
            println!("MEMORY_SAMPLE iteration={iteration}");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    app.focus = Focus::Viewer;
    app.handle_key(KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(app.tabs.is_empty());
}

#[test]
fn saving_releases_removed_render_entries() {
    crate::config::use_isolated_test_config();
    let root = tempfile::tempdir().unwrap();
    let mut app = App::new(root.path().into(), None, None);
    app.math_mode = crate::config::MathMode::Image;
    load_document(
        &mut app,
        "/fake/save.md",
        "```mermaid\ngraph LR\nA --> B\n```\n\n$$x^2$$",
        false,
    );
    let (diagram, formula) = seed_render_cache(&mut app);
    app.apply_file_saved(PathBuf::from("/fake/save.md"), "# Saved".into());
    assert_eq!(app.tabs.active_tab().unwrap().view.content, "# Saved");
    assert!(app.mermaid_cache.get(diagram).is_none());
    assert!(app.math_cache.get(formula).is_none());
    assert_eq!(
        app.mermaid_cache.height(diagram, "", 1000),
        DEFAULT_MERMAID_HEIGHT
    );
}
