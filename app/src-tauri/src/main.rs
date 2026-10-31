//! CustomWin desktop app. Runs unelevated; anything that needs administrator rights goes
//! through `customwin-helper.exe` and a single UAC prompt per operation.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use customwin_core::analysis::{self, Answers, Insight, Recommendation, SystemInfo};
use customwin_core::drivers::DriverCheck;
use customwin_core::engine::{self, TweakState};
use customwin_core::journal::Journal;
use customwin_core::wallpaper::{self, WallpaperConfig};
use customwin_core::windows::{self as win, WindowsBackend};
use customwin_core::{Catalog, HelperRequest, HelperResponse, OpResult, Operation};
use serde::Serialize;
use tauri::State;

struct AppState {
    catalog: Arc<Catalog>,
    catalog_json: serde_json::Value,
    /// Last analysis, reused by `recommend` for laptop detection.
    info: Mutex<Option<SystemInfo>>,
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Runs blocking work (registry, services, UAC wait) off the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> CmdResult<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(err)?
}

#[tauri::command]
fn get_catalog(state: State<'_, AppState>) -> serde_json::Value {
    state.catalog_json.clone()
}

#[derive(Serialize)]
struct StatesView {
    states: HashMap<String, TweakState>,
    journaled: Vec<String>,
    elevated: bool,
}

#[tauri::command]
async fn get_states(state: State<'_, AppState>) -> CmdResult<StatesView> {
    let catalog = state.catalog.clone();
    blocking(move || {
        let states = catalog
            .tweaks
            .iter()
            .map(|t| (t.id.clone(), engine::state(&WindowsBackend, t)))
            .collect();
        let journal = Journal::load(&win::journal_path()).map_err(err)?;
        Ok(StatesView {
            states,
            journaled: journal.entries.into_iter().map(|e| e.tweak_id).collect(),
            elevated: win::is_elevated(),
        })
    })
    .await
}

#[derive(Serialize)]
struct AnalysisView {
    info: SystemInfo,
    insights: Vec<Insight>,
}

#[tauri::command]
async fn analyze(state: State<'_, AppState>) -> CmdResult<AnalysisView> {
    let info = blocking(|| Ok(win::collect_system_info())).await?;
    let today = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    let insights = analysis::insights(&info, today);
    *state.info.lock().map_err(err)? = Some(info.clone());
    Ok(AnalysisView { info, insights })
}

#[tauri::command]
async fn recommend(state: State<'_, AppState>, answers: Answers) -> CmdResult<Vec<Recommendation>> {
    let catalog = state.catalog.clone();
    let info = state.info.lock().map_err(err)?.clone().unwrap_or_default();
    blocking(move || {
        Ok(analysis::recommend(&catalog, &answers, &info, |t| {
            engine::state(&WindowsBackend, t)
        }))
    })
    .await
}

#[derive(Serialize)]
struct RunResult {
    results: Vec<OpResult>,
    warnings: Vec<String>,
}

#[tauri::command]
async fn run_operation(
    state: State<'_, AppState>,
    operation: Operation,
    restore_point: bool,
) -> CmdResult<RunResult> {
    let catalog = state.catalog.clone();
    blocking(move || {
        let journal_path = win::journal_path();
        let journal = Journal::load(&journal_path).map_err(err)?;
        let needs_admin = engine::requires_admin(&catalog, &journal, &operation);

        if needs_admin && !win::is_elevated() {
            // A restore point is only worth it for system-wide changes; per-user ones
            // are fully covered by the journal.
            return run_through_helper(HelperRequest {
                operation,
                journal_path,
                restore_point,
                user_sid: win::current_user_sid(),
            });
        }

        let mut warnings = Vec::new();
        if needs_admin && restore_point && matches!(operation, Operation::Apply(_)) {
            if let Err(e) = win::create_restore_point("CustomWin") {
                warnings.push(format!("no restore point: {e}"));
            }
        }
        let results =
            engine::run(&WindowsBackend, &catalog, &journal_path, &operation).map_err(err)?;
        Ok(RunResult { results, warnings })
    })
    .await
}

fn helper_path() -> CmdResult<PathBuf> {
    let exe = std::env::current_exe().map_err(err)?;
    let helper = exe.with_file_name("customwin-helper.exe");
    if helper.exists() {
        Ok(helper)
    } else {
        Err(format!(
            "{} is missing; reinstall CustomWin",
            helper.display()
        ))
    }
}

fn run_through_helper(request: HelperRequest) -> CmdResult<RunResult> {
    let dir = win::data_dir().join("requests");
    std::fs::create_dir_all(&dir).map_err(err)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let request_path = dir.join(format!("{stamp}.json"));
    let response_path = dir.join(format!("{stamp}.json.response.json"));
    std::fs::write(&request_path, serde_json::to_vec(&request).map_err(err)?).map_err(err)?;

    let outcome = win::run_elevated(
        &helper_path()?,
        &format!("--request \"{}\"", request_path.display()),
    );
    let response = std::fs::read(&response_path);
    let _ = std::fs::remove_file(&request_path);
    let _ = std::fs::remove_file(&response_path);
    outcome.map_err(err)?;

    let response: HelperResponse =
        serde_json::from_slice(&response.map_err(|e| format!("helper gave no answer: {e}"))?)
            .map_err(err)?;
    match response.error {
        Some(e) => Err(e),
        None => Ok(RunResult {
            results: response.results,
            warnings: response.warnings,
        }),
    }
}

/// Online, on demand only: compares installed NVIDIA drivers with NVIDIA's latest.
#[tauri::command]
async fn check_drivers(state: State<'_, AppState>) -> CmdResult<Vec<DriverCheck>> {
    let cached = state.info.lock().map_err(err)?.clone();
    blocking(move || {
        let info = cached.unwrap_or_else(win::collect_system_info);
        win::check_nvidia_drivers(&info.gpus).map_err(err)
    })
    .await
}

/// Opens an NVIDIA page in the default browser. Only nvidia.com links are accepted.
#[tauri::command]
async fn open_nvidia_page(url: String) -> CmdResult<()> {
    let safe = url.starts_with("https://www.nvidia.com/")
        && url
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/:.-_?=&%".contains(c));
    if !safe {
        return Err("refused URL".into());
    }
    blocking(move || win::open_url(&url).map_err(err)).await
}

#[derive(Serialize)]
struct WallpaperView {
    config: WallpaperConfig,
    monitors: Vec<win::Monitor>,
    running: bool,
}

fn desktop_exe() -> CmdResult<PathBuf> {
    let exe = std::env::current_exe()
        .map_err(err)?
        .with_file_name("customwin-desktop.exe");
    if exe.exists() {
        Ok(exe)
    } else {
        Err(format!("{} is missing; reinstall CustomWin", exe.display()))
    }
}

fn wallpaper_view(config: WallpaperConfig, running: bool) -> WallpaperView {
    WallpaperView {
        config,
        monitors: win::monitors(),
        running,
    }
}

#[tauri::command]
async fn get_wallpaper() -> CmdResult<WallpaperView> {
    blocking(|| {
        let config = WallpaperConfig::load(&win::wallpaper_config_path()).map_err(err)?;
        Ok(wallpaper_view(config, win::desktop_running()))
    })
    .await
}

/// Saves the configuration and starts, reloads or stops the wallpaper process to match.
#[tauri::command]
async fn set_wallpaper(config: WallpaperConfig) -> CmdResult<WallpaperView> {
    blocking(move || {
        // Only check what the user just picked: a video that was already configured may have
        // been deleted since, and must still be removable or replaceable.
        let previous = WallpaperConfig::load(&win::wallpaper_config_path()).unwrap_or_default();
        for v in config.added_videos(&previous) {
            if !wallpaper::is_video(v) || !v.is_file() {
                return Err(format!("not a readable video: {}", v.display()));
            }
        }
        config.save(&win::wallpaper_config_path()).map_err(err)?;

        let play = config.enabled && config.has_video();
        let exe = desktop_exe()?;
        win::set_autostart((play && config.start_with_windows).then_some(exe.as_path()))
            .map_err(err)?;
        if !play {
            win::desktop_stop();
        } else if !win::desktop_reload() {
            win::desktop_start(&exe).map_err(err)?;
        }
        // Give the process a moment to show up (or go away) before reporting.
        let mut running = win::desktop_running();
        for _ in 0..15 {
            if running == play {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            running = win::desktop_running();
        }
        Ok(wallpaper_view(config, running))
    })
    .await
}

#[tauri::command]
async fn pick_video(window: tauri::WebviewWindow) -> CmdResult<Option<String>> {
    let owner = window.hwnd().map_or(0, |h| h.0 as isize);
    blocking(move || {
        Ok(win::pick_video(owner, "Choisir une vidéo")
            .map_err(err)?
            .map(|p| p.display().to_string()))
    })
    .await
}

#[tauri::command]
async fn restart_explorer() -> CmdResult<()> {
    blocking(|| win::restart_explorer().map_err(err)).await
}

fn main() {
    let catalog = Catalog::embedded().expect("the embedded catalog is validated by tests");
    let catalog_json = serde_json::to_value(catalog.view()).expect("catalog serializes");
    tauri::Builder::default()
        .manage(AppState {
            catalog: Arc::new(catalog),
            catalog_json,
            info: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            get_catalog,
            get_states,
            analyze,
            recommend,
            run_operation,
            check_drivers,
            open_nvidia_page,
            get_wallpaper,
            set_wallpaper,
            pick_video,
            restart_explorer
        ])
        .run(tauri::generate_context!())
        .expect("error while running CustomWin");
}
