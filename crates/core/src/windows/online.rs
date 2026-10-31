//! The only network access CustomWin makes, and only when the user asks for it: looking
//! up the latest NVIDIA driver. Uses WinHTTP (system proxy, system certificates) so it
//! adds no dependency.

use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

use crate::analysis::{GpuInfo, GpuVendor};
use crate::drivers::{self, DriverCheck};
use crate::Error;

struct Handle(*mut std::ffi::c_void);

impl Handle {
    fn new(h: *mut std::ffi::c_void, what: &str) -> Result<Self, Error> {
        if h.is_null() {
            Err(Error::System(format!(
                "{what} failed: {}",
                windows::core::Error::from_thread().message()
            )))
        } else {
            Ok(Handle(h))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// HTTPS GET, returns the body. Bodies are capped at 4 MB.
fn https_get(host: &str, path: &str) -> Result<String, Error> {
    let net = |e: windows::core::Error| Error::System(format!("{host}: {}", e.message()));
    unsafe {
        let session = Handle::new(
            WinHttpOpen(
                &HSTRING::from(concat!("CustomWin/", env!("CARGO_PKG_VERSION"))),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ),
            "WinHttpOpen",
        )?;
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 10_000, 20_000).map_err(net)?;
        let connect = Handle::new(
            WinHttpConnect(
                session.0,
                &HSTRING::from(host),
                INTERNET_DEFAULT_HTTPS_PORT,
                0,
            ),
            host,
        )?;
        let request = Handle::new(
            WinHttpOpenRequest(
                connect.0,
                &HSTRING::from("GET"),
                &HSTRING::from(path),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ),
            host,
        )?;
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(net)?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(net)?;

        let mut status = 0u32;
        let mut len = size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as _),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(net)?;
        if status != 200 {
            return Err(Error::System(format!("{host} answered HTTP {status}")));
        }

        let mut body = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                chunk.as_mut_ptr().cast(),
                chunk.len() as u32,
                &mut read,
            )
            .map_err(net)?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > 4 << 20 {
                return Err(Error::System(format!("{host}: response too large")));
            }
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }
}

/// Looks up the latest Game Ready driver for each NVIDIA GPU.
pub fn check_nvidia_drivers(gpus: &[GpuInfo]) -> Result<Vec<DriverCheck>, Error> {
    let nvidia: Vec<&GpuInfo> = gpus
        .iter()
        .filter(|g| g.vendor == GpuVendor::Nvidia)
        .collect();
    if nvidia.is_empty() {
        return Ok(Vec::new());
    }
    let products = drivers::parse_products(&https_get(drivers::LOOKUP_HOST, drivers::LOOKUP_PATH)?);
    let mut out = Vec::new();
    for gpu in nvidia {
        let Some(product) = drivers::find_product(&products, &gpu.name) else {
            continue;
        };
        let body = https_get(
            drivers::DRIVER_HOST,
            &drivers::driver_path(product.psid, product.pfid),
        )?;
        if let Some(latest) = drivers::parse_latest(&body) {
            out.push(drivers::check(
                &gpu.name,
                gpu.driver_display_version.as_deref(),
                latest,
            ));
        }
    }
    Ok(out)
}
