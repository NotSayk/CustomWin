//! Tiny append-only log in `%LOCALAPPDATA%\CustomWin\desktop.log`, to diagnose a
//! wallpaper that does not show up. Reset at each start once it passes 256 KB.

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

fn path() -> std::path::PathBuf {
    customwin_core::windows::data_dir().join("desktop.log")
}

pub fn start() {
    let p = path();
    if std::fs::metadata(&p).is_ok_and(|m| m.len() > 256 * 1024) {
        let _ = std::fs::remove_file(&p);
    }
    write(&format!(
        "--- customwin-desktop {} started",
        env!("CARGO_PKG_VERSION")
    ));
}

pub fn write(msg: &str) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(p)
    {
        let _ = writeln!(f, "[{secs}] {msg}");
    }
}
