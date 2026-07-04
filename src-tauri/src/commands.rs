use tauri::{AppHandle, Manager, State};

use crate::overlay::PendingCapture;
use crate::{capture, output, overlay};

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

/// Captures every monitor, stashes the frames, and opens one region-picker
/// overlay window per monitor on top of them.
#[tauri::command]
pub fn start_region_capture(app: AppHandle, pending: State<PendingCapture>) -> Result<(), String> {
    // A stale overlay from an earlier, unfinished capture would collide on
    // window labels; clear it first so PrintScreen always works.
    overlay::close_overlays(&app);

    let monitors = capture::capture_all_monitors().map_err(|e| e.to_string())?;
    let placements: Vec<overlay::Placement> = monitors
        .iter()
        .map(|m| overlay::Placement {
            x: m.x,
            y: m.y,
            width: m.width,
            height: m.height,
        })
        .collect();
    // Stash before opening: the overlay page requests its frame as soon as
    // it loads.
    *pending.0.lock().unwrap() = monitors;
    overlay::open_overlays(&app, &placements).map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(serde::Serialize)]
pub struct MonitorMeta {
    /// This monitor's origin in virtual desktop (physical) coordinates.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Returns position/size metadata for one pending monitor frame. The pixel
/// data itself is served by the `capture:` protocol, not over IPC.
#[tauri::command]
pub fn get_pending_capture_meta(
    pending: State<PendingCapture>,
    monitor: usize,
) -> Result<MonitorMeta, String> {
    let monitors = pending.0.lock().unwrap();
    let m = monitors.get(monitor).ok_or("no such monitor")?;
    Ok(MonitorMeta {
        x: m.x,
        y: m.y,
        width: m.width,
        height: m.height,
    })
}

/// Crops the given monitor's frozen frame to the selected region, saves it,
/// and closes all overlay windows.
#[tauri::command]
pub fn finish_region_capture(
    app: AppHandle,
    pending: State<PendingCapture>,
    monitor: usize,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<String, String> {
    let saved_path = {
        let monitors = pending.0.lock().unwrap();
        let m = monitors.get(monitor).ok_or("no such monitor")?;
        let cropped = m.cropped(x, y, width, height);
        output::save_capture(&cropped)?
    };
    overlay::close_overlays(&app);
    Ok(saved_path.to_string_lossy().into_owned())
}

/// Called by an overlay page once its frozen frame is painted; only then
/// is the window shown, so no black flash is ever visible.
#[tauri::command]
pub fn overlay_ready(window: tauri::WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

/// Closes all overlay windows without saving anything.
#[tauri::command]
pub fn cancel_region_capture(app: AppHandle) {
    overlay::close_overlays(&app);
}

/// Returns the screen rect (x, y, width, height) of the top-level window
/// under the given point, ignoring our own overlay windows.
#[tauri::command]
pub fn window_rect_at_point(app: AppHandle, x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
    overlay::window_rect_at_point(&app, x, y)
}

/// Non-IPC entry point for triggering a region capture from the tray menu
/// or the PrintScreen hook, where there's no invoke context to pull a
/// `State` from automatically.
pub fn trigger_region_capture(app: &AppHandle) {
    let pending = app.state::<PendingCapture>();
    if let Err(e) = start_region_capture(app.clone(), pending) {
        eprintln!("region capture failed: {e}");
    }
}
