//! Reads and writes the Windows theme (mode, accent, transparency) and applies the
//! `.theme` files Windows ships, the same registry values Settings > Personalization
//! uses. Changes are announced with `WM_SETTINGCHANGE "ImmersiveColorSet"`, which apps,
//! Start and the taskbar listen to.

use std::path::{Path, PathBuf};

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{LPARAM, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SHLoadIndirectString, SIIGBF_RESIZETOFIT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, SystemParametersInfoW, HWND_BROADCAST, SMTO_ABORTIFHUNG, SPIF_SENDCHANGE,
    SPIF_UPDATEINIFILE, SPI_SETDESKWALLPAPER, WM_SETTINGCHANGE,
};
use windows_registry::{Type, CURRENT_USER};

use super::map_err;
use crate::theme::{
    abgr, accent_palette, apply_theme_file, from_abgr, palette_bytes, parse_theme, ThemeFile,
    ThemeState,
};
use crate::Error;

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const DWM: &str = r"Software\Microsoft\Windows\DWM";
const ACCENT: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent";
const DESKTOP: &str = r"Control Panel\Desktop";

fn read_u32(path: &str, name: &str) -> Option<u32> {
    let key = CURRENT_USER.open(path).ok()?;
    key.get_u32(name)
        .ok()
        .or_else(|| key.get_string(name).ok()?.trim().parse().ok())
}

pub fn theme_state() -> ThemeState {
    let flag = |path, name, default| read_u32(path, name).map_or(default, |v| v != 0);
    ThemeState {
        apps_light: flag(PERSONALIZE, "AppsUseLightTheme", true),
        system_light: flag(PERSONALIZE, "SystemUsesLightTheme", false),
        transparency: flag(PERSONALIZE, "EnableTransparency", true),
        accent: read_u32(DWM, "AccentColor").map_or(0x0078D4, from_abgr),
        auto_accent: flag(DESKTOP, "AutoColorization", false),
        accent_on_taskbar: flag(PERSONALIZE, "ColorPrevalence", false),
        accent_on_title_bars: flag(DWM, "ColorPrevalence", false),
    }
}

pub fn set_theme_state(s: &ThemeState) -> Result<(), Error> {
    let current = theme_state();
    let open = |path: &str| CURRENT_USER.create(path).map_err(|e| map_err(path, e));
    let put = |path: &str, name: &str, v: u32| -> Result<(), Error> {
        open(path)?
            .set_u32(name, v)
            .map_err(|e| map_err(&format!(r"{path}\{name}"), e))
    };
    put(PERSONALIZE, "AppsUseLightTheme", s.apps_light as u32)?;
    put(PERSONALIZE, "SystemUsesLightTheme", s.system_light as u32)?;
    put(PERSONALIZE, "EnableTransparency", s.transparency as u32)?;
    put(PERSONALIZE, "ColorPrevalence", s.accent_on_taskbar as u32)?;
    put(DWM, "ColorPrevalence", s.accent_on_title_bars as u32)?;
    put(DESKTOP, "AutoColorization", s.auto_accent as u32)?;
    // Windows's own shades of an accent are slightly hand-tuned: only replace them when
    // the accent really changes.
    if !s.auto_accent && (s.accent != current.accent || current.auto_accent) {
        let palette = accent_palette(s.accent);
        open(ACCENT)?
            .set_bytes("AccentPalette", Type::Bytes, &palette_bytes(&palette))
            .map_err(|e| map_err(ACCENT, e))?;
        put(ACCENT, "AccentColorMenu", abgr(palette[3]))?;
        put(ACCENT, "StartColorMenu", abgr(palette[4]))?;
        put(DWM, "AccentColor", abgr(palette[3]))?;
        // ARGB with the 0xC4 alpha Windows itself writes.
        put(DWM, "ColorizationColor", 0xC400_0000 | palette[3])?;
        put(DWM, "ColorizationAfterglow", 0xC400_0000 | palette[3])?;
    }
    broadcast();
    Ok(())
}

fn broadcast() {
    unsafe {
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("ImmersiveColorSet").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            1000,
            None,
        );
    }
}

/// `%SystemRoot%\…` → `C:\Windows\…`. Unknown variables are left as they are.
fn expand(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        let Some(len) = rest[start + 1..].find('%') else {
            break;
        };
        let name = &rest[start + 1..start + 1 + len];
        out.push_str(&rest[..start]);
        match std::env::var(name) {
            Ok(v) => out.push_str(&v),
            Err(_) => out.push_str(&rest[start..start + len + 2]),
        }
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    out
}

/// `@%SystemRoot%\System32\themeui.dll,-2122` → "Windows (dark)", in the user's language.
fn resolve_name(raw: &str) -> Option<String> {
    if !raw.starts_with('@') {
        return Some(raw.to_string());
    }
    let source = HSTRING::from(expand(raw));
    let mut buf = [0u16; 256];
    unsafe { SHLoadIndirectString(PCWSTR(source.as_ptr()), &mut buf, None) }.ok()?;
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..n])).filter(|s| !s.is_empty())
}

fn read_theme_file(path: &Path) -> Option<ThemeFile> {
    let bytes = std::fs::read(path).ok()?;
    // Windows writes these files in UTF-16 LE (with BOM) or in the ANSI code page.
    let text = match bytes.strip_prefix(&[0xFF, 0xFE]) {
        Some(b) => String::from_utf16_lossy(
            &b.chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        ),
        // ANSI: Latin-1 covers the accented letters of French and English names.
        None => String::from_utf8(bytes.clone())
            .unwrap_or_else(|_| bytes.iter().map(|&b| b as char).collect()),
    };
    let (mut theme, raw_name) = parse_theme(&text, expand);
    theme.path = path.to_path_buf();
    theme.name = raw_name
        .as_deref()
        .and_then(resolve_name)
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    Some(theme)
}

/// Themes installed for everyone and the user's own, each with a wallpaper preview.
pub fn windows_themes() -> Vec<ThemeFile> {
    let mut dirs = Vec::new();
    if let Ok(root) = std::env::var("SystemRoot") {
        dirs.push(PathBuf::from(root).join(r"Resources\Themes"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        dirs.push(PathBuf::from(local).join(r"Microsoft\Windows\Themes"));
    }
    let mut themes: Vec<ThemeFile> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("theme"))
        })
        .filter_map(|p| read_theme_file(&p))
        // Without a wallpaper or colours there is nothing CustomWin would apply.
        .filter(|t| t.wallpaper.as_ref().is_some_and(|w| w.is_file()))
        .collect();
    // Shell previews need a single-threaded apartment.
    let paths: Vec<PathBuf> = themes.iter().filter_map(|t| t.wallpaper.clone()).collect();
    let thumbs = std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let t: Vec<Option<String>> = paths.iter().map(|p| thumbnail(p, 192, 108)).collect();
        CoUninitialize();
        t
    })
    .join()
    .unwrap_or_default();
    for (t, thumb) in themes.iter_mut().zip(thumbs) {
        t.thumbnail = thumb;
    }
    themes
}

/// Applies a theme's wallpaper, mode and accent. Returns the new state.
pub fn apply_windows_theme(path: &Path) -> Result<ThemeState, Error> {
    let theme = read_theme_file(path)
        .ok_or_else(|| Error::Invalid(format!("{}: not a theme file", path.display())))?;
    if let Some(wallpaper) = theme.wallpaper.as_ref().filter(|w| w.is_file()) {
        CURRENT_USER
            .create(DESKTOP)
            .and_then(|k| {
                k.set_string("WallpaperStyle", "10")?; // fill
                k.set_string("TileWallpaper", "0")
            })
            .map_err(|e| map_err(DESKTOP, e))?;
        let mut wide = super::wide(&wallpaper.to_string_lossy());
        unsafe {
            SystemParametersInfoW(
                SPI_SETDESKWALLPAPER,
                0,
                Some(wide.as_mut_ptr().cast()),
                SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
            )
        }
        .map_err(|e| map_err("wallpaper", e))?;
    }
    let next = apply_theme_file(theme_state(), &theme);
    set_theme_state(&next)?;
    Ok(next)
}

/// A shell preview of an image as a `data:image/bmp` URL.
fn thumbnail(path: &Path, width: i32, height: i32) -> Option<String> {
    unsafe {
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        let bitmap = factory
            .GetImage(
                SIZE {
                    cx: width,
                    cy: height,
                },
                SIIGBF_RESIZETOFIT,
            )
            .ok()?;
        let bmp = bitmap_to_bmp(bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        Some(format!("data:image/bmp;base64,{}", base64(&bmp?)))
    }
}

unsafe fn bitmap_to_bmp(bitmap: HBITMAP) -> Option<Vec<u8>> {
    let mut info = BITMAP::default();
    if unsafe {
        GetObjectW(
            HGDIOBJ(bitmap.0),
            size_of::<BITMAP>() as i32,
            Some(&mut info as *mut _ as *mut _),
        )
    } == 0
    {
        return None;
    }
    let (w, h) = (info.bmWidth, info.bmHeight);
    let mut header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; (w * h * 4).max(0) as usize];
    let dc = unsafe { CreateCompatibleDC(None) };
    let lines = unsafe {
        GetDIBits(
            dc,
            bitmap,
            0,
            h as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut header,
            DIB_RGB_COLORS,
        )
    };
    let _ = unsafe { DeleteDC(dc) };
    if lines == 0 {
        return None;
    }
    let mut out = Vec::with_capacity(54 + pixels.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + pixels.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    let ih = header.bmiHeader;
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&ih.biWidth.to_le_bytes());
    out.extend_from_slice(&ih.biHeight.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]); // BI_RGB, size and resolution unused
    out.extend_from_slice(&pixels);
    Some(out)
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn expands_known_variables_only() {
        let root = std::env::var("SystemRoot").unwrap();
        assert_eq!(expand(r"%SystemRoot%\Web"), format!(r"{root}\Web"));
        assert_eq!(expand(r"%NoSuchVar_CW%\x"), r"%NoSuchVar_CW%\x");
        assert_eq!(expand("100%"), "100%");
    }
}
