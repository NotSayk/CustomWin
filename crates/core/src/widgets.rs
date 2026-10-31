//! Desktop widgets and the transparent taskbar: configuration shared by the app (which
//! edits it) and `customwin-desktop.exe` (which draws it), plus the layout math. Pure code.
//!
//! Widgets are drawn on the wallpaper layer, behind the desktop icons: they never cover a
//! window, never take a click, and hide with the desktop when a game runs.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::wallpaper::Rect;
use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    /// Time and date, in the user's format.
    Clock,
    /// Processor and memory load.
    System,
    /// Title and artist of what is playing (any app that shows in the volume flyout).
    Media,
}

impl WidgetKind {
    pub const ALL: [WidgetKind; 3] = [WidgetKind::Clock, WidgetKind::System, WidgetKind::Media];

    /// Size in device-independent pixels (96 DPI).
    pub fn size(self) -> (i32, i32) {
        match self {
            WidgetKind::Clock => (280, 118),
            WidgetKind::System => (280, 104),
            WidgetKind::Media => (320, 76),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Corner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Widget {
    pub kind: WidgetKind,
    #[serde(default)]
    pub enabled: bool,
    /// GDI device name of the screen; `None` for the main screen.
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub corner: Corner,
}

impl Widget {
    fn default_for(kind: WidgetKind) -> Self {
        Widget {
            kind,
            enabled: false,
            device: None,
            corner: match kind {
                WidgetKind::Media => Corner::BottomRight,
                _ => Corner::TopRight,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WidgetsConfig {
    pub widgets: Vec<Widget>,
    /// Hides the taskbar's background (Windows 11, see `crates/taskbar`).
    pub transparent_taskbar: bool,
}

impl Default for WidgetsConfig {
    fn default() -> Self {
        WidgetsConfig {
            widgets: WidgetKind::ALL.map(Widget::default_for).to_vec(),
            transparent_taskbar: false,
        }
    }
}

impl WidgetsConfig {
    pub fn load(path: &Path) -> Result<Self, Error> {
        let mut c: Self = crate::wallpaper::load_json(path)?;
        // Files written by older versions may lack a kind: add it, switched off.
        for kind in WidgetKind::ALL {
            if !c.widgets.iter().any(|w| w.kind == kind) {
                c.widgets.push(Widget::default_for(kind));
            }
        }
        Ok(c)
    }

    pub fn save(&self, path: &Path) -> Result<(), Error> {
        crate::wallpaper::save_json(self, path)
    }

    pub fn enabled(&self) -> impl Iterator<Item = &Widget> {
        self.widgets.iter().filter(|w| w.enabled)
    }

    /// Whether `customwin-desktop.exe` has anything to do for this configuration.
    pub fn active(&self) -> bool {
        self.transparent_taskbar || self.enabled().next().is_some()
    }
}

/// Space between the screen edge (outside the taskbar) and the widgets, and between two
/// widgets of the same corner, in device-independent pixels.
pub const MARGIN: i32 = 24;
pub const GAP: i32 = 12;

/// Places widgets of `sizes` (physical pixels) in `corner` of `work` (the screen minus
/// the taskbar), stacked away from the corner in the given order.
pub fn stack(work: Rect, corner: Corner, sizes: &[(i32, i32)], margin: i32, gap: i32) -> Vec<Rect> {
    let top = matches!(corner, Corner::TopLeft | Corner::TopRight);
    let left = matches!(corner, Corner::TopLeft | Corner::BottomLeft);
    let mut y = if top {
        work.top + margin
    } else {
        work.bottom - margin
    };
    sizes
        .iter()
        .map(|&(w, h)| {
            let x = if left {
                work.left + margin
            } else {
                work.right - margin - w
            };
            let top_edge = if top { y } else { y - h };
            if top {
                y += h + gap;
            } else {
                y -= h + gap;
            }
            Rect {
                left: x,
                top: top_edge,
                right: x + w,
                bottom: top_edge + h,
            }
        })
        .collect()
}

/// Processor load between two `GetSystemTimes` samples `(idle, kernel, user)`; kernel
/// time includes idle time. Percent, 0 when nothing elapsed.
pub fn cpu_percent(before: (u64, u64, u64), after: (u64, u64, u64)) -> u32 {
    let idle = after.0.saturating_sub(before.0);
    let total = after.1.saturating_sub(before.1) + after.2.saturating_sub(before.2);
    if total == 0 {
        return 0;
    }
    (100 * total.saturating_sub(idle) / total).min(100) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1032,
    };

    #[test]
    fn stacks_from_each_corner() {
        let sizes = [(200, 100), (200, 50)];
        let tr = stack(SCREEN, Corner::TopRight, &sizes, 24, 12);
        assert_eq!(
            tr[0],
            Rect {
                left: 1696,
                top: 24,
                right: 1896,
                bottom: 124
            }
        );
        assert_eq!(tr[1].top, 136);
        let bl = stack(SCREEN, Corner::BottomLeft, &sizes, 24, 12);
        assert_eq!(
            bl[0],
            Rect {
                left: 24,
                top: 908,
                right: 224,
                bottom: 1008
            }
        );
        assert_eq!(bl[1].bottom, 896);
    }

    #[test]
    fn cpu_load() {
        // 1000 ticks elapsed, 250 of them idle.
        assert_eq!(cpu_percent((0, 0, 0), (250, 600, 400)), 75);
        assert_eq!(cpu_percent((5, 5, 5), (5, 5, 5)), 0);
    }

    #[test]
    fn defaults_and_old_files() {
        let c = WidgetsConfig::default();
        assert!(!c.active() && c.widgets.len() == 3);
        let dir = std::env::temp_dir().join(format!("customwin-widgets-{}", std::process::id()));
        let path = dir.join("widgets.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, r#"{"widgets":[{"kind":"clock","enabled":true}]}"#).unwrap();
        let c = WidgetsConfig::load(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(c.active());
        assert_eq!(c.widgets.len(), 3, "missing kinds are added");
        assert_eq!(c.enabled().count(), 1);
    }
}
