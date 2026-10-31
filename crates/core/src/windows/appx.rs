//! Store apps of the current user: listed and removed through WinRT's `PackageManager`,
//! reinstalled with `winget` from the Microsoft Store.

use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::HSTRING;
use windows::Management::Deployment::PackageManager;

use super::map_err;
use crate::Error;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// (package name, full name) of the current user's packages. Listing takes tens of
/// milliseconds and a state refresh asks once per app tweak, so it is cached briefly.
type PackageList = Vec<(String, String)>;
static CACHE: Mutex<Option<(Instant, PackageList)>> = Mutex::new(None);
const CACHE_TTL: Duration = Duration::from_secs(5);

fn packages() -> Result<PackageList, Error> {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, list)) = cache.as_ref() {
        if at.elapsed() < CACHE_TTL {
            return Ok(list.clone());
        }
    }
    let err = |e| map_err("PackageManager", e);
    let manager = PackageManager::new().map_err(err)?;
    // An empty security id means the current user, which needs no administrator rights.
    let mut list = Vec::new();
    for package in manager
        .FindPackagesByUserSecurityId(&HSTRING::new())
        .map_err(err)?
    {
        let id = package.Id().map_err(err)?;
        list.push((
            id.Name().map_err(err)?.to_string(),
            id.FullName().map_err(err)?.to_string(),
        ));
    }
    *cache = Some((Instant::now(), list.clone()));
    Ok(list)
}

fn invalidate() {
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

pub fn installed(package: &str) -> Result<bool, Error> {
    Ok(packages()?
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(package)))
}

pub fn remove(package: &str) -> Result<(), Error> {
    let err = |e| map_err(package, e);
    let manager = PackageManager::new().map_err(err)?;
    // Several versions or architectures can be installed side by side.
    let full_names: Vec<String> = packages()?
        .into_iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(package))
        .map(|(_, full)| full)
        .collect();
    invalidate();
    for full in full_names {
        manager
            .RemovePackageAsync(&HSTRING::from(full))
            .map_err(err)?
            .join()
            .map_err(err)?;
    }
    Ok(())
}

pub fn install(store_id: &str) -> Result<(), Error> {
    invalidate();
    let status = Command::new("winget")
        .args([
            "install",
            "--id",
            store_id,
            "--source",
            "msstore",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--silent",
            "--disable-interactivity",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| {
            Error::Unavailable(format!(
                "winget not found ({e}); reinstall the app from the Microsoft Store"
            ))
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::System(format!(
            "winget could not reinstall {store_id} (code {:?}); reinstall it from the Microsoft Store",
            status.code()
        )))
    }
}
