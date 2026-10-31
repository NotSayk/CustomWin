//! What the app needs to drive the animated wallpaper and the widgets: screens, a video
//! picker, the `customwin-desktop.exe` process, and starting it with Windows.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, PostMessageW, MONITORINFOF_PRIMARY, WM_CLOSE,
};
use windows_registry::CURRENT_USER;

use super::map_err;
use crate::wallpaper::{Rect, AUTOSTART_NAME, CONTROL_CLASS, MSG_RELOAD};
use crate::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Monitor {
    /// GDI device name, e.g. `\\.\DISPLAY1`.
    pub device: String,
    /// Physical pixels (the caller must be per-monitor DPI aware to get them).
    pub rect: Rect,
    /// `rect` minus the taskbar and other docked bars.
    pub work: Rect,
    pub primary: bool,
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn collect(
        h: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> windows::core::BOOL {
        let out = unsafe { &mut *(data.0 as *mut Vec<Monitor>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if unsafe { GetMonitorInfoW(h, &mut info.monitorInfo) }.as_bool() {
            let rect = |r: RECT| Rect {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            };
            out.push(Monitor {
                device: String::from_utf16_lossy(&info.szDevice)
                    .trim_end_matches('\0')
                    .to_string(),
                rect: rect(info.monitorInfo.rcMonitor),
                work: rect(info.monitorInfo.rcWork),
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            });
        }
        true.into()
    }
    let mut out: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut out as *mut _ as isize),
        );
    }
    // Primary first, then left to right.
    out.sort_by_key(|m| (!m.primary, m.rect.left, m.rect.top));
    out
}

/// Native "Open" dialog filtered on videos. `owner` is the parent window handle (0 for
/// none). Returns `None` if the user cancels. Runs on its own STA thread.
pub fn pick_video(owner: isize, title: &str) -> Result<Option<PathBuf>, Error> {
    let title = title.to_string();
    std::thread::spawn(move || unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| map_err("COM", e))?;
        let result = (|| -> Result<Option<PathBuf>, Error> {
            let err = |e| map_err("file dialog", e);
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).map_err(err)?;
            let spec = HSTRING::from(
                crate::wallpaper::VIDEO_EXTENSIONS
                    .iter()
                    .map(|e| format!("*.{e}"))
                    .collect::<Vec<_>>()
                    .join(";"),
            );
            let name = HSTRING::from("Vidéos");
            let filters = [COMDLG_FILTERSPEC {
                pszName: PCWSTR(name.as_ptr()),
                pszSpec: PCWSTR(spec.as_ptr()),
            }];
            dialog.SetFileTypes(&filters).map_err(err)?;
            dialog.SetTitle(&HSTRING::from(title)).map_err(err)?;
            dialog
                .SetOptions(
                    dialog.GetOptions().map_err(err)? | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM,
                )
                .map_err(err)?;
            let parent = (owner != 0).then_some(HWND(owner as *mut _));
            if let Err(e) = dialog.Show(parent) {
                // ERROR_CANCELLED
                return if e.code().0 as u32 == 0x8007_04C7 {
                    Ok(None)
                } else {
                    Err(err(e))
                };
            }
            let item = dialog.GetResult().map_err(err)?;
            let path = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(err)?;
            let out = path.to_string().map_err(|e| Error::System(e.to_string()))?;
            CoTaskMemFree(Some(path.0 as *const _));
            Ok(Some(PathBuf::from(out)))
        })();
        CoUninitialize();
        result
    })
    .join()
    .map_err(|_| Error::System("file dialog crashed".into()))?
}

fn control_window() -> Option<HWND> {
    unsafe { FindWindowW(&HSTRING::from(CONTROL_CLASS), PCWSTR::null()) }.ok()
}

pub fn desktop_running() -> bool {
    control_window().is_some()
}

/// Asks a running `customwin-desktop.exe` to reload its configuration.
pub fn desktop_reload() -> bool {
    control_window()
        .is_some_and(|h| unsafe { PostMessageW(Some(h), MSG_RELOAD, WPARAM(0), LPARAM(0)) }.is_ok())
}

pub fn desktop_stop() {
    if let Some(h) = control_window() {
        let _ = unsafe { PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
}

pub fn desktop_start(exe: &Path) -> Result<(), Error> {
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    std::process::Command::new(exe)
        .creation_flags(DETACHED_PROCESS)
        .spawn()
        .map(|_| ())
        .map_err(|e| Error::System(format!("{}: {e}", exe.display())))
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Starts `exe` at sign-in (`Some`), or stops doing so (`None`).
pub fn set_autostart(exe: Option<&Path>) -> Result<(), Error> {
    let key = CURRENT_USER
        .create(RUN_KEY)
        .map_err(|e| map_err(RUN_KEY, e))?;
    match exe {
        Some(p) => key.set_string(AUTOSTART_NAME, format!("\"{}\"", p.display())),
        None => match key.remove_value(AUTOSTART_NAME) {
            Err(e) if e.code().0 as u32 == 0x8007_0002 => Ok(()),
            r => r,
        },
    }
    .map_err(|e| map_err(RUN_KEY, e))
}

pub fn wallpaper_config_path() -> PathBuf {
    super::data_dir().join("wallpaper.json")
}

pub fn widgets_config_path() -> PathBuf {
    super::data_dir().join("widgets.json")
}
