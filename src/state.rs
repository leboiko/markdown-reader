use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const APP_NAME: &str = "markdown-reader";
const STATE_FILE: &str = "state.toml";

/// The last known session for a particular root directory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Session {
    /// Canonical absolute path of the last open file.
    #[serde(default)]
    pub file: PathBuf,
    /// Scroll offset in rendered lines at the time of last save.
    #[serde(default)]
    pub scroll: u32,
}

/// Persisted sessions keyed by canonical root path.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppState {
    #[serde(default)]
    pub sessions: HashMap<PathBuf, Session>,
}

impl AppState {
    /// Load state from disk, returning defaults on any failure.
    pub fn load() -> Self {
        let Some(path) = state_path() else {
            return Self::default();
        };
        let Ok(text) = fs::read_to_string(&path) else {
            return Self::default();
        };
        toml::from_str(&text).unwrap_or_default()
    }

    /// Persist state to disk. Silently swallows any I/O error.
    pub fn save(&self) {
        let Some(path) = state_path() else {
            return;
        };
        if let Some(parent) = path.parent()
            && fs::create_dir_all(parent).is_err()
        {
            return;
        }
        let Ok(text) = toml::to_string_pretty(self) else {
            return;
        };
        let _ = fs::write(&path, text);
    }

    /// Update or insert the session for `root`, then save.
    pub fn update_session(&mut self, root: &Path, file: PathBuf, scroll: u32) {
        self.sessions
            .insert(root.to_path_buf(), Session { file, scroll });
        self.save();
    }
}

/// Resolve the platform path for the state file.
///
/// Prefers `dirs::state_dir()` (XDG `$XDG_STATE_HOME` on Linux); falls back
/// to `dirs::data_dir()` on platforms (e.g., macOS) that have no dedicated
/// state directory.
fn state_path() -> Option<PathBuf> {
    let base = dirs::state_dir().or_else(dirs::data_dir)?;
    let mut path = base;
    path.push(APP_NAME);
    path.push(STATE_FILE);
    Some(path)
}
