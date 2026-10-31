//! `customwin-desktop.exe` — the animated wallpaper.
//!
//! Started by the CustomWin app (or at sign-in), reads
//! `%LOCALAPPDATA%\CustomWin\wallpaper.json`, and exits on its own when there is
//! nothing to play. `--windowed` shows the first wallpaper in a normal window instead of
//! the desktop, to check video decoding on its own. Diagnostics go to `desktop.log`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod log;
#[cfg(windows)]
mod render;
#[cfg(windows)]
mod shell;
#[cfg(windows)]
mod watch;

#[cfg(windows)]
fn main() {
    let windowed = std::env::args().any(|a| a == "--windowed");
    if let Err(e) = app::run(windowed) {
        log::write(&format!("fatal: {e}"));
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("customwin-desktop only runs on Windows");
    std::process::exit(1);
}
