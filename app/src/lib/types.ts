// Mirrors the serde output of customwin-core. Keep in sync with crates/core/src/*.rs.

export type Lang = "fr" | "en";
export type Localized = { fr: string; en: string };

export type Category = "privacy" | "ads" | "interface" | "explorer" | "gaming" | "power" | "updates" | "expert";
export type Risk = "safe" | "low" | "medium" | "expert";
export type Evidence = "functional" | "measured" | "situational";
export type Restart = "none" | "explorer" | "sign_out" | "reboot";

export interface Tweak {
  id: string;
  category: Category;
  name: Localized;
  description: Localized;
  risk: Risk;
  evidence: Evidence;
  restart: Restart;
  tags: string[];
  note: Localized | null;
  requires_admin: boolean;
}

export interface Profile {
  id: string;
  name: Localized;
  description: Localized;
  tweaks: string[];
}

export interface CatalogView {
  tweaks: Tweak[];
  profiles: Profile[];
}

export type TweakState =
  | { state: "applied" }
  | { state: "not_applied" }
  | { state: "partial" }
  | { state: "unavailable"; reason: string };

export interface StatesView {
  states: Record<string, TweakState>;
  /** Tweaks CustomWin applied and can undo. */
  journaled: string[];
  elevated: boolean;
}

export type Operation = { op: "apply"; ids: string[] } | { op: "revert"; ids: string[] } | { op: "revert_all" };

export interface OpResult {
  tweak_id: string;
  error: string | null;
}

export interface RunResult {
  results: OpResult[];
  warnings: string[];
}

export type GpuVendor = "nvidia" | "amd" | "intel" | "other";
export type NetKind = "ethernet" | "wifi" | "other";

export interface SystemInfo {
  os: { name: string; display_version: string; build: number; ubr: number; edition: string };
  cpu: { name: string; logical_cores: number };
  memory_bytes: number;
  gpus: {
    name: string;
    vendor: GpuVendor;
    vram_bytes: number;
    driver_version: string | null;
    driver_display_version: string | null;
    driver_date: string | null;
  }[];
  power: { active_scheme: string; active_scheme_name: string; has_battery: boolean; on_battery: boolean };
  features: {
    game_mode: boolean | null;
    hags: boolean | null;
    memory_integrity: boolean | null;
    background_recording: boolean | null;
  };
  network: { name: string; description: string; kind: NetKind; connected: boolean; link_speed_bps: number }[];
}

export type Severity = "info" | "suggestion" | "warning";

export interface Insight {
  id: string;
  severity: Severity;
  title: Localized;
  detail: Localized;
  tweak: string | null;
}

export interface Analysis {
  info: SystemInfo;
  insights: Insight[];
}

export type Goal = "gaming" | "performance" | "privacy" | "clean";

export interface Answers {
  goals: Goal[];
  include_expert: boolean;
  is_laptop: boolean | null;
}

export interface Recommendation {
  tweak_id: string;
  goals: Goal[];
}
