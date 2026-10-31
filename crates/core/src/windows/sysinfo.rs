//! Hardware and settings probes. Every probe is best-effort: a failing one leaves its
//! field at the default instead of failing the whole analysis.

use windows::core::GUID;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
};
use windows::Win32::NetworkManagement::IpHelper::*;
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::System::Power::{
    GetSystemPowerStatus, PowerReadFriendlyName, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_registry::{CURRENT_USER, LOCAL_MACHINE};

use super::backend::WindowsBackend;
use crate::analysis::*;
use crate::backend::SystemBackend;

const DISPLAY_CLASS: &str =
    r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";

pub fn collect_system_info() -> SystemInfo {
    SystemInfo {
        os: os(),
        cpu: cpu(),
        memory_bytes: memory(),
        gpus: gpus(),
        power: power(),
        features: features(),
        network: network(),
    }
}

fn os() -> OsInfo {
    let Ok(k) = LOCAL_MACHINE.open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion") else {
        return OsInfo::default();
    };
    let build: u32 = k
        .get_string("CurrentBuildNumber")
        .ok()
        .and_then(|b| b.parse().ok())
        .unwrap_or(0);
    let mut name = k.get_string("ProductName").unwrap_or_default();
    // ProductName still says "Windows 10" on Windows 11.
    if build >= 22000 {
        name = name.replacen("Windows 10", "Windows 11", 1);
    }
    OsInfo {
        name,
        display_version: k.get_string("DisplayVersion").unwrap_or_default(),
        build,
        ubr: k.get_u32("UBR").unwrap_or(0),
        edition: k.get_string("EditionID").unwrap_or_default(),
    }
}

fn cpu() -> CpuInfo {
    let name = LOCAL_MACHINE
        .open(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0")
        .and_then(|k| k.get_string("ProcessorNameString"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let logical_cores = std::thread::available_parallelism().map_or(0, |n| n.get() as u32);
    CpuInfo {
        name,
        logical_cores,
    }
}

fn memory() -> u64 {
    let mut m = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    match unsafe { GlobalMemoryStatusEx(&mut m) } {
        Ok(()) => m.ullTotalPhys,
        Err(_) => 0,
    }
}

struct DriverEntry {
    desc: String,
    version: Option<String>,
    date: Option<String>,
}

fn display_drivers() -> Vec<DriverEntry> {
    let Ok(class) = LOCAL_MACHINE.open(DISPLAY_CLASS) else {
        return Vec::new();
    };
    let Ok(names) = class.keys() else {
        return Vec::new();
    };
    names
        .filter(|n| n.chars().all(|c| c.is_ascii_digit()))
        .filter_map(|n| class.open(&n).ok())
        .filter_map(|k| {
            Some(DriverEntry {
                desc: k.get_string("DriverDesc").ok()?,
                version: k.get_string("DriverVersion").ok(),
                date: k
                    .get_string("DriverDate")
                    .ok()
                    .and_then(|d| normalize_driver_date(&d)),
            })
        })
        .collect()
}

fn gpus() -> Vec<GpuInfo> {
    let drivers = display_drivers();
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
        return Vec::new();
    };
    let mut out: Vec<GpuInfo> = Vec::new();
    for i in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
            break;
        };
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let name = String::from_utf16_lossy(&desc.Description)
            .trim_end_matches('\0')
            .trim()
            .to_string();
        // One physical GPU shows up once per output on some systems.
        if out.iter().any(|g| g.name == name) {
            continue;
        }
        let vendor = GpuVendor::from_pci_id(desc.VendorId);
        let driver = drivers.iter().find(|d| d.desc == name);
        let driver_version = driver.and_then(|d| d.version.clone());
        let driver_display_version = match vendor {
            GpuVendor::Nvidia => driver_version.as_deref().and_then(nvidia_display_version),
            _ => driver_version.clone(),
        };
        out.push(GpuInfo {
            name,
            vendor,
            vram_bytes: desc.DedicatedVideoMemory as u64,
            driver_version,
            driver_display_version,
            driver_date: driver.and_then(|d| d.date.clone()),
        });
    }
    out
}

fn power() -> PowerInfo {
    let scheme = WindowsBackend.power_active_scheme().unwrap_or_default();
    let mut status = SYSTEM_POWER_STATUS::default();
    let ok = unsafe { GetSystemPowerStatus(&mut status) }.is_ok();
    PowerInfo {
        active_scheme_name: scheme_name(&scheme).unwrap_or_default(),
        active_scheme: scheme,
        // BatteryFlag 128 = no system battery, 255 = unknown.
        has_battery: ok && status.BatteryFlag != 128 && status.BatteryFlag != 255,
        on_battery: ok && status.ACLineStatus == 0,
    }
}

fn scheme_name(guid: &str) -> Option<String> {
    let g = GUID::try_from(guid).ok()?;
    let mut size = 0u32;
    let _ = unsafe { PowerReadFriendlyName(None, Some(&g), None, None, None, &mut size) };
    if size == 0 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    let rc = unsafe {
        PowerReadFriendlyName(
            None,
            Some(&g),
            None,
            None,
            Some(buf.as_mut_ptr().cast()),
            &mut size,
        )
    };
    (rc.0 == 0).then(|| {
        String::from_utf16_lossy(&buf)
            .trim_end_matches('\0')
            .to_string()
    })
}

fn dword(hive: &windows_registry::Key, path: &str, name: &str) -> Option<u32> {
    hive.open(path).and_then(|k| k.get_u32(name)).ok()
}

fn features() -> Features {
    Features {
        game_mode: dword(CURRENT_USER, r"Software\Microsoft\GameBar", "AutoGameModeEnabled").map(|v| v != 0),
        hags: dword(LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers", "HwSchMode").map(|v| v == 2),
        memory_integrity: dword(
            LOCAL_MACHINE,
            r"SYSTEM\CurrentControlSet\Control\DeviceGuard\Scenarios\HypervisorEnforcedCodeIntegrity",
            "Enabled",
        )
        .map(|v| v != 0),
        background_recording: dword(
            CURRENT_USER,
            r"Software\Microsoft\Windows\CurrentVersion\GameDVR",
            "HistoricalCaptureEnabled",
        )
        .map(|v| v != 0),
    }
}

const IF_TYPE_ETHERNET: u32 = 6;
const IF_TYPE_WIFI: u32 = 71;

fn network() -> Vec<NetAdapter> {
    let flags = GAA_FLAG_SKIP_ANYCAST
        | GAA_FLAG_SKIP_MULTICAST
        | GAA_FLAG_SKIP_DNS_SERVER
        | GAA_FLAG_SKIP_UNICAST;
    let mut size = 16 * 1024u32;
    let mut buf: Vec<u64>;
    loop {
        buf = vec![0u64; (size as usize).div_ceil(8)];
        let rc = unsafe {
            GetAdaptersAddresses(0, flags, None, Some(buf.as_mut_ptr().cast()), &mut size)
        };
        match rc {
            0 => break,
            111 => continue, // ERROR_BUFFER_OVERFLOW: `size` now holds the needed size.
            _ => return Vec::new(),
        }
    }
    let mut out = Vec::new();
    let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    while let Some(a) = unsafe { p.as_ref() } {
        p = a.Next;
        let kind = match a.IfType {
            IF_TYPE_ETHERNET => NetKind::Ethernet,
            IF_TYPE_WIFI => NetKind::Wifi,
            _ => continue,
        };
        let description = unsafe { a.Description.to_string() }.unwrap_or_default();
        let lower = description.to_ascii_lowercase();
        if [
            "virtual",
            "hyper-v",
            "vpn",
            "tap-",
            "wan miniport",
            "loopback",
        ]
        .iter()
        .any(|v| lower.contains(v))
        {
            continue;
        }
        out.push(NetAdapter {
            name: unsafe { a.FriendlyName.to_string() }.unwrap_or_default(),
            description,
            kind,
            connected: a.OperStatus == IfOperStatusUp,
            link_speed_bps: if a.TransmitLinkSpeed == u64::MAX {
                0
            } else {
                a.TransmitLinkSpeed
            },
        });
    }
    out
}
