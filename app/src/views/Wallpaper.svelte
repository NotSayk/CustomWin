<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { t } from "../lib/i18n.svelte";
  import Toggle from "../lib/Toggle.svelte";
  import type { Fit, Monitor, WallpaperConfig, WallpaperView } from "../lib/types";

  let view = $state<WallpaperView | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      view = await api.getWallpaper();
    } catch (e) {
      error = String(e);
    }
  });

  /** Saves a modified copy of the configuration; the backend applies it right away. */
  async function save(change: (c: WallpaperConfig) => void) {
    if (!view) return;
    const next: WallpaperConfig = JSON.parse(JSON.stringify(view.config));
    change(next);
    busy = true;
    error = null;
    try {
      view = await api.setWallpaper(next);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function screenOf(c: WallpaperConfig, device: string) {
    let s = c.screens.find((x) => x.device.toLowerCase() === device.toLowerCase());
    if (!s) {
      s = { device, video: null, fit: "cover" };
      c.screens.push(s);
    }
    return s;
  }

  async function choose(device: string | null) {
    const path = await api.pickVideo().catch((e) => {
      error = String(e);
      return null;
    });
    if (!path) return;
    await save((c) => {
      if (device) screenOf(c, device).video = path;
      else c.span_video = path;
      c.enabled = true;
    });
  }

  const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;
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

  function hasVideo(m: Monitor) {
    if (!view) return false;
    if (view.config.layout === "span") return !!view.config.span_video;
    return !!view.config.screens.find((s) => s.device.toLowerCase() === m.device.toLowerCase())?.video;
  }
</script>

<header>
  <div>
    <h1>{t("wp.title")}</h1>
    <p class="muted">{t("wp.subtitle")}</p>
  </div>
  {#if view}
    <div class="switch">
      <span class="pill" class:on={view.running}>{view.running ? t("wp.running") : t("wp.stopped")}</span>
      <Toggle
        checked={view.config.enabled}
        label={t("wp.enable")}
        disabled={busy}
        onchange={(on) => save((c) => (c.enabled = on))}
      />
    </div>
  {/if}
</header>

{#if error}<p class="error">{t("error.generic")} {error}</p>{/if}

{#if view}
  {@const c = view.config}
  {#if bounds}
    <svg class="map" viewBox="{bounds.left} {bounds.top} {bounds.width} {bounds.height}" role="img" aria-label="Screens">
      {#each view.monitors as m, i (m.device)}
        {@const w = m.rect.right - m.rect.left}
        {@const h = m.rect.bottom - m.rect.top}
        <rect
          x={m.rect.left + w * 0.01}
          y={m.rect.top + h * 0.02}
          width={w * 0.98}
          height={h * 0.96}
          rx={h * 0.04}
          class:video={hasVideo(m)}
        />
        <text x={m.rect.left + w / 2} y={m.rect.top + h / 2} font-size={Math.min(w, h) * 0.22}>{i + 1}</text>
      {/each}
    </svg>
  {/if}

  <div class="segmented" role="group">
    {#each ["per_screen", "span"] as const as layout (layout)}
      <button
        class:active={c.layout === layout}
        aria-pressed={c.layout === layout}
        disabled={busy}
        onclick={() => save((x) => (x.layout = layout))}>{t(`wp.layout.${layout}`)}</button
      >
    {/each}
  </div>

  {#snippet slot(label: string, detail: string, video: string | null, fit: Fit, device: string | null)}
    <div class="card slot">
      <div class="grow">
        <strong>{label}</strong>
        <span class="muted small">{detail}</span>
        <p class="file" class:muted={!video} title={video ?? ""}>{video ? fileName(video) : t("wp.no_video")}</p>
      </div>
      <div class="controls">
        <select
          value={fit}
          disabled={busy || !video}
          onchange={(e) => {
            const v = e.currentTarget.value as Fit;
            save((x) => (device ? (screenOf(x, device).fit = v) : (x.span_fit = v)));
          }}
        >
          <option value="cover">{t("wp.fit.cover")}</option>
          <option value="contain">{t("wp.fit.contain")}</option>
        </select>
        <button disabled={busy} onclick={() => choose(device)}>{video ? t("wp.change") : t("wp.choose")}</button>
        {#if video}
          <button
            class="ghost"
            disabled={busy}
            onclick={() => save((x) => (device ? (screenOf(x, device).video = null) : (x.span_video = null)))}
            >{t("wp.remove")}</button
          >
        {/if}
      </div>
    </div>
  {/snippet}

  <section>
    {#if c.layout === "per_screen"}
      {#each view.monitors as m, i (m.device)}
        {@const s = c.screens.find((x) => x.device.toLowerCase() === m.device.toLowerCase())}
        {@render slot(
          t("wp.screen", { n: i + 1 }),
          `${size(m)}${m.primary ? ` · ${t("wp.primary")}` : ""}`,
          s?.video ?? null,
          s?.fit ?? "cover",
          m.device,
        )}
      {/each}
    {:else if bounds}
      {@render slot(t("wp.all_screens"), `${bounds.width}×${bounds.height}`, c.span_video, c.span_fit, null)}
    {/if}
  </section>

  {@const playable = c.layout === "span" ? !!c.span_video : c.screens.some((s) => s.video)}
  {#if c.enabled && !playable}
    <p class="hint">{t("wp.need_video")}</p>
  {:else if c.enabled && !view.running && !busy}
    <p class="hint">{t("wp.not_running")}</p>
  {/if}

  <section>
    <h2>{t("wp.options")}</h2>
    <div class="card options">
      <label
        ><input
          type="checkbox"
          checked={c.pause_when_maximized}
          disabled={busy}
          onchange={(e) => {
            const v = e.currentTarget.checked;
            save((x) => (x.pause_when_maximized = v));
          }}
        />
        {t("wp.pause_maximized")}</label
      >
      <label
        ><input
          type="checkbox"
          checked={c.pause_on_battery}
          disabled={busy}
          onchange={(e) => {
            const v = e.currentTarget.checked;
            save((x) => (x.pause_on_battery = v));
          }}
        />
        {t("wp.pause_battery")}</label
      >
      <label
        ><input
          type="checkbox"
          checked={c.start_with_windows}
          disabled={busy}
          onchange={(e) => {
            const v = e.currentTarget.checked;
            save((x) => (x.start_with_windows = v));
          }}
        />
        {t("wp.autostart")}</label
      >
      <p class="muted small">{t("wp.always")}</p>
    </div>
    <p class="muted small formats">{t("wp.formats")}</p>
  </section>

  <div class="card soon">
    <span class="badge accent">{t("soon.label")}</span>
    <p class="muted">{t("soon.themes.more")}</p>
  </div>
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
  .switch {
    display: flex;
    align-items: center;
    gap: 12px;
    flex: none;
  }
  .pill {
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
  .map rect {
    fill: var(--surface-2);
    stroke: var(--border);
    stroke-width: 0.4%;
  }
  .map rect.video {
    fill: color-mix(in srgb, var(--accent) 25%, var(--surface));
    stroke: var(--accent);
  }
  .map text {
    fill: var(--text);
    text-anchor: middle;
    dominant-baseline: central;
    font-weight: 700;
  }
  .segmented {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 9px;
    padding: 3px;
    gap: 3px;
    margin-bottom: 16px;
    background: var(--surface);
  }
  .segmented button {
    border: none;
    background: transparent;
    padding: 6px 14px;
  }
  .segmented button.active {
    background: var(--surface-2);
    font-weight: 600;
    color: var(--accent);
  }
  section {
    margin-bottom: 24px;
  }
  .slot {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-bottom: 10px;
    flex-wrap: wrap;
  }
  .slot strong {
    margin-right: 8px;
  }
  .grow {
    flex: 1;
    min-width: 200px;
  }
  .file {
    margin-top: 4px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .controls {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }
  select {
    font: inherit;
    color: inherit;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 6px 10px;
  }
  .small {
    font-size: 12.5px;
  }
  .options {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  label {
    display: inline-flex;
    gap: 10px;
    align-items: center;
    cursor: pointer;
  }
  .formats {
    margin-top: 10px;
  }
  .hint {
    color: var(--warn);
    margin: -8px 0 20px;
  }
  .error {
    color: var(--danger);
    margin-bottom: 16px;
  }
  .soon {
    display: flex;
    gap: 12px;
    align-items: center;
    flex-wrap: wrap;
  }
</style>
