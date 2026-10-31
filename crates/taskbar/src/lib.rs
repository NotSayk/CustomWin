//! `customwin_taskbar.dll` — makes the Windows 11 taskbar transparent from inside Explorer.
//!
//! Since Windows 11 22H2 the taskbar is a XAML island that paints its own acrylic
//! background, so `SetWindowCompositionAttribute` on `Shell_TrayWnd` no longer changes
//! anything. Instead, `customwin-desktop.exe` asks Windows to load this DLL into Explorer
//! with `InitializeXamlDiagnosticsEx`, the documented API behind Visual Studio's Live
//! Visual Tree (interfaces from the SDK's `xamlOM.h`). The DLL watches the visual tree
//! and sets the opacity of the taskbar's two background rectangles (`BackgroundFill`,
//! `BackgroundStroke`) to 0; putting back their original opacity restores the look.
//!
//! Nothing is written to the registry or to Explorer's files: restarting Explorer undoes
//! everything. The code runs inside Explorer, and the release profile aborts on panic, so
//! it never unwraps.

/// CLSID of the extension object, passed to `InitializeXamlDiagnosticsEx`.
pub const CLSID: u128 = 0x8e1d3e5a_6c1b_4d3e_9a57_3c2b7a1f0c11;
/// Class of the message-only window the DLL creates on each taskbar UI thread.
pub const HOOK_CLASS: &str = "CustomWinTaskbarHook";
/// `WM_APP + 2`, posted to the hook windows: `wparam` 1 = transparent, 0 = normal.
pub const MSG_SET: u32 = 0x8000 + 2;
/// Initialization data meaning "start transparent".
pub const INIT_TRANSPARENT: &str = "transparent";

#[cfg(windows)]
mod tap;
