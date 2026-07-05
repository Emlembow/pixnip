use std::path::PathBuf;

use crate::capture::CapturedMonitor;

/// Default save location: `%LOCALAPPDATA%\Pixnip\Screenshots`. Deliberately
/// not `Pictures\Screenshots` — the Pictures library falls under Windows'
/// Controlled Folder Access, which silently blocks unsigned apps from
/// writing there (and asking users to punch holes in ransomware protection
/// is not acceptable). Local app data is never CFA-protected.
pub fn screenshots_dir() -> Result<PathBuf, String> {
    let base = dirs::data_local_dir().ok_or("could not resolve local app data")?;
    Ok(base.join("Pixnip").join("Screenshots"))
}

/// The folder screenshots land in. Used by "Open Screenshots Folder".
pub fn effective_screenshots_dir() -> Result<PathBuf, String> {
    screenshots_dir()
}

/// Encodes a captured monitor as PNG and saves it into the screenshots
/// folder, creating it if needed. Returns the written file path.
pub fn save_capture(capture: &CapturedMonitor) -> Result<PathBuf, String> {
    let filename = format!("{}.png", timestamped_filename());
    try_save(capture, &screenshots_dir()?, &filename)
}

/// Saves already-encoded PNG bytes (from the editor's export canvas).
pub fn save_png_bytes(png_bytes: &[u8]) -> Result<PathBuf, String> {
    let dir = screenshots_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.png", timestamped_filename()));
    std::fs::write(&path, png_bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

fn try_save(
    capture: &CapturedMonitor,
    dir: &std::path::Path,
    filename: &str,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(filename);
    image::save_buffer(
        &path,
        &capture.rgba,
        capture.width,
        capture.height,
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())?;
    Ok(path)
}

/// Writes PNG bytes to a temp staging folder for a native drag-out. The
/// file keeps the normal screenshot naming, since whatever app receives
/// the drop keeps the dragged file's name.
pub fn stage_drag_file(png_bytes: &[u8]) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join("Pixnip").join("drag");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Best-effort cleanup of earlier drags; the current session's files
    // must survive until the receiving app has copied them.
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                if modified
                    .elapsed()
                    .map(|d| d.as_secs() > 3600)
                    .unwrap_or(false)
                {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
    let path = dir.join(format!("{}.png", timestamped_filename()));
    std::fs::write(&path, png_bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

fn timestamped_filename() -> String {
    use windows::Win32::System::SystemInformation::GetLocalTime;
    let now = unsafe { GetLocalTime() };
    format!(
        "Screenshot {:04}-{:02}-{:02} {:02}{:02}{:02}",
        now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute, now.wSecond
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises the real capture -> save pipeline against actual displays.
    /// Requires an interactive desktop session; run with:
    /// cargo test -- --ignored --nocapture save_capture_writes_a_real_screenshot
    #[test]
    #[ignore]
    fn save_capture_writes_a_real_screenshot() {
        let monitors = crate::capture::capture_all_monitors().expect("capture failed");
        assert!(!monitors.is_empty(), "expected at least one monitor");
        for m in &monitors {
            let path = save_capture(m).expect("save failed");
            println!("saved {}x{} -> {}", m.width, m.height, path.display());
            assert!(path.exists());
        }
    }
}
