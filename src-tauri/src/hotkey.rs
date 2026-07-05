//! Intercepts the PrintScreen key system-wide via a low-level keyboard hook
//! so Pixnip can handle it instead of (or alongside) Windows' own binding.

use std::sync::mpsc::Sender;
use std::sync::OnceLock;

use windows::core::w;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY_CURRENT_USER,
    KEY_READ, KEY_WRITE, REG_DWORD,
};
use windows::Win32::UI::Input::KeyboardAndMouse::VK_SNAPSHOT;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, MessageBoxW, SetWindowsHookExW,
    TranslateMessage, UnhookWindowsHookEx, IDYES, KBDLLHOOKSTRUCT, MB_ICONQUESTION, MB_YESNO, MSG,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

static TRIGGER: OnceLock<Sender<()>> = OnceLock::new();

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
    if code >= 0 {
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

/// If Windows is currently set to open Snipping Tool on PrintScreen, asks
/// the user (once, ever) whether to turn that off so the two don't fight
/// over the key. The registry value is absent by default on a fresh system
/// (which still means "enabled"), so absence is treated the same as `1`.
pub fn maybe_prompt_disable_snipping_binding() {
    unsafe {
        if already_asked() {
            return;
        }
        mark_asked();

        let mut hkey = Default::default();
        let path = w!("Control Panel\\Keyboard");
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            path,
            Some(0),
            KEY_READ | KEY_WRITE,
            &mut hkey,
        )
        .is_err()
        {
            return;
        }

        let value_name = w!("PrintScreenKeyForSnippingEnabled");
        let mut data: u32 = 1;
        let mut size = std::mem::size_of::<u32>() as u32;
        let _ = RegQueryValueExW(
            hkey,
            value_name,
            None,
            None,
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut size),
        );

        if data == 1 {
            let choice = MessageBoxW(
                None,
                w!(
                    "Windows currently opens Snipping Tool when you press PrintScreen.\n\n\
                     Disable that so Pixnip can use PrintScreen instead?"
                ),
                w!("Pixnip"),
                MB_YESNO | MB_ICONQUESTION,
            );
            if choice == IDYES {
                let zero: u32 = 0;
                let _ = RegSetValueExW(
                    hkey,
                    value_name,
                    Some(0),
                    REG_DWORD,
                    Some(std::slice::from_raw_parts(
                        &zero as *const u32 as *const u8,
                        4,
                    )),
                );
            }
        }

        let _ = RegCloseKey(hkey);
    }
}

unsafe fn already_asked() -> bool {
    let mut hkey = Default::default();
    if RegOpenKeyExW(
        HKEY_CURRENT_USER,
        w!("Software\\Pixnip"),
        Some(0),
        KEY_READ,
        &mut hkey,
    )
    .is_err()
    {
        return false;
    }
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let ok = RegQueryValueExW(
        hkey,
        w!("PromptedPrintScreenBinding"),
        None,
        None,
        Some(&mut data as *mut u32 as *mut u8),
        Some(&mut size),
    )
    .is_ok();
    let _ = RegCloseKey(hkey);
    ok && data == 1
}

unsafe fn mark_asked() {
    let mut hkey = Default::default();
    if RegCreateKeyW(HKEY_CURRENT_USER, w!("Software\\Pixnip"), &mut hkey).is_err() {
        return;
    }
    let one: u32 = 1;
    let _ = RegSetValueExW(
        hkey,
        w!("PromptedPrintScreenBinding"),
        Some(0),
        REG_DWORD,
        Some(std::slice::from_raw_parts(
            &one as *const u32 as *const u8,
            4,
        )),
    );
    let _ = RegCloseKey(hkey);
}
