use ignore::{WalkBuilder, WalkState};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A node in the discovered markdown file tree.
///
/// Directories are included only when they (transitively) contain at least one
/// markdown file. Non-markdown files are excluded entirely.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Absolute path to the file or directory.
    pub path: PathBuf,
    /// Display name (file-name component only).
    pub name: String,
    /// `true` if this entry is a directory.
    pub is_dir: bool,
    /// Direct children (populated for directories, empty for files).
    pub children: Vec<FileEntry>,
}

impl FileEntry {
    /// Walk `root` and return a sorted tree of markdown-related entries.
    ///
    /// The tree is sorted with directories before files, then alphabetically
    /// within each group. Hidden directories and paths matched by `.gitignore`
    /// are excluded by the `ignore` crate.
    ///
    /// # Arguments
    ///
    /// * `root` - The directory to start from.
    pub fn discover(root: &Path) -> Vec<FileEntry> {
        let markdown_paths = walk_markdown_files(root);
        build_tree(root, markdown_paths)
    }

    /// Collect all non-directory paths from the tree into a flat `Vec`.
    ///
    /// This is used by the content-search path to iterate over every file.
    pub fn flat_paths(entries: &[FileEntry]) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        collect_flat_paths(entries, &mut paths);
        paths
    }
}

/// Recursively append file paths (non-directories) from `entries` into `out`.
fn collect_flat_paths(entries: &[FileEntry], out: &mut Vec<PathBuf>) {
    for entry in entries {
        if !entry.is_dir {
            out.push(entry.path.clone());
        }
        collect_flat_paths(&entry.children, out);
    }
}

/// Run a single parallel walk of `root` and return every markdown file found.
///
/// Using `build_parallel()` amortises the gitignore/hidden-file filtering
/// across worker threads. In contrast, a recursive per-directory walker with
/// `max_depth(1)` re-reads and re-compiles the ignore matchers at every level,
/// which is pathologically slow on large monorepos.
fn walk_markdown_files(root: &Path) -> Vec<PathBuf> {
    let paths: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    WalkBuilder::new(root)
        .hidden(false)
        .build_parallel()
        .run(|| {
            let paths = &paths;
            Box::new(move |result| {
                let Ok(entry) = result else {
                    return WalkState::Continue;
                };
                if entry.file_type().is_some_and(|ft| ft.is_file())
                    && entry
                        .path()
                        .extension()
                        .is_some_and(|ext| ext == "md" || ext == "markdown")
                {
                    paths.lock().unwrap().push(entry.path().to_path_buf());
                }
                WalkState::Continue
            })
        });
    paths.into_inner().unwrap_or_default()
}

/// Fold a flat list of markdown paths into a sorted `FileEntry` tree rooted at
/// `root`. Only directories that transitively contain a markdown file appear in
/// the output — because every leaf inserted here is a markdown file, this falls
/// out for free.
fn build_tree(root: &Path, paths: Vec<PathBuf>) -> Vec<FileEntry> {
    let mut root_entry = FileEntry {
        path: root.to_path_buf(),
        name: String::new(),
        is_dir: true,
        children: Vec::new(),
    };

    for path in paths {
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        insert_path(&mut root_entry, root, rel);
    }

    sort_entries(&mut root_entry.children);
    root_entry.children
}

/// Insert `rel` (a path relative to `root`) into `parent`, creating directory
/// nodes along the way.
fn insert_path(parent: &mut FileEntry, root: &Path, rel: &Path) {
    let mut current = parent;
    let mut abs = root.to_path_buf();
    let components: Vec<_> = rel.components().collect();
    let last = components.len().saturating_sub(1);

    for (idx, comp) in components.iter().enumerate() {
        let name = comp.as_os_str().to_string_lossy().to_string();
        abs.push(&name);
        let is_dir = idx < last;

        let existing = current
            .children
            .iter()
            .position(|child| child.name == name);

        let child_idx = match existing {
            Some(i) => i,
            None => {
                current.children.push(FileEntry {
                    path: abs.clone(),
                    name,
                    is_dir,
                    children: Vec::new(),
                });
                current.children.len() - 1
            }
        };

        current = &mut current.children[child_idx];
    }
}

/// Sort in-place: directories first, then files, alphabetical within each group.
/// Recurses into every directory.
fn sort_entries(entries: &mut Vec<FileEntry>) {
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
    for entry in entries {
        if entry.is_dir {
            sort_entries(&mut entry.children);
        }
    }
}
