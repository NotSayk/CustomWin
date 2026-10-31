//! What covers the wallpaper. Only the foreground window is checked, which is what
//! matters for games and maximized apps, and costs nothing between events.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFOEXW, MONITOR_DEFAULTTONULL,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Set by the event hook; the main loop re-evaluates pauses when it sees it.
pub static CHANGED: AtomicBool = AtomicBool::new(true);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Foreground {
    /// Monitor the foreground window is on.
    pub device: Option<String>,
    pub fullscreen: bool,
    pub maximized: bool,
}

const SHELL_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "CustomWinWallpaper",
];

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn foreground() -> Foreground {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() || IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() {
            return Foreground::default();
        }
        if SHELL_CLASSES.contains(&class_name(hwnd).as_str()) {
            return Foreground::default();
        }
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut u32 as _, 4).is_ok()
            && cloaked != 0
        {
            return Foreground::default();
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if monitor.is_invalid() || !GetMonitorInfoW(monitor, &mut info.monitorInfo).as_bool() {
            return Foreground::default();
        }
        let mut r = RECT::default();
        let _ = GetWindowRect(hwnd, &mut r);
        let m = info.monitorInfo.rcMonitor;
        Foreground {
            device: Some(
                String::from_utf16_lossy(&info.szDevice)
                    .trim_end_matches('\0')
                    .to_string(),
            ),
            fullscreen: r.left <= m.left
                && r.top <= m.top
                && r.right >= m.right
                && r.bottom >= m.bottom,
            maximized: IsZoomed(hwnd).as_bool(),
        }
    }
}

unsafe extern "system" fn on_event(
    _: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    // Location changes fire for every window and the cursor: keep only the foreground
    // window's own moves (maximize, restore, resize).
    if event == EVENT_OBJECT_LOCATIONCHANGE
        && (id_object != OBJID_WINDOW.0 || hwnd != unsafe { GetForegroundWindow() })
    {
        return;
    }
    CHANGED.store(true, Ordering::Relaxed);
}

/// Out-of-context hooks: delivered through this thread's message loop.
pub fn install_hooks() {
    unsafe {
        for (min, max) in [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
            (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
        ] {
            let _ = SetWinEventHook(min, max, None, Some(on_event), 0, 0, WINEVENT_OUTOFCONTEXT);
        }
    }
}

/// Whether a window handle still exists (Explorer restarts destroy our windows).
pub fn alive(hwnd: HWND) -> bool {
    let r: BOOL = unsafe { IsWindow(Some(hwnd)) };
    r.as_bool()
}
