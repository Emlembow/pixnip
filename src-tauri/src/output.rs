use std::path::PathBuf;

use crate::capture::CapturedMonitor;

/// Default save location: `Pictures\Screenshots`, resolved via the OS
/// known-folder API rather than a hardcoded path.
pub fn screenshots_dir() -> Result<PathBuf, String> {
    let pictures = dirs::picture_dir().ok_or("could not resolve the Pictures folder")?;
    Ok(pictures.join("Screenshots"))
}

/// Fallback when the primary folder is unwritable — typically because the
/// Pictures folder is OneDrive-redirected and Windows' Controlled Folder
/// Access blocks unrecognized apps (reported, misleadingly, as "file not
/// found"). Local app data is never CFA-protected.
fn fallback_screenshots_dir() -> Result<PathBuf, String> {
    let base = dirs::data_local_dir().ok_or("could not resolve local app data")?;
    Ok(base.join("Pixnip").join("Screenshots"))
}

/// The folder screenshots actually land in: the primary if writable,
/// otherwise the fallback. Used by "Open Screenshots Folder".
pub fn effective_screenshots_dir() -> Result<PathBuf, String> {
    if let Ok(dir) = screenshots_dir() {
        if dir_is_writable(&dir) {
            return Ok(dir);
        }
    }
    fallback_screenshots_dir()
}

fn dir_is_writable(dir: &std::path::Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".pixnip-write-probe");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Encodes a captured monitor as PNG and saves it into the screenshots
/// folder (falling back to a non-protected folder if the primary one is
/// blocked), creating the folder if needed. Returns the written file path.
pub fn save_capture(capture: &CapturedMonitor) -> Result<PathBuf, String> {
    let filename = format!("{}.png", timestamped_filename());

    let primary_err = match screenshots_dir().and_then(|dir| try_save(capture, &dir, &filename)) {
        Ok(path) => return Ok(path),
        Err(e) => e,
    };

    let fallback = fallback_screenshots_dir()?;
    let path = try_save(capture, &fallback, &filename)
        .map_err(|e| format!("primary save failed ({primary_err}); fallback failed ({e})"))?;
    eprintln!(
        "primary screenshots folder unwritable ({primary_err}); saved to {}",
        path.display()
    );
    Ok(path)
}

/// Saves already-encoded PNG bytes (from the editor's export canvas) with
/// the same primary/fallback folder logic as raw captures.
pub fn save_png_bytes(png_bytes: &[u8]) -> Result<PathBuf, String> {
    let filename = format!("{}.png", timestamped_filename());

    let write_to = |dir: &std::path::Path| -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(&filename);
        std::fs::write(&path, png_bytes).map_err(|e| e.to_string())?;
        Ok(path)
    };

    let primary_err = match screenshots_dir().and_then(|dir| write_to(&dir)) {
        Ok(path) => return Ok(path),
        Err(e) => e,
    };
    let fallback = fallback_screenshots_dir()?;
    let path = write_to(&fallback)
        .map_err(|e| format!("primary save failed ({primary_err}); fallback failed ({e})"))?;
    eprintln!(
        "primary screenshots folder unwritable ({primary_err}); saved to {}",
        path.display()
    );
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
            let path = save_capture(m).expect(
                "save failed - if the screenshots folder lives under OneDrive, check Windows \
                 Security > Ransomware protection > Controlled folder access and allow this exe",
            );
            println!("saved {}x{} -> {}", m.width, m.height, path.display());
            assert!(path.exists());
        }
    }
}
