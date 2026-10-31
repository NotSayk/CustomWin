<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { t } from "../lib/i18n.svelte";
  import Toggle from "../lib/Toggle.svelte";
  import type { ThemeFile, ThemeState } from "../lib/types";

  let current = $state<ThemeState | null>(null);
  let accents = $state<number[]>([]);
  let themes = $state<ThemeFile[] | null>(null);
  let applied = $state<string | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);

  onMount(async () => {
    try {
      const v = await api.getTheme();
      current = v.state;
      accents = v.accents;
    } catch (e) {
      error = String(e);
    }
    // Previews take a moment: the rest of the page does not wait for them.
    api
      .listWindowsThemes()
      .then((list) => (themes = list))
      .catch((e) => {
        themes = [];
        error = String(e);
      });
  });

  async function run(task: () => Promise<ThemeState>) {
    busy = true;
    error = null;
    try {
      current = await task();
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }

  function save(change: (s: ThemeState) => void) {
    if (!current) return;
    const next = { ...current };
    change(next);
    return run(() => api.setTheme(next));
  }

  function apply(theme: ThemeFile) {
    applied = theme.path;
    return run(() => api.applyWindowsTheme(theme.path));
  }

  const hex = (c: number) => `#${c.toString(16).padStart(6, "0")}`;
</script>

<header>
  <h1>{t("theme.title")}</h1>
  <p class="muted">{t("theme.subtitle")}</p>
</header>

{#if error}<p class="error">{t("error.generic")} {error}</p>{/if}

<section>
  <h2>{t("theme.presets")}</h2>
  {#if themes === null}
    <p class="muted">{t("theme.loading")}</p>
  {:else}
    <div class="themes">
      {#each themes as theme (theme.path)}
        <button class="theme" class:active={applied === theme.path} disabled={busy} onclick={() => apply(theme)}>
          {#if theme.thumbnail}
            <img src={theme.thumbnail} alt="" />
          {:else}
            <span class="img"></span>
          {/if}
          <span class="name">
            {#if theme.accent !== null}<span class="dot" style:background={hex(theme.accent)}></span>{/if}
            {theme.name}
          </span>
        </button>
      {/each}
    </div>
    <p class="muted small">{t("theme.presets.hint")}</p>
  {/if}
</section>

{#if current}
  {@const c = current}
  <section>
    <h2>{t("theme.mode")}</h2>
    <div class="card rows">
      {#each [["apps_light", "theme.apps"], ["system_light", "theme.system"]] as const as [key, label] (key)}
        <div class="row">
          <span>{t(label)}</span>
          <div class="segmented" role="group" aria-label={t(label)}>
            {#each [true, false] as light (light)}
              <button
                class:active={c[key] === light}
                aria-pressed={c[key] === light}
                disabled={busy}
                onclick={() => save((s) => (s[key] = light))}>{light ? t("theme.light") : t("theme.dark")}</button
              >
            {/each}
          </div>
        </div>
      {/each}
    </div>
  </section>

  <section>
    <h2>{t("theme.accent")}</h2>
    <div class="card">
      <div class="row">
        <span>{t("theme.accent.auto")}</span>
        <Toggle
          checked={c.auto_accent}
          label={t("theme.accent.auto")}
          disabled={busy}
          onchange={(on) => save((s) => (s.auto_accent = on))}
        />
      </div>
      <div class="swatches" class:off={c.auto_accent}>
        {#each accents as accent (accent)}
          <button
            class="swatch"
            class:active={!c.auto_accent && c.accent === accent}
            style:background={hex(accent)}
            aria-label={hex(accent)}
            aria-pressed={!c.auto_accent && c.accent === accent}
            disabled={busy || c.auto_accent}
            onclick={() => save((s) => (s.accent = accent))}
          ></button>
        {/each}
      </div>
    </div>
  </section>

  <section>
    <h2>{t("theme.effects")}</h2>
    <div class="card rows">
      <div class="row">
        <span>{t("theme.transparency")}</span>
        <Toggle
          checked={c.transparency}
          label={t("theme.transparency")}
          disabled={busy}
          onchange={(on) => save((s) => (s.transparency = on))}
        />
      </div>
      <div class="row">
        <span>
          {t("theme.accent_taskbar")}
          {#if c.system_light}<span class="muted small block">{t("theme.accent_taskbar.hint")}</span>{/if}
        </span>
        <Toggle
          checked={c.accent_on_taskbar}
          label={t("theme.accent_taskbar")}
          disabled={busy}
          onchange={(on) => save((s) => (s.accent_on_taskbar = on))}
        />
      </div>
      <div class="row">
        <span>{t("theme.accent_titles")}</span>
        <Toggle
          checked={c.accent_on_title_bars}
          label={t("theme.accent_titles")}
          disabled={busy}
          onchange={(on) => save((s) => (s.accent_on_title_bars = on))}
        />
      </div>
    </div>
  </section>
{/if}

<hr />

<style>
  header {
    margin-bottom: 24px;
  }
  header p {
    max-width: 620px;
  }
  section {
    margin-bottom: 24px;
  }
  .themes {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(168px, 1fr));
    gap: 12px;
    margin-bottom: 10px;
  }
  .theme {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 6px 6px 10px;
    text-align: left;
    background: var(--surface);
    border: 2px solid transparent;
  }
  .theme:hover:not(:disabled) {
    border-color: var(--border);
  }
  .theme.active {
    border-color: var(--accent);
  }
  .theme img,
  .theme .img {
    display: block;
    width: 100%;
    aspect-ratio: 16 / 9;
    object-fit: cover;
    border-radius: 6px;
    background: var(--surface-2);
  }
  .name {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 4px;
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dot {
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .segmented {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: 9px;
    padding: 3px;
    gap: 3px;
    background: var(--surface-2);
  }
  .segmented button {
    border: none;
    background: transparent;
    padding: 5px 14px;
  }
  .segmented button.active {
    background: var(--surface);
    font-weight: 600;
    color: var(--accent);
  }
  .swatches {
    display: grid;
    grid-template-columns: repeat(auto-fill, 34px);
    gap: 8px;
    margin-top: 16px;
  }
  .swatches.off {
    opacity: 0.4;
  }
  .swatch {
    width: 34px;
    height: 34px;
    padding: 0;
    border-radius: 8px;
    border: 2px solid transparent;
  }
  .swatch.active {
    border-color: var(--text);
    box-shadow: inset 0 0 0 2px var(--bg);
  }
  .small {
    font-size: 12.5px;
  }
  .block {
    display: block;
    margin-top: 2px;
  }
  .error {
    color: var(--danger);
    margin-bottom: 16px;
  }
  hr {
    border: none;
    border-top: 1px solid var(--border);
    margin: 8px 0 32px;
  }
</style>
