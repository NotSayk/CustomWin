//! The real [`SystemBackend`]: registry, services, power plans, `SystemParametersInfo`.

use windows::core::{GUID, PCWSTR};
use windows::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HLOCAL};
use windows::Win32::System::Power::{PowerGetActiveScheme, PowerSetActiveScheme};
use windows::Win32::System::Services::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows_registry::{Key, Type, CURRENT_USER, LOCAL_MACHINE};

use super::{map_err, wide};
use crate::backend::{ServiceState, SystemBackend};
use crate::model::{Hive, RegValue, StartType, SysParam};
use crate::Error;

pub struct WindowsBackend;

fn hive_key(h: Hive) -> &'static Key {
    match h {
        Hive::HKCU => CURRENT_USER,
        Hive::HKLM => LOCAL_MACHINE,
    }
}

fn is_not_found(e: &windows::core::Error) -> bool {
    // ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND
    matches!(e.code().0 as u32, 0x8007_0002 | 0x8007_0003)
}

fn reg_type_code(t: Type) -> u32 {
    match t {
        Type::String => 1,
        Type::ExpandString => 2,
        Type::Bytes => 3,
        Type::U32 => 4,
        Type::MultiString => 7,
        Type::U64 => 11,
        Type::Other(n) => n,
    }
}

impl SystemBackend for WindowsBackend {
    fn reg_key_exists(&self, hive: Hive, path: &str) -> Result<bool, Error> {
        match hive_key(hive).open(path) {
            Ok(_) => Ok(true),
            Err(e) if is_not_found(&e) => Ok(false),
            Err(e) => Err(map_err(path, e)),
        }
    }

    fn reg_read(&self, hive: Hive, path: &str, name: &str) -> Result<Option<RegValue>, Error> {
        let key = match hive_key(hive).open(path) {
            Ok(k) => k,
            Err(e) if is_not_found(&e) => return Ok(None),
            Err(e) => return Err(map_err(path, e)),
        };
        let ty = match key.get_type(name) {
            Ok(t) => t,
            Err(e) if is_not_found(&e) => return Ok(None),
            Err(e) => return Err(map_err(path, e)),
        };
        let ctx = |e| map_err(&format!(r"{path}\{name}"), e);
        Ok(Some(match ty {
            Type::U32 => RegValue::Dword(key.get_u32(name).map_err(ctx)?),
            Type::U64 => RegValue::Qword(key.get_u64(name).map_err(ctx)?),
            Type::String => RegValue::String(key.get_string(name).map_err(ctx)?),
            other => {
                let v = key.get_value(name).map_err(ctx)?;
                RegValue::Raw {
                    reg_type: reg_type_code(other),
                    data: v.to_vec(),
                }
            }
        }))
    }

    fn reg_write(&self, hive: Hive, path: &str, name: &str, value: &RegValue) -> Result<(), Error> {
        let ctx = |e| map_err(path, e);
        let key = hive_key(hive).create(path).map_err(ctx)?;
        match value {
            RegValue::Dword(v) => key.set_u32(name, *v),
            RegValue::Qword(v) => key.set_u64(name, *v),
            RegValue::String(s) => key.set_string(name, s),
            RegValue::Raw { reg_type, data } => key.set_bytes(name, Type::from(*reg_type), data),
        }
        .map_err(ctx)
    }

    fn reg_delete_value(&self, hive: Hive, path: &str, name: &str) -> Result<(), Error> {
        let key = match hive_key(hive).options().read().write().open(path) {
            Ok(k) => k,
            Err(e) if is_not_found(&e) => return Ok(()),
            Err(e) => return Err(map_err(path, e)),
        };
        match key.remove_value(name) {
            Ok(()) => Ok(()),
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(map_err(path, e)),
        }
    }

    fn reg_delete_key_if_empty(&self, hive: Hive, path: &str) -> Result<bool, Error> {
        let ctx = |e| map_err(path, e);
        let key = match hive_key(hive).open(path) {
            Ok(k) => k,
            Err(e) if is_not_found(&e) => return Ok(false),
            Err(e) => return Err(ctx(e)),
        };
        let empty = key.keys().map_err(ctx)?.next().is_none()
            && key.values().map_err(ctx)?.next().is_none();
        drop(key);
        if empty {
            hive_key(hive).remove_tree(path).map_err(ctx)?;
        }
        Ok(empty)
    }

    fn service_query(&self, name: &str) -> Result<Option<ServiceState>, Error> {
        let Some(svc) = Service::open(name, SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS)? else {
            return Ok(None);
        };
        let startup = svc.start_type()?;
        let running = svc.state()? != SERVICE_STOPPED;
        Ok(Some(ServiceState { startup, running }))
    }

    fn service_set_startup(&self, name: &str, startup: StartType) -> Result<(), Error> {
        let svc = Service::open(name, SERVICE_CHANGE_CONFIG)?.ok_or_else(|| missing(name))?;
        let code = match startup {
            StartType::Boot => 0,
            StartType::System => 1,
            StartType::Automatic => 2,
            StartType::Manual => 3,
            StartType::Disabled => 4,
        };
        unsafe {
            ChangeServiceConfigW(
                svc.0,
                ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                SERVICE_START_TYPE(code),
                SERVICE_ERROR(SERVICE_NO_CHANGE),
                PCWSTR::null(),
                PCWSTR::null(),
                None,
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
            )
        }
        .map_err(|e| map_err(name, e))
    }

    fn service_stop(&self, name: &str) -> Result<(), Error> {
        let svc = Service::open(name, SERVICE_STOP | SERVICE_QUERY_STATUS)?
            .ok_or_else(|| missing(name))?;
        let mut status = SERVICE_STATUS::default();
        match unsafe { ControlService(svc.0, SERVICE_CONTROL_STOP, &mut status) } {
            Ok(()) => Ok(()),
            // ERROR_SERVICE_NOT_ACTIVE
            Err(e) if e.code().0 as u32 == 0x8007_0426 => Ok(()),
            Err(e) => Err(map_err(name, e)),
        }
    }

    fn service_start(&self, name: &str) -> Result<(), Error> {
        let svc = Service::open(name, SERVICE_START)?.ok_or_else(|| missing(name))?;
        match unsafe { StartServiceW(svc.0, None) } {
            Ok(()) => Ok(()),
            // ERROR_SERVICE_ALREADY_RUNNING
            Err(e) if e.code().0 as u32 == 0x8007_0420 => Ok(()),
            Err(e) => Err(map_err(name, e)),
        }
    }

    fn power_active_scheme(&self) -> Result<String, Error> {
        let mut guid: *mut GUID = std::ptr::null_mut();
        let rc = unsafe { PowerGetActiveScheme(None, &mut guid) };
        if rc != ERROR_SUCCESS || guid.is_null() {
            return Err(Error::System(format!(
                "PowerGetActiveScheme failed ({})",
                rc.0
            )));
        }
        let s = guid_to_string(unsafe { &*guid });
        unsafe { LocalFree(Some(HLOCAL(guid.cast()))) };
        Ok(s)
    }

    fn power_set_active_scheme(&self, guid: &str) -> Result<(), Error> {
        let g = GUID::try_from(guid).map_err(|_| Error::Invalid(format!("bad GUID {guid}")))?;
        match unsafe { PowerSetActiveScheme(None, Some(&g)) } {
            ERROR_SUCCESS => Ok(()),
            // ERROR_FILE_NOT_FOUND / ERROR_INVALID_PARAMETER: the plan is not installed.
            rc if rc.0 == 2 || rc.0 == 87 => Err(Error::Unavailable(format!(
                "power plan {guid} is not installed"
            ))),
            rc => Err(Error::System(format!(
                "PowerSetActiveScheme failed ({})",
                rc.0
            ))),
        }
    }

    fn sysparam_get(&self, param: SysParam) -> Result<bool, Error> {
        let none = SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0);
        let err = |e| map_err("SystemParametersInfo", e);
        unsafe {
            match param {
                SysParam::ClientAreaAnimation => {
                    let mut v = windows::core::BOOL(0);
                    SystemParametersInfoW(
                        SPI_GETCLIENTAREAANIMATION,
                        0,
                        Some(&mut v as *mut _ as _),
                        none,
                    )
                    .map_err(err)?;
                    Ok(v.as_bool())
                }
                SysParam::WindowAnimation => {
                    let mut ai = ANIMATIONINFO {
                        cbSize: size_of::<ANIMATIONINFO>() as u32,
                        iMinAnimate: 0,
                    };
                    SystemParametersInfoW(
                        SPI_GETANIMATION,
                        ai.cbSize,
                        Some(&mut ai as *mut _ as _),
                        none,
                    )
                    .map_err(err)?;
                    Ok(ai.iMinAnimate != 0)
                }
                SysParam::MouseAcceleration => {
                    let mut m = [0i32; 3];
                    SystemParametersInfoW(SPI_GETMOUSE, 0, Some(m.as_mut_ptr() as _), none)
                        .map_err(err)?;
                    Ok(m[2] != 0)
                }
            }
        }
    }

    fn sysparam_set(&self, param: SysParam, value: bool) -> Result<(), Error> {
        let persist = SPIF_UPDATEINIFILE | SPIF_SENDCHANGE;
        let err = |e| map_err("SystemParametersInfo", e);
        unsafe {
            match param {
                SysParam::ClientAreaAnimation => {
                    // pvParam carries the BOOL itself for this action.
                    SystemParametersInfoW(
                        SPI_SETCLIENTAREAANIMATION,
                        0,
                        Some(value as usize as _),
                        persist,
                    )
                    .map_err(err)
                }
                SysParam::WindowAnimation => {
                    let mut ai = ANIMATIONINFO {
                        cbSize: size_of::<ANIMATIONINFO>() as u32,
                        iMinAnimate: value as i32,
                    };
                    SystemParametersInfoW(
                        SPI_SETANIMATION,
                        ai.cbSize,
                        Some(&mut ai as *mut _ as _),
                        persist,
                    )
                    .map_err(err)
                }
                SysParam::MouseAcceleration => {
                    // Windows defaults when "Enhance pointer precision" is on: 6, 10, 1.
                    let mut m: [i32; 3] = if value { [6, 10, 1] } else { [0, 0, 0] };
                    SystemParametersInfoW(SPI_SETMOUSE, 0, Some(m.as_mut_ptr() as _), persist)
                        .map_err(err)
                }
            }
        }
    }
}

fn missing(name: &str) -> Error {
    Error::Unavailable(format!("service {name} not found"))
}

/// Lowercase, no braces.
pub(crate) fn guid_to_string(g: &GUID) -> String {
    format!("{g:?}").to_ascii_lowercase()
}

/// Owned service handle, closed on drop.
struct Service(SC_HANDLE);

impl Service {
    /// `Ok(None)` if the service does not exist.
    fn open(name: &str, access: u32) -> Result<Option<Self>, Error> {
        let scm = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
            .map_err(|e| map_err("service manager", e))?;
        let scm = Service(scm);
        let wname = wide(name);
        match unsafe { OpenServiceW(scm.0, PCWSTR(wname.as_ptr()), access) } {
            Ok(h) => Ok(Some(Service(h))),
            // ERROR_SERVICE_DOES_NOT_EXIST
            Err(e) if e.code().0 as u32 == 0x8007_0424 => Ok(None),
            Err(e) => Err(map_err(name, e)),
        }
    }

    fn start_type(&self) -> Result<StartType, Error> {
        let mut needed = 0u32;
        let _ = unsafe { QueryServiceConfigW(self.0, None, 0, &mut needed) };
        // u64 storage keeps the buffer aligned for QUERY_SERVICE_CONFIGW.
        let mut buf = vec![0u64; (needed as usize).div_ceil(8).max(1)];
        let cfg = buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW;
        unsafe { QueryServiceConfigW(self.0, Some(cfg), (buf.len() * 8) as u32, &mut needed) }
            .map_err(|e| map_err("QueryServiceConfig", e))?;
        Ok(match unsafe { (*cfg).dwStartType.0 } {
            0 => StartType::Boot,
            1 => StartType::System,
            2 => StartType::Automatic,
            3 => StartType::Manual,
            _ => StartType::Disabled,
        })
    }

    fn state(&self) -> Result<SERVICE_STATUS_CURRENT_STATE, Error> {
        let mut status = SERVICE_STATUS::default();
        unsafe { QueryServiceStatus(self.0, &mut status) }
            .map_err(|e| map_err("QueryServiceStatus", e))?;
        Ok(status.dwCurrentState)
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        let _ = unsafe { CloseServiceHandle(self.0) };
    }
}
