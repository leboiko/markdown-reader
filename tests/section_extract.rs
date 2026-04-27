//! Integration test: `--section NAME` extracts a heading section and prints it to stdout.
//!
//! Runs the compiled binary against a temporary markdown file and asserts
//! stdout content and exit code.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolve the path to the compiled debug binary.
fn binary() -> PathBuf {
    // `CARGO_BIN_EXE_markdown-reader` is set by Cargo when running integration
    // tests for binaries in the same workspace package. Fall back to a relative
    // path for manual invocation.
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_markdown-reader") {
        return p.into();
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("markdown-reader")
}

/// Fixture document shared across several tests.
const FIXTURE: &str = "\
# Title

## Foo
body1

## Bar
body2
";

/// Happy path: the binary extracts the first matching section and exits 0.
#[test]
fn extracts_first_matching_section_foo() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("doc.md");
    fs::write(&path, FIXTURE).unwrap();

    let output = Command::new(binary())
        .args(["--section", "Foo", path.to_str().unwrap()])
        .output()
        .expect("failed to run binary");

    assert!(
        output.status.success(),
        "exit should be 0 for a found section; got {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout is not valid UTF-8");
    // The extracted block should start with the heading and contain the body.
    assert!(
        stdout.starts_with("## Foo"),
        "output should start with '## Foo', got: {stdout:?}"
    );
    assert!(
        stdout.contains("body1"),
        "output should contain 'body1', got: {stdout:?}"
    );
    // Should not include the next same-level heading.
    assert!(
        !stdout.contains("## Bar"),
        "output must not include the next heading '## Bar', got: {stdout:?}"
    );
}

/// Happy path: a second, different section is also extractable from the same file.
#[test]
fn extracts_first_matching_section_bar() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("doc.md");
    fs::write(&path, FIXTURE).unwrap();

    let output = Command::new(binary())
        .args(["--section", "Bar", path.to_str().unwrap()])
        .output()
        .expect("failed to run binary");

    assert!(
        output.status.success(),
        "exit should be 0 for a found section; got {}",
        output.status,
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("## Bar"), "expected '## Bar' heading");
    assert!(stdout.contains("body2"), "expected 'body2' in output");
}

/// Case-insensitive substring match: lowercase query matches mixed-case heading.
#[test]
fn case_insensitive_match_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("ci.md");
    fs::write(&path, "# Features Overview\n\nsome content\n").unwrap();

    let output = Command::new(binary())
        .args(["--section", "features", path.to_str().unwrap()])
        .output()
        .expect("failed to run binary");

    assert!(
        output.status.success(),
        "case-insensitive match should succeed; exit={}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Features Overview"),
        "expected heading in output, got: {stdout:?}"
    );
}

/// Section terminated by a higher-level (lower H-number) heading.
#[test]
fn stops_at_higher_level_heading() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("doc.md");
    fs::write(&path, "# Top\n\n## Sub\nbody\n\n# Other\n").unwrap();

    let output = Command::new(binary())
        .args(["--section", "Sub", path.to_str().unwrap()])
        .output()
        .expect("failed to run binary");

    assert!(output.status.success(), "expected exit 0");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("body"), "expected body text");
    assert!(
        !stdout.contains("# Other"),
        "should not include the terminating heading"
    );
}

/// The last section in the document extends to end of file.
#[test]
fn last_section_extends_to_eof() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("doc.md");
    fs::write(&path, "# Intro\n\nbody\n\n# Appendix\n\nappendix body\n").unwrap();

    let output = Command::new(binary())
        .args(["--section", "Appendix", path.to_str().unwrap()])
        .output()
        .expect("failed to run binary");

    assert!(output.status.success(), "expected exit 0");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("appendix body"),
        "last section should include all remaining content"
    );
}

/// No matching heading → exit code 1, nothing useful on stdout.
#[test]
fn no_match_exits_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("doc.md");
    fs::write(&path, FIXTURE).unwrap();

    let output = Command::new(binary())
        .args(["--section", "DoesNotExist", path.to_str().unwrap()])
        .output()
        .expect("failed to run binary");

    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit code 1 for no-match; stdout={}",
        String::from_utf8_lossy(&output.stdout),
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.is_empty(),
        "stdout must be empty on no-match, got: {stdout:?}"
    );
}
