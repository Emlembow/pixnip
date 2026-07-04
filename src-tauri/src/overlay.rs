//! Region-picker overlay: one borderless always-on-top window per monitor
//! showing the frozen capture, with drag-select and window-snap.

use std::sync::Mutex;

use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, SetWindowPos,
    WindowFromPoint, GA_ROOT, GWL_EXSTYLE, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, WS_EX_TRANSPARENT,
};

use crate::capture::CapturedMonitor;

pub struct PendingCapture(pub Mutex<Vec<CapturedMonitor>>);

impl PendingCapture {
    pub fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }
}

/// Position and size (physical pixels) for one overlay window.
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

const OVERLAY_LABEL_PREFIX: &str = "overlay-";

/// Opens one overlay window per captured monitor, positioned and sized to
/// exactly cover it.
pub fn open_overlays(app: &AppHandle, placements: &[Placement]) -> tauri::Result<()> {
    for (index, placement) in placements.iter().enumerate() {
        let label = format!("{OVERLAY_LABEL_PREFIX}{index}");
        let window = WebviewWindowBuilder::new(
            app,
            &label,
            WebviewUrl::App(format!("overlay.html?monitor={index}").into()),
        )
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(true)
        .visible(false)
        .build()?;

        // Physical pixels, to line up exactly with the capture buffer
        // regardless of this monitor's DPI scale factor.
        window.set_position(PhysicalPosition::new(placement.x, placement.y))?;
        window.set_size(PhysicalSize::new(placement.width, placement.height))?;
        // Stays hidden until the page has painted the frozen frame and
        // calls `overlay_ready` — otherwise a black window flashes first.
    }
    Ok(())
}

/// Destroys every open overlay window immediately (bypassing the
/// close-request path, which the main window uses for hide-to-tray).
pub fn close_overlays(app: &AppHandle) {
    for window in app.webview_windows().values() {
        if window.label().starts_with(OVERLAY_LABEL_PREFIX) {
            let _ = window.destroy();
        }
    }
}

/// Finds the top-level window rect under the given point in physical screen
/// coordinates, ignoring our own overlay windows. Used for window-snap.
pub fn window_rect_at_point(app: &AppHandle, x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
    let overlay_hwnds: Vec<HWND> = app
        .webview_windows()
        .values()
        .filter(|w| w.label().starts_with(OVERLAY_LABEL_PREFIX))
        .filter_map(|w| w.hwnd().ok())
        .collect();

    unsafe {
        let mut saved_styles = Vec::with_capacity(overlay_hwnds.len());
        for &hwnd in &overlay_hwnds {
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            saved_styles.push(style);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_TRANSPARENT.0 as isize);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }

        let found = WindowFromPoint(POINT { x, y });
        let root = if found.0.is_null() {
            None
        } else {
            let root = GetAncestor(found, GA_ROOT);
            if root.0.is_null() {
                None
            } else {
                Some(root)
            }
        };

        for (i, &hwnd) in overlay_hwnds.iter().enumerate() {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, saved_styles[i]);
        }

        let root = root?;
        if overlay_hwnds.contains(&root) {
            return None;
        }

        let mut rect = RECT::default();
        GetWindowRect(root, &mut rect).ok()?;
        Some((
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        ))
    }
}
