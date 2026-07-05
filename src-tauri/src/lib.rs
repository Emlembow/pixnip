mod capture;
mod commands;
mod editor;
mod hotkey;
mod output;
mod overlay;
mod settings;

use editor::PendingEdit;
use overlay::PendingCapture;

use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::GlobalShortcutExt as _;

/// The capture shortcut used when PrintScreen interception is off.
const ALT_SHORTCUT: &str = "ctrl+alt+s";

/// Applies the chosen capture-key mode: either swallow PrintScreen (and
/// turn off Windows' Snipping Tool binding so they don't fight), or leave
/// PrintScreen alone and bind Ctrl+Alt+S instead.
fn apply_hotkey_mode(app: &AppHandle, use_print_screen: bool) {
    hotkey::set_intercept(use_print_screen);
    if use_print_screen {
        hotkey::disable_windows_snipping_binding();
        let _ = app.global_shortcut().unregister(ALT_SHORTCUT);
    } else if let Err(e) = app.global_shortcut().register(ALT_SHORTCUT) {
        eprintln!("failed to register {ALT_SHORTCUT}: {e}");
    }
}

fn apply_autostart(app: &AppHandle, on: bool) {
    // Under MSIX (Store install), registry Run keys are virtualized and
    // never execute at logon — start-with-Windows must go through the
    // StartupTask declared in Package.appxmanifest instead.
    if windows::ApplicationModel::Package::Current().is_ok() {
        let result = (|| -> windows::core::Result<()> {
            let task = windows::ApplicationModel::StartupTask::GetAsync(
                &windows::core::HSTRING::from("PixnipStartup"),
            )?
            .get()?;
            if on {
                // May surface a system consent prompt; the returned state
                // reflects the user's answer.
                let state = task.RequestEnableAsync()?.get()?;
                eprintln!("startup task state: {state:?}");
            } else {
                task.Disable()?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            eprintln!("startup task update failed: {e}");
        }
        return;
    }

    let autolaunch = app.autolaunch();
    let result = if on {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
    if let Err(e) = result {
        eprintln!("autostart update failed: {e}");
    }
}

/// Builds the tray icon and menu. Runs after the first-run questions have
/// been answered so the check states reflect the settled settings.
fn build_tray(app: &AppHandle, s: settings::Settings) -> tauri::Result<()> {
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
    let autostart_item = CheckMenuItem::with_id(
        app,
        "autostart",
        "Start with Windows",
        true,
        s.start_with_windows,
        None::<&str>,
    )?;
    let prtsc_item = CheckMenuItem::with_id(
        app,
        "use_prtsc",
        "Use PrintScreen Key (off: Ctrl+Alt+S)",
        true,
        s.use_print_screen,
        None::<&str>,
    )?;
    let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &capture_region,
            &capture_full,
            &open_folder,
            &PredefinedMenuItem::separator(app)?,
            &autostart_item,
            &prtsc_item,
            &PredefinedMenuItem::separator(app)?,
            &settings_item,
            &quit,
        ],
    )?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| match event.id.as_ref() {
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
            // Check items toggle themselves on click; is_checked() is the
            // new state.
            "autostart" => {
                let on = autostart_item.is_checked().unwrap_or(false);
                let mut s = settings::load();
                s.start_with_windows = on;
                settings::save(&s);
                apply_autostart(app, on);
            }
            "use_prtsc" => {
                let on = prtsc_item.is_checked().unwrap_or(true);
                let mut s = settings::load();
                s.use_print_screen = on;
                settings::save(&s);
                apply_hotkey_mode(app, on);
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
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut builder = tauri::Builder::default();

    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }))
            .plugin(tauri_plugin_drag::init())
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                None,
            ))
            .plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(|app, _shortcut, event| {
                        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                            commands::trigger_region_capture(app);
                        }
                    })
                    .build(),
            );
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .manage(PendingCapture::new())
        .manage(PendingEdit::new())
        .invoke_handler(tauri::generate_handler![
            commands::capture_full_screen,
            commands::start_region_capture,
            commands::get_pending_capture_meta,
            commands::overlay_ready,
            commands::finish_region_capture,
            commands::cancel_region_capture,
            commands::window_rect_at_point,
            commands::get_edit_meta,
            commands::editor_ready,
            commands::save_edited,
            commands::copy_edited,
            commands::cancel_edit,
            commands::export_drag_file,
            commands::frontend_log,
        ])
        // Serves the frozen monitor frames to overlay windows as raw RGBA
        // bytes (http://capture.localhost/<monitor-index>). No PNG encode
        // here and no decode in the webview — the overlay blits the bytes
        // straight onto a canvas, so the frame appears near-instantly.
        .register_uri_scheme_protocol("capture", |ctx, request| {
            let path = request.uri().path().trim_start_matches('/').to_owned();
            let frame_response = |m: &capture::CapturedMonitor| {
                tauri::http::Response::builder()
                    .header("Content-Type", "application/octet-stream")
                    .header("Access-Control-Allow-Origin", "*")
                    .header("X-Frame-Width", m.width.to_string())
                    .header("X-Frame-Height", m.height.to_string())
                    .header(
                        "Access-Control-Expose-Headers",
                        "X-Frame-Width, X-Frame-Height",
                    )
                    .body(m.rgba.clone())
                    .unwrap()
            };
            let not_found = || {
                tauri::http::Response::builder()
                    .status(404)
                    .body(Vec::new())
                    .unwrap()
            };

            if path == "edit" {
                let edit = ctx.app_handle().state::<PendingEdit>();
                let guard = edit.0.lock().unwrap();
                return match guard.as_ref() {
                    Some(m) => frame_response(m),
                    None => not_found(),
                };
            }

            let monitor: usize = path.parse().unwrap_or(usize::MAX);
            let pending = ctx.app_handle().state::<PendingCapture>();
            let monitors = pending.0.lock().unwrap();
            match monitors.get(monitor) {
                Some(m) => frame_response(m),
                None => not_found(),
            }
        })
        .setup(|app| {
            // The PrintScreen hook is always installed but only swallows
            // the key once settings say so (set_intercept below).
            let hook_handle = app.handle().clone();
            std::thread::spawn(move || {
                hotkey::run_blocking(move || {
                    commands::trigger_region_capture(&hook_handle);
                });
            });

            // First-run questions (blocking native prompts, so off the main
            // thread), then build the tray with the settled check states
            // and apply both settings.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut s = settings::load();
                if !s.first_run_done {
                    s.start_with_windows = hotkey::ask_yes_no(
                        "Start Pixnip automatically with Windows?\n\n\
                         It runs quietly in the tray, ready to capture.",
                    );
                    s.use_print_screen = hotkey::ask_yes_no(
                        "Use the PrintScreen key to capture?\n\n\
                         Yes \u{2014} PrintScreen opens Pixnip's capture \
                         (Windows' Snipping Tool binding is turned off).\n\
                         No \u{2014} Pixnip binds Ctrl+Alt+S and leaves \
                         PrintScreen alone.\n\n\
                         You can change both choices any time from the tray menu.",
                    );
                    s.first_run_done = true;
                    settings::save(&s);
                    apply_autostart(&handle, s.start_with_windows);
                }
                apply_hotkey_mode(&handle, s.use_print_screen);

                let tray_handle = handle.clone();
                let _ = handle.run_on_main_thread(move || {
                    if let Err(e) = build_tray(&tray_handle, s) {
                        eprintln!("failed to build tray: {e}");
                    }
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
