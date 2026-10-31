//! Main loop, windows and pause logic.
//!
//! When nothing plays, the thread sleeps in `WaitMessage` (0 % CPU, decoders paused).
//! When something plays, it wakes once per display refresh, asks each video for a new
//! frame and copies the new ones to their screens on the GPU. Widgets add a one-second
//! timer, stopped whenever none of them can be seen.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    GetSystemPowerStatus, RegisterPowerSettingNotification, POWERBROADCAST_SETTING,
    SYSTEM_POWER_STATUS,
};
use windows::Win32::System::ProcessStatus::K32EmptyWorkingSet;
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::render::{Gpu, Surface, Video};
use crate::shell::{self, Host};
use crate::widgets::{Content, Painter, Sampler, WidgetWindow};
use crate::{log, taskbar, watch, widgets};
use customwin_core::wallpaper::{self, Fit, Layout, Rect, WallpaperConfig};
use customwin_core::widgets::{WidgetKind, WidgetsConfig};
use customwin_core::windows as win;

static RELOAD: AtomicBool = AtomicBool::new(false);
static QUIT: AtomicBool = AtomicBool::new(false);
static LOCKED: AtomicBool = AtomicBool::new(false);
static DISPLAY_OFF: AtomicBool = AtomicBool::new(false);
static UNLOAD: AtomicBool = AtomicBool::new(false);
static TICK: AtomicBool = AtomicBool::new(false);
static TASKBAR_RETRY: AtomicBool = AtomicBool::new(false);
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

const WALL_CLASS: PCWSTR = w!("CustomWinWallpaper");
const RETRY_TIMER: usize = 1;
const UNLOAD_TIMER: usize = 2;
const WIDGET_TIMER: usize = 3;
const TASKBAR_TIMER: usize = 4;
/// How long every wallpaper must stay paused before its decoder is released.
const UNLOAD_AFTER_MS: u32 = 60_000;

struct Placed {
    surface: Surface,
    video: PathBuf,
    fit: Fit,
    devices: Vec<String>,
    active: bool,
}

struct App {
    gpu: Gpu,
    windowed: bool,
    control: HWND,
    config: WallpaperConfig,
    videos: HashMap<PathBuf, Video>,
    placed: Vec<Placed>,
    attached: bool,
    widgets_config: WidgetsConfig,
    widgets: Vec<WidgetWindow>,
    painter: Option<Painter>,
    sampler: Sampler,
    /// The taskbar was made transparent and must be given back its look on exit.
    taskbar_on: bool,
    taskbar_tries: u32,
}

pub fn run(windowed: bool) -> windows_core::Result<()> {
    unsafe {
        // One instance: a second launch just asks the first one to reload.
        let _mutex = CreateMutexW(None, true, w!("Local\\CustomWinDesktop"))?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            win::desktop_reload();
            return Ok(());
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        log::start();

        let instance = GetModuleHandleW(None)?.into();
        let control_class = HSTRING::from(wallpaper::CONTROL_CLASS);
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(control_proc),
            hInstance: instance,
            lpszClassName: PCWSTR(control_class.as_ptr()),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(wall_proc),
            hInstance: instance,
            lpszClassName: WALL_CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            ..Default::default()
        });
        // Hidden top-level window (not message-only: it must get broadcasts such as
        // TaskbarCreated and WM_DISPLAYCHANGE).
        let control = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR(control_class.as_ptr()),
            w!("CustomWin Desktop"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )?;
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(w!("TaskbarCreated")),
            Ordering::Relaxed,
        );
        let _ = WTSRegisterSessionNotification(control, NOTIFY_FOR_THIS_SESSION);
        let _ = RegisterPowerSettingNotification(
            HANDLE(control.0),
            &GUID_CONSOLE_DISPLAY_STATE,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        watch::install_hooks();

        let gpu = Gpu::new().inspect_err(|e| log::write(&format!("GPU init failed: {e}")))?;
        let mut app = App {
            gpu,
            windowed,
            control,
            config: WallpaperConfig::default(),
            videos: HashMap::new(),
            placed: Vec::new(),
            attached: false,
            widgets_config: WidgetsConfig::default(),
            widgets: Vec::new(),
            painter: None,
            sampler: Sampler::new(),
            taskbar_on: false,
            taskbar_tries: 0,
        };
        app.rebuild();

        let mut msg = MSG::default();
        'main: loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    break 'main;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if QUIT.load(Ordering::Relaxed) {
                break;
            }
            if RELOAD.swap(false, Ordering::Relaxed) || app.lost_windows() {
                app.rebuild();
                continue;
            }
            if watch::CHANGED.swap(false, Ordering::Relaxed) {
                app.update_pauses();
            }
            if UNLOAD.swap(false, Ordering::Relaxed) {
                app.unload_paused();
            }
            if TICK.swap(false, Ordering::Relaxed) {
                app.tick_widgets();
            }
            if TASKBAR_RETRY.swap(false, Ordering::Relaxed) {
                app.sync_taskbar();
            }
            if app.placed.iter().any(|p| p.active) {
                app.gpu.wait_vblank();
                app.render();
            } else {
                let _ = WaitMessage();
            }
        }
        app.teardown();
        log::write("stopped");
        Ok(())
    }
}

impl App {
    fn destroy_windows(&mut self) {
        let walls = self.placed.drain(..).map(|p| p.surface.hwnd);
        for hwnd in walls.chain(self.widgets.drain(..).map(|w| w.hwnd)) {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }

    fn teardown(&mut self) {
        self.destroy_windows();
        self.videos.clear();
        if std::mem::take(&mut self.attached) {
            shell::repaint_static_wallpaper();
        }
        if self.taskbar_on {
            if let Err(e) = taskbar::set(false) {
                log::write(&format!("taskbar: {e}"));
            }
        }
    }

    /// Explorer restarted (or crashed): our child windows went with it.
    fn lost_windows(&self) -> bool {
        let walls = self.placed.iter().map(|p| p.surface.hwnd);
        walls
            .chain(self.widgets.iter().map(|w| w.hwnd))
            .any(|h| !watch::alive(h))
    }

    fn rebuild(&mut self) {
        self.destroy_windows();
        self.config = WallpaperConfig::load(&win::wallpaper_config_path()).unwrap_or_else(|e| {
            log::write(&format!("config: {e}"));
            WallpaperConfig::default()
        });
        self.widgets_config =
            WidgetsConfig::load(&win::widgets_config_path()).unwrap_or_else(|e| {
                log::write(&format!("widgets: {e}"));
                WidgetsConfig::default()
            });
        let play = self.config.enabled && self.config.has_video();
        let show_widgets = !self.windowed && self.widgets_config.enabled().next().is_some();
        self.sync_taskbar();
        if !play && !show_widgets && !self.widgets_config.transparent_taskbar {
            log::write("nothing to do: exiting");
            QUIT.store(true, Ordering::Relaxed);
            return;
        }
        if !play {
            self.videos.clear();
        }
        if !play && !show_widgets {
            return; // the taskbar alone needs no window
        }

        let host = if self.windowed {
            Host::Windowed
        } else {
            match shell::find_host() {
                Some(h) => h,
                None => {
                    // Typically at sign-in, before Explorer is ready: retry in 2 s.
                    unsafe { SetTimer(Some(self.control), RETRY_TIMER, 2000, None) };
                    return;
                }
            }
        };
        unsafe {
            let _ = KillTimer(Some(self.control), RETRY_TIMER);
        }

        let plan = if play { self.plan() } else { Vec::new() };
        log::write(&format!("{} wallpaper surface(s)", plan.len()));
        self.videos
            .retain(|path, _| plan.iter().any(|(_, v, _, _)| v == path));
        for (rect, video, fit, devices) in plan {
            if !self.videos.contains_key(&video) {
                match Video::open(&self.gpu, &video) {
                    Ok(v) => {
                        self.videos.insert(video.clone(), v);
                    }
                    Err(e) => {
                        log::write(&format!("cannot open {}: {e}", video.display()));
                        continue;
                    }
                }
            }
            match self.create_surface(host, rect) {
                Ok(surface) => self.placed.push(Placed {
                    surface,
                    video,
                    fit,
                    devices,
                    active: false,
                }),
                Err(e) => log::write(&format!("cannot create the wallpaper window: {e}")),
            }
        }
        if show_widgets {
            self.build_widgets(host);
        }
        self.attached = !self.windowed && !(self.placed.is_empty() && self.widgets.is_empty());
        watch::CHANGED.store(true, Ordering::Relaxed);
    }

    /// Widget windows go in after the wallpaper ones: right below the icons, above the
    /// video.
    fn build_widgets(&mut self, host: Host) {
        match &mut self.painter {
            Some(p) => p.accent = win::theme_state().accent,
            None => match Painter::new(&self.gpu) {
                Ok(p) => self.painter = Some(p),
                Err(e) => {
                    log::write(&format!("widgets: Direct2D unavailable: {e}"));
                    return;
                }
            },
        }
        for place in widgets::layout(&self.widgets_config, &win::monitors()) {
            let hwnd = match self.create_window(host, place.rect) {
                Ok(h) => h,
                Err(e) => {
                    log::write(&format!("cannot create a widget window: {e}"));
                    continue;
                }
            };
            match self
                .gpu
                .transparent_chain(hwnd, place.rect.width(), place.rect.height())
            {
                Ok(Some(chain)) => self.widgets.push(WidgetWindow::new(hwnd, &place, chain)),
                other => {
                    match other {
                        Err(e) => log::write(&format!("widget surface: {e}")),
                        _ => log::write("widgets need DirectComposition"),
                    }
                    unsafe {
                        let _ = DestroyWindow(hwnd);
                    }
                }
            }
        }
        log::write(&format!("{} widget(s)", self.widgets.len()));
    }

    /// Redraws the visible widgets whose text changed.
    fn tick_widgets(&mut self) {
        let Some(painter) = &self.painter else {
            return;
        };
        if self
            .widgets
            .iter()
            .any(|w| w.active && w.kind == WidgetKind::System)
        {
            self.sampler.refresh_cpu();
        }
        let mut contents: HashMap<WidgetKind, Content> = HashMap::new();
        for w in self.widgets.iter_mut().filter(|w| w.active) {
            let content = contents
                .entry(w.kind)
                .or_insert_with(|| self.sampler.content(w.kind))
                .clone();
            if let Err(e) = painter.update(w, content) {
                log::write(&format!("widget draw: {e}"));
            }
        }
    }

    /// Applies `transparent_taskbar`. Explorer may not be ready yet (sign-in, restart):
    /// a few retries then.
    fn sync_taskbar(&mut self) {
        if self.windowed {
            return;
        }
        let want = self.widgets_config.transparent_taskbar;
        if !want && !self.taskbar_on {
            return;
        }
        unsafe {
            let _ = KillTimer(Some(self.control), TASKBAR_TIMER);
        }
        match taskbar::set(want) {
            Ok(()) => {
                if want != self.taskbar_on {
                    log::write(if want {
                        "taskbar: transparent"
                    } else {
                        "taskbar: normal"
                    });
                }
                self.taskbar_on = want;
                self.taskbar_tries = 0;
            }
            Err(e) => {
                log::write(&format!("taskbar: {e}"));
                if want && self.taskbar_tries < 5 {
                    self.taskbar_tries += 1;
                    unsafe { SetTimer(Some(self.control), TASKBAR_TIMER, 3000, None) };
                }
            }
        }
    }

    /// (screen rect, video, fit, monitors covered) for each window to create.
    fn plan(&self) -> Vec<(Rect, PathBuf, Fit, Vec<String>)> {
        let monitors = win::monitors();
        let mut plan = Vec::new();
        match self.config.layout {
            Layout::PerScreen => {
                for m in &monitors {
                    let Some(sc) = self.config.screen(&m.device) else {
                        continue;
                    };
                    let Some(video) = sc.video.clone() else {
                        continue;
                    };
                    if video.exists() {
                        plan.push((m.rect, video, sc.fit, vec![m.device.clone()]));
                    } else {
                        log::write(&format!("video not found: {}", video.display()));
                    }
                }
            }
            Layout::Span => {
                if let (Some(video), Some(first)) =
                    (self.config.span_video.clone(), monitors.first())
                {
                    let all = monitors
                        .iter()
                        .fold(first.rect, |acc, m| acc.union(&m.rect));
                    let devices = monitors.iter().map(|m| m.device.clone()).collect();
                    if video.exists() {
                        plan.push((all, video, self.config.span_fit, devices));
                    } else {
                        log::write(&format!("video not found: {}", video.display()));
                    }
                }
            }
        }
        if self.windowed {
            // Preview: the first wallpaper only, in a 960×540 window.
            plan.truncate(1);
            for p in &mut plan {
                p.0 = Rect {
                    left: 100,
                    top: 100,
                    right: 1060,
                    bottom: 640,
                };
            }
        }
        plan
    }

    fn create_surface(&self, host: Host, rect: Rect) -> windows_core::Result<Surface> {
        let hwnd = self.create_window(host, rect)?;
        unsafe {
            let mut client = RECT::default();
            GetClientRect(hwnd, &mut client)?;
            Surface::new(
                &self.gpu,
                hwnd,
                client.right - client.left,
                client.bottom - client.top,
            )
            .inspect_err(|_| {
                let _ = DestroyWindow(hwnd);
            })
        }
    }

    /// A window moved into the wallpaper layer, covering `rect` (screen coordinates).
    fn create_window(&self, host: Host, rect: Rect) -> windows_core::Result<HWND> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let (style, mut ex) = if self.windowed {
                (WS_OVERLAPPEDWINDOW | WS_VISIBLE, WS_EX_NOREDIRECTIONBITMAP)
            } else {
                (
                    WS_POPUP,
                    WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                )
            };
            if self.gpu.needs_redirection() {
                ex &= !WS_EX_NOREDIRECTIONBITMAP;
            }
            let hwnd = CreateWindowExW(
                ex,
                WALL_CLASS,
                w!("CustomWin — fond animé"),
                style,
                rect.left,
                rect.top,
                rect.width(),
                rect.height(),
                None,
                None,
                Some(instance.into()),
                None,
            )?;
            shell::attach(hwnd, host, rect);
            Ok(hwnd)
        }
    }

    fn update_pauses(&mut self) {
        let fg = watch::foreground();
        let fg = wallpaper::Foreground {
            device: fg.device,
            fullscreen: fg.fullscreen,
            maximized: fg.maximized,
        };
        let global = LOCKED.load(Ordering::Relaxed)
            || DISPLAY_OFF.load(Ordering::Relaxed)
            || (self.config.pause_on_battery && on_battery());
        for p in &mut self.placed {
            p.active =
                wallpaper::should_play(&p.devices, &fg, self.config.pause_when_maximized, global);
        }
        // Widgets cost almost nothing: only a lock, a dark screen or a window over them
        // stops them, not the battery.
        let hidden = LOCKED.load(Ordering::Relaxed) || DISPLAY_OFF.load(Ordering::Relaxed);
        for w in &mut self.widgets {
            w.active = wallpaper::should_play(std::slice::from_ref(&w.device), &fg, true, hidden);
        }
        unsafe {
            if self.widgets.iter().any(|w| w.active) {
                SetTimer(Some(self.control), WIDGET_TIMER, 1000, None);
                TICK.store(true, Ordering::Relaxed);
            } else {
                let _ = KillTimer(Some(self.control), WIDGET_TIMER);
            }
        }
        for (path, video) in &mut self.videos {
            let play = self.placed.iter().any(|p| p.active && p.video == *path);
            video.set_playing(&self.gpu, play);
        }
        // Any change restarts the countdown; it only runs out after a quiet minute of pause.
        unsafe {
            if self.videos.values().any(|v| !v.playing()) {
                SetTimer(Some(self.control), UNLOAD_TIMER, UNLOAD_AFTER_MS, None);
            } else {
                let _ = KillTimer(Some(self.control), UNLOAD_TIMER);
            }
        }
    }

    /// Frees the decoders of videos that have been paused for a while (game, lock screen…)
    /// and hands the memory back to Windows. The windows keep their last image.
    fn unload_paused(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.control), UNLOAD_TIMER);
        }
        let freed = self.videos.values_mut().fold(false, |f, v| v.unload() | f);
        if freed {
            log::write("paused for a while: decoder released");
            unsafe {
                let _ = K32EmptyWorkingSet(GetCurrentProcess());
            }
        }
    }

    fn render(&mut self) {
        let fresh: Vec<&PathBuf> = self
            .videos
            .iter()
            .filter(|(_, v)| v.playing() && v.new_frame())
            .map(|(p, _)| p)
            .collect();
        for p in self
            .placed
            .iter()
            .filter(|p| p.active && fresh.contains(&&p.video))
        {
            if let Some(video) = self.videos.get(&p.video) {
                if let Err(e) = p.surface.draw(&self.gpu, video, p.fit) {
                    log::write(&format!("draw: {e}"));
                }
            }
        }
    }
}

fn on_battery() -> bool {
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut s) }.is_ok() && s.ACLineStatus == 0
}

unsafe extern "system" fn control_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TIMER if wparam.0 == UNLOAD_TIMER => UNLOAD.store(true, Ordering::Relaxed),
        WM_TIMER if wparam.0 == WIDGET_TIMER => TICK.store(true, Ordering::Relaxed),
        WM_TIMER if wparam.0 == TASKBAR_TIMER => TASKBAR_RETRY.store(true, Ordering::Relaxed),
        WM_TIMECHANGE => TICK.store(true, Ordering::Relaxed),
        wallpaper::MSG_RELOAD | WM_DISPLAYCHANGE | WM_TIMER => {
            RELOAD.store(true, Ordering::Relaxed)
        }
        WM_POWERBROADCAST => {
            if wparam.0 as u32 == PBT_POWERSETTINGCHANGE {
                let setting = unsafe { &*(lparam.0 as *const POWERBROADCAST_SETTING) };
                if setting.PowerSetting == GUID_CONSOLE_DISPLAY_STATE {
                    // 0 = off, 1 = on, 2 = dimmed.
                    DISPLAY_OFF.store(setting.Data[0] == 0, Ordering::Relaxed);
                }
            }
            watch::CHANGED.store(true, Ordering::Relaxed);
            return LRESULT(1);
        }
        WM_WTSSESSION_CHANGE => {
            match wparam.0 as u32 {
                WTS_SESSION_LOCK => LOCKED.store(true, Ordering::Relaxed),
                WTS_SESSION_UNLOCK => LOCKED.store(false, Ordering::Relaxed),
                _ => {}
            }
            watch::CHANGED.store(true, Ordering::Relaxed);
        }
        WM_CLOSE | WM_ENDSESSION => QUIT.store(true, Ordering::Relaxed),
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => {
            log::write("Explorer restarted: reattaching");
            RELOAD.store(true, Ordering::Relaxed);
        }
        _ => return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
    LRESULT(0)
}

unsafe extern "system" fn wall_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // Only the --windowed preview can be closed.
        WM_CLOSE => {
            QUIT.store(true, Ordering::Relaxed);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
