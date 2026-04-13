use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The git status of a single path relative to the working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitFileStatus {
    /// Newly staged file (`A `) or untracked file (`??`).
    New,
    /// Any tracked file with staged or unstaged modifications.
    Modified,
}

/// Run `git status --porcelain=v1 -unormal` rooted at `dir` and return a map
/// from absolute path to [`GitFileStatus`].
///
/// All ancestor directories of every changed path are also inserted with
/// [`GitFileStatus::Modified`] so that the file-tree panel can highlight entire
/// subtrees that contain changes.
///
/// Returns an empty map when `dir` is not a git repository or when git is not
/// installed — callers treat the absence of an entry as "clean".
pub fn collect(dir: &Path) -> HashMap<PathBuf, GitFileStatus> {
    let output = match Command::new("git")
        .args(["status", "--porcelain=v1", "-unormal"])
        .current_dir(dir)
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        // Not a git repo, git not installed, or any other error — degrade gracefully.
        _ => return HashMap::new(),
    };

    let text = match std::str::from_utf8(&output) {
        Ok(s) => s,
        Err(_) => return HashMap::new(),
    };

    let mut map: HashMap<PathBuf, GitFileStatus> = HashMap::new();

    for line in text.lines() {
        // Porcelain v1: two-char XY status code followed by a space and path.
        // Renamed entries use " -> " but only the destination path matters.
        if line.len() < 4 {
            continue;
        }
        let xy = &line[..2];
        let raw_path = if let Some(arrow) = line[3..].rfind(" -> ") {
            &line[3 + arrow + 4..]
        } else {
            &line[3..]
        };

        let status = xy_to_status(xy);
        let abs = dir.join(raw_path);
        insert_with_ancestors(&mut map, abs, status, dir);
    }

    map
}

/// Map a two-character porcelain XY code to a [`GitFileStatus`].
fn xy_to_status(xy: &str) -> GitFileStatus {
    match xy {
        "??" => GitFileStatus::New,
        "A " => GitFileStatus::New,
        _ => GitFileStatus::Modified,
    }
}

/// Insert `path` into `map` with `status`, then walk up to (but not including)
/// `root` and insert each ancestor directory as [`GitFileStatus::Modified`].
///
/// A directory already stored as `New` is upgraded to `Modified` only when the
/// caller provides `Modified`; `New` is never downgraded.
fn insert_with_ancestors(
    map: &mut HashMap<PathBuf, GitFileStatus>,
    path: PathBuf,
    status: GitFileStatus,
    root: &Path,
) {
    map.entry(path.clone())
        .and_modify(|existing| {
            if status == GitFileStatus::Modified {
                *existing = GitFileStatus::Modified;
            }
        })
        .or_insert(status);

    let mut current = path.as_path();
    while let Some(parent) = current.parent() {
        if parent == root || !parent.starts_with(root) {
            break;
        }
        map.entry(parent.to_path_buf())
            .and_modify(|existing| {
                if *existing != GitFileStatus::Modified {
                    *existing = GitFileStatus::Modified;
                }
            })
            .or_insert(GitFileStatus::Modified);
        current = parent;
    }
}
