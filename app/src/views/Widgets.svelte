<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { t, type Key } from "../lib/i18n.svelte";
  import Toggle from "../lib/Toggle.svelte";
  import type { Corner, Monitor, Widget, WidgetKind, WidgetsConfig, WidgetsView } from "../lib/types";

  let view = $state<WidgetsView | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      view = await api.getWidgets();
    } catch (e) {
      error = String(e);
    }
  });

  async function save(change: (c: WidgetsConfig) => void) {
    if (!view) return;
    const next: WidgetsConfig = JSON.parse(JSON.stringify(view.config));
    change(next);
    busy = true;
    error = null;
    try {
      view = await api.setWidgets(next);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function edit(kind: WidgetKind, change: (w: Widget) => void) {
    return save((c) => {
      const w = c.widgets.find((x) => x.kind === kind);
      if (w) change(w);
    });
  }

  const KINDS: { kind: WidgetKind; name: Key; desc: Key; icon: string }[] = [
    { kind: "clock", name: "wg.clock", desc: "wg.clock.desc", icon: "M12 7v5l3 2M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Z" },
    { kind: "system", name: "wg.system", desc: "wg.system.desc", icon: "M4 19h16M7 16V9m5 7V5m5 11v-4" },
    { kind: "media", name: "wg.media", desc: "wg.media.desc", icon: "M9 18V5l11-2v13M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0Zm11-2a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z" },
  ];
  const CORNERS: Corner[] = ["top_left", "top_right", "bottom_left", "bottom_right"];
  /** Same sizes as `WidgetKind::size` on the Rust side (96 DPI). */
  const SIZE: Record<WidgetKind, [number, number]> = { clock: [280, 118], system: [280, 104], media: [320, 76] };

  const size = (m: Monitor) => `${m.rect.right - m.rect.left}×${m.rect.bottom - m.rect.top}`;

  const bounds = $derived.by(() => {
    const ms = view?.monitors ?? [];
    if (!ms.length) return null;
    const left = Math.min(...ms.map((m) => m.rect.left));
    const top = Math.min(...ms.map((m) => m.rect.top));
    const right = Math.max(...ms.map((m) => m.rect.right));
    const bottom = Math.max(...ms.map((m) => m.rect.bottom));
    return { left, top, width: right - left, height: bottom - top };
  });

  /** Where each enabled widget lands, as `customwin-desktop` stacks them. */
  const preview = $derived.by(() => {
    if (!view) return [];
    const ms = view.monitors;
    const primary = ms.find((m) => m.primary) ?? ms[0];
    const screenOf = (w: Widget) => ms.find((m) => m.device.toLowerCase() === w.device?.toLowerCase()) ?? primary;
    const out: { kind: WidgetKind; x: number; y: number; w: number; h: number }[] = [];
    for (const m of ms) {
      for (const corner of CORNERS) {
        const top = corner.startsWith("top");
        const left = corner.endsWith("left");
        let y = top ? m.work.top + 24 : m.work.bottom - 24;
        for (const w of view.config.widgets.filter((x) => x.enabled && x.corner === corner && screenOf(x) === m)) {
          const [ww, hh] = SIZE[w.kind];
          const x = left ? m.work.left + 24 : m.work.right - 24 - ww;
          out.push({ kind: w.kind, x, y: top ? y : y - hh, w: ww, h: hh });
          y += top ? hh + 12 : -(hh + 12);
        }
      }
    }
    return out;
  });

  const anyEnabled = $derived(!!view && (view.config.transparent_taskbar || view.config.widgets.some((w) => w.enabled)));
</script>

<header>
  <div>
    <h1>{t("wg.title")}</h1>
    <p class="muted">{t("wg.subtitle")}</p>
  </div>
  {#if view && anyEnabled}
    <span class="pill" class:on={view.running}>{view.running ? t("wp.running") : t("wp.stopped")}</span>
  {/if}
</header>

{#if error}<p class="error">{t("error.generic")} {error}</p>{/if}

{#if view}
  {#if bounds}
    <svg class="map" viewBox="{bounds.left} {bounds.top} {bounds.width} {bounds.height}" role="img" aria-label="Screens">
      {#each view.monitors as m (m.device)}
        {@const w = m.rect.right - m.rect.left}
        {@const h = m.rect.bottom - m.rect.top}
        <rect class="screen" x={m.rect.left + w * 0.01} y={m.rect.top + h * 0.02} width={w * 0.98} height={h * 0.96} rx={h * 0.04} />
      {/each}
      {#each preview as p, i (i)}
        <rect class="widget" x={p.x} y={p.y} width={p.w} height={p.h} rx="14" />
      {/each}
    </svg>
  {/if}

  <section>
    {#each KINDS as k (k.kind)}
      {@const w = view.config.widgets.find((x) => x.kind === k.kind)}
      {#if w}
        <div class="card widget-card">
          <svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d={k.icon} /></svg>
          <div class="grow">
            <strong>{t(k.name)}</strong>
            <p class="muted small">{t(k.desc)}</p>
            {#if w.enabled}
              <div class="controls">
                <select
                  aria-label={t("wg.screen")}
                  value={w.device ?? ""}
                  disabled={busy}
                  onchange={(e) => {
                    const v = e.currentTarget.value;
                    edit(k.kind, (x) => (x.device = v || null));
                  }}
                >
                  <option value="">{t("wg.main_screen")}</option>
                  {#each view.monitors as m, i (m.device)}
                    <option value={m.device}>{t("wp.screen", { n: i + 1 })} · {size(m)}</option>
                  {/each}
                </select>
                <select
                  aria-label="Position"
                  value={w.corner}
                  disabled={busy}
                  onchange={(e) => {
                    const v = e.currentTarget.value as Corner;
                    edit(k.kind, (x) => (x.corner = v));
                  }}
                >
                  {#each CORNERS as corner (corner)}
                    <option value={corner}>{t(`wg.corner.${corner}`)}</option>
                  {/each}
                </select>
              </div>
            {/if}
          </div>
          <Toggle checked={w.enabled} label={t(k.name)} disabled={busy} onchange={(on) => edit(k.kind, (x) => (x.enabled = on))} />
        </div>
      {/if}
    {/each}
  </section>

  <section>
    <h2>{t("wg.taskbar")}</h2>
    <div class="card widget-card">
      <svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M3 17h18v4H3zM7 19h1m3 0h2" /></svg>
      <div class="grow">
        <strong>{t("wg.taskbar.transparent")}</strong>
        <p class="muted small">{t("wg.taskbar.desc")}</p>
        <p class="muted small how">{t("wg.taskbar.how")}</p>
      </div>
      <Toggle
        checked={view.config.transparent_taskbar}
        label={t("wg.taskbar.transparent")}
        disabled={busy}
        onchange={(on) => save((c) => (c.transparent_taskbar = on))}
      />
    </div>
  </section>

  {#if anyEnabled && !view.running && !busy}
    <p class="hint">{t("wg.not_running")}</p>
  {/if}
  <p class="muted small">{t("wg.autostart")}</p>
{/if}

<style>
  header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 20px;
    margin-bottom: 24px;
  }
  header p {
    max-width: 620px;
  }
  .pill {
    flex: none;
    font-size: 12px;
    padding: 2px 10px;
    border-radius: 999px;
    border: 1px solid var(--border);
    color: var(--text-2);
  }
  .pill.on {
    color: var(--ok);
    border-color: color-mix(in srgb, var(--ok) 45%, transparent);
  }
  .map {
    display: block;
    width: 100%;
    max-width: 520px;
    max-height: 180px;
    margin: 0 auto 20px;
  }
  .map .screen {
    fill: var(--surface-2);
    stroke: var(--border);
    stroke-width: 0.4%;
  }
  .map .widget {
    fill: color-mix(in srgb, var(--accent) 35%, var(--surface));
    stroke: var(--accent);
    stroke-width: 0.3%;
  }
  section {
    margin-bottom: 24px;
  }
  .widget-card {
    display: flex;
    align-items: flex-start;
    gap: 16px;
    margin-bottom: 10px;
  }
  .icon {
    flex: none;
    width: 22px;
    height: 22px;
    margin-top: 2px;
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.7;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .small {
    font-size: 12.5px;
    margin-top: 2px;
  }
  .how {
    margin-top: 8px;
  }
  .controls {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 12px;
  }
  select {
    font: inherit;
    color: inherit;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 6px 10px;
  }
  .hint {
    color: var(--warn);
    margin-bottom: 12px;
  }
  .error {
    color: var(--danger);
    margin-bottom: 16px;
  }
</style>
