use tauri::{AppHandle, Manager, State};

use crate::editor::PendingEdit;
use crate::overlay::PendingCapture;
use crate::{capture, editor, output, overlay};

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

/// Blocks until no webview window with any of the given label prefixes
/// remains registered. Building a webview while another one's destroy is
/// still queued deadlocks wry's nested message pump on Windows, so every
/// close-then-create sequence must wait this out in between.
///
/// Must be called off the main thread — the teardown it waits for is
/// processed *by* the main thread's event loop.
fn wait_for_window_teardown(app: &AppHandle, prefixes: &[&str]) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        let any_left = app
            .webview_windows()
            .keys()
            .any(|label| prefixes.iter().any(|p| label.starts_with(p)));
        if !any_left {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    eprintln!("wait_for_window_teardown: timed out waiting for {prefixes:?}");
}

/// Serializes capture/editor window flows so two rapid triggers can't
/// interleave their close-wait-create sequences.
static FLOW_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn region_capture_flow(app: &AppHandle) -> Result<(), String> {
    let _guard = FLOW_LOCK.lock().unwrap();

    // Clear any stale overlay from an earlier, unfinished capture.
    overlay::close_overlays(app);
    wait_for_window_teardown(app, &[overlay::OVERLAY_LABEL_PREFIX]);

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
    *app.state::<PendingCapture>().0.lock().unwrap() = monitors;
    overlay::open_overlays(app, &placements).map_err(|e| e.to_string())?;
    Ok(())
}

/// Captures every monitor, stashes the frames, and opens one region-picker
/// overlay window per monitor on top of them.
#[tauri::command]
pub fn start_region_capture(app: AppHandle) {
    trigger_region_capture(&app);
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

#[derive(serde::Deserialize)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Crops the given monitor's frozen frame to the selected region, closes
/// the overlays, and hands the crop to the editor window for markup.
#[tauri::command]
pub fn finish_region_capture(
    app: AppHandle,
    pending: State<PendingCapture>,
    edit: State<PendingEdit>,
    monitor: usize,
    region: Region,
) -> Result<(), String> {
    let cropped = {
        let monitors = pending.0.lock().unwrap();
        let m = monitors.get(monitor).ok_or("no such monitor")?;
        m.cropped(region.x, region.y, region.width, region.height)
    };
    let (w, h) = (cropped.width, cropped.height);
    *edit.0.lock().unwrap() = Some(cropped);

    // Tauri runs sync commands on the main thread, so doing the
    // close-then-create here (or via run_on_main_thread, which executes
    // inline when already on the main thread) deadlocks: building a
    // webview pumps messages, and pumping a queued webview destroy
    // re-enters wry's web-context lock. Hand the sequence to a worker
    // thread that waits for the teardown to finish first.
    let app = app.clone();
    std::thread::spawn(move || {
        let _guard = FLOW_LOCK.lock().unwrap();
        overlay::close_overlays(&app);
        editor::close_editors(&app);
        wait_for_window_teardown(
            &app,
            &[overlay::OVERLAY_LABEL_PREFIX, editor::EDITOR_LABEL_PREFIX],
        );
        if let Err(e) = editor::open_editor(&app, w, h) {
            eprintln!("open_editor failed: {e}");
        }
    });
    Ok(())
}

/// Returns width/height of the image awaiting editing (pixels come over
/// the `capture:` protocol at path `/edit`).
#[tauri::command]
pub fn get_edit_meta(edit: State<PendingEdit>) -> Result<(u32, u32), String> {
    let guard = edit.0.lock().unwrap();
    let m = guard.as_ref().ok_or("nothing to edit")?;
    Ok((m.width, m.height))
}

/// Lets webview pages surface errors in the process log, where they're
/// actually visible during development.
#[tauri::command]
pub fn frontend_log(message: String) {
    eprintln!("[frontend] {message}");
}

/// Called by the editor page once its image is painted.
#[tauri::command]
pub fn editor_ready(window: tauri::WebviewWindow) {
    let _ = window.show();
    let _ = window.set_focus();
}

/// Saves the edited PNG produced by the editor canvas and (optionally)
/// puts the bitmap on the clipboard. Closes the editor window.
#[tauri::command]
pub fn save_edited(
    app: AppHandle,
    png_bytes: Vec<u8>,
    copy_to_clipboard: bool,
) -> Result<String, String> {
    let decoded = image::load_from_memory_with_format(&png_bytes, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?
        .into_rgba8();

    let path = output::save_png_bytes(&png_bytes)?;

    if copy_to_clipboard {
        let (w, h) = decoded.dimensions();
        let result = arboard::Clipboard::new().and_then(|mut cb| {
            cb.set_image(arboard::ImageData {
                width: w as usize,
                height: h as usize,
                bytes: decoded.into_raw().into(),
            })
        });
        if let Err(e) = result {
            eprintln!("clipboard copy failed: {e}");
        }
    }

    editor::close_editors(&app);
    Ok(path.to_string_lossy().into_owned())
}

/// Copies the edited PNG to the clipboard without saving or closing the
/// editor — for pasting straight into a chat while continuing to edit.
#[tauri::command]
pub fn copy_edited(png_bytes: Vec<u8>) -> Result<(), String> {
    let decoded = image::load_from_memory_with_format(&png_bytes, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let (w, h) = decoded.dimensions();
    arboard::Clipboard::new()
        .and_then(|mut cb| {
            cb.set_image(arboard::ImageData {
                width: w as usize,
                height: h as usize,
                bytes: decoded.into_raw().into(),
            })
        })
        .map_err(|e| e.to_string())
}

/// Stages the edited PNG as a real file in a temp folder and returns its
/// path, so the editor can start a native drag-out with it — dropping the
/// annotated screenshot straight into Slack, a browser upload box, or
/// Explorer without saving first.
#[tauri::command]
pub fn export_drag_file(png_bytes: Vec<u8>) -> Result<String, String> {
    output::stage_drag_file(&png_bytes).map(|p| p.to_string_lossy().into_owned())
}

/// Discards the pending edit and closes the editor window.
#[tauri::command]
pub fn cancel_edit(app: AppHandle, edit: State<PendingEdit>) {
    *edit.0.lock().unwrap() = None;
    editor::close_editors(&app);
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

/// Entry point for triggering a region capture from anywhere (tray menu,
/// PrintScreen hook, IPC). Runs the whole flow on a worker thread: the
/// close-wait-create sequence must never run on the main thread (see
/// `wait_for_window_teardown`), and capturing off-thread keeps the UI
/// responsive besides.
pub fn trigger_region_capture(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = region_capture_flow(&app) {
            eprintln!("region capture failed: {e}");
        }
    });
}
