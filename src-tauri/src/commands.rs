use crate::{capture, output};

/// Captures every monitor (tone-mapped for HDR) and saves each to the
/// screenshots folder. Returns the saved file paths.
#[tauri::command]
pub fn capture_full_screen() -> Result<Vec<String>, String> {
    let monitors = capture::capture_all_monitors().map_err(|e| e.to_string())?;
    monitors
        .iter()
        .map(|m| output::save_capture(m).map(|p| p.to_string_lossy().into_owned()))
        .collect()
}
