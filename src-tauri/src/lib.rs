mod capture;
mod commands;
mod hotkey;
mod output;
mod overlay;

use overlay::PendingCapture;

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, WindowEvent,
};

/// Encodes a captured frame as PNG tuned for speed, not size — the overlay
/// needs to appear near-instantly and the bytes never leave the machine.
fn encode_fast_png(m: &capture::CapturedMonitor) -> Result<Vec<u8>, image::ImageError> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::ImageEncoder;

    let mut bytes = Vec::new();
    PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, FilterType::NoFilter)
        .write_image(&m.rgba, m.width, m.height, image::ExtendedColorType::Rgba8)?;
    Ok(bytes)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .manage(PendingCapture::new())
        .invoke_handler(tauri::generate_handler![
            commands::capture_full_screen,
            commands::start_region_capture,
            commands::get_pending_capture_meta,
            commands::finish_region_capture,
            commands::cancel_region_capture,
            commands::window_rect_at_point,
        ])
        // Serves the frozen monitor frames to overlay windows as raw PNG
        // bytes (http://capture.localhost/<monitor-index>) — far faster
        // than pushing a multi-megapixel image through JSON IPC.
        .register_uri_scheme_protocol("capture", |ctx, request| {
            let monitor: usize = request
                .uri()
                .path()
                .trim_start_matches('/')
                .parse()
                .unwrap_or(usize::MAX);
            let pending = ctx.app_handle().state::<PendingCapture>();
            let monitors = pending.0.lock().unwrap();
            match monitors.get(monitor).map(encode_fast_png) {
                Some(Ok(bytes)) => tauri::http::Response::builder()
                    .header("Content-Type", "image/png")
                    .body(bytes)
                    .unwrap(),
                _ => tauri::http::Response::builder()
                    .status(404)
                    .body(Vec::new())
                    .unwrap(),
            }
        })
        .setup(|app| {
            let capture_region =
                MenuItem::with_id(app, "capture_region", "Capture Region", true, None::<&str>)?;
            let capture_full = MenuItem::with_id(
                app,
                "capture_full_screen",
                "Capture Full Screen",
                true,
                None::<&str>,
            )?;
            let open_folder = MenuItem::with_id(
                app,
                "open_screenshots_folder",
                "Open Screenshots Folder",
                true,
                None::<&str>,
            )?;
            let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &capture_region,
                    &capture_full,
                    &open_folder,
                    &settings,
                    &quit,
                ],
            )?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "capture_region" => commands::trigger_region_capture(app),
                    "capture_full_screen" => {
                        if let Err(e) = commands::capture_full_screen() {
                            eprintln!("capture failed: {e}");
                        }
                    }
                    "open_screenshots_folder" => {
                        if let Ok(dir) = output::effective_screenshots_dir() {
                            let _ = std::fs::create_dir_all(&dir);
                            let _ = std::process::Command::new("explorer").arg(dir).spawn();
                        }
                    }
                    "settings" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            hotkey::maybe_prompt_disable_snipping_binding();
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                hotkey::run_blocking(move || {
                    commands::trigger_region_capture(&app_handle);
                });
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // Only the main (settings) window hides to the tray on close;
            // transient windows like the capture overlays must really close,
            // or their labels stay taken and the next capture fails.
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    window.hide().unwrap();
                    api.prevent_close();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
