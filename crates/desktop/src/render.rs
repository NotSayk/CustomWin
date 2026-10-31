//! GPU side: one D3D11 device, one Media Foundation engine per video file, one
//! DirectComposition swap chain per wallpaper window.
//!
//! Decoding is done by the GPU (DXVA through the DXGI device manager) and frames are
//! copied GPU-to-GPU into the swap chain (`TransferVideoFrame`): the CPU only wakes up
//! once per display refresh to check for a new frame.

use std::path::{Path, PathBuf};

use windows::core::{implement, Interface, BSTR};
use windows::Win32::Foundation::{HMODULE, HWND, RECT, S_OK};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows_core::Result;

use crate::log;
use customwin_core::wallpaper::{frame, Fit};

pub struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    factory: IDXGIFactory2,
    /// `None` where DirectComposition is missing (Wine): plain HWND swap chains then.
    dcomp: Option<IDCompositionDevice>,
    manager: IMFDXGIDeviceManager,
    engines: IMFMediaEngineClassFactory,
    output: Option<IDXGIOutput>,
}

/// Logs which call failed (for `desktop.log`) and passes the error on.
fn step(name: &'static str) -> impl Fn(windows_core::Error) -> windows_core::Error {
    move |e| {
        log::write(&format!("{name} failed: {e}"));
        e
    }
}

impl Gpu {
    pub fn new() -> Result<Self> {
        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(step("MFStartup"))?;
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(step("D3D11CreateDevice"))?;
            let device = device.ok_or_else(windows_core::Error::empty)?;
            let context = context.ok_or_else(windows_core::Error::empty)?;
            // Media Foundation decodes on its own threads with the same device.
            let _ = device
                .cast::<ID3D11Multithread>()
                .map_err(step("ID3D11Multithread"))?
                .SetMultithreadProtected(true);

            let dxgi: IDXGIDevice = device.cast()?;
            let adapter = dxgi.GetAdapter()?;
            let factory: IDXGIFactory2 = adapter.GetParent()?;
            let output = adapter.EnumOutputs(0).ok();
            let dcomp: Option<IDCompositionDevice> = DCompositionCreateDevice(&dxgi)
                .map_err(step(
                    "DCompositionCreateDevice (falling back to HWND swap chains)",
                ))
                .ok();

            let mut token = 0u32;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager)
                .map_err(step("MFCreateDXGIDeviceManager"))?;
            let manager = manager.ok_or_else(windows_core::Error::empty)?;
            manager
                .ResetDevice(&device, token)
                .map_err(step("ResetDevice"))?;

            let engines: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER)
                    .map_err(step("MFMediaEngineClassFactory"))?;
            Ok(Gpu {
                device,
                context,
                factory,
                dcomp,
                manager,
                engines,
                output,
            })
        }
    }

    /// Whether windows must keep a GDI redirection surface (HWND swap chain fallback).
    pub fn needs_redirection(&self) -> bool {
        self.dcomp.is_none()
    }

    pub fn dxgi_device(&self) -> Result<IDXGIDevice> {
        self.device.cast()
    }

    /// A swap chain with per-pixel transparency shown in `hwnd`, for the widgets. Needs
    /// DirectComposition (`None` without it).
    pub fn transparent_chain(
        &self,
        hwnd: HWND,
        width: i32,
        height: i32,
    ) -> Result<Option<TransparentChain>> {
        let Some(dcomp) = &self.dcomp else {
            return Ok(None);
        };
        unsafe {
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width.max(1) as u32,
                Height: height.max(1) as u32,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                ..Default::default()
            };
            let swap = self
                .factory
                .CreateSwapChainForComposition(&self.device, &desc, None)
                .map_err(step("CreateSwapChainForComposition (widget)"))?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;
            Ok(Some(TransparentChain {
                swap,
                _composition: (target, visual),
            }))
        }
    }

    /// Blocks until the next vertical blank of the main display (~16 ms at 60 Hz).
    pub fn wait_vblank(&self) {
        match &self.output {
            Some(o) if unsafe { o.WaitForVBlank() }.is_ok() => {}
            _ => std::thread::sleep(std::time::Duration::from_millis(16)),
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        let _ = unsafe { MFShutdown() };
    }
}

/// Logs Media Foundation errors (unsupported codec, unreadable file…).
#[implement(IMFMediaEngineNotify)]
struct Notify {
    path: String,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, param1: usize, param2: u32) -> Result<()> {
        if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 as u32 {
            log::write(&format!(
                "video error in {}: MF_MEDIA_ENGINE_ERR {param1}, HRESULT {param2:#010x} \
                 (unsupported codec? WebM/VP9 and AV1 need the free Store extensions)",
                self.path
            ));
        }
        Ok(())
    }
}

pub struct Video {
    pub path: PathBuf,
    /// `None` once unloaded after a long pause: the decoder and its surfaces are the bulk
    /// of the process's memory, and are rebuilt when playback resumes.
    engine: Option<IMFMediaEngine>,
    playing: bool,
}

impl Video {
    pub fn open(gpu: &Gpu, path: &Path) -> Result<Self> {
        Ok(Video {
            path: path.to_path_buf(),
            engine: Some(Self::create_engine(gpu, path)?),
            playing: false,
        })
    }

    fn create_engine(gpu: &Gpu, path: &Path) -> Result<IMFMediaEngine> {
        unsafe {
            let notify: IMFMediaEngineNotify = Notify {
                path: path.display().to_string(),
            }
            .into();
            let mut attrs = None;
            MFCreateAttributes(&mut attrs, 3)?;
            let attrs = attrs.ok_or_else(windows_core::Error::empty)?;
            attrs.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, &gpu.manager)?;
            attrs.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify)?;
            attrs.SetUINT32(
                &MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT,
                DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32,
            )?;
            let engine = gpu
                .engines
                .CreateInstance(0, &attrs)
                .map_err(step("MediaEngine"))?;
            engine.SetMuted(true)?;
            engine.SetLoop(true)?;
            engine.SetAutoPlay(false)?;
            engine.SetSource(&BSTR::from(path.to_string_lossy().as_ref()))?;
            Ok(engine)
        }
    }

    pub fn set_playing(&mut self, gpu: &Gpu, play: bool) {
        if play && self.engine.is_none() {
            match Self::create_engine(gpu, &self.path) {
                Ok(e) => self.engine = Some(e),
                Err(e) => {
                    log::write(&format!("reload {}: {e}", self.path.display()));
                    return;
                }
            }
        }
        let Some(engine) = &self.engine else {
            return;
        };
        if play != self.playing {
            let r = unsafe {
                if play {
                    engine.Play()
                } else {
                    engine.Pause()
                }
            };
            match r {
                Ok(()) => self.playing = play,
                Err(e) => log::write(&format!("play/pause {}: {e}", self.path.display())),
            }
        }
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Releases the decoder of a paused video. Returns whether anything was freed.
    pub fn unload(&mut self) -> bool {
        if self.playing {
            return false;
        }
        match self.engine.take() {
            Some(engine) => {
                let _ = unsafe { engine.Shutdown() };
                true
            }
            None => false,
        }
    }

    /// Whether a new frame is ready. `OnVideoStreamTick` returns S_FALSE when there is
    /// none, which the high-level wrapper would report as success: call the vtable.
    pub fn new_frame(&self) -> bool {
        let Some(engine) = &self.engine else {
            return false;
        };
        let mut pts = 0i64;
        let hr = unsafe {
            (Interface::vtable(engine).OnVideoStreamTick)(Interface::as_raw(engine), &mut pts)
        };
        hr == S_OK
    }

    fn size(&self) -> (u32, u32) {
        let (mut w, mut h) = (0u32, 0u32);
        if let Some(engine) = &self.engine {
            let _ = unsafe { engine.GetNativeVideoSize(Some(&mut w), Some(&mut h)) };
        }
        (w, h)
    }
}

impl Drop for Video {
    fn drop(&mut self) {
        if let Some(engine) = &self.engine {
            let _ = unsafe { engine.Shutdown() };
        }
    }
}

pub struct TransparentChain {
    pub swap: IDXGISwapChain1,
    _composition: (IDCompositionTarget, IDCompositionVisual),
}

/// A DirectComposition swap chain shown in one window.
pub struct Surface {
    pub hwnd: HWND,
    swap: IDXGISwapChain1,
    width: i32,
    height: i32,
    _composition: Option<(IDCompositionTarget, IDCompositionVisual)>,
}

impl Surface {
    pub fn new(gpu: &Gpu, hwnd: HWND, width: i32, height: i32) -> Result<Self> {
        unsafe {
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width.max(1) as u32,
                Height: height.max(1) as u32,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                ..Default::default()
            };
            let Some(dcomp) = &gpu.dcomp else {
                let swap = gpu
                    .factory
                    .CreateSwapChainForHwnd(&gpu.device, hwnd, &desc, None, None)
                    .map_err(step("CreateSwapChainForHwnd"))?;
                return Ok(Surface {
                    hwnd,
                    swap,
                    width,
                    height,
                    _composition: None,
                });
            };
            let swap = gpu
                .factory
                .CreateSwapChainForComposition(&gpu.device, &desc, None)
                .map_err(step("CreateSwapChainForComposition"))?;
            let target = dcomp
                .CreateTargetForHwnd(hwnd, true)
                .map_err(step("CreateTargetForHwnd"))?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;
            Ok(Surface {
                hwnd,
                swap,
                width,
                height,
                _composition: Some((target, visual)),
            })
        }
    }

    /// Copies the video's current frame into the window and presents it. Returns false
    /// while the video size is not known yet (nothing is drawn).
    pub fn draw(&self, gpu: &Gpu, video: &Video, fit: Fit) -> Result<bool> {
        let Some(engine) = &video.engine else {
            return Ok(false);
        };
        let (vw, vh) = video.size();
        let Some((src, dst)) = frame(fit, vw, vh, self.width, self.height) else {
            return Ok(false);
        };
        unsafe {
            let back: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            if fit == Fit::Contain {
                // Black bars: flip-model buffers start undefined.
                let mut rtv = None;
                gpu.device
                    .CreateRenderTargetView(&back, None, Some(&mut rtv))?;
                if let Some(rtv) = rtv {
                    gpu.context
                        .ClearRenderTargetView(&rtv, &[0.0, 0.0, 0.0, 1.0]);
                }
            }
            let src = MFVideoNormalizedRect {
                left: src.left,
                top: src.top,
                right: src.right,
                bottom: src.bottom,
            };
            let dst = RECT {
                left: dst.left,
                top: dst.top,
                right: dst.right,
                bottom: dst.bottom,
            };
            let black = MFARGB {
                rgbBlue: 0,
                rgbGreen: 0,
                rgbRed: 0,
                rgbAlpha: 255,
            };
            engine.TransferVideoFrame(&back, Some(&src), &dst, Some(&black))?;
            self.swap.Present(0, DXGI_PRESENT(0)).ok()?;
        }
        Ok(true)
    }
}
