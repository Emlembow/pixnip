use std::path::PathBuf;

use crate::capture::CapturedMonitor;

/// Default save location: `Pictures\Screenshots`, resolved via the OS
/// known-folder API rather than a hardcoded path.
pub fn screenshots_dir() -> Result<PathBuf, String> {
    let pictures = dirs::picture_dir().ok_or("could not resolve the Pictures folder")?;
    Ok(pictures.join("Screenshots"))
}

/// Encodes a captured monitor as PNG and saves it into the screenshots
/// folder, creating the folder if needed. Returns the written file path.
pub fn save_capture(capture: &CapturedMonitor) -> Result<PathBuf, String> {
    let dir = screenshots_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let path = dir.join(format!("{}.png", timestamped_filename()));
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
