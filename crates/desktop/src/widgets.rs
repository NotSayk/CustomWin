//! Desktop widgets: small transparent windows on the wallpaper layer (behind the icons),
//! drawn with Direct2D on the wallpaper's GPU device.
//!
//! A one-second timer samples the data; a widget is only redrawn when its text changes,
//! and not sampled at all while a game or a maximized app covers its screen, the PC is
//! locked or the display is off.

use std::mem::ManuallyDrop;

use windows::core::{w, PCWSTR};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSessionManager as MediaManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Win32::Foundation::{FILETIME, HWND, RECT};
use windows::Win32::Globalization::{
    GetDateFormatEx, GetTimeFormatEx, GetUserDefaultUILanguage, DATE_LONGDATE, TIME_NOSECONDS,
};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{IDXGISurface, DXGI_PRESENT};
use windows::Win32::Graphics::Gdi::{MonitorFromRect, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::SystemInformation::{
    GetLocalTime, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows::Win32::System::Threading::GetSystemTimes;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_core::Result;

use crate::render::{Gpu, TransparentChain};
use customwin_core::wallpaper::Rect;
use customwin_core::widgets::{self, Corner, WidgetKind, WidgetsConfig};
use customwin_core::windows::Monitor;

/// What a widget shows; drawing is skipped when it did not change.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Clock {
        time: String,
        date: String,
    },
    System {
        cpu: u32,
        ram: u32,
        ram_text: String,
    },
    /// `None` when nothing plays: the widget is then invisible.
    Media(Option<(String, String)>),
}

pub struct Placement {
    pub kind: WidgetKind,
    pub device: String,
    /// Physical pixels, screen coordinates.
    pub rect: Rect,
    pub dpi: f32,
}

fn dpi_of(rect: &Rect) -> f32 {
    let r = RECT {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    };
    let (mut x, mut y) = (96u32, 96u32);
    unsafe {
        let m = MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST);
        let _ = GetDpiForMonitor(m, MDT_EFFECTIVE_DPI, &mut x, &mut y);
    }
    x.max(96) as f32
}

/// Where each enabled widget goes. A widget set on a screen that is gone moves to the
/// main one.
pub fn layout(config: &WidgetsConfig, monitors: &[Monitor]) -> Vec<Placement> {
    let Some(primary) = monitors.iter().find(|m| m.primary).or(monitors.first()) else {
        return Vec::new();
    };
    let screen_of = |device: &Option<String>| {
        device
            .as_ref()
            .and_then(|d| monitors.iter().find(|m| m.device.eq_ignore_ascii_case(d)))
            .unwrap_or(primary)
    };
    let mut out = Vec::new();
    for m in monitors {
        let dpi = dpi_of(&m.rect);
        let px = |v: i32| (v as f32 * dpi / 96.0).round() as i32;
        for corner in [
            Corner::TopLeft,
            Corner::TopRight,
            Corner::BottomLeft,
            Corner::BottomRight,
        ] {
            let kinds: Vec<WidgetKind> = config
                .enabled()
                .filter(|w| w.corner == corner && screen_of(&w.device).device == m.device)
                .map(|w| w.kind)
                .collect();
            let sizes: Vec<(i32, i32)> = kinds
                .iter()
                .map(|k| (px(k.size().0), px(k.size().1)))
                .collect();
            let rects = widgets::stack(
                m.work,
                corner,
                &sizes,
                px(widgets::MARGIN),
                px(widgets::GAP),
            );
            for (kind, rect) in kinds.into_iter().zip(rects) {
                out.push(Placement {
                    kind,
                    device: m.device.clone(),
                    rect,
                    dpi,
                });
            }
        }
    }
    out
}

pub struct WidgetWindow {
    pub hwnd: HWND,
    pub kind: WidgetKind,
    pub device: String,
    pub active: bool,
    chain: TransparentChain,
    dpi: f32,
    shown: Option<Content>,
}

impl WidgetWindow {
    pub fn new(hwnd: HWND, place: &Placement, chain: TransparentChain) -> Self {
        WidgetWindow {
            hwnd,
            kind: place.kind,
            device: place.device.clone(),
            active: false,
            chain,
            dpi: place.dpi,
            shown: None,
        }
    }
}

fn french() -> bool {
    // LANG_FRENCH, whatever the country.
    let lang = unsafe { GetUserDefaultUILanguage() };
    lang & 0x3FF == 0x0C
}

fn color(rgb: u32, a: f32) -> D2D1_COLOR_F {
    let c = |shift: u32| ((rgb >> shift) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F {
        r: c(16),
        g: c(8),
        b: c(0),
        a,
    }
}

fn rect(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: l,
        top: t,
        right: r,
        bottom: b,
    }
}

struct Formats {
    time: IDWriteTextFormat,
    date: IDWriteTextFormat,
    title: IDWriteTextFormat,
    label: IDWriteTextFormat,
    value: IDWriteTextFormat,
    icon: IDWriteTextFormat,
}

pub struct Painter {
    ctx: ID2D1DeviceContext,
    brush: ID2D1SolidColorBrush,
    text: Formats,
    /// `0xRRGGBB`, refreshed on every reload.
    pub accent: u32,
    french: bool,
}

impl Painter {
    pub fn new(gpu: &Gpu) -> Result<Self> {
        unsafe {
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let device = factory.CreateDevice(&gpu.dxgi_device()?)?;
            let ctx = device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            let brush = ctx.CreateSolidColorBrush(&color(0xFFFFFF, 1.0), None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let format = |family: PCWSTR, size: f32, weight: DWRITE_FONT_WEIGHT, trailing: bool| {
                let f = dw.CreateTextFormat(
                    family,
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!(""),
                )?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                let ellipsis = dw.CreateEllipsisTrimmingSign(&f)?;
                f.SetTrimming(
                    &DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        delimiter: 0,
                        delimiterCount: 0,
                    },
                    &ellipsis,
                )?;
                if trailing {
                    f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
                }
                Ok::<_, windows_core::Error>(f)
            };
            let ui = w!("Segoe UI Variable Display");
            let text = Formats {
                time: format(ui, 54.0, DWRITE_FONT_WEIGHT_LIGHT, false)?,
                date: format(ui, 15.0, DWRITE_FONT_WEIGHT_NORMAL, false)?,
                title: format(ui, 15.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, false)?,
                label: format(ui, 13.0, DWRITE_FONT_WEIGHT_NORMAL, false)?,
                value: format(ui, 13.0, DWRITE_FONT_WEIGHT_SEMI_BOLD, true)?,
                icon: format(
                    w!("Segoe UI Symbol"),
                    20.0,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    false,
                )?,
            };
            Ok(Painter {
                ctx,
                brush,
                text,
                accent: customwin_core::windows::theme_state().accent,
                french: french(),
            })
        }
    }

    /// Draws `content` if it changed since the last time.
    pub fn update(&self, w: &mut WidgetWindow, content: Content) -> Result<()> {
        if w.shown.as_ref() == Some(&content) {
            return Ok(());
        }
        self.draw(w, &content)?;
        w.shown = Some(content);
        Ok(())
    }

    fn text(&self, s: &str, format: &IDWriteTextFormat, r: D2D_RECT_F, rgb: u32, a: f32) {
        let s: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            self.brush.SetColor(&color(rgb, a));
            self.ctx.DrawText(
                &s,
                format,
                &r,
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    fn fill(&self, r: D2D_RECT_F, radius: f32, rgb: u32, a: f32) {
        unsafe {
            self.brush.SetColor(&color(rgb, a));
            self.ctx.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: r,
                    radiusX: radius,
                    radiusY: radius,
                },
                &self.brush,
            );
        }
    }

    /// The translucent card every widget sits on.
    fn card(&self, width: f32, height: f32) {
        self.fill(rect(0.0, 0.0, width, height), 14.0, 0x101014, 0.55);
        unsafe {
            self.brush.SetColor(&color(0xFFFFFF, 0.10));
            self.ctx.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: rect(0.5, 0.5, width - 0.5, height - 0.5),
                    radiusX: 14.0,
                    radiusY: 14.0,
                },
                &self.brush,
                1.0,
                None,
            );
        }
    }

    fn bar(&self, top: f32, width: f32, percent: u32) {
        let (left, right) = (20.0, width - 20.0);
        self.fill(rect(left, top, right, top + 6.0), 3.0, 0xFFFFFF, 0.15);
        let filled = left + (right - left) * percent.min(100) as f32 / 100.0;
        // The accent colour, turning orange then red as the load climbs.
        let rgb = match percent {
            0..=74 => self.accent,
            75..=89 => 0xF7630C,
            _ => 0xE81123,
        };
        if filled > left + 6.0 {
            self.fill(rect(left, top, filled, top + 6.0), 3.0, rgb, 1.0);
        }
    }

    fn draw(&self, w: &WidgetWindow, content: &Content) -> Result<()> {
        let (width, height) = (w.kind.size().0 as f32, w.kind.size().1 as f32);
        unsafe {
            let surface: IDXGISurface = w.chain.swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: w.dpi,
                dpiY: w.dpi,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: ManuallyDrop::new(None),
            };
            let target = self
                .ctx
                .CreateBitmapFromDxgiSurface(&surface, Some(&props))?;
            self.ctx.SetTarget(&target);
            self.ctx.SetDpi(w.dpi, w.dpi);
            self.ctx.BeginDraw();
            self.ctx.Clear(Some(&color(0, 0.0)));
            match content {
                Content::Clock { time, date } => {
                    self.card(width, height);
                    self.text(
                        time,
                        &self.text.time,
                        rect(20.0, 6.0, width - 20.0, 80.0),
                        0xFFFFFF,
                        1.0,
                    );
                    self.text(
                        date,
                        &self.text.date,
                        rect(22.0, 80.0, width - 20.0, 104.0),
                        0xFFFFFF,
                        0.8,
                    );
                }
                Content::System { cpu, ram, ram_text } => {
                    self.card(width, height);
                    let (cpu_label, ram_label) = if self.french {
                        ("Processeur", "Mémoire")
                    } else {
                        ("Processor", "Memory")
                    };
                    let right = width - 20.0;
                    self.text(
                        cpu_label,
                        &self.text.label,
                        rect(20.0, 12.0, right, 32.0),
                        0xFFFFFF,
                        0.75,
                    );
                    self.text(
                        &format!("{cpu} %"),
                        &self.text.value,
                        rect(20.0, 12.0, right, 32.0),
                        0xFFFFFF,
                        1.0,
                    );
                    self.bar(36.0, width, *cpu);
                    self.text(
                        ram_label,
                        &self.text.label,
                        rect(20.0, 54.0, right, 74.0),
                        0xFFFFFF,
                        0.75,
                    );
                    self.text(
                        ram_text,
                        &self.text.value,
                        rect(20.0, 54.0, right, 74.0),
                        0xFFFFFF,
                        1.0,
                    );
                    self.bar(78.0, width, *ram);
                }
                Content::Media(Some((title, artist))) => {
                    self.card(width, height);
                    self.fill(rect(16.0, 16.0, 60.0, 60.0), 10.0, self.accent, 1.0);
                    self.text(
                        "\u{266A}",
                        &self.text.icon,
                        rect(29.0, 22.0, 60.0, 56.0),
                        0xFFFFFF,
                        1.0,
                    );
                    self.text(
                        title,
                        &self.text.title,
                        rect(74.0, 15.0, width - 18.0, 37.0),
                        0xFFFFFF,
                        1.0,
                    );
                    self.text(
                        artist,
                        &self.text.label,
                        rect(74.0, 40.0, width - 18.0, 60.0),
                        0xFFFFFF,
                        0.7,
                    );
                }
                Content::Media(None) => {}
            }
            let end = self.ctx.EndDraw(None, None);
            self.ctx.SetTarget(None::<&ID2D1Image>);
            end?;
            w.chain.swap.Present(0, DXGI_PRESENT(0)).ok()
        }
    }
}

/// Reads what the widgets show. Values are sampled once per tick, whatever the number
/// of widgets.
#[derive(Default)]
pub struct Sampler {
    cpu_times: Option<(u64, u64, u64)>,
    cpu: u32,
    media: Option<MediaManager>,
    media_failed: bool,
    french: bool,
}

fn ticks(f: FILETIME) -> u64 {
    (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

impl Sampler {
    pub fn new() -> Self {
        Sampler {
            french: french(),
            ..Default::default()
        }
    }

    /// Call once per tick before [`Sampler::content`].
    pub fn refresh_cpu(&mut self) {
        let (mut idle, mut kernel, mut user) = Default::default();
        if unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.is_ok() {
            let now = (ticks(idle), ticks(kernel), ticks(user));
            if let Some(before) = self.cpu_times {
                self.cpu = widgets::cpu_percent(before, now);
            }
            self.cpu_times = Some(now);
        }
    }

    pub fn content(&mut self, kind: WidgetKind) -> Content {
        match kind {
            WidgetKind::Clock => self.clock(),
            WidgetKind::System => self.system(),
            WidgetKind::Media => Content::Media(self.media()),
        }
    }

    fn clock(&self) -> Content {
        let mut buf = [0u16; 128];
        let time = unsafe {
            GetTimeFormatEx(
                PCWSTR::null(),
                TIME_NOSECONDS,
                None,
                PCWSTR::null(),
                Some(&mut buf),
            )
        };
        let time = String::from_utf16_lossy(&buf[..(time.max(1) - 1) as usize]);
        let n = unsafe {
            GetDateFormatEx(
                PCWSTR::null(),
                DATE_LONGDATE,
                None,
                PCWSTR::null(),
                Some(&mut buf),
                PCWSTR::null(),
            )
        };
        let date = String::from_utf16_lossy(&buf[..(n.max(1) - 1) as usize]);
        // "samedi 31 octobre 2026" → "Samedi 31 octobre": the year is noise on a clock.
        let year = unsafe { GetLocalTime() }.wYear;
        let date = date
            .trim_end_matches(&year.to_string())
            .trim_end_matches(|c: char| c == ',' || c.is_whitespace());
        Content::Clock {
            time,
            date: capitalize(date),
        }
    }

    fn system(&self) -> Content {
        let mut m = MEMORYSTATUSEX {
            dwLength: size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        let _ = unsafe { GlobalMemoryStatusEx(&mut m) };
        let gb = |b: u64| b as f64 / (1u64 << 30) as f64;
        let used = gb(m.ullTotalPhys - m.ullAvailPhys);
        let total = gb(m.ullTotalPhys);
        let ram_text = if self.french {
            format!("{used:.1} / {total:.0} Go").replace('.', ",")
        } else {
            format!("{used:.1} / {total:.0} GB")
        };
        Content::System {
            cpu: self.cpu,
            ram: m.dwMemoryLoad,
            ram_text,
        }
    }

    /// Title and artist of the session Windows considers current, while it plays.
    fn media(&mut self) -> Option<(String, String)> {
        if self.media.is_none() && !self.media_failed {
            self.media = MediaManager::RequestAsync().and_then(|op| op.join()).ok();
            self.media_failed = self.media.is_none();
        }
        let session = self.media.as_ref()?.GetCurrentSession().ok()?;
        let status = session.GetPlaybackInfo().ok()?.PlaybackStatus().ok()?;
        if status != PlaybackStatus::Playing {
            return None;
        }
        let props = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
        let title = props.Title().ok()?.to_string();
        let artist = props.Artist().map(|a| a.to_string()).unwrap_or_default();
        (!title.is_empty()).then_some((title, artist))
    }
}
