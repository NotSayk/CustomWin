//! Windows theme: light/dark mode, accent colour, transparency, and the `.theme` files
//! Windows ships. Pure code; reading and writing the registry is in
//! `windows::theme`.
//!
//! These are plain user preferences, exactly what Settings > Personalization changes:
//! the current values are always read back from Windows, so there is nothing to journal
//! and switching back is just another change.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The personalization settings CustomWin exposes, as Windows currently has them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeState {
    /// Apps in light mode (`AppsUseLightTheme`).
    pub apps_light: bool,
    /// Taskbar, Start and the rest of the shell in light mode (`SystemUsesLightTheme`).
    pub system_light: bool,
    /// Transparency effects (`EnableTransparency`).
    pub transparency: bool,
    /// Accent colour, `0xRRGGBB`.
    pub accent: u32,
    /// Windows picks the accent from the wallpaper (`AutoColorization`).
    pub auto_accent: bool,
    /// Accent on Start and the taskbar (`ColorPrevalence`); Windows only honours it with
    /// the shell in dark mode.
    pub accent_on_taskbar: bool,
    /// Accent on title bars and window borders (`DWM\ColorPrevalence`).
    pub accent_on_title_bars: bool,
}

/// The accent colours offered in the app: Windows's own palette from Settings.
pub const ACCENTS: &[u32] = &[
    0x0078D4, 0x0063B1, 0x8764B8, 0x744DA9, 0xB146C2, 0xE3008C, 0xEA005E, 0xE81123, 0xCA5010,
    0xF7630C, 0xFFB900, 0x498205, 0x107C10, 0x00B294, 0x038387, 0x2D7D9A, 0x567C73, 0x68768A,
    0x515C6B, 0x4C4A48,
];

/// The eight colours of `Explorer\Accent\AccentPalette`, lightest first: three lighter
/// shades, the accent itself (index 3), three darker shades, and a spare entry.
/// Windows uses the shades for hover states and for Start (index 4).
pub fn accent_palette(rgb: u32) -> [u32; 8] {
    let (h, s, l) = to_hsl(rgb);
    let shade = |dl: f64| from_hsl(h, s, (l + dl).clamp(0.04, 0.94));
    [
        shade(0.38),
        shade(0.23),
        shade(0.07),
        rgb & 0xFF_FFFF,
        shade(-0.04),
        shade(-0.13),
        shade(-0.21),
        0x88_8888,
    ]
}

/// Registry form of a colour: `0xAABBGGRR` with full alpha.
pub fn abgr(rgb: u32) -> u32 {
    0xFF00_0000 | ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

/// Inverse of [`abgr`], ignoring alpha.
pub fn from_abgr(v: u32) -> u32 {
    ((v & 0xFF) << 16) | (v & 0xFF00) | ((v >> 16) & 0xFF)
}

/// The palette as the 32 `REG_BINARY` bytes Windows stores (R, G, B, 0 per entry).
pub fn palette_bytes(palette: &[u32; 8]) -> Vec<u8> {
    palette
        .iter()
        .flat_map(|c| [(c >> 16) as u8, (c >> 8) as u8, *c as u8, 0])
        .collect()
}

fn to_hsl(rgb: u32) -> (f64, f64, f64) {
    let [r, g, b] = [(rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF].map(|c| c as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> u32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u32;
    (byte(r) << 16) | (byte(g) << 8) | byte(b)
}

/// What CustomWin applies from a `.theme` file: its wallpaper, colours and modes. Sounds,
/// cursors and icons are left as they are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeFile {
    pub path: PathBuf,
    /// Resolved display name (the file's `DisplayName` is often a resource reference).
    pub name: String,
    pub wallpaper: Option<PathBuf>,
    /// `0xRRGGBB`, from `ColorizationColor`.
    pub accent: Option<u32>,
    pub auto_accent: bool,
    pub apps_light: Option<bool>,
    pub system_light: Option<bool>,
    /// Small JPEG/BMP preview as a `data:` URL, filled in by the Windows side.
    pub thumbnail: Option<String>,
}

/// Parses a `.theme` file (an INI file). `expand` resolves `%SystemRoot%`-style
/// variables. Returns the raw `DisplayName` alongside, to be resolved by the caller.
pub fn parse_theme(text: &str, expand: impl Fn(&str) -> String) -> (ThemeFile, Option<String>) {
    let mut section = String::new();
    let mut theme = ThemeFile::default();
    let mut display_name = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with(';') || line.is_empty() {
            continue;
        }
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s.to_ascii_lowercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        let mode = |v: &str| match v.to_ascii_lowercase().as_str() {
            "light" => Some(true),
            "dark" => Some(false),
            _ => None,
        };
        match (section.as_str(), key.as_str()) {
            ("theme", "displayname") => display_name = Some(value.to_string()),
            ("control panel\\desktop", "wallpaper") if !value.is_empty() => {
                theme.wallpaper = Some(PathBuf::from(expand(value)));
            }
            ("visualstyles", "colorizationcolor") => {
                let hex = value.trim_start_matches("0X").trim_start_matches("0x");
                theme.accent = u32::from_str_radix(hex, 16).ok().map(|v| v & 0xFF_FFFF);
            }
            ("visualstyles", "autocolorization") => theme.auto_accent = value == "1",
            ("visualstyles", "appmode") => theme.apps_light = mode(value),
            ("visualstyles", "systemmode") => theme.system_light = mode(value),
            _ => {}
        }
    }
    (theme, display_name)
}

/// The state after applying `theme` on top of `current`.
pub fn apply_theme_file(current: ThemeState, theme: &ThemeFile) -> ThemeState {
    ThemeState {
        apps_light: theme.apps_light.unwrap_or(current.apps_light),
        system_light: theme.system_light.unwrap_or(current.system_light),
        accent: theme.accent.unwrap_or(current.accent),
        auto_accent: theme.auto_accent,
        ..current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_conversions() {
        assert_eq!(abgr(0x0078D4), 0xFFD4_7800);
        assert_eq!(from_abgr(0xFFD4_7800), 0x0078D4);
        for c in ACCENTS {
            let (h, s, l) = to_hsl(*c);
            let back = from_hsl(h, s, l);
            for shift in [16, 8, 0] {
                let d = ((back >> shift) & 0xFF) as i32 - ((c >> shift) & 0xFF) as i32;
                assert!(d.abs() <= 1, "{c:06X} -> {back:06X}");
            }
        }
    }

    #[test]
    fn palette_goes_from_light_to_dark() {
        let p = accent_palette(0x0078D4);
        assert_eq!(p[3], 0x0078D4);
        let lum = |c: u32| to_hsl(c).2;
        assert!(p[..7].windows(2).all(|w| lum(w[0]) > lum(w[1])));
        let bytes = palette_bytes(&p);
        assert_eq!(bytes.len(), 32);
        assert_eq!(&bytes[12..16], &[0x00, 0x78, 0xD4, 0]);
    }

    #[test]
    fn parses_a_windows_theme() {
        let text = "; Copyright\r\n[Theme]\r\nDisplayName=@%SystemRoot%\\System32\\themeui.dll,-2122\r\n\
            [Control Panel\\Desktop]\r\nWallpaper=%SystemRoot%\\Web\\Wallpaper\\ThemeA\\img20.jpg\r\n\
            [VisualStyles]\r\nAutoColorization=0\r\nColorizationColor=0XC4680081\r\nSystemMode=Dark\r\nAppMode=Light\r\n";
        let (t, name) = parse_theme(text, |s| s.replace("%SystemRoot%", r"C:\Windows"));
        assert_eq!(
            name.as_deref(),
            Some(r"@%SystemRoot%\System32\themeui.dll,-2122")
        );
        assert_eq!(
            t.wallpaper,
            Some(PathBuf::from(r"C:\Windows\Web\Wallpaper\ThemeA\img20.jpg"))
        );
        assert_eq!(t.accent, Some(0x680081));
        assert_eq!(
            (t.apps_light, t.system_light, t.auto_accent),
            (Some(true), Some(false), false)
        );

        let current = ThemeState {
            apps_light: false,
            system_light: true,
            transparency: true,
            accent: 0x0078D4,
            auto_accent: true,
            accent_on_taskbar: false,
            accent_on_title_bars: true,
        };
        let next = apply_theme_file(current, &t);
        assert_eq!(
            (next.apps_light, next.system_light, next.accent),
            (true, false, 0x680081)
        );
        assert!(next.transparency && next.accent_on_title_bars && !next.auto_accent);
    }
}
