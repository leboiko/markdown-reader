use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::theme::Theme;

const APP_NAME: &str = "markdown-reader";
const CONFIG_FILE: &str = "config.toml";

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
