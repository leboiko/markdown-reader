use std::path::{Path, PathBuf};

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
        let mut entries = Vec::new();
        collect_entries(root, &mut entries);
        entries.sort_by(|a, b| {
            // Directories first, then alphabetical within each group.
            b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name))
        });
        entries
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

/// Recursively collect one level of entries under `dir`, then recurse for
/// subdirectories. Directories without any (nested) markdown files are pruned.
fn collect_entries(dir: &Path, entries: &mut Vec<FileEntry>) {
    let walker = ignore::WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(false)
        .sort_by_file_name(|a, b| a.cmp(b))
        .build();

    for result in walker {
        let Ok(entry) = result else { continue };
        let path = entry.path().to_path_buf();

        // Skip the directory itself (the walk always yields it as the first entry).
        if path == dir {
            continue;
        }

        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        if entry.file_type().is_some_and(|ft| ft.is_dir()) {
            let mut children = Vec::new();
            collect_entries(&path, &mut children);
            children.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));

            // Only include directories that contain markdown files (directly or nested).
            if has_markdown_files(&children) {
                entries.push(FileEntry {
                    path,
                    name,
                    is_dir: true,
                    children,
                });
            }
        } else if path
            .extension()
            .is_some_and(|ext| ext == "md" || ext == "markdown")
        {
            entries.push(FileEntry {
                path,
                name,
                is_dir: false,
                children: Vec::new(),
            });
        }
    }
}

/// Return `true` if `entries` contains at least one non-directory file
/// (searched recursively through nested directories).
fn has_markdown_files(entries: &[FileEntry]) -> bool {
    entries.iter().any(|e| {
        if e.is_dir {
            has_markdown_files(&e.children)
        } else {
            true
        }
    })
}
