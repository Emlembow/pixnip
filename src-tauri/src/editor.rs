//! Editor window: opened with a captured region after selection, hosts the
//! markup/crop UI. The image travels the same raw-RGBA route as the overlay
//! (via the `capture:` protocol, path `/edit`).

use std::sync::Mutex;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::capture::CapturedMonitor;

pub struct PendingEdit(pub Mutex<Option<CapturedMonitor>>);

impl PendingEdit {
    pub fn new() -> Self {
        Self(Mutex::new(None))
    }
}

pub const EDITOR_LABEL_PREFIX: &str = "editor-";

/// Destroys any open editor window.
pub fn close_editors(app: &AppHandle) {
    for window in app.webview_windows().values() {
        if window.label().starts_with(EDITOR_LABEL_PREFIX) {
            let _ = window.destroy();
        }
    }
}

/// Opens the editor window sized to fit the image comfortably. The window
/// stays hidden until the page reports the image painted.
///
/// The caller must have closed any previous editor window *and waited for
/// its teardown to complete* — building a webview while another's destroy
/// is queued deadlocks the event loop. Labels are unique per session so a
/// window still mid-teardown can never collide with the new one.
pub fn open_editor(app: &AppHandle, image_width: u32, image_height: u32) -> tauri::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SESSION: AtomicU64 = AtomicU64::new(0);

    // Fit the image plus toolbar chrome into a sensible window, leaving the
    // exact scaling to CSS. Logical pixels; the webview handles DPI.
    let chrome_height = 96.0;
    let max_w = 1400.0_f64;
    let max_h = 900.0_f64;
    let scale = (max_w / image_width as f64)
        .min((max_h - chrome_height) / image_height as f64)
        .min(1.0);
    let win_w = (image_width as f64 * scale).max(560.0);
    let win_h = image_height as f64 * scale + chrome_height;

    let label = format!(
        "{EDITOR_LABEL_PREFIX}{}",
        SESSION.fetch_add(1, Ordering::Relaxed)
    );
    WebviewWindowBuilder::new(app, &label, WebviewUrl::App("editor.html".into()))
        .title("Pixnip")
        .inner_size(win_w, win_h)
        .min_inner_size(560.0, 400.0)
        .visible(false)
        .center()
        .build()?;

    Ok(())
}
