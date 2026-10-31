//! Windows-only implementation: the real system backend, hardware probes and process
//! helpers.

mod appx;
mod backend;
mod desktop;
mod nvapi;
mod online;
mod process;
mod sysinfo;

pub use backend::WindowsBackend;
pub use desktop::{
    desktop_reload, desktop_running, desktop_start, desktop_stop, monitors, pick_video,
    set_autostart, wallpaper_config_path, Monitor,
};
pub use online::check_nvidia_drivers;
pub use process::{
    create_restore_point, current_user_sid, data_dir, is_elevated, journal_path, open_url,
    restart_explorer, run_elevated,
};
pub use sysinfo::collect_system_info;

use crate::Error;

/// NUL-terminated UTF-16.
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub(crate) fn map_err(context: &str, e: windows::core::Error) -> Error {
    match e.code().0 as u32 {
        0x8007_0005 => Error::AccessDenied(context.to_string()),
        0x8007_0002 | 0x8007_0003 => Error::Unavailable(format!("{context} not found")),
        _ => Error::System(format!("{context}: {}", e.message())),
    }
}
