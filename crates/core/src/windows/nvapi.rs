//! Minimal NVAPI client for driver settings (DRS) of the global profile.
//!
//! NVAPI ships with the NVIDIA driver as `nvapi64.dll` and exposes a single export,
//! `nvapi_QueryInterface`, that maps function ids to pointers. Ids, structure layout and
//! error codes come from NVIDIA's MIT-licensed SDK (github.com/NVIDIA/nvapi:
//! `nvapi_interface.h`, `nvapi.h`, `nvapi_lite_common.h`).

use std::ffi::c_void;

use windows::core::{s, w};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

use crate::backend::NvidiaSetting;
use crate::Error;

const ID_INITIALIZE: u32 = 0x0150_e828;
const ID_DRS_CREATE_SESSION: u32 = 0x0694_d52e;
const ID_DRS_DESTROY_SESSION: u32 = 0xdad9_cff8;
const ID_DRS_LOAD_SETTINGS: u32 = 0x375d_bd6b;
const ID_DRS_SAVE_SETTINGS: u32 = 0xfcbc_7e14;
const ID_DRS_GET_BASE_PROFILE: u32 = 0xda84_66a0;
const ID_DRS_GET_SETTING: u32 = 0x73bf_8338;
const ID_DRS_SET_SETTING: u32 = 0x577d_d202;
const ID_DRS_DELETE_PROFILE_SETTING: u32 = 0xe4a2_6362;

const NVAPI_OK: i32 = 0;
const NVAPI_NVIDIA_DEVICE_NOT_FOUND: i32 = -6;
const NVAPI_INVALID_USER_PRIVILEGE: i32 = -137;
const NVAPI_SETTING_NOT_FOUND: i32 = -160;

const NVDRS_DWORD_TYPE: u32 = 0;
const NVDRS_CURRENT_PROFILE_LOCATION: u32 = 0;

/// `NVDRS_SETTING_V1` under `#pragma pack(4)`. The value unions are kept as raw bytes
/// (their largest member, `NVDRS_BINARY_SETTING`, is 4 + 4096 bytes); DWORD values sit
/// in the first four.
#[repr(C)]
struct NvdrsSetting {
    version: u32,
    setting_name: [u16; 2048],
    setting_id: u32,
    setting_type: u32,
    setting_location: u32,
    is_current_predefined: u32,
    is_predefined_valid: u32,
    predefined_value: [u8; 4100],
    current_value: [u8; 4100],
}

const _: () = assert!(size_of::<NvdrsSetting>() == 12320);
/// `MAKE_NVAPI_VERSION(NVDRS_SETTING_V1, 1)`.
const NVDRS_SETTING_VER1: u32 = size_of::<NvdrsSetting>() as u32 | (1 << 16);

impl NvdrsSetting {
    fn boxed() -> Box<Self> {
        // 12 KB: keep it off the stack. All-zero is a valid value for this plain struct.
        let layout = std::alloc::Layout::new::<Self>();
        let mut s = unsafe {
            let ptr = std::alloc::alloc_zeroed(layout).cast::<Self>();
            if ptr.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            Box::from_raw(ptr)
        };
        s.version = NVDRS_SETTING_VER1;
        s
    }

    fn dword(bytes: &[u8; 4100]) -> u32 {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }
}

type QueryInterface = unsafe extern "C" fn(u32) -> *const c_void;
type Handle = *mut c_void;

/// Reinterprets a function pointer returned by NVAPI as its typed signature.
///
/// # Safety
/// `F` must be the `extern "C"` function type NVAPI documents for that pointer.
unsafe fn fn_ptr<F: Copy>(p: *const c_void) -> F {
    const { assert!(size_of::<F>() == size_of::<*const c_void>()) };
    unsafe { std::mem::transmute_copy::<*const c_void, F>(&p) }
}

struct Api {
    initialize: unsafe extern "C" fn() -> i32,
    create_session: unsafe extern "C" fn(*mut Handle) -> i32,
    destroy_session: unsafe extern "C" fn(Handle) -> i32,
    load_settings: unsafe extern "C" fn(Handle) -> i32,
    save_settings: unsafe extern "C" fn(Handle) -> i32,
    get_base_profile: unsafe extern "C" fn(Handle, *mut Handle) -> i32,
    get_setting: unsafe extern "C" fn(Handle, Handle, u32, *mut NvdrsSetting) -> i32,
    set_setting: unsafe extern "C" fn(Handle, Handle, *mut NvdrsSetting) -> i32,
    delete_profile_setting: unsafe extern "C" fn(Handle, Handle, u32) -> i32,
}

fn unavailable() -> Error {
    Error::Unavailable("no NVIDIA driver".into())
}

fn check(call: &str, status: i32) -> Result<(), Error> {
    match status {
        NVAPI_OK => Ok(()),
        NVAPI_NVIDIA_DEVICE_NOT_FOUND => Err(unavailable()),
        NVAPI_INVALID_USER_PRIVILEGE => Err(Error::AccessDenied("NVIDIA driver settings".into())),
        code => Err(Error::System(format!("NVAPI {call} failed ({code})"))),
    }
}

impl Api {
    fn load() -> Result<Self, Error> {
        unsafe {
            // The driver keeps nvapi64.dll in System32; it is never unloaded, as NVAPI
            // keeps internal state across calls.
            let module: HMODULE = LoadLibraryW(w!("nvapi64.dll")).map_err(|_| unavailable())?;
            let export =
                GetProcAddress(module, s!("nvapi_QueryInterface")).ok_or_else(unavailable)?;
            let query: QueryInterface = fn_ptr(export as *const c_void);
            let get = |id: u32| -> Result<*const c_void, Error> {
                let p = query(id);
                if p.is_null() {
                    Err(Error::Unavailable(format!(
                        "NVAPI function {id:#x} missing in this driver"
                    )))
                } else {
                    Ok(p)
                }
            };
            let api = Api {
                initialize: fn_ptr(get(ID_INITIALIZE)?),
                create_session: fn_ptr(get(ID_DRS_CREATE_SESSION)?),
                destroy_session: fn_ptr(get(ID_DRS_DESTROY_SESSION)?),
                load_settings: fn_ptr(get(ID_DRS_LOAD_SETTINGS)?),
                save_settings: fn_ptr(get(ID_DRS_SAVE_SETTINGS)?),
                get_base_profile: fn_ptr(get(ID_DRS_GET_BASE_PROFILE)?),
                get_setting: fn_ptr(get(ID_DRS_GET_SETTING)?),
                set_setting: fn_ptr(get(ID_DRS_SET_SETTING)?),
                delete_profile_setting: fn_ptr(get(ID_DRS_DELETE_PROFILE_SETTING)?),
            };
            check("Initialize", (api.initialize)())?;
            Ok(api)
        }
    }
}

/// A loaded DRS session on the global (base) profile, destroyed on drop.
struct Session {
    api: Api,
    session: Handle,
    profile: Handle,
}

impl Session {
    fn open() -> Result<Self, Error> {
        let api = Api::load()?;
        let mut session: Handle = std::ptr::null_mut();
        check("DRS_CreateSession", unsafe {
            (api.create_session)(&mut session)
        })?;
        let mut s = Session {
            api,
            session,
            profile: std::ptr::null_mut(),
        };
        check("DRS_LoadSettings", unsafe {
            (s.api.load_settings)(s.session)
        })?;
        let mut profile: Handle = std::ptr::null_mut();
        check("DRS_GetBaseProfile", unsafe {
            (s.api.get_base_profile)(s.session, &mut profile)
        })?;
        s.profile = profile;
        Ok(s)
    }

    fn save(&self) -> Result<(), Error> {
        check("DRS_SaveSettings", unsafe {
            (self.api.save_settings)(self.session)
        })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe { (self.api.destroy_session)(self.session) };
    }
}

pub fn get(setting: u32) -> Result<NvidiaSetting, Error> {
    let s = Session::open()?;
    let mut v = NvdrsSetting::boxed();
    match unsafe { (s.api.get_setting)(s.session, s.profile, setting, &mut *v) } {
        NVAPI_SETTING_NOT_FOUND => Ok(NvidiaSetting {
            value: None,
            user_set: false,
        }),
        status => {
            check("DRS_GetSetting", status)?;
            if v.setting_type != NVDRS_DWORD_TYPE {
                return Err(Error::System(format!(
                    "NVIDIA setting {setting:#x} is not a DWORD"
                )));
            }
            Ok(NvidiaSetting {
                value: Some(NvdrsSetting::dword(&v.current_value)),
                user_set: v.setting_location == NVDRS_CURRENT_PROFILE_LOCATION
                    && v.is_current_predefined == 0,
            })
        }
    }
}

pub fn set(setting: u32, value: u32) -> Result<(), Error> {
    let s = Session::open()?;
    let mut v = NvdrsSetting::boxed();
    v.setting_id = setting;
    v.setting_type = NVDRS_DWORD_TYPE;
    v.current_value[..4].copy_from_slice(&value.to_le_bytes());
    check("DRS_SetSetting", unsafe {
        (s.api.set_setting)(s.session, s.profile, &mut *v)
    })?;
    s.save()
}

pub fn reset(setting: u32) -> Result<(), Error> {
    let s = Session::open()?;
    match unsafe { (s.api.delete_profile_setting)(s.session, s.profile, setting) } {
        NVAPI_SETTING_NOT_FOUND => Ok(()),
        status => {
            check("DRS_DeleteProfileSetting", status)?;
            s.save()
        }
    }
}
