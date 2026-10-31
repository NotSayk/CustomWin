//! Process-level helpers: elevation, restarting Explorer, restore points.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    GetTokenInformation, TokenElevation, TokenUser, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::System::Restore::*;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, WaitForSingleObject, INFINITE,
};
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SEE_MASK_NO_CONSOLE, SHELLEXECUTEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

use super::{map_err, wide};
use crate::Error;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `%LOCALAPPDATA%\CustomWin`.
pub fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("CustomWin")
}

pub fn journal_path() -> PathBuf {
    data_dir().join("journal.json")
}

/// SID of the account running this process, e.g. `S-1-5-21-…-1001`.
pub fn current_user_sid() -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        // u64 storage keeps TOKEN_USER aligned.
        let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            len,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        if !ok {
            return None;
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut text = windows::core::PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
        let sid = text.to_string().ok();
        LocalFree(Some(HLOCAL(text.0.cast())));
        sid
    }
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut len = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as _),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// Starts `exe` elevated (UAC prompt) and waits for it. Returns its exit code.
pub fn run_elevated(exe: &Path, args: &str) -> Result<u32, Error> {
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let params = wide(args);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NO_CONSOLE,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe {
        if let Err(e) = ShellExecuteExW(&mut info) {
            // ERROR_CANCELLED: the user said no to the UAC prompt.
            if e.code().0 as u32 == 0x8007_04C7 {
                return Err(Error::AccessDenied(
                    "administrator rights were declined".into(),
                ));
            }
            return Err(map_err("helper", e));
        }
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code = 0u32;
        let r = GetExitCodeProcess(info.hProcess, &mut code);
        let _ = CloseHandle(info.hProcess);
        r.map_err(|e| map_err("helper", e))?;
        Ok(code)
    }
}

fn explorer_running() -> bool {
    Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq explorer.exe", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| {
            String::from_utf8_lossy(&o.stdout)
                .to_ascii_lowercase()
                .contains("explorer.exe")
        })
}

/// Restarts the shell so taskbar and context-menu tweaks show up. Must run unelevated:
/// an Explorer started from an elevated process would itself be elevated.
pub fn restart_explorer() -> Result<(), Error> {
    let io = |e: std::io::Error| Error::System(format!("explorer: {e}"));
    Command::new("taskkill")
        .args(["/F", "/IM", "explorer.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(io)?;
    // Winlogon usually relaunches the shell by itself; only start it if it did not.
    for _ in 0..10 {
        std::thread::sleep(std::time::Duration::from_millis(300));
        if explorer_running() {
            return Ok(());
        }
    }
    Command::new("explorer.exe").spawn().map_err(io)?;
    Ok(())
}

/// Opens a URL in the default browser.
pub fn open_url(url: &str) -> Result<(), Error> {
    let verb = wide("open");
    let file = wide(url);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        nShow: windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| map_err("browser", e))
}

/// Creates a System Restore point. Needs administrator rights. Windows skips it silently
/// if another point was created in the last 24 hours, which still counts as success.
pub fn create_restore_point(description: &str) -> Result<(), Error> {
    // The struct is packed: build the description apart, then move it in.
    let mut text = [0u16; 256];
    for (dst, src) in text.iter_mut().zip(description.encode_utf16().take(255)) {
        *dst = src;
    }
    let mut info = RESTOREPOINTINFOW {
        dwEventType: BEGIN_SYSTEM_CHANGE,
        dwRestorePtType: MODIFY_SETTINGS,
        llSequenceNumber: 0,
        szDescription: text,
    };
    let mut status = STATEMGRSTATUS::default();
    unsafe {
        if !SRSetRestorePointW(&info, &mut status).as_bool() {
            return Err(match status.nStatus.0 {
                // ERROR_SERVICE_DISABLED
                1058 => Error::Unavailable("System Restore is turned off".into()),
                code => Error::System(format!("restore point failed ({code})")),
            });
        }
        info.dwEventType = END_SYSTEM_CHANGE;
        info.llSequenceNumber = status.llSequenceNumber;
        let _ = SRSetRestorePointW(&info, &mut status);
    }
    Ok(())
}
