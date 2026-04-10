use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const APP_NAME: &str = "markdown-reader";
const STATE_FILE: &str = "state.toml";

/// Persisted state for a single open tab.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TabSession {
    /// Canonical absolute path of the open file.
    pub file: PathBuf,
    /// Scroll offset in rendered lines at the time of last save.
    #[serde(default)]
    pub scroll: u32,
}

/// Multi-tab session for a particular root directory.
///
/// Deserialized via [`SessionCompat`] for backwards-compatibility with the
/// v0.2.0 single-file format.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(from = "SessionCompat")]
pub struct Session {
    /// Open tabs in display order.
    pub tabs: Vec<TabSession>,
    /// 0-based index of the active tab among `tabs`.
    pub active: usize,
}

/// Untagged union that handles both the new multi-tab format and the legacy
/// single-file format written by v0.2.0.
///
/// Serde discriminates the two variants by looking for the `tabs` key: if
/// present the payload is `New`; otherwise it tries `Legacy` (which has a
/// `file` key). The variants are disjoint, so this is reliable.
#[derive(Deserialize)]
#[serde(untagged)]
enum SessionCompat {
    New { tabs: Vec<TabSession>, active: usize },
    Legacy { file: PathBuf, #[serde(default)] scroll: u32 },
}

impl From<SessionCompat> for Session {
    fn from(v: SessionCompat) -> Self {
        match v {
            SessionCompat::New { tabs, active } => Self { tabs, active },
            SessionCompat::Legacy { file, scroll } => Self {
                tabs: vec![TabSession { file, scroll }],
                active: 0,
            },
        }
    }
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

    /// Replace the session for `root` with a snapshot of `tabs` and `active_idx`.
    pub fn update_session(
        &mut self,
        root: &Path,
        tabs: Vec<TabSession>,
        active_idx: usize,
    ) {
        self.sessions.insert(
            root.to_path_buf(),
            Session { tabs, active: active_idx },
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_legacy_deserialize() {
        let toml = r#"file = "/some/file.md"
scroll = 42
"#;
        let session: Session = toml::from_str(toml).expect("deserialize");
        assert_eq!(session.tabs.len(), 1);
        assert_eq!(session.tabs[0].file, PathBuf::from("/some/file.md"));
        assert_eq!(session.tabs[0].scroll, 42);
        assert_eq!(session.active, 0);
    }

    #[test]
    fn session_new_roundtrip() {
        let original = Session {
            tabs: vec![
                TabSession { file: PathBuf::from("/a.md"), scroll: 0 },
                TabSession { file: PathBuf::from("/b.md"), scroll: 10 },
                TabSession { file: PathBuf::from("/c.md"), scroll: 5 },
            ],
            active: 1,
        };
        let serialized = toml::to_string_pretty(&original).expect("serialize");
        let deserialized: Session = toml::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized, original);
    }
}
