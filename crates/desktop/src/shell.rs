//! Finding the layer between the static wallpaper and the desktop icons.
//!
//! Sending the undocumented-but-stable message `0x052C` to `Progman` makes Explorer
//! split the desktop into a wallpaper layer (`WorkerW`) and an icon layer
//! (`SHELLDLL_DefView`). Where that `WorkerW` lives changed in Windows 11 24H2:
//!
//! - **before 24H2**: a top-level `WorkerW` right behind the top-level window that hosts
//!   `SHELLDLL_DefView`. Our windows become its children.
//! - **24H2 and later**: `SHELLDLL_DefView` and the `WorkerW` are both children of
//!   `Progman`. Our windows become children of `Progman`, placed in the z-order right
//!   below the icons (and so above the static wallpaper).

use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::MapWindowPoints;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::log;
use customwin_core::wallpaper::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    /// Classic layout: child of the top-level `WorkerW`.
    WorkerW(HWND),
    /// 24H2+: child of `Progman`, right below `SHELLDLL_DefView`.
    Progman { progman: HWND, icons: HWND },
    /// `--windowed`: a normal window, to test rendering without touching the desktop.
    Windowed,
}

fn find(parent: Option<HWND>, after: Option<HWND>, class: PCWSTR) -> Option<HWND> {
    unsafe { FindWindowExW(parent, after, class, PCWSTR::null()) }
        .ok()
        .filter(|h| !h.is_invalid())
}

pub fn find_host() -> Option<Host> {
    let progman = find(None, None, w!("Progman"))?;
    // Ask Explorer to create the wallpaper layer (no-op if it already exists).
    unsafe {
        let mut result = 0usize;
        let _ = SendMessageTimeoutW(
            progman,
            0x052C,
            WPARAM(0xD),
            LPARAM(0x1),
            SMTO_NORMAL,
            1000,
            Some(&mut result),
        );
    }

    if find(Some(progman), None, w!("WorkerW")).is_some() {
        if let Some(icons) = find(Some(progman), None, w!("SHELLDLL_DefView")) {
            log::write("desktop layout: 24H2+ (WorkerW inside Progman)");
            return Some(Host::Progman { progman, icons });
        }
    }

    // Classic: the WorkerW that follows the top-level window holding the icons.
    unsafe extern "system" fn visit(top: HWND, data: LPARAM) -> BOOL {
        let out = unsafe { &mut *(data.0 as *mut Option<HWND>) };
        if find(Some(top), None, w!("SHELLDLL_DefView")).is_some() {
            *out = find(None, Some(top), w!("WorkerW"));
            return false.into();
        }
        true.into()
    }
    let mut worker: Option<HWND> = None;
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut worker as *mut _ as isize));
    }
    match worker {
        Some(w) => {
            log::write("desktop layout: classic (top-level WorkerW)");
            Some(Host::WorkerW(w))
        }
        None => {
            log::write("desktop layout: not found (is Explorer running?)");
            None
        }
    }
}

/// Moves a freshly created popup window into the wallpaper layer, covering `screen`
/// (in screen coordinates).
pub fn attach(hwnd: HWND, host: Host, screen: Rect) {
    let (parent, insert_after) = match host {
        Host::WorkerW(w) => (w, None),
        Host::Progman { progman, icons } => (progman, Some(icons)),
        Host::Windowed => return,
    };
    unsafe {
        let _ = SetParent(hwnd, Some(parent));
        SetWindowLongPtrW(
            hwnd,
            GWL_STYLE,
            (WS_CHILD | WS_VISIBLE | WS_DISABLED).0 as isize,
        );
        // The parent's client area starts at the virtual screen's origin: convert.
        let mut pts = [
            POINT {
                x: screen.left,
                y: screen.top,
            },
            POINT {
                x: screen.right,
                y: screen.bottom,
            },
        ];
        MapWindowPoints(None, Some(parent), &mut pts);
        let mut flags = SWP_NOACTIVATE | SWP_FRAMECHANGED | SWP_SHOWWINDOW;
        if insert_after.is_none() {
            flags |= SWP_NOZORDER;
        }
        let _ = SetWindowPos(
            hwnd,
            insert_after,
            pts[0].x,
            pts[0].y,
            pts[1].x - pts[0].x,
            pts[1].y - pts[0].y,
            flags,
        );
    }
}

/// Makes Explorer repaint the static wallpaper once our windows are gone (otherwise the
/// last video frame can stay on screen until the next refresh).
pub fn repaint_static_wallpaper() {
    unsafe {
        let mut path = [0u16; 520];
        if SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            path.len() as u32,
            Some(path.as_mut_ptr().cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
        {
            // Same wallpaper, not written to the profile: just a refresh broadcast.
            let _ = SystemParametersInfoW(
                SPI_SETDESKWALLPAPER,
                0,
                Some(path.as_mut_ptr().cast()),
                SPIF_SENDCHANGE,
            );
        }
    }
}
