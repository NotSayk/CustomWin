//! Transparent taskbar: loads `customwin_taskbar.dll` into Explorer through XAML
//! diagnostics (see `crates/taskbar`), then switches it on and off with a message.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::core::{s, w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{FreeLibrary, HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetWindowThreadProcessId, PostMessageW, HWND_MESSAGE,
};
use windows_core::{Error, Result, GUID, HRESULT};

use customwin_taskbar::{CLSID, HOOK_CLASS, INIT_TRANSPARENT, MSG_SET};

type InitializeXamlDiagnosticsEx =
    unsafe extern "system" fn(PCWSTR, u32, PCWSTR, PCWSTR, GUID, PCWSTR) -> HRESULT;

/// The DLL's message windows, one per taskbar UI thread; none until it is loaded.
fn hooks() -> Vec<HWND> {
    let class = HSTRING::from(HOOK_CLASS);
    let mut out = Vec::new();
    let mut after = None;
    while let Ok(h) = unsafe { FindWindowExW(Some(HWND_MESSAGE), after, &class, PCWSTR::null()) } {
        if h.is_invalid() || out.len() > 16 {
            break;
        }
        out.push(h);
        after = Some(h);
    }
    out
}

/// Explorer process the DLL was last loaded into: never load it twice in one process
/// (the second copy would take the first one's 0 opacity for the original).
static LOADED_INTO: AtomicU32 = AtomicU32::new(0);

fn explorer_pid() -> Result<u32> {
    let mut pid = 0u32;
    unsafe {
        let tray = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null())?;
        GetWindowThreadProcessId(tray, Some(&mut pid));
    }
    Ok(pid)
}

/// Makes the taskbar transparent or gives it back its normal look.
pub fn set(transparent: bool) -> Result<()> {
    let mut hooks = hooks();
    if hooks.is_empty() {
        let pid = explorer_pid()?;
        if LOADED_INTO.load(Ordering::Relaxed) != pid {
            // Not loaded: nothing to undo, and loading it starts transparent.
            if transparent {
                inject(pid)?;
                LOADED_INTO.store(pid, Ordering::Relaxed);
            }
            return Ok(());
        }
        // Loaded a moment ago and still finding the taskbar: wait for its windows.
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            hooks = self::hooks();
            if !hooks.is_empty() {
                break;
            }
        }
    }
    for h in hooks {
        unsafe { PostMessageW(Some(h), MSG_SET, WPARAM(transparent as usize), LPARAM(0))? };
    }
    Ok(())
}

/// Explorer keeps the DLL loaded (and its file locked) until it exits, so it gets a
/// versioned copy in the data folder rather than the installed file: updates and
/// uninstalls are never blocked.
fn staged_dll() -> Result<PathBuf> {
    let source = std::env::current_exe()
        .map_err(|e| Error::new(windows::Win32::Foundation::E_FAIL, e.to_string()))?
        .with_file_name("customwin_taskbar.dll");
    let target = customwin_core::windows::data_dir().join(format!(
        "customwin_taskbar-{}.dll",
        env!("CARGO_PKG_VERSION")
    ));
    let size = |p: &PathBuf| std::fs::metadata(p).map(|m| m.len()).ok();
    if size(&source).is_none() {
        return Err(Error::new(
            windows::Win32::Foundation::E_FAIL,
            format!("{} is missing", source.display()),
        ));
    }
    if size(&source) != size(&target) {
        if let Some(dir) = target.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Locked means Explorer still uses an identical earlier copy: keep it.
        if let Err(e) = std::fs::copy(&source, &target) {
            if size(&target).is_none() {
                return Err(Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    e.to_string(),
                ));
            }
        }
    }
    Ok(target)
}

fn inject(pid: u32) -> Result<()> {
    let dll = HSTRING::from(staged_dll()?.as_os_str());
    unsafe {
        let xaml = LoadLibraryExW(
            w!("Windows.UI.Xaml.dll"),
            None,
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )?;
        let result = (|| {
            let f = GetProcAddress(xaml, s!("InitializeXamlDiagnosticsEx"))
                .ok_or_else(Error::from_thread)?;
            let f: InitializeXamlDiagnosticsEx = std::mem::transmute(f);
            let init = HSTRING::from(INIT_TRANSPARENT);
            let mut hr = HRESULT(0);
            // Each diagnostics client needs its own endpoint name; another tool (or a
            // previous Explorer session's leftovers) may hold the first ones.
            for i in 1..=8 {
                let name = HSTRING::from(format!("VisualDiagConnection{i}"));
                hr = f(
                    PCWSTR(name.as_ptr()),
                    pid,
                    PCWSTR::null(),
                    PCWSTR(dll.as_ptr()),
                    GUID::from_u128(CLSID),
                    PCWSTR(init.as_ptr()),
                );
                if hr.is_ok() || hr.0 as u32 != 0x8007_00B7 {
                    break; // anything but ERROR_ALREADY_EXISTS
                }
            }
            hr.ok()
        })();
        let _ = FreeLibrary(xaml);
        result
    }
}
