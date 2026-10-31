//! The XAML diagnostics extension ("TAP") itself.
//!
//! Windows loads the DLL into Explorer, creates our object through `DllGetClassObject`
//! and hands it a site implementing `IXamlDiagnostics` and `IVisualTreeService`. We then
//! subscribe to visual tree changes: the whole existing tree is replayed first, then every
//! later change (a taskbar appears on a new screen, Explorer rebuilds it…).
//!
//! Callbacks arrive on the XAML UI thread that owns the element, which is the only thread
//! allowed to touch it. A message-only window is created on each such thread so that
//! `customwin-desktop.exe` can switch the effect on and off later: its window procedure
//! runs on that same thread.

// COM method names follow xamlOM.h.
#![allow(non_snake_case)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::core::{implement, interface, w, Interface, Ref, BSTR, GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_POINTER, HINSTANCE, HMODULE, HWND, LPARAM,
    LRESULT, S_FALSE, S_OK, WPARAM,
};
use windows::Win32::System::Com::{
    CoInitializeEx, IClassFactory, IClassFactory_Impl, COINIT_MULTITHREADED,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows::Win32::System::Ole::{IObjectWithSite, IObjectWithSite_Impl};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassW, HWND_MESSAGE, WINDOW_EX_STYLE, WINDOW_STYLE,
    WNDCLASSW,
};
use windows_core::{IInspectable, IUnknown, IUnknown_Vtbl};

use crate::{CLSID, HOOK_CLASS, INIT_TRANSPARENT, MSG_SET};

// ---- xamlOM.h ----

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SourceInfo {
    file_name: *const u16,
    line: u32,
    column: u32,
    char_position: u32,
    hash: *const u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ParentChildRelation {
    parent: u64,
    child: u64,
    child_index: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct VisualElement {
    handle: u64,
    source: SourceInfo,
    type_name: *const u16,
    name: *const u16,
    children: u32,
}

const MUTATION_ADD: i32 = 0;

#[interface("AA7A8931-80E4-4FEC-8F3B-553F87B4966E")]
unsafe trait IVisualTreeServiceCallback: IUnknown {
    fn OnVisualTreeChange(
        &self,
        relation: ParentChildRelation,
        element: VisualElement,
        mutation: i32,
    ) -> HRESULT;
}

#[interface("BAD9EB88-AE77-4397-B948-5FA2DB0A19EA")]
unsafe trait IVisualTreeServiceCallback2: IVisualTreeServiceCallback {
    fn OnElementStateChanged(&self, element: u64, state: i32, context: PCWSTR) -> HRESULT;
}

/// Only the first method is called; the rest of the vtable is never touched.
#[interface("A593B11A-D17F-48BB-8F66-83910731C8A5")]
unsafe trait IVisualTreeService: IUnknown {
    fn AdviseVisualTreeChange(&self, callback: *mut c_void) -> HRESULT;
}

#[interface("18C9E2B6-3F43-4116-9F2B-FF935D7770D2")]
unsafe trait IXamlDiagnostics: IUnknown {
    fn GetDispatcher(&self, out: *mut *mut c_void) -> HRESULT;
    fn GetUiLayer(&self, out: *mut *mut c_void) -> HRESULT;
    fn GetApplication(&self, out: *mut *mut c_void) -> HRESULT;
    fn GetIInspectableFromHandle(&self, handle: u64, out: *mut *mut c_void) -> HRESULT;
    fn GetHandleFromIInspectable(&self, instance: *mut c_void, out: *mut u64) -> HRESULT;
    fn HitTest(
        &self,
        rect: windows::Win32::Foundation::RECT,
        count: *mut u32,
        out: *mut *mut u64,
    ) -> HRESULT;
    fn RegisterInstance(&self, instance: *mut c_void, out: *mut u64) -> HRESULT;
    fn GetInitializationData(&self, out: *mut BSTR) -> HRESULT;
}

// ---- windows.ui.xaml.h: the start of Windows.UI.Xaml.IUIElement ----

/// Declared on IUnknown with IInspectable's three methods spelled out (windows-core has
/// no implementable IInspectable base for `#[interface]`).
#[interface("676d0be9-b65c-41c6-ba40-58cf87f201c1")]
unsafe trait IUIElement: IUnknown {
    fn GetIids(&self, count: *mut u32, iids: *mut *mut GUID) -> HRESULT;
    fn GetRuntimeClassName(&self, out: *mut *mut c_void) -> HRESULT;
    fn GetTrustLevel(&self, out: *mut i32) -> HRESULT;
    fn get_DesiredSize(&self, out: *mut [f32; 2]) -> HRESULT;
    fn get_AllowDrop(&self, out: *mut u8) -> HRESULT;
    fn put_AllowDrop(&self, value: u8) -> HRESULT;
    fn get_Opacity(&self, out: *mut f64) -> HRESULT;
    fn put_Opacity(&self, value: f64) -> HRESULT;
}

// ---- state ----

static TRANSPARENT: AtomicBool = AtomicBool::new(false);

struct Node {
    type_name: String,
    parent: u64,
}

/// Every element seen so far, to check a rectangle's ancestors. Shared by all UI threads.
static TREE: Mutex<Option<HashMap<u64, Node>>> = Mutex::new(None);

struct Target {
    handle: u64,
    element: IUIElement,
    original: f64,
}

thread_local! {
    /// Background rectangles owned by this UI thread.
    static TARGETS: RefCell<Vec<Target>> = const { RefCell::new(Vec::new()) };
    static HOOK: RefCell<Option<HWND>> = const { RefCell::new(None) };
}

fn log(msg: &str) {
    let Some(dir) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let path = std::path::Path::new(&dir)
        .join("CustomWin")
        .join("taskbar.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 64 * 1024) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "[thread {}] {msg}", unsafe { GetCurrentThreadId() });
    }
}

fn text(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    // A BSTR is length-prefixed, but the callback strings are also NUL-terminated.
    let len = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

fn apply(target: &Target, transparent: bool) {
    let value = if transparent { 0.0 } else { target.original };
    let hr = unsafe { target.element.put_Opacity(value) };
    if hr.is_err() {
        log(&format!("put_Opacity({value}) failed: {hr:?}"));
    }
}

/// Applies the current setting to every rectangle of the calling thread.
fn apply_all() {
    let transparent = TRANSPARENT.load(Ordering::Relaxed);
    TARGETS.with(|t| {
        if let Ok(t) = t.try_borrow() {
            t.iter().for_each(|x| apply(x, transparent));
        }
    });
}

unsafe extern "system" fn hook_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == MSG_SET {
        TRANSPARENT.store(wparam.0 != 0, Ordering::Relaxed);
        apply_all();
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn this_module() -> HINSTANCE {
    let mut module = HMODULE::default();
    unsafe {
        let _ = GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(hook_proc as *const u16),
            &mut module,
        );
    }
    module.into()
}

/// Creates this thread's hook window once.
fn ensure_hook() {
    HOOK.with(|h| {
        let Ok(mut h) = h.try_borrow_mut() else {
            return;
        };
        if h.is_some() {
            return;
        }
        let class: Vec<u16> = HOOK_CLASS.encode_utf16().chain([0]).collect();
        let instance = this_module();
        unsafe {
            // Fails harmlessly when another thread already registered it.
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(hook_proc),
                hInstance: instance,
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            });
            match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                PCWSTR(class.as_ptr()),
                w!("CustomWin taskbar"),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance),
                None,
            ) {
                Ok(hwnd) => *h = Some(hwnd),
                Err(e) => log(&format!("hook window: {e}")),
            }
        }
    });
}

fn is_taskbar_background(relation: &ParentChildRelation, element: &VisualElement) -> bool {
    let name = text(element.name);
    if name != "BackgroundFill" && name != "BackgroundStroke" {
        return false;
    }
    let Ok(tree) = TREE.lock() else {
        return false;
    };
    let Some(tree) = tree.as_ref() else {
        return false;
    };
    // The taskbar's own controls live in the `Taskbar` namespace.
    let mut at = relation.parent;
    for _ in 0..64 {
        let Some(node) = tree.get(&at) else {
            return false;
        };
        if node.type_name.starts_with("Taskbar.") {
            return true;
        }
        at = node.parent;
    }
    false
}

#[implement(IVisualTreeServiceCallback2)]
struct Watcher {
    diag: IXamlDiagnostics,
}

impl IVisualTreeServiceCallback_Impl for Watcher_Impl {
    unsafe fn OnVisualTreeChange(
        &self,
        relation: ParentChildRelation,
        element: VisualElement,
        mutation: i32,
    ) -> HRESULT {
        if let Ok(mut tree) = TREE.lock() {
            let tree = tree.get_or_insert_with(HashMap::new);
            if mutation == MUTATION_ADD {
                tree.insert(
                    element.handle,
                    Node {
                        type_name: text(element.type_name),
                        parent: relation.parent,
                    },
                );
            } else {
                tree.remove(&element.handle);
            }
        }
        if mutation != MUTATION_ADD {
            TARGETS.with(|t| {
                if let Ok(mut t) = t.try_borrow_mut() {
                    t.retain(|x| x.handle != element.handle);
                }
            });
            return S_OK;
        }
        if !is_taskbar_background(&relation, &element) {
            return S_OK;
        }

        let mut raw = std::ptr::null_mut();
        let hr = unsafe {
            self.diag
                .GetIInspectableFromHandle(element.handle, &mut raw)
        };
        if hr.is_err() || raw.is_null() {
            log(&format!("GetIInspectableFromHandle: {hr:?}"));
            return S_OK;
        }
        let inspectable = unsafe { IInspectable::from_raw(raw) };
        let Ok(ui) = inspectable.cast::<IUIElement>() else {
            log("background is not a UIElement");
            return S_OK;
        };
        let mut original = 1.0;
        let _ = unsafe { ui.get_Opacity(&mut original) };
        if original <= 0.0 {
            original = 1.0; // already hidden by an earlier copy: 1 is the shell's own value
        }
        let target = Target {
            handle: element.handle,
            element: ui,
            original,
        };
        apply(&target, TRANSPARENT.load(Ordering::Relaxed));
        log(&format!(
            "found {} (opacity {original})",
            text(element.name)
        ));
        ensure_hook();
        TARGETS.with(|t| {
            if let Ok(mut t) = t.try_borrow_mut() {
                t.push(target);
            }
        });
        S_OK
    }
}

impl IVisualTreeServiceCallback2_Impl for Watcher_Impl {
    unsafe fn OnElementStateChanged(&self, _: u64, _: i32, _: PCWSTR) -> HRESULT {
        S_OK
    }
}

/// Moves a COM pointer to the thread that subscribes (subscribing from the UI thread that
/// calls `SetSite` would deadlock: the existing tree is replayed on it synchronously).
struct SendPtr<T>(T);
unsafe impl<T> Send for SendPtr<T> {}

#[implement(IObjectWithSite)]
struct Tap {
    site: Mutex<Option<IUnknown>>,
}

impl IObjectWithSite_Impl for Tap_Impl {
    fn SetSite(&self, site: Ref<IUnknown>) -> windows_core::Result<()> {
        let site = site.cloned();
        if let Ok(mut s) = self.site.lock() {
            *s = site.clone();
        }
        let Some(site) = site else {
            return Ok(());
        };
        let diag: IXamlDiagnostics = site.cast()?;
        let service: IVisualTreeService = site.cast()?;
        let mut init = BSTR::new();
        let _ = unsafe { diag.GetInitializationData(&mut init) };
        TRANSPARENT.store(init == INIT_TRANSPARENT, Ordering::Relaxed);
        log(&format!("loaded into Explorer ({init})"));

        let watcher: IVisualTreeServiceCallback2 = Watcher { diag }.into();
        let job = SendPtr((service, watcher));
        std::thread::spawn(move || {
            let job = job;
            let (service, watcher) = &job.0;
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                let hr = service.AdviseVisualTreeChange(watcher.as_raw());
                if hr.is_err() {
                    log(&format!("AdviseVisualTreeChange failed: {hr:?}"));
                }
            }
        });
        Ok(())
    }

    fn GetSite(&self, iid: *const GUID, out: *mut *mut c_void) -> windows_core::Result<()> {
        if out.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        let site = self.site.lock().ok().and_then(|s| s.clone());
        match site {
            Some(s) => unsafe { s.query(iid, out).ok() },
            None => {
                unsafe { *out = std::ptr::null_mut() };
                Err(windows::Win32::Foundation::E_FAIL.into())
            }
        }
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        if out.is_null() || iid.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe { *out = std::ptr::null_mut() };
        if outer.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let tap: IObjectWithSite = Tap {
            site: Mutex::new(None),
        }
        .into();
        unsafe { tap.query(iid, out).ok() }
    }

    fn LockServer(&self, _: windows_core::BOOL) -> windows_core::Result<()> {
        Ok(())
    }
}

#[no_mangle]
unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if clsid.is_null() || iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    unsafe { *out = std::ptr::null_mut() };
    if unsafe { *clsid } != GUID::from_u128(CLSID) {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = Factory.into();
    unsafe { factory.query(iid, out) }
}

/// Explorer keeps the DLL until it exits: the hook windows and subscriptions live in it.
#[no_mangle]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}
