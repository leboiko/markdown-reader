//! Link validation for markdown file trees.
//!
//! [`check_dir`] walks a directory recursively (respecting `.gitignore`),
//! parses every `.md` file with `pulldown-cmark`, extracts all links, and
//! validates them:
//!
//! - Same-file anchors (`#heading`) — checked against the file's headings.
//! - Cross-file links (`./other.md`) — checked that the target file exists.
//! - Cross-file anchors (`./other.md#section`) — file AND anchor checked.
//! - External (`http(s)://`) — HEAD request via `ureq` when
//!   `CheckOpts::check_external` is set. Up to 10 parallel workers, each
//!   honouring `CheckOpts::external_timeout_secs`. Non-http schemes
//!   (`mailto:`, `ftp://`, etc.) are silently ignored.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ignore::WalkBuilder;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::markdown::heading_to_anchor;

// ── Public surface ────────────────────────────────────────────────────────────

/// Options controlling link-validation behaviour.
#[derive(Debug, Clone)]
pub struct CheckOpts {
    /// When `true`, `http(s)://` links are validated via HEAD requests.
    pub check_external: bool,
    /// Timeout in seconds for each external HEAD request (default: 10).
    pub external_timeout_secs: u64,
}

impl Default for CheckOpts {
    fn default() -> Self {
        Self {
            check_external: false,
            external_timeout_secs: 10,
        }
    }
}

/// A single broken link found during validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenLink {
    /// 1-based source line where the link appears.
    pub line: u32,
    /// Human-readable description of why the link is broken.
    pub reason: String,
    /// The raw link target as it appears in the markdown source.
    pub raw_target: String,
}

/// Validation results for one `.md` file.
#[derive(Debug, Clone)]
pub struct FileReport {
    /// Path of the file that was validated (relative to the scan root).
    pub path: PathBuf,
    /// All broken links found in this file.
    pub broken: Vec<BrokenLink>,
}

/// Aggregated report returned by [`check_dir`].
#[derive(Debug)]
pub struct CheckReport {
    /// Per-file results — only files with at least one broken link are included.
    pub files: Vec<FileReport>,
    /// Total number of `.md` files that were scanned.
    pub files_scanned: usize,
    /// Total number of broken links across all files.
    pub broken_count: usize,
    /// Wall-clock time for the full scan.
    pub elapsed: std::time::Duration,
}

impl CheckReport {
    /// Returns `true` when no broken links were found.
    pub fn is_clean(&self) -> bool {
        self.broken_count == 0
    }

    /// Print a human-readable summary to stdout.
    pub fn print(&self, root: &Path) {
        println!("Checking links in {} ...\n", root.display());

        for file_report in &self.files {
            println!("{}:", file_report.path.display());
            for broken in &file_report.broken {
                println!(
                    "  line {}: {}  [{}]",
                    broken.line, broken.reason, broken.raw_target
                );
            }
            println!();
        }

        let secs = self.elapsed.as_secs_f64();
        if self.broken_count == 0 {
            println!(
                "All links OK. Scanned {} file(s) in {:.2}s.",
                self.files_scanned, secs
            );
        } else {
            let file_count = self.files.len();
            println!(
                "{} broken link(s) across {} file(s) ({} .md files scanned in {:.2}s).",
                self.broken_count, file_count, self.files_scanned, secs
            );
        }
    }
}

/// Walk `root` recursively, validate every `.md` file, and return an
/// aggregated [`CheckReport`].
///
/// # Arguments
///
/// * `root` - Directory to scan (recursively).
/// * `opts` - Validation options (e.g. whether to check external links).
///
/// # Panics
///
/// Does not panic; individual file-read errors are silently skipped.
pub fn check_dir(root: &Path, opts: &CheckOpts) -> CheckReport {
    let started = Instant::now();

    // ── Phase 1: collect all markdown files and parse their headings ──────────
    // We need the heading sets before validating cross-file anchor links, so we
    // do a first pass to build an index.
    let md_paths = collect_md_files(root);
    let files_scanned = md_paths.len();

    // anchor_index maps each absolute path → set of anchor slugs present in
    // the file. We populate this in a single pass so cross-file anchor checks
    // are O(1) lookups.
    let anchor_index: HashMap<PathBuf, HashSet<String>> = md_paths
        .iter()
        .map(|p| {
            let anchors = parse_anchors_from_file(p);
            (p.clone(), anchors)
        })
        .collect();

    // ── Phase 2: collect external links (if check_external is enabled) ────────
    // Gather all external URLs across all files, keyed by URL so each unique
    // URL is only HEAD-requested once even if it appears multiple times.
    //
    // Map: url → Vec<(abs_path, line)> so we can attribute results back.
    let mut external_map: HashMap<String, Vec<(PathBuf, u32)>> = HashMap::new();

    if opts.check_external {
        for abs_path in &md_paths {
            let content = match std::fs::read_to_string(abs_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            for raw in extract_links(&content) {
                if matches!(classify_url(&raw.url, abs_path.parent().unwrap_or(Path::new("."))), LinkKind::External) {
                    external_map
                        .entry(raw.url)
                        .or_default()
                        .push((abs_path.clone(), raw.line));
                }
            }
        }

        let ext_count = external_map.len();
        if ext_count > 0 {
            println!(
                "Checking {} external link(s)... (this may take a few seconds)",
                ext_count
            );
        }
    }

    // ── Phase 3: perform external HEAD requests ───────────────────────────────
    // Results: url → Option<String> (None = OK, Some(msg) = broken reason).
    let external_results: HashMap<String, Option<String>> = if opts.check_external && !external_map.is_empty() {
        check_external_links(external_map.keys().cloned().collect(), opts)
    } else {
        HashMap::new()
    };

    // ── Phase 4: validate links in every file ─────────────────────────────────
    let mut file_reports: Vec<FileReport> = Vec::new();
    let mut total_broken = 0usize;

    for abs_path in &md_paths {
        let content = match std::fs::read_to_string(abs_path) {
            Ok(c) => c,
            // Unreadable file — skip silently; this is uncommon (permissions,
            // binary files named .md, etc.) and not a link-validation concern.
            Err(_) => continue,
        };

        let mut broken = validate_links(abs_path, &content, &anchor_index);

        // Append external-link results for this file.
        if opts.check_external {
            for (url, occurrences) in &external_map {
                let broken_reason = external_results
                    .get(url)
                    .and_then(|r| r.as_ref());

                if let Some(reason) = broken_reason {
                    for (path, line) in occurrences {
                        if path == abs_path {
                            broken.push(BrokenLink {
                                line: *line,
                                reason: format!("{} [external]", reason),
                                raw_target: url.clone(),
                            });
                        }
                    }
                }
            }
        }

        // Sort by line number for stable output within a file.
        broken.sort_by_key(|b| b.line);

        if !broken.is_empty() {
            total_broken += broken.len();
            let rel_path = abs_path
                .strip_prefix(root)
                .unwrap_or(abs_path)
                .to_path_buf();
            file_reports.push(FileReport {
                path: rel_path,
                broken,
            });
        }
    }

    // Sort by file path for stable, predictable output.
    file_reports.sort_by(|a, b| a.path.cmp(&b.path));

    CheckReport {
        files: file_reports,
        files_scanned,
        broken_count: total_broken,
        elapsed: started.elapsed(),
    }
}

// ── External link checker ─────────────────────────────────────────────────────

/// The outcome of checking a single external URL.
#[derive(Debug)]
enum ExternalOutcome {
    /// The URL was reachable and returned a success status (2xx).
    Ok,
    /// The URL returned an error status or was unreachable.
    Broken(String),
}

/// Perform HEAD requests for all `urls`, honouring `opts.external_timeout_secs`
/// and capping concurrency at 10 parallel threads.
///
/// URLs are processed in waves of up to `MAX_WORKERS`. Within each wave every
/// request runs concurrently; the next wave does not start until all threads in
/// the current wave have finished. This keeps the concurrency cap simple and
/// avoids unbounded thread creation.
///
/// Returns a map from URL → `None` (OK) or `Some(reason)` (broken).
fn check_external_links(
    urls: Vec<String>,
    opts: &CheckOpts,
) -> HashMap<String, Option<String>> {
    const MAX_WORKERS: usize = 10;
    const MAX_REDIRECTS: u32 = 5;

    let timeout = Duration::from_secs(opts.external_timeout_secs);

    // Channel for results: worker sends (url, outcome) back to the collector.
    // We drop the sending end held by the main thread once all scoped workers
    // have sent their results (the scope guarantees this).
    let (tx, rx) = mpsc::channel::<(String, ExternalOutcome)>();

    // Process in waves bounded by MAX_WORKERS.  `std::thread::scope` ensures
    // all spawned threads have joined before the scope block exits, so there is
    // no risk of the sender outliving the receiver.
    std::thread::scope(|scope| {
        for chunk in urls.chunks(MAX_WORKERS) {
            // Spawn one thread per URL in this chunk.
            let handles: Vec<_> = chunk
                .iter()
                .map(|url| {
                    let tx = tx.clone();
                    let url = url.clone();
                    scope.spawn(move || {
                        let outcome = head_request(&url, timeout, MAX_REDIRECTS);
                        // Receiver outlives the scope, so send can only fail if the
                        // receiver was dropped prematurely — which cannot happen here.
                        let _ = tx.send((url, outcome));
                    })
                })
                .collect();

            // Wait for this wave to complete before spawning the next.
            for handle in handles {
                // ScopedJoinHandle::join() only fails if the thread panicked;
                // head_request never panics, so we can safely ignore the result.
                let _ = handle.join();
            }
        }
    });

    // Drop the main thread's sender so the receiver iterator terminates.
    drop(tx);

    let mut results = HashMap::new();
    for (url, outcome) in rx {
        let entry = match outcome {
            ExternalOutcome::Ok => None,
            ExternalOutcome::Broken(reason) => Some(reason),
        };
        results.insert(url, entry);
    }
    results
}

/// Perform a single HEAD request, following up to `max_redirects` 3xx
/// responses. Returns [`ExternalOutcome`] indicating success or the failure
/// reason.
///
/// Uses ureq 3.x `Config::builder()` API: non-2xx responses surface as
/// `Error::StatusCode(u16)`, timeouts as `Error::Timeout`, DNS failures as
/// `Error::HostNotFound`, and IO problems as `Error::Io`.
fn head_request(url: &str, timeout: Duration, max_redirects: u32) -> ExternalOutcome {
    use ureq::config::Config;

    let agent: ureq::Agent = Config::builder()
        .timeout_global(Some(timeout))
        .max_redirects(max_redirects)
        .build()
        .into();

    match agent.head(url).call() {
        Ok(_response) => {
            // ureq 3.x only reaches Ok(_) for 2xx (and followed redirects
            // resolving to 2xx) when `http_status_as_error` is true (default).
            ExternalOutcome::Ok
        }
        Err(ureq::Error::StatusCode(code)) => {
            // ureq 3.x surfaces non-2xx as StatusCode errors when redirect
            // following is exhausted or when the server returns 4xx/5xx.
            let reason = http_status_reason(code);
            ExternalOutcome::Broken(reason)
        }
        Err(ureq::Error::Timeout(_)) => {
            ExternalOutcome::Broken("connection timeout".to_string())
        }
        Err(ureq::Error::HostNotFound) => {
            ExternalOutcome::Broken("host not found (DNS failure)".to_string())
        }
        Err(ureq::Error::Io(e)) => {
            ExternalOutcome::Broken(format!("connection error: {}", e))
        }
        Err(e) => ExternalOutcome::Broken(format!("request error: {}", e)),
    }
}

/// Produce a human-readable string for an HTTP status code.
fn http_status_reason(code: u16) -> String {
    let label = match code {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        410 => "Gone",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "HTTP error",
    };
    format!("{} {}", code, label)
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Collect all `.md` files under `root` using the `ignore` crate's walker,
/// which honours `.gitignore` rules and skips hidden directories.
fn collect_md_files(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for entry in WalkBuilder::new(root).build().flatten() {
        let path = entry.into_path();
        if path.is_file() && path.extension().is_some_and(|e| e == "md") {
            paths.push(path);
        }
    }
    // Sort for deterministic iteration order (WalkBuilder may yield in any order).
    paths.sort();
    paths
}

/// Parse all headings from `path` and return their anchor slugs.
///
/// Uses `pulldown-cmark` — headings inside code fences are correctly excluded.
fn parse_anchors_from_file(path: &Path) -> HashSet<String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return HashSet::new(),
    };
    parse_anchors(&content)
}

/// Extract anchor slugs from all headings in a markdown string.
///
/// Duplicate heading texts produce duplicate slugs; callers that need
/// disambiguation must handle it themselves (GitHub disambiguates with `-1`,
/// `-2`, etc., but for link validation we accept any occurrence as valid).
fn parse_anchors(content: &str) -> HashSet<String> {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH;

    let parser = Parser::new_ext(content, opts);
    let mut anchors = HashSet::new();
    let mut in_heading = false;
    let mut heading_text = String::new();

    for event in parser {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                in_heading = true;
                heading_text.clear();
            }
            // Collapse the guard into the match arm to satisfy clippy::collapsible_match.
            Event::End(TagEnd::Heading(_)) if in_heading => {
                anchors.insert(heading_to_anchor(&heading_text));
                in_heading = false;
                heading_text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {}
            Event::Text(text) | Event::Code(text) if in_heading => {
                // pulldown-cmark yields inline code spans as `Event::Code`
                // even inside headings. GitHub includes inline code text in
                // the slug, so we do the same.
                heading_text.push_str(&text);
            }
            _ => {}
        }
    }
    anchors
}

/// The category of a link target, used to decide how to validate it.
#[derive(Debug)]
enum LinkKind {
    /// `#fragment` — same-file anchor check.
    SameFileAnchor(String),
    /// `./other.md` or `path/other.md` — cross-file, no anchor.
    CrossFile(PathBuf),
    /// `./other.md#fragment` — cross-file with anchor.
    CrossFileAnchor(PathBuf, String),
    /// `http(s)://...` — external; only checked when `opts.check_external`.
    External,
    /// `mailto:`, `ftp://`, etc. — silently ignored.
    Ignored,
}

/// Classify a raw URL string into a [`LinkKind`].
fn classify_url(url: &str, file_dir: &Path) -> LinkKind {
    if let Some(fragment) = url.strip_prefix('#') {
        // Same-file anchor.
        return LinkKind::SameFileAnchor(fragment.to_string());
    }

    if url.starts_with("http://") || url.starts_with("https://") {
        return LinkKind::External;
    }

    // Any other scheme (mailto:, ftp:, tel:, etc.) — skip silently.
    if url.contains("://") || url.starts_with("mailto:") {
        return LinkKind::Ignored;
    }

    // Relative path — may contain a `#fragment`.
    // Split on the first `#` to separate path from fragment.
    let (path_part, fragment) = match url.find('#') {
        Some(idx) => (&url[..idx], Some(&url[idx + 1..])),
        None => (url, None),
    };

    // Resolve relative path against the directory that contains this file.
    let target = file_dir.join(path_part);

    match fragment {
        Some(frag) => LinkKind::CrossFileAnchor(target, frag.to_string()),
        None => LinkKind::CrossFile(target),
    }
}

/// A raw link extracted from a markdown source, with its approximate line.
struct RawLink {
    /// The URL/target exactly as written in the markdown source.
    url: String,
    /// 1-based line number derived from pulldown-cmark's byte-offset span.
    line: u32,
}

/// Extract all links from a markdown string, together with their source lines.
///
/// pulldown-cmark resolves reference-style links (`[text][label]` +
/// `[label]: url`) automatically, so no special handling is required here.
fn extract_links(content: &str) -> Vec<RawLink> {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH;

    // Build a byte-offset → line-number map so we can convert the span start
    // offsets that pulldown-cmark gives us into 1-based line numbers.
    let line_starts = build_line_starts(content);

    // `into_offset_iter()` wraps each event with its byte range so we can
    // derive source line numbers without a separate byte-scan pass.
    let parser = Parser::new_ext(content, opts).into_offset_iter();

    let mut links = Vec::new();
    let mut current_link: Option<(String, u32)> = None;

    for (event, range) in parser {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                let line = byte_offset_to_line(range.start, &line_starts);
                current_link = Some((dest_url.into_string(), line));
            }
            Event::End(TagEnd::Link) => {
                if let Some((url, line)) = current_link.take() {
                    links.push(RawLink { url, line });
                }
            }
            _ => {}
        }
    }
    links
}

/// Build a sorted list of byte offsets where each line starts.
///
/// `line_starts[i]` is the byte offset of the first character on line `i`
/// (0-indexed). The vector always starts with `0`.
fn build_line_starts(content: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, ch) in content.char_indices() {
        if ch == '\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// Convert a byte offset to a 1-based line number using the pre-built line
/// start table.
fn byte_offset_to_line(offset: usize, line_starts: &[usize]) -> u32 {
    // Binary search for the largest start ≤ offset; the index is the 0-based
    // line number. Add 1 for 1-based output.
    let idx = line_starts.partition_point(|&s| s <= offset);
    // `partition_point` returns the index *after* the last match, so subtract
    // 1 to get the line that contains `offset`.
    idx.saturating_sub(1) as u32 + 1
}

/// Validate all links in `content` (from file `abs_path`) and return any
/// broken ones (internal only — external links are handled separately).
///
/// # Arguments
///
/// * `abs_path`     - Absolute path of the file being validated.
/// * `content`      - Raw markdown source of that file.
/// * `anchor_index` - Pre-built map of `absolute_path → anchor slug set`.
fn validate_links(
    abs_path: &Path,
    content: &str,
    anchor_index: &HashMap<PathBuf, HashSet<String>>,
) -> Vec<BrokenLink> {
    let file_dir = abs_path.parent().unwrap_or(Path::new("."));
    let self_anchors = parse_anchors(content);
    let raw_links = extract_links(content);
    let mut broken = Vec::new();

    for raw in raw_links {
        match classify_url(&raw.url, file_dir) {
            LinkKind::SameFileAnchor(anchor) => {
                if !self_anchors.contains(&anchor) {
                    broken.push(BrokenLink {
                        line: raw.line,
                        // Show the raw target in the reason so it's grep-friendly.
                        reason: format!("broken anchor {}", &raw.url),
                        raw_target: raw.url,
                    });
                }
            }

            LinkKind::CrossFile(target) => {
                // Canonicalise the path so that `..` components are resolved
                // without requiring the file to exist (which `canonicalize`
                // would need). We use `normalize_path` below.
                let resolved = normalize_path(&target);
                if !resolved.exists() {
                    broken.push(BrokenLink {
                        line: raw.line,
                        reason: format!("missing file {}", &raw.url),
                        raw_target: raw.url,
                    });
                }
            }

            LinkKind::CrossFileAnchor(target, anchor) => {
                let resolved = normalize_path(&target);
                if !resolved.exists() {
                    broken.push(BrokenLink {
                        line: raw.line,
                        reason: format!("missing file {}", &raw.url),
                        raw_target: raw.url,
                    });
                } else {
                    // File exists — check the anchor within it.
                    let anchors = anchor_index.get(&resolved).cloned().unwrap_or_else(|| {
                        // The file exists but wasn't in the index (e.g. not a
                        // tracked .md file or added after the index was built).
                        // Parse it on demand.
                        parse_anchors_from_file(&resolved)
                    });
                    if !anchors.contains(&anchor) {
                        broken.push(BrokenLink {
                            line: raw.line,
                            reason: format!("broken cross-file anchor {}", &raw.url),
                            raw_target: raw.url,
                        });
                    }
                }
            }

            // External links are handled in the external-check phase.
            // Non-http schemes (mailto:, ftp:, etc.) are silently ignored.
            LinkKind::External | LinkKind::Ignored => {}
        }
    }

    broken
}

/// Resolve `..` and `.` components in a path without requiring it to exist
/// on disk (unlike `std::fs::canonicalize`).
///
/// This is a best-effort normalisation: it handles the common `./foo/../bar`
/// patterns that appear in markdown cross-file links.
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                // Pop the last real component if possible.
                if !out.pop() {
                    out.push(component);
                }
            }
            std::path::Component::CurDir => {
                // Skip `.` components.
            }
            other => out.push(other),
        }
    }
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::net::TcpListener;
    use std::io::{Read, Write};
    use tempfile::TempDir;

    /// Helper: write files into a temp directory and return (TempDir, PathBuf
    /// of the temp dir root). `TempDir` must be kept alive for the test.
    fn make_temp_dir(files: &[(&str, &str)]) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let root = dir.path().to_path_buf();
        for (name, content) in files {
            let path = root.join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("failed to create subdir");
            }
            fs::write(&path, content).expect("failed to write test file");
        }
        (dir, root)
    }

    // ── Tiny HTTP mock server ─────────────────────────────────────────────────
    //
    // Spins up a TcpListener on a random port and services exactly one HEAD
    // request with a fixed status line. The server thread exits after serving
    // that one request, so tests must not reuse the same mock across multiple
    // calls to `head_request`.

    /// Bind a TcpListener on a random OS-assigned port and return it with its
    /// local address string `"http://127.0.0.1:<port>"`.
    fn bind_mock_server() -> (TcpListener, String) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local_addr");
        let base_url = format!("http://127.0.0.1:{}", addr.port());
        (listener, base_url)
    }

    /// Spawn a thread that accepts exactly one connection and responds with the
    /// given HTTP status line (e.g. `"HTTP/1.1 200 OK"`).
    fn serve_once(listener: TcpListener, status_line: &'static str) {
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // Drain the incoming request so the client doesn't get a reset.
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    status_line
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
    }

    /// Spawn a thread that accepts one connection and responds with a 301
    /// redirect to `location`, then closes.
    fn serve_redirect(listener: TcpListener, location: String) {
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 301 Moved Permanently\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    location
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
    }

    // ── parse_anchors ─────────────────────────────────────────────────────────

    #[test]
    fn parse_anchors_extracts_heading_slugs() {
        let content = "# Hello World\n\n## API v2.0\n\nsome text\n";
        let anchors = parse_anchors(content);
        assert!(anchors.contains("hello-world"), "expected 'hello-world'");
        assert!(anchors.contains("api-v20"), "expected 'api-v20'");
    }

    // ── validate_links / unit-level ───────────────────────────────────────────

    #[test]
    fn valid_internal_anchor_passes() {
        let (_dir, root) = make_temp_dir(&[("doc.md", "# Title\n\n[link](#title)\n")]);
        let report = check_dir(&root, &CheckOpts::default());
        assert_eq!(report.broken_count, 0, "expected no broken links");
    }

    #[test]
    fn broken_internal_anchor_reported() {
        let (_dir, root) = make_temp_dir(&[("doc.md", "# Title\n\n[link](#nonexistent)\n")]);
        let report = check_dir(&root, &CheckOpts::default());
        assert_eq!(report.broken_count, 1, "expected exactly one broken link");
        assert_eq!(report.files[0].broken[0].raw_target, "#nonexistent");
    }

    #[test]
    fn valid_cross_file_link_passes() {
        let (_dir, root) = make_temp_dir(&[("a.md", "[link](./b.md)\n"), ("b.md", "# B file\n")]);
        let report = check_dir(&root, &CheckOpts::default());
        assert_eq!(report.broken_count, 0, "expected no broken links");
    }

    #[test]
    fn missing_file_reported() {
        let (_dir, root) = make_temp_dir(&[("a.md", "[link](./nonexistent.md)\n")]);
        let report = check_dir(&root, &CheckOpts::default());
        assert_eq!(report.broken_count, 1, "expected exactly one broken link");
        assert_eq!(report.files[0].broken[0].raw_target, "./nonexistent.md");
    }

    #[test]
    fn cross_file_with_valid_anchor_passes() {
        let (_dir, root) = make_temp_dir(&[
            ("a.md", "[link](./b.md#real-section)\n"),
            ("b.md", "# Real Section\n\nsome content.\n"),
        ]);
        let report = check_dir(&root, &CheckOpts::default());
        assert_eq!(report.broken_count, 0, "expected no broken links");
    }

    #[test]
    fn cross_file_with_bad_anchor_reported() {
        let (_dir, root) = make_temp_dir(&[
            ("a.md", "[link](./b.md#fake)\n"),
            ("b.md", "# Real Section\n\nsome content.\n"),
        ]);
        let report = check_dir(&root, &CheckOpts::default());
        assert_eq!(report.broken_count, 1, "expected exactly one broken link");
        assert!(
            report.files[0].broken[0].raw_target.contains("#fake"),
            "raw_target should contain #fake"
        );
    }

    #[test]
    fn external_link_skipped_silently_when_check_external_off() {
        let (_dir, root) = make_temp_dir(&[("doc.md", "[link](https://example.com)\n")]);
        let report = check_dir(
            &root,
            &CheckOpts {
                check_external: false,
                ..CheckOpts::default()
            },
        );
        assert_eq!(report.broken_count, 0, "external links must be skipped");
    }

    // ── External link checks ──────────────────────────────────────────────────

    #[test]
    fn external_link_with_2xx_passes() {
        let (listener, base_url) = bind_mock_server();
        serve_once(listener, "HTTP/1.1 200 OK");

        let outcome = head_request(&base_url, Duration::from_secs(5), 5);
        assert!(
            matches!(outcome, ExternalOutcome::Ok),
            "200 OK should pass"
        );
    }

    #[test]
    fn external_link_with_4xx_reported() {
        let (listener, base_url) = bind_mock_server();
        serve_once(listener, "HTTP/1.1 404 Not Found");

        let outcome = head_request(&base_url, Duration::from_secs(5), 5);
        match outcome {
            ExternalOutcome::Broken(reason) => {
                assert!(
                    reason.contains("404"),
                    "expected 404 in reason, got: {reason}"
                );
            }
            ExternalOutcome::Ok => panic!("404 response should be reported as broken"),
        }
    }

    #[test]
    fn external_link_with_5xx_reported() {
        let (listener, base_url) = bind_mock_server();
        serve_once(listener, "HTTP/1.1 500 Internal Server Error");

        let outcome = head_request(&base_url, Duration::from_secs(5), 5);
        match outcome {
            ExternalOutcome::Broken(reason) => {
                assert!(
                    reason.contains("500"),
                    "expected 500 in reason, got: {reason}"
                );
            }
            ExternalOutcome::Ok => panic!("500 response should be reported as broken"),
        }
    }

    #[test]
    fn external_link_redirect_followed() {
        // Two listeners: src returns 301 → dest returns 200.
        let (listener_dest, dest_url) = bind_mock_server();
        serve_once(listener_dest, "HTTP/1.1 200 OK");

        let (listener_src, src_url) = bind_mock_server();
        serve_redirect(listener_src, dest_url);

        let outcome = head_request(&src_url, Duration::from_secs(5), 5);
        assert!(
            matches!(outcome, ExternalOutcome::Ok),
            "redirect chain ending in 200 should pass"
        );
    }

    #[test]
    fn external_link_dns_failure_reported() {
        // Use a hostname that definitely won't resolve (RFC 2606 .invalid TLD).
        let url = "http://this-will-never-resolve.invalid/path";
        let outcome = head_request(url, Duration::from_secs(5), 5);
        assert!(
            matches!(outcome, ExternalOutcome::Broken(_)),
            "DNS failure should be reported as broken"
        );
    }

    #[test]
    fn external_link_connection_error_reported() {
        // Bind a listener, note its port, then drop it — so nothing is
        // listening when head_request tries to connect.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);

        let url = format!("http://127.0.0.1:{}/", port);
        let outcome = head_request(&url, Duration::from_secs(5), 5);
        assert!(
            matches!(outcome, ExternalOutcome::Broken(_)),
            "connection refused should be reported as broken"
        );
    }

    #[test]
    fn check_dir_reports_external_broken_link() {
        let (listener, base_url) = bind_mock_server();
        serve_once(listener, "HTTP/1.1 404 Not Found");

        let content = format!("[broken external]({})\n", base_url);
        let (_dir, root) = make_temp_dir(&[("doc.md", &content)]);

        let report = check_dir(
            &root,
            &CheckOpts {
                check_external: true,
                external_timeout_secs: 5,
            },
        );

        assert_eq!(report.broken_count, 1, "expected exactly one broken link");
        let broken = &report.files[0].broken[0];
        assert!(
            broken.reason.contains("404"),
            "reason should mention 404: {}",
            broken.reason
        );
        assert!(
            broken.reason.contains("[external]"),
            "reason should be tagged [external]: {}",
            broken.reason
        );
    }

    #[test]
    fn check_dir_passes_external_2xx_link() {
        let (listener, base_url) = bind_mock_server();
        serve_once(listener, "HTTP/1.1 200 OK");

        let content = format!("[valid external]({})\n", base_url);
        let (_dir, root) = make_temp_dir(&[("doc.md", &content)]);

        let report = check_dir(
            &root,
            &CheckOpts {
                check_external: true,
                external_timeout_secs: 5,
            },
        );

        assert_eq!(report.broken_count, 0, "200 external link should pass");
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    #[test]
    fn normalize_path_resolves_parent_components() {
        let p = PathBuf::from("/tmp/docs/../other.md");
        assert_eq!(normalize_path(&p), PathBuf::from("/tmp/other.md"));
    }

    #[test]
    fn byte_offset_to_line_maps_correctly() {
        // "abc\ndef\n" — line 1 starts at 0, line 2 starts at 4.
        let content = "abc\ndef\n";
        let starts = build_line_starts(content);
        assert_eq!(byte_offset_to_line(0, &starts), 1);
        assert_eq!(byte_offset_to_line(3, &starts), 1); // the '\n' itself
        assert_eq!(byte_offset_to_line(4, &starts), 2); // 'd'
        assert_eq!(byte_offset_to_line(7, &starts), 2);
    }
}
