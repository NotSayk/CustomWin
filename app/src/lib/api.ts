// Thin wrapper over the Tauri commands defined in src-tauri/src/main.rs.
// Outside Tauri (plain `npm run dev` in a browser) it falls back to an in-memory mock so
// the interface can be worked on from any OS.

import type {
  Analysis,
  Answers,
  CatalogView,
  DriverCheck,
  Operation,
  Recommendation,
  RunResult,
  StatesView,
  ThemeFile,
  ThemeState,
  ThemeView,
  WallpaperConfig,
  WallpaperView,
  WidgetsConfig,
  WidgetsView,
} from "./types";

type Invoke = <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;

const inTauri = "__TAURI_INTERNALS__" in window;

let invokeImpl: Promise<Invoke> | null = null;
function backend(): Promise<Invoke> {
  invokeImpl ??= inTauri
    ? import("@tauri-apps/api/core").then((m) => m.invoke as Invoke)
    : import.meta.env.DEV
      ? import("./mock").then((m) => m.invoke as Invoke)
      : Promise.reject(new Error("CustomWin must run inside its Windows app"));
  return invokeImpl;
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const invoke = await backend();
  return invoke<T>(cmd, args);
}

export const api = {
  catalog: () => call<CatalogView>("get_catalog"),
  states: () => call<StatesView>("get_states"),
  analyze: () => call<Analysis>("analyze"),
  recommend: (answers: Answers) => call<Recommendation[]>("recommend", { answers }),
  run: (operation: Operation, restorePoint: boolean) => call<RunResult>("run_operation", { operation, restorePoint }),
  restartExplorer: () => call<void>("restart_explorer"),
  /** Goes online: only on an explicit click. */
  checkDrivers: () => call<DriverCheck[]>("check_drivers"),
  openNvidiaPage: (url: string) => call<void>("open_nvidia_page", { url }),
  getWallpaper: () => call<WallpaperView>("get_wallpaper"),
  setWallpaper: (config: WallpaperConfig) => call<WallpaperView>("set_wallpaper", { config }),
  pickVideo: () => call<string | null>("pick_video"),
  getWidgets: () => call<WidgetsView>("get_widgets"),
  setWidgets: (config: WidgetsConfig) => call<WidgetsView>("set_widgets", { config }),
  getTheme: () => call<ThemeView>("get_theme"),
  setTheme: (state: ThemeState) => call<ThemeState>("set_theme", { state }),
  listWindowsThemes: () => call<ThemeFile[]>("list_windows_themes"),
  applyWindowsTheme: (path: string) => call<ThemeState>("apply_windows_theme", { path }),
};
