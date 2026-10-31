//! Animated wallpaper: configuration shared by the app (which edits it) and
//! `customwin-desktop.exe` (which renders it), plus the framing math. Pure code.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;

/// Window class of the desktop process's control window, used by the app to find it.
pub const CONTROL_CLASS: &str = "CustomWinDesktopControl";
/// `WM_APP + 1`: reload `wallpaper.json`.
pub const MSG_RELOAD: u32 = 0x8000 + 1;
/// Run-at-login value name under `HKCU\…\CurrentVersion\Run`.
pub const AUTOSTART_NAME: &str = "CustomWinDesktop";

/// Containers Media Foundation reads out of the box on Windows 11 (WebM/VP9 and AV1 may
/// need the free Store extensions).
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "mkv", "webm", "wmv", "avi"];

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.iter().any(|v| v.eq_ignore_ascii_case(e)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Fills the screen, cropping what overflows. No black bars.
    #[default]
    Cover,
    /// Shows the whole video, with black bars if the aspect ratios differ.
    Contain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// One video per screen; a screen without a video keeps the normal wallpaper.
    #[default]
    PerScreen,
    /// One video stretched across the bounding box of all screens.
    Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenConfig {
    /// GDI device name, e.g. `\\.\DISPLAY1`.
    pub device: String,
    pub video: Option<PathBuf>,
    #[serde(default)]
    pub fit: Fit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WallpaperConfig {
    pub enabled: bool,
    pub layout: Layout,
    pub screens: Vec<ScreenConfig>,
    /// Used with [`Layout::Span`].
    pub span_video: Option<PathBuf>,
    pub span_fit: Fit,
    /// Pause on a screen whose foreground app is maximized (full screen always pauses).
    pub pause_when_maximized: bool,
    pub pause_on_battery: bool,
    pub start_with_windows: bool,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        WallpaperConfig {
            enabled: false,
            layout: Layout::PerScreen,
            screens: Vec::new(),
            span_video: None,
            span_fit: Fit::Cover,
            pause_when_maximized: true,
            pause_on_battery: true,
            start_with_windows: false,
        }
    }
}

/// Reads a JSON settings file: missing → defaults, corrupt → error (not silently reset).
pub(crate) fn load_json<T: Default + serde::de::DeserializeOwned>(path: &Path) -> Result<T, Error> {
    match std::fs::read(path) {
        // Editors on Windows (and PowerShell 5.1) like to prepend a UTF-8 BOM.
        Ok(b) => serde_json::from_slice(b.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&b))
            .map_err(|e| Error::Invalid(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(Error::System(format!("{}: {e}", path.display()))),
    }
}

/// Writes a JSON settings file atomically (temporary file, then rename).
pub(crate) fn save_json<T: Serialize>(value: &T, path: &Path) -> Result<(), Error> {
    let io = |e: std::io::Error| Error::System(format!("{}: {e}", path.display()));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(value).map_err(|e| Error::Invalid(e.to_string()))?,
    )
    .map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

impl WallpaperConfig {
    /// Missing file → defaults. A corrupt file is an error rather than silently reset.
    pub fn load(path: &Path) -> Result<Self, Error> {
        load_json(path)
    }

    pub fn save(&self, path: &Path) -> Result<(), Error> {
        save_json(self, path)
    }

    /// Every video path in the configuration, whatever the layout.
    pub fn videos(&self) -> impl Iterator<Item = &PathBuf> {
        self.screens
            .iter()
            .filter_map(|s| s.video.as_ref())
            .chain(self.span_video.as_ref())
    }

    /// Videos that `self` references and `previous` did not: the only ones worth checking
    /// on save. A path that was already configured may since have been deleted, and that
    /// must not stop the user from removing or replacing it.
    pub fn added_videos<'a>(&'a self, previous: &'a Self) -> impl Iterator<Item = &'a PathBuf> {
        self.videos()
            .filter(move |v| !previous.videos().any(|p| p == *v))
    }

    /// Whether there is anything to play.
    pub fn has_video(&self) -> bool {
        match self.layout {
            Layout::PerScreen => self.screens.iter().any(|s| s.video.is_some()),
            Layout::Span => self.span_video.is_some(),
        }
    }

    pub fn screen(&self, device: &str) -> Option<&ScreenConfig> {
        self.screens
            .iter()
            .find(|s| s.device.eq_ignore_ascii_case(device))
    }
}

/// Pixel rectangle, right/bottom exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
    pub fn union(&self, o: &Rect) -> Rect {
        Rect {
            left: self.left.min(o.left),
            top: self.top.min(o.top),
            right: self.right.max(o.right),
            bottom: self.bottom.max(o.bottom),
        }
    }
    pub fn contains(&self, o: &Rect) -> bool {
        self.left <= o.left && self.top <= o.top && self.right >= o.right && self.bottom >= o.bottom
    }
}

/// Source region of the video, normalized to 0..1.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct NormRect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// Where to take the video from and where to draw it, for a `target_w × target_h`
/// surface. `None` until the video size is known.
pub fn frame(
    fit: Fit,
    video_w: u32,
    video_h: u32,
    target_w: i32,
    target_h: i32,
) -> Option<(NormRect, Rect)> {
    if video_w == 0 || video_h == 0 || target_w <= 0 || target_h <= 0 {
        return None;
    }
    let full_src = NormRect {
        left: 0.0,
        top: 0.0,
        right: 1.0,
        bottom: 1.0,
    };
    let full_dst = Rect {
        left: 0,
        top: 0,
        right: target_w,
        bottom: target_h,
    };
    let video = video_w as f64 / video_h as f64;
    let target = target_w as f64 / target_h as f64;
    Some(match fit {
        Fit::Cover if video > target => {
            // Wider than the screen: crop left and right.
            let f = (target / video) as f32;
            (
                NormRect {
                    left: (1.0 - f) / 2.0,
                    right: (1.0 + f) / 2.0,
                    ..full_src
                },
                full_dst,
            )
        }
        Fit::Cover => {
            let f = (video / target) as f32;
            (
                NormRect {
                    top: (1.0 - f) / 2.0,
                    bottom: (1.0 + f) / 2.0,
                    ..full_src
                },
                full_dst,
            )
        }
        Fit::Contain if video > target => {
            // Wider: full width, bars above and below.
            let h = (target_w as f64 / video).round() as i32;
            let top = (target_h - h) / 2;
            (
                full_src,
                Rect {
                    top,
                    bottom: top + h,
                    ..full_dst
                },
            )
        }
        Fit::Contain => {
            let w = (target_h as f64 * video).round() as i32;
            let left = (target_w - w) / 2;
            (
                full_src,
                Rect {
                    left,
                    right: left + w,
                    ..full_dst
                },
            )
        }
    })
}

/// The foreground window, as far as pausing is concerned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Foreground {
    /// Screen it is on; `None` for the desktop, the taskbar or nothing.
    pub device: Option<String>,
    pub fullscreen: bool,
    pub maximized: bool,
}

/// Whether a wallpaper drawn on `devices` should play.
///
/// - `global_pause` (locked, display off, on battery) stops everything;
/// - a full-screen app (a game) pauses the screen it is on;
/// - a maximized app does too if `pause_when_maximized`;
/// - a video spanning several screens only pauses for full screen: a maximized window
///   hides one screen, the others still show the video.
pub fn should_play(
    devices: &[String],
    fg: &Foreground,
    pause_when_maximized: bool,
    global_pause: bool,
) -> bool {
    if global_pause {
        return false;
    }
    let Some(device) = fg.device.as_deref() else {
        return true;
    };
    if !devices.iter().any(|d| d.eq_ignore_ascii_case(device)) {
        return true;
    }
    let covered = fg.fullscreen || (fg.maximized && pause_when_maximized);
    !(covered && (devices.len() == 1 || fg.fullscreen))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(device: &str, video: Option<&str>) -> ScreenConfig {
        ScreenConfig {
            device: device.into(),
            video: video.map(PathBuf::from),
            fit: Fit::Cover,
        }
    }

    #[test]
    fn only_newly_added_videos_are_checked() {
        let previous = WallpaperConfig {
            screens: vec![screen("A", Some("gone.mp4")), screen("B", Some("gone.mp4"))],
            span_video: Some("gone.mp4".into()),
            ..Default::default()
        };
        // Removing one slot keeps the others, which are unchanged and so not re-checked.
        let mut next = previous.clone();
        next.screens[0].video = None;
        assert_eq!(next.added_videos(&previous).count(), 0);
        // Picking a new file is checked.
        next.screens[0].video = Some("new.mp4".into());
        let added: Vec<_> = next.added_videos(&previous).collect();
        assert_eq!(added, [&PathBuf::from("new.mp4")]);
    }

    #[test]
    fn load_accepts_a_utf8_bom() {
        let path = std::env::temp_dir().join(format!("customwin-bom-{}.json", std::process::id()));
        std::fs::write(&path, b"\xEF\xBB\xBF{\"enabled\": true}").unwrap();
        let loaded = WallpaperConfig::load(&path);
        std::fs::remove_file(&path).ok();
        assert!(loaded.unwrap().enabled);
    }

    #[test]
    fn cover_crops_the_overflowing_axis() {
        // 16:9 video on a 21:9 screen: crop top and bottom.
        let (src, dst) = frame(Fit::Cover, 1920, 1080, 3440, 1440).unwrap();
        assert_eq!(
            dst,
            Rect {
                left: 0,
                top: 0,
                right: 3440,
                bottom: 1440
            }
        );
        assert_eq!((src.left, src.right), (0.0, 1.0));
        assert!((src.bottom - src.top - (1.7778 / 2.3889)).abs() < 0.001);
        assert!((src.top - (1.0 - src.bottom)).abs() < 1e-6, "centered");

        // 16:9 video on a 16:10 screen: crop left and right.
        let (src, _) = frame(Fit::Cover, 1920, 1080, 1920, 1200).unwrap();
        assert_eq!((src.top, src.bottom), (0.0, 1.0));
        assert!((src.right - src.left - 0.9).abs() < 0.001);
    }

    #[test]
    fn contain_letterboxes() {
        let (src, dst) = frame(Fit::Contain, 1920, 1080, 1920, 1200).unwrap();
        assert_eq!(
            src,
            NormRect {
                left: 0.0,
                top: 0.0,
                right: 1.0,
                bottom: 1.0
            }
        );
        assert_eq!(
            dst,
            Rect {
                left: 0,
                top: 60,
                right: 1920,
                bottom: 1140
            }
        );
        let (_, dst) = frame(Fit::Contain, 1080, 1920, 1920, 1080).unwrap(); // portrait video
        assert_eq!(dst.height(), 1080);
        assert_eq!(dst.left, (1920 - dst.width()) / 2);
        assert!(
            frame(Fit::Cover, 0, 0, 1920, 1080).is_none(),
            "size unknown yet"
        );
    }

    #[test]
    fn same_aspect_is_untouched() {
        let (src, dst) = frame(Fit::Cover, 3840, 2160, 2560, 1440).unwrap();
        assert_eq!(
            src,
            NormRect {
                left: 0.0,
                top: 0.0,
                right: 1.0,
                bottom: 1.0
            }
        );
        assert_eq!(dst.width(), 2560);
    }

    #[test]
    fn config_roundtrip_and_defaults() {
        let dir = std::env::temp_dir().join(format!("customwin-wp-{}", std::process::id()));
        let path = dir.join("wallpaper.json");
        let c = WallpaperConfig::load(&path).unwrap();
        assert!(!c.enabled && c.pause_on_battery && c.pause_when_maximized && !c.has_video());

        let c = WallpaperConfig {
            enabled: true,
            screens: vec![ScreenConfig {
                device: r"\\.\DISPLAY1".into(),
                video: Some(r"C:\v.mp4".into()),
                fit: Fit::Contain,
            }],
            ..Default::default()
        };
        c.save(&path).unwrap();
        let back = WallpaperConfig::load(&path).unwrap();
        assert_eq!(back, c);
        assert!(back.has_video());
        assert_eq!(back.screen(r"\\.\display1").unwrap().fit, Fit::Contain);
        // Older or partial files still load.
        std::fs::write(&path, r#"{"enabled":true}"#).unwrap();
        assert!(WallpaperConfig::load(&path).unwrap().pause_on_battery);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn pause_rules() {
        let one = vec![r"\\.\DISPLAY1".to_string()];
        let both = vec![r"\\.\DISPLAY1".to_string(), r"\\.\DISPLAY2".to_string()];
        let fg = |dev: &str, fullscreen, maximized| Foreground {
            device: Some(dev.into()),
            fullscreen,
            maximized,
        };

        assert!(
            should_play(&one, &Foreground::default(), true, false),
            "desktop in front"
        );
        assert!(
            !should_play(&one, &Foreground::default(), true, true),
            "locked / battery"
        );
        assert!(
            !should_play(&one, &fg(r"\\.\DISPLAY1", true, false), false, false),
            "game"
        );
        assert!(
            !should_play(&one, &fg(r"\\.\display1", false, true), true, false),
            "maximized"
        );
        assert!(
            should_play(&one, &fg(r"\\.\DISPLAY1", false, true), false, false),
            "option off"
        );
        assert!(
            should_play(&one, &fg(r"\\.\DISPLAY2", true, false), true, false),
            "other screen"
        );
        assert!(
            should_play(&one, &fg(r"\\.\DISPLAY1", false, false), true, false),
            "small window"
        );

        assert!(
            should_play(&both, &fg(r"\\.\DISPLAY2", false, true), true, false),
            "span: one screen hidden"
        );
        assert!(
            !should_play(&both, &fg(r"\\.\DISPLAY2", true, false), true, false),
            "span: game"
        );
    }

    #[test]
    fn video_extensions() {
        assert!(is_video(Path::new(r"C:\a\b.MP4")) && is_video(Path::new("x.webm")));
        assert!(!is_video(Path::new("x.exe")) && !is_video(Path::new("mp4")));
    }

    #[test]
    fn rect_helpers() {
        let a = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let b = Rect {
            left: 1920,
            top: -200,
            right: 4480,
            bottom: 1240,
        };
        assert_eq!(
            a.union(&b),
            Rect {
                left: 0,
                top: -200,
                right: 4480,
                bottom: 1240
            }
        );
        assert!(a.union(&b).contains(&a) && !a.contains(&b));
    }
}
