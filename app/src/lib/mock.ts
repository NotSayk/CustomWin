// Browser-only stand-in for the Tauri backend, used by `npm run dev` outside Windows.
// The catalog comes from `npm run mock-catalog` (exported by the Rust helper), so it is the
// real one; the machine and the system state are simulated.

import type {
  Analysis,
  Answers,
  CatalogView,
  Monitor,
  Operation,
  Recommendation,
  RunResult,
  StatesView,
  ThemeFile,
  ThemeState,
  TweakState,
  WallpaperConfig,
  WidgetsConfig,
} from "./types";

const monitors: Monitor[] = [
  {
    device: "\\\\.\\DISPLAY1",
    rect: { left: 0, top: 0, right: 2560, bottom: 1440 },
    work: { left: 0, top: 0, right: 2560, bottom: 1392 },
    primary: true,
  },
  {
    device: "\\\\.\\DISPLAY2",
    rect: { left: 2560, top: 180, right: 4480, bottom: 1260 },
    work: { left: 2560, top: 180, right: 4480, bottom: 1212 },
    primary: false,
  },
];

let wallpaper: WallpaperConfig = {
  enabled: true,
  layout: "per_screen",
  screens: [{ device: "\\\\.\\DISPLAY1", video: "C:\\Users\\Moi\\Videos\\Wallpapers\\pluie-neon-tokyo.mp4", fit: "cover" }],
  span_video: null,
  span_fit: "cover",
  pause_when_maximized: true,
  pause_on_battery: true,
  start_with_windows: false,
};

let widgets: WidgetsConfig = {
  widgets: [
    { kind: "clock", enabled: true, device: null, corner: "top_right" },
    { kind: "system", enabled: false, device: null, corner: "top_right" },
    { kind: "media", enabled: false, device: null, corner: "bottom_right" },
  ],
  transparent_taskbar: false,
};

let theme: ThemeState = {
  apps_light: false,
  system_light: false,
  transparency: true,
  accent: 0x0078d4,
  auto_accent: false,
  accent_on_taskbar: false,
  accent_on_title_bars: false,
};

const themes: ThemeFile[] = [
  ["Windows (clair)", 0x0078d4, true, "#9cc7ef"],
  ["Windows (sombre)", 0x0078d4, false, "#1b2a4a"],
  ["Éclat", 0x680081, false, "#3b1450"],
  ["Lever de soleil", 0x459bac, true, "#f0b27a"],
].map(([name, accent, light, color]) => ({
  path: `C:\Windows\Resources\Themes\${name}.theme`,
  name: name as string,
  wallpaper: null,
  accent: accent as number,
  auto_accent: false,
  apps_light: light as boolean,
  system_light: light as boolean,
  thumbnail: `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="192" height="108"><rect width="192" height="108" fill="${color}"/></svg>`)}`,
}));

const accents = [
  0x0078d4, 0x0063b1, 0x8764b8, 0x744da9, 0xb146c2, 0xe3008c, 0xea005e, 0xe81123, 0xca5010, 0xf7630c, 0xffb900,
  0x498205, 0x107c10, 0x00b294, 0x038387, 0x2d7d9a, 0x567c73, 0x68768a, 0x515c6b, 0x4c4a48,
];

let catalog: CatalogView | null = null;
const applied = new Set<string>(["explorer.show_extensions"]);
const journaled = new Set<string>();

async function getCatalog(): Promise<CatalogView> {
  if (!catalog) {
    const res = await fetch("/mock-catalog.json");
    if (!res.ok) throw new Error("run `npm run mock-catalog` first");
    catalog = (await res.json()) as CatalogView;
  }
  return catalog;
}

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));

const analysis: Analysis = {
  info: {
    os: { name: "Windows 11 Pro", display_version: "25H2", build: 26200, ubr: 6899, edition: "Professional" },
    cpu: { name: "AMD Ryzen 7 7800X3D 8-Core Processor", logical_cores: 16 },
    memory_bytes: 32 * 2 ** 30,
    gpus: [
      {
        name: "NVIDIA GeForce RTX 4070",
        vendor: "nvidia",
        vram_bytes: 12 * 2 ** 30,
        driver_version: "32.0.15.6094",
        driver_display_version: "560.94",
        driver_date: "2024-08-14",
      },
    ],
    power: { active_scheme: "381b4222-f694-41f0-9685-ff5bb260df2e", active_scheme_name: "Équilibré", has_battery: false, on_battery: false },
    features: { game_mode: null, hags: false, memory_integrity: true, background_recording: true },
    network: [
      { name: "Ethernet", description: "Realtek Gaming 2.5GbE", kind: "ethernet", connected: true, link_speed_bps: 100_000_000 },
      { name: "Wi-Fi", description: "MediaTek Wi-Fi 6E", kind: "wifi", connected: false, link_speed_bps: 0 },
    ],
  },
  insights: [
    {
      id: "gpu.driver_old",
      severity: "warning",
      title: { fr: "Pilote NVIDIA ancien (560.94)", en: "Old NVIDIA driver (560.94)" },
      detail: {
        fr: "Ton pilote pour NVIDIA GeForce RTX 4070 a 26 mois. Les nouveaux pilotes corrigent des bugs et améliorent les jeux récents : mets-le à jour via l'application NVIDIA.",
        en: "Your NVIDIA GeForce RTX 4070 driver is 26 months old. Newer drivers fix bugs and improve recent games: update it through the NVIDIA app.",
      },
      tweak: null,
    },
    {
      id: "network.ethernet_100m",
      severity: "warning",
      title: { fr: "Ethernet limité à 100 Mb/s (Ethernet)", en: "Ethernet capped at 100 Mb/s (Ethernet)" },
      detail: {
        fr: "La carte négocie 100 Mb/s au lieu de 1 Gb/s ou plus : c'est presque toujours un câble abîmé ou de catégorie trop ancienne (Cat5), ou un port de box limité.",
        en: "The adapter negotiates 100 Mb/s instead of 1 Gb/s or more: almost always a damaged or too-old (Cat5) cable, or a limited router port.",
      },
      tweak: null,
    },
    {
      id: "gaming.background_recording",
      severity: "suggestion",
      title: { fr: "Enregistrement en arrière-plan actif", en: "Background recording is on" },
      detail: {
        fr: "La Game Bar enregistre en permanence pendant que tu joues, ce qui occupe l'encodeur vidéo.",
        en: "The Game Bar records constantly while you play, which keeps the video encoder busy.",
      },
      tweak: "gaming.game_dvr_off",
    },
  ],
};

function states(c: CatalogView): Record<string, TweakState> {
  return Object.fromEntries(
    c.tweaks.map((t) => [t.id, applied.has(t.id) ? { state: "applied" } : { state: "not_applied" }]),
  );
}

export async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
  const c = await getCatalog();
  switch (cmd) {
    case "get_catalog":
      return c as T;
    case "get_states":
      return { states: states(c), journaled: [...journaled], elevated: false } satisfies StatesView as T;
    case "analyze":
      await delay(600);
      return analysis as T;
    case "recommend": {
      const a = args.answers as Answers;
      const laptop = a.is_laptop ?? analysis.info.power.has_battery;
      const recs: Recommendation[] = c.tweaks
        .filter((t) => (a.include_expert || t.risk !== "expert") && !(laptop && t.tags.includes("desktop-only")))
        .filter((t) => !applied.has(t.id))
        .map((t) => ({ tweak_id: t.id, goals: a.goals.filter((g) => t.tags.includes(g)) }))
        .filter((r) => r.goals.length > 0);
      return recs as T;
    }
    case "run_operation": {
      await delay(500);
      const op = args.operation as Operation;
      const ids = op.op === "revert_all" ? [...journaled].reverse() : op.ids;
      for (const id of ids) {
        if (op.op === "apply") {
          applied.add(id);
          journaled.add(id);
        } else {
          applied.delete(id);
          journaled.delete(id);
        }
      }
      return { results: ids.map((tweak_id) => ({ tweak_id, error: null })), warnings: [] } satisfies RunResult as T;
    }
    case "restart_explorer":
    case "open_nvidia_page":
      return undefined as T;
    case "get_wallpaper":
      return { config: wallpaper, monitors, running: wallpaper.enabled } as T;
    case "set_wallpaper":
      await delay(300);
      wallpaper = args.config as WallpaperConfig;
      return {
        config: wallpaper,
        monitors,
        running: wallpaper.enabled && (wallpaper.layout === "span" ? !!wallpaper.span_video : wallpaper.screens.some((s) => s.video)),
      } as T;
    case "get_widgets":
      return { config: widgets, monitors, running: true } as T;
    case "set_widgets":
      await delay(200);
      widgets = args.config as WidgetsConfig;
      return { config: widgets, monitors, running: true } as T;
    case "get_theme":
      return { state: theme, accents } as T;
    case "set_theme":
      await delay(150);
      theme = args.state as ThemeState;
      return theme as T;
    case "list_windows_themes":
      await delay(300);
      return themes as T;
    case "apply_windows_theme": {
      await delay(400);
      const t = themes.find((x) => x.path === args.path)!;
      theme = { ...theme, accent: t.accent!, apps_light: t.apps_light!, system_light: t.system_light! };
      return theme as T;
    }
    case "pick_video":
      return "C:\\Users\\Moi\\Videos\\Wallpapers\\aurore-boreale-4k.mp4" as T;
    case "check_drivers":
      await delay(700);
      return [
        {
          gpu: "NVIDIA GeForce RTX 4070",
          installed: "560.94",
          latest: { version: "621.01", release_date: "2026-10-21", details_url: "https://www.nvidia.com/en-us/drivers/details/282412/" },
          update_available: true,
        },
      ] as T;
    default:
      throw new Error(`mock: unknown command ${cmd}`);
  }
}
