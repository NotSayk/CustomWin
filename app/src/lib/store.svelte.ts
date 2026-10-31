import { api } from "./api";
import { loc, t } from "./i18n.svelte";
import type { CatalogView, Operation, Restart, Tweak, TweakState } from "./types";

export interface Toast {
  kind: "ok" | "error";
  message: string;
  failures: { name: string; error: string }[];
  warnings: string[];
  restart: Restart;
}

export const store = $state({
  catalog: null as CatalogView | null,
  states: {} as Record<string, TweakState>,
  journaled: [] as string[],
  busy: false,
  toast: null as Toast | null,
  loadError: null as string | null,
});

export function tweakById(id: string): Tweak | undefined {
  return store.catalog?.tweaks.find((t) => t.id === id);
}

export function isApplied(id: string): boolean {
  return store.states[id]?.state === "applied";
}

export async function load() {
  try {
    store.catalog = await api.catalog();
    await refreshStates();
  } catch (e) {
    store.loadError = String(e);
  }
}

export async function refreshStates() {
  const s = await api.states();
  store.states = s.states;
  store.journaled = s.journaled;
}

const restartRank: Record<Restart, number> = { none: 0, explorer: 1, sign_out: 2, reboot: 3 };

/** Runs an operation and reports the outcome in a toast. */
export async function run(operation: Operation, restorePoint = false) {
  store.busy = true;
  try {
    const res = await api.run(operation, restorePoint);
    const ok = res.results.filter((r) => r.error === null);
    const failures = res.results
      .filter((r) => r.error !== null)
      .map((r) => {
        const tw = tweakById(r.tweak_id);
        return { name: tw ? loc(tw.name) : r.tweak_id, error: r.error ?? "" };
      });
    let restart: Restart = "none";
    for (const r of ok) {
      const need = tweakById(r.tweak_id)?.restart ?? "none";
      if (restartRank[need] > restartRank[restart]) restart = need;
    }
    store.toast = {
      kind: failures.length ? "error" : "ok",
      message: t("toast.done", { n: ok.length }),
      failures,
      warnings: res.warnings,
      restart,
    };
  } catch (e) {
    store.toast = { kind: "error", message: `${t("error.generic")} ${e}`, failures: [], warnings: [], restart: "none" };
  } finally {
    await refreshStates().catch(() => {});
    store.busy = false;
  }
}
