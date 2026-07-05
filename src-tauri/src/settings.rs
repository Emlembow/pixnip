//! Persisted app settings: a small JSON file in the user's config dir.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(default)]
pub struct Settings {
    /// Launch with Windows, sitting in the tray.
    pub start_with_windows: bool,
    /// true: intercept the PrintScreen key (overriding Windows' Snipping
    /// Tool binding). false: bind Ctrl+Alt+S instead and leave PrintScreen
    /// alone.
    pub use_print_screen: bool,
    /// Whether the first-run questions have been asked.
    pub first_run_done: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start_with_windows: false,
            use_print_screen: true,
            first_run_done: false,
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("Pixnip").join("settings.json"))
}

pub fn load() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) {
    let Some(path) = settings_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(settings) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!("failed to save settings: {e}");
            }
        }
        Err(e) => eprintln!("failed to serialize settings: {e}"),
    }
}
