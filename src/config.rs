use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::theme::Theme;

const APP_NAME: &str = "markdown-reader";
const CONFIG_FILE: &str = "config.toml";

/// Which side of the viewer the file-tree panel is rendered on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TreePosition {
    #[default]
    Left,
    Right,
}

/// How to render the inline preview for a content-search result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchPreview {
    /// Show the full matched line (trimmed).  More readable; may wrap on narrow
    /// terminals.
    #[default]
    FullLine,
    /// Show an ~80-character window centred on the first match occurrence.
    /// Compact, uniform row height.
    Snippet,
}

/// All persisted user settings.
///
/// `#[serde(default)]` on every field ensures that config files written by
/// older versions of the app (missing newer fields) still parse correctly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub show_line_numbers: bool,
    #[serde(default)]
    pub tree_position: TreePosition,
    #[serde(default)]
    pub search_preview: SearchPreview,
}

impl Config {
    /// Load settings from disk, returning defaults on any I/O or parse failure.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };
        let Ok(text) = fs::read_to_string(&path) else {
            return Self::default();
        };
        toml::from_str(&text).unwrap_or_default()
    }

    /// Persist settings to disk. Silently swallows any I/O error.
    pub fn save(&self) {
        let Some(path) = config_path() else {
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
}

fn config_path() -> Option<PathBuf> {
    let mut path = dirs::config_dir()?;
    path.push(APP_NAME);
    path.push(CONFIG_FILE);
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SearchPreview` must round-trip through TOML with the default value.
    #[test]
    fn search_preview_default_round_trips() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).expect("serialization failed");
        let deserialized: Config = toml::from_str(&serialized).expect("deserialization failed");
        assert_eq!(deserialized.search_preview, SearchPreview::FullLine);
    }

    /// A TOML file that omits `search_preview` must deserialize to `FullLine`.
    #[test]
    fn search_preview_missing_field_defaults_to_full_line() {
        let toml_str = r#"theme = "default""#;
        let config: Config = toml::from_str(toml_str).expect("deserialization failed");
        assert_eq!(config.search_preview, SearchPreview::default());
    }
}
