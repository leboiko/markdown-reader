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

/// Default value for [`UpdatesConfig::check_for_updates`].
///
/// Returns `true` so the feature is on by default for new installs.
/// Users who prefer no network activity can set `check_for_updates = false`
/// in the `[updates]` section of `config.toml`.
fn default_check_for_updates() -> bool {
    true
}

/// Settings that control the automatic update-notification feature.
///
/// Serialised as the `[updates]` TOML table inside `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatesConfig {
    /// When `true` (the default), the app checks crates.io once per 24 hours
    /// in a background thread.  If a newer version is found, a brief upgrade
    /// banner is printed to stderr when you quit.
    ///
    /// Set to `false` to disable the check entirely (no network activity).
    #[serde(default = "default_check_for_updates")]
    pub check_for_updates: bool,
}

impl Default for UpdatesConfig {
    fn default() -> Self {
        Self {
            check_for_updates: default_check_for_updates(),
        }
    }
}

/// Default value for [`Config::use_hybrid_by_default`].
///
/// Returns `true` so that lowercase `i` opens hybrid live-preview mode for
/// new installs.  Users who prefer the old fullscreen edtui behaviour can set
/// `use_hybrid_by_default = false` in `config.toml` to restore the pre-1.33.0
/// mapping while regressions are still being filed.
fn default_use_hybrid_by_default() -> bool {
    true
}

/// Default value for [`Config::show_file_tree`].
///
/// Returns `true` so existing users keep seeing the file tree at launch.
fn default_show_file_tree() -> bool {
    true
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
    Auto,
    /// Always use figurehead Unicode text rendering. Never spawns image tasks.
    ///
    /// The default mode. CPU-lighter than `Auto`, works inside tmux and any
    /// terminal without graphics protocol support.  Existing config files with
    /// `mermaid_mode = "auto"` keep that setting; only users with no explicit
    /// `mermaid_mode` in their TOML are affected by this default change.
    #[default]
    Text,
    /// Only use the image pipeline when a graphics protocol is available.
    ///
    /// Falls back directly to raw source when graphics are not available —
    /// figurehead is not tried. Useful when you want images or nothing.
    Image,
}

/// Which layered-layout backend to use when rendering text-mode flowchart and
/// state diagrams via `mermaid-text`.
///
/// `mermaid-text` ships two backends. `Sugiyama` (the historical default since
/// the library's 0.17.0 release) is `ascii-dag`-backed with proper crossing
/// minimisation and Brandes-Köpf coordinate assignment — it tends to produce
/// the cleanest layouts for flat dependency graphs. `Native` is the in-house
/// layered layout that has fuller coverage of subgraph-heavy diagrams,
/// parallel-edge groups, and nested direction overrides.
///
/// `Auto` is a conservative selector that picks `Native` only for the one
/// shape where Sugiyama is known to render less compactly — a `subgraph`
/// block with an inner `direction` override — and falls back to `Sugiyama`
/// for every other diagram.  It is opt-in for now; the plan is to promote it
/// to the default once it has a release cycle of real-world exercise.
///
/// This setting only affects text-mode flowchart and state diagrams; sequence,
/// pie, ER, mindmap and the various beta diagram types have their own
/// pipelines and are unaffected. Image-mode rendering (which goes through
/// `mermaid-rs-renderer`) is also unaffected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MermaidTextBackend {
    /// Conservative selector — `Native` for subgraphs with inner `direction`
    /// overrides, `Sugiyama` everywhere else.
    Auto,
    /// `ascii-dag`-backed Sugiyama layout. The historical default.
    #[default]
    Sugiyama,
    /// In-house layered layout with fuller subgraph and parallel-edge coverage.
    Native,
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
    /// Whether the file tree is visible when the app starts.
    ///
    /// Runtime toggles via `H` do not rewrite this startup default.
    #[serde(default = "default_show_file_tree")]
    pub show_file_tree: bool,
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
    /// Which layered-layout backend to use when rendering text-mode flowchart
    /// and state diagrams.  See [`MermaidTextBackend`] for the trade-offs.
    #[serde(default)]
    pub mermaid_text_backend: MermaidTextBackend,
    /// When `true` (the default), `i` opens hybrid live-preview mode and `I`
    /// opens the legacy fullscreen edtui.  Set to `false` to restore the
    /// pre-1.33.0 behaviour (`i` → fullscreen, `I` → hybrid) as an opt-out
    /// while regressions are being filed.
    #[serde(default = "default_use_hybrid_by_default")]
    pub use_hybrid_by_default: bool,
    /// Automatic update-notification settings.
    ///
    /// Serialised as `[updates]` in `config.toml`.
    #[serde(default)]
    pub updates: UpdatesConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            show_line_numbers: false,
            show_file_tree: default_show_file_tree(),
            tree_position: TreePosition::default(),
            search_preview: SearchPreview::default(),
            mermaid_mode: MermaidMode::default(),
            mermaid_max_height: default_mermaid_max_height(),
            mermaid_text_backend: MermaidTextBackend::default(),
            use_hybrid_by_default: default_use_hybrid_by_default(),
            updates: UpdatesConfig::default(),
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

    /// Return a [`MermaidTextBackend`] label suitable for display (e.g. in the UI).
    pub fn mermaid_text_backend_label(backend: MermaidTextBackend) -> &'static str {
        match backend {
            MermaidTextBackend::Auto => "Backend: Auto (subgraph-aware)",
            MermaidTextBackend::Sugiyama => "Backend: Sugiyama (default)",
            MermaidTextBackend::Native => "Backend: Native (subgraph-friendly)",
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

    /// A TOML file without `show_file_tree` must keep the historical visible tree.
    #[test]
    fn show_file_tree_missing_field_defaults_to_true() {
        let toml_str = r#"theme = "default""#;
        let config: Config = toml::from_str(toml_str).expect("deserialization failed");
        assert!(config.show_file_tree);
    }

    /// An explicit `show_file_tree = false` must be honoured.
    #[test]
    fn show_file_tree_explicit_false_is_honoured() {
        let toml_str = r#"
theme = "default"
show_file_tree = false
"#;
        let config: Config = toml::from_str(toml_str).expect("deserialization failed");
        assert!(!config.show_file_tree);
    }

    /// `show_file_tree = false` must survive a TOML round-trip.
    #[test]
    fn show_file_tree_false_round_trips() {
        let config = Config {
            show_file_tree: false,
            ..Config::default()
        };
        let serialized = toml::to_string_pretty(&config).expect("serialization failed");
        let deserialized: Config = toml::from_str(&serialized).expect("deserialization failed");
        assert!(!deserialized.show_file_tree);
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

    /// A non-default `MermaidTextBackend` must survive a TOML round-trip.
    /// Catches: a `Default` impl that masks the deserialised value, a
    /// serde-rename mismatch, or a missing `#[serde(default)]` attribute.
    #[test]
    fn mermaid_text_backend_round_trips() {
        let config = Config {
            mermaid_text_backend: MermaidTextBackend::Native,
            ..Config::default()
        };
        let serialized = toml::to_string_pretty(&config).expect("serialization failed");
        let deserialized: Config = toml::from_str(&serialized).expect("deserialization failed");
        assert_eq!(
            deserialized.mermaid_text_backend,
            MermaidTextBackend::Native
        );
    }

    /// A TOML file without `mermaid_text_backend` must default to `Sugiyama`.
    /// This is the user-visible promise that pre-1.34.48 config files keep
    /// rendering identically: Sugiyama has been the in-library default since
    /// `mermaid-text` 0.17.0.
    #[test]
    fn mermaid_text_backend_missing_field_defaults_to_sugiyama() {
        let toml_str = r#"theme = "default""#;
        let config: Config = toml::from_str(toml_str).expect("deserialization failed");
        assert_eq!(config.mermaid_text_backend, MermaidTextBackend::Sugiyama);
    }

    /// An explicit `mermaid_text_backend = "native"` in TOML must be honoured.
    /// Pairs with the round-trip test to guarantee the default doesn't mask a
    /// user-supplied value (a class of bug we have hit before).
    #[test]
    fn mermaid_text_backend_explicit_native_is_honoured() {
        let toml_str = r#"
theme = "default"
mermaid_text_backend = "native"
"#;
        let config: Config = toml::from_str(toml_str).expect("deserialization failed");
        assert_eq!(config.mermaid_text_backend, MermaidTextBackend::Native);
    }

    /// An explicit `mermaid_text_backend = "auto"` in TOML must deserialise to
    /// the `Auto` variant — pins the serde rename and guards against the
    /// new variant being silently dropped by a missing match arm.
    #[test]
    fn mermaid_text_backend_explicit_auto_is_honoured() {
        let toml_str = r#"
theme = "default"
mermaid_text_backend = "auto"
"#;
        let config: Config = toml::from_str(toml_str).expect("deserialization failed");
        assert_eq!(config.mermaid_text_backend, MermaidTextBackend::Auto);
    }

    /// `Auto` must survive a TOML round-trip — the persistence path is the
    /// usual culprit when a new variant breaks down silently.
    #[test]
    fn mermaid_text_backend_auto_round_trips() {
        let config = Config {
            mermaid_text_backend: MermaidTextBackend::Auto,
            ..Config::default()
        };
        let serialized = toml::to_string_pretty(&config).expect("serialization failed");
        let deserialized: Config = toml::from_str(&serialized).expect("deserialization failed");
        assert_eq!(deserialized.mermaid_text_backend, MermaidTextBackend::Auto);
    }

    /// Every variant must have a distinct, non-empty label so the popup
    /// renders a real choice for the user.  Catches a forgotten match arm
    /// in `mermaid_text_backend_label` (which would not be flagged by the
    /// compiler since the return type is `&'static str`).
    #[test]
    fn mermaid_text_backend_label_covers_all_variants() {
        let auto = Config::mermaid_text_backend_label(MermaidTextBackend::Auto);
        let sugiyama = Config::mermaid_text_backend_label(MermaidTextBackend::Sugiyama);
        let native = Config::mermaid_text_backend_label(MermaidTextBackend::Native);
        assert!(!auto.is_empty() && !sugiyama.is_empty() && !native.is_empty());
        assert_ne!(auto, sugiyama);
        assert_ne!(auto, native);
        assert_ne!(sugiyama, native);
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

    /// A TOML file without `mermaid_mode` must default to `Text`.
    #[test]
    fn mermaid_mode_missing_field_defaults_to_text() {
        let toml_str = r#"theme = "default""#;
        let config: Config = toml::from_str(toml_str).expect("deserialization failed");
        assert_eq!(config.mermaid_mode, MermaidMode::Text);
    }

    /// `use_hybrid_by_default` must survive a TOML round-trip with the value `false`.
    #[test]
    fn use_hybrid_by_default_roundtrip_false() {
        let config = Config {
            use_hybrid_by_default: false,
            ..Config::default()
        };
        let serialized = toml::to_string_pretty(&config).expect("serialization failed");
        let deserialized: Config = toml::from_str(&serialized).expect("deserialization failed");
        assert!(!deserialized.use_hybrid_by_default);
    }

    /// A TOML file without `use_hybrid_by_default` must default to `true`.
    #[test]
    fn use_hybrid_by_default_missing_field_defaults_to_true() {
        let toml_str = r#"theme = "default""#;
        let config: Config = toml::from_str(toml_str).expect("deserialization failed");
        assert!(config.use_hybrid_by_default);
    }

    /// `[updates]` section must round-trip with `check_for_updates = false`.
    #[test]
    fn updates_check_for_updates_roundtrip_false() {
        let config = Config {
            updates: UpdatesConfig {
                check_for_updates: false,
            },
            ..Config::default()
        };
        let serialized = toml::to_string_pretty(&config).expect("serialization failed");
        let deserialized: Config = toml::from_str(&serialized).expect("deserialization failed");
        assert!(!deserialized.updates.check_for_updates);
    }

    /// A TOML file without an `[updates]` section must default to `check_for_updates = true`.
    #[test]
    fn updates_missing_section_defaults_to_check_enabled() {
        let toml_str = r#"theme = "default""#;
        let config: Config = toml::from_str(toml_str).expect("deserialization failed");
        assert!(config.updates.check_for_updates);
    }

    /// An explicit `[updates] check_for_updates = false` must be honoured.
    #[test]
    fn updates_explicit_false_is_honoured() {
        let toml_str = r#"
theme = "default"

[updates]
check_for_updates = false
"#;
        let config: Config = toml::from_str(toml_str).expect("deserialization failed");
        assert!(!config.updates.check_for_updates);
    }
}
