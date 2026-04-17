use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::theme::Theme;

const APP_NAME: &str = "markdown-reader";
const CONFIG_FILE: &str = "config.toml";

/// Default value for [`Config::mermaid_max_height`].
///
/// 30 lines is a comfortable default — large enough to show a typical diagram
/// without consuming the entire viewport.
fn default_mermaid_max_height() -> u32 {
    30
}

/// Which side of the viewer the file-tree panel is rendered on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TreePosition {
    #[default]
    Left,
    Right,
}

/// Controls how mermaid diagrams are rendered in the viewer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MermaidMode {
    /// Try image rendering (when graphics are available), then figurehead
    /// Unicode box-drawing text, then raw source as a last resort.
    ///
    /// For diagram types with known image-render issues (e.g. `stateDiagram`),
    /// figurehead is tried first; the image pipeline is skipped for those types.
    #[default]
    Auto,
    /// Always use figurehead Unicode text rendering. Never spawns image tasks.
    ///
    /// CPU-lighter than `Auto`, works inside tmux and any terminal without
    /// graphics protocol support, but lower visual fidelity.
    Text,
    /// Only use the image pipeline when a graphics protocol is available.
    ///
    /// Falls back directly to raw source when graphics are not available —
    /// figurehead is not tried. Useful when you want images or nothing.
    Image,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub show_line_numbers: bool,
    #[serde(default)]
    pub tree_position: TreePosition,
    #[serde(default)]
    pub search_preview: SearchPreview,
    /// How mermaid diagrams are rendered. See [`MermaidMode`] for details.
    #[serde(default)]
    pub mermaid_mode: MermaidMode,
    /// Maximum height of a mermaid diagram block in display lines.
    ///
    /// Diagrams taller than this are clamped. Tune this if your most common
    /// diagrams are either clipped or consuming too much viewport space.
    /// The minimum is always 8 lines regardless of this setting.
    ///
    /// There is no UI widget for this field — edit `config.toml` directly.
    #[serde(default = "default_mermaid_max_height")]
    pub mermaid_max_height: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            show_line_numbers: false,
            tree_position: TreePosition::default(),
            search_preview: SearchPreview::default(),
            mermaid_mode: MermaidMode::default(),
            mermaid_max_height: default_mermaid_max_height(),
        }
    }
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

    /// Return a [`MermaidMode`] label suitable for display (e.g. in the UI).
    pub fn mermaid_mode_label(mode: MermaidMode) -> &'static str {
        match mode {
            MermaidMode::Auto => "Auto",
            MermaidMode::Text => "Text only",
            MermaidMode::Image => "Image only",
        }
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

    /// `mermaid_max_height` must survive a TOML round-trip with a custom value.
    #[test]
    fn mermaid_max_height_config_roundtrip() {
        let config = Config {
            mermaid_max_height: 25,
            ..Config::default()
        };
        let serialized = toml::to_string_pretty(&config).expect("serialization failed");
        let deserialized: Config = toml::from_str(&serialized).expect("deserialization failed");
        assert_eq!(deserialized.mermaid_max_height, 25);
    }

    /// A TOML file without `mermaid_max_height` must use the default (30).
    #[test]
    fn mermaid_max_height_missing_field_defaults_to_30() {
        let toml_str = r#"theme = "default""#;
        let config: Config = toml::from_str(toml_str).expect("deserialization failed");
        assert_eq!(config.mermaid_max_height, 30);
    }

    /// `MermaidMode` must round-trip through TOML.
    #[test]
    fn mermaid_mode_round_trips() {
        let config = Config {
            mermaid_mode: MermaidMode::Text,
            ..Config::default()
        };
        let serialized = toml::to_string_pretty(&config).expect("serialization failed");
        let deserialized: Config = toml::from_str(&serialized).expect("deserialization failed");
        assert_eq!(deserialized.mermaid_mode, MermaidMode::Text);
    }

    /// A TOML file without `mermaid_mode` must default to `Auto`.
    #[test]
    fn mermaid_mode_missing_field_defaults_to_auto() {
        let toml_str = r#"theme = "default""#;
        let config: Config = toml::from_str(toml_str).expect("deserialization failed");
        assert_eq!(config.mermaid_mode, MermaidMode::Auto);
    }
}
