//! Intercepts the PrintScreen key system-wide via a low-level keyboard hook
//! so Pixnip can handle it instead of (or alongside) Windows' own binding.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD,
};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_SNAPSHOT;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, MessageBoxW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, IDYES, KBDLLHOOKSTRUCT, MB_ICONQUESTION, MB_YESNO, MSG,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

static TRIGGER: OnceLock<Sender<()>> = OnceLock::new();

/// Whether the hook currently swallows PrintScreen. Off until settings are
/// applied, so the key is never stolen before the user has opted in.
static INTERCEPT: AtomicBool = AtomicBool::new(false);

/// Turns PrintScreen interception on or off without reinstalling the hook.
pub fn set_intercept(on: bool) {
    INTERCEPT.store(on, Ordering::Relaxed);
}

/// Installs the hook and pumps messages for its lifetime. Blocks, so run it
/// on its own dedicated thread. `on_trigger` fires (on a separate worker
/// thread) whenever PrintScreen is pressed.
pub fn run_blocking(on_trigger: impl Fn() + Send + 'static) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let _ = TRIGGER.set(tx);

    std::thread::spawn(move || {
        for () in rx {
            on_trigger();
        }
    });

    unsafe {
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), None, 0)
            .expect("failed to install PrintScreen keyboard hook");

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        let _ = UnhookWindowsHookEx(hook);
    }
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && INTERCEPT.load(Ordering::Relaxed) {
        let msg = wparam.0 as u32;
        if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
            let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            if info.vkCode == VK_SNAPSHOT.0 as u32 {
                if let Some(tx) = TRIGGER.get() {
                    let _ = tx.send(());
                }
                return LRESULT(1);
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// Blocking native Yes/No prompt; used for the first-run questions, which
/// must work before any webview window exists.
pub fn ask_yes_no(text: &str) -> bool {
    let text_w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text_w.as_ptr()),
            w!("Pixnip"),
            MB_YESNO | MB_ICONQUESTION,
        ) == IDYES
    }
}

/// Turns off Windows' own "PrintScreen opens Snipping Tool" binding so it
/// doesn't fight Pixnip for the key. Called when the user opts into
/// PrintScreen capture (first run or the tray toggle) — consent is the
/// opt-in itself, so no extra prompt.
pub fn disable_windows_snipping_binding() {
    unsafe {
        let mut hkey = Default::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Control Panel\\Keyboard"),
            Some(0),
            KEY_READ | KEY_WRITE,
            &mut hkey,
        )
        .is_err()
        {
            return;
        }
        let zero: u32 = 0;
        let _ = RegSetValueExW(
            hkey,
            w!("PrintScreenKeyForSnippingEnabled"),
            Some(0),
            REG_DWORD,
            Some(std::slice::from_raw_parts(
                &zero as *const u32 as *const u8,
                4,
            )),
        );
        let _ = RegCloseKey(hkey);
    }
}
