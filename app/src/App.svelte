<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "./lib/api";
  import { i18n, setLang, t, type Key } from "./lib/i18n.svelte";
  import { load, run, store } from "./lib/store.svelte";
  import Analysis from "./views/Analysis.svelte";
  import Soon from "./views/Soon.svelte";
  import Tweaks from "./views/Tweaks.svelte";
  import Wallpaper from "./views/Wallpaper.svelte";

  type View = "analysis" | "tweaks" | "themes" | "widgets";
  const nav: { id: View; label: Key; soon?: Key; icon: string }[] = [
    { id: "analysis", label: "nav.analysis", icon: "M4 19h16M7 16V9m5 7V5m5 11v-4" },
    { id: "tweaks", label: "nav.tweaks", icon: "M4 7h10m4 0h2M4 17h4m4 0h8M14 4v6m-6 4v6" },
    { id: "themes", label: "nav.themes", icon: "M12 3a9 9 0 1 0 0 18c1 0 1.5-.8 1.5-1.5 0-1.5-1.5-1.5-1.5-3S13 14 14.5 14H17a4 4 0 0 0 4-4c0-3.9-4-7-9-7Z" },
    { id: "widgets", label: "nav.widgets", soon: "soon.widgets", icon: "M4 4h7v7H4zm9 0h7v7h-7zM4 13h7v7H4zm9 0h7v7h-7z" },
  ];

  let view = $state<View>("analysis");
  let main: HTMLElement;
  let confirmUndo = $state(false);
  const current = $derived(nav.find((n) => n.id === view)!);

  onMount(load);

  async function undoAll() {
    confirmUndo = false;
    await run({ op: "revert_all" });
  }

  async function restartExplorer() {
    if (store.toast) store.toast.restart = "none";
    await api.restartExplorer().catch((e) => {
      store.toast = { kind: "error", message: `${t("error.generic")} ${e}`, failures: [], warnings: [], restart: "none" };
    });
  }
</script>

<div class="shell">
  <aside>
    <div class="brand">
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 3h8v8H3zm10 0h8v8h-8zM3 13h8v8H3z" /><circle cx="17" cy="17" r="4" /></svg>
      CustomWin
    </div>
    <nav>
      {#each nav as item (item.id)}
        <button class="nav" class:active={view === item.id} aria-current={view === item.id ? "page" : undefined} onclick={() => {
            view = item.id;
            main.scrollTop = 0;
          }}>
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d={item.icon} /></svg>
          <span>{t(item.label)}</span>
          {#if item.soon}<span class="soon">{t("nav.soon")}</span>{/if}
        </button>
      {/each}
    </nav>
    <div class="foot">
      <button class="undo" disabled={store.busy || store.journaled.length === 0} onclick={() => (confirmUndo = true)}>
        {t("undo.all")}
        {#if store.journaled.length}<span class="count">{store.journaled.length}</span>{/if}
      </button>
      <div class="lang" role="group" aria-label="Language">
        <button class="ghost" class:active={i18n.lang === "fr"} onclick={() => setLang("fr")}>FR</button>
        <button class="ghost" class:active={i18n.lang === "en"} onclick={() => setLang("en")}>EN</button>
      </div>
    </div>
  </aside>

  <main bind:this={main}>
    {#if store.loadError}
      <p class="error">{t("error.generic")} {store.loadError}</p>
    {:else if !store.catalog}
      <p class="muted">…</p>
    {:else if view === "analysis"}
      <Analysis />
    {:else if view === "tweaks"}
      <Tweaks />
    {:else if view === "themes"}
      <Wallpaper />
    {:else if current.soon}
      <Soon title={current.label} body={current.soon} />
    {/if}
  </main>
</div>

{#if store.busy}
  <div class="progress" aria-hidden="true"></div>
{/if}

{#if store.toast}
  {@const toast = store.toast}
  <div class="toast card" class:error={toast.kind === "error"} role="status">
    <div class="grow">
      <strong>{toast.message}</strong>
      {#if toast.failures.length}
        <p>{t("toast.failed", { n: toast.failures.length })}</p>
        <ul>
          {#each toast.failures as f (f.name)}<li><b>{f.name}</b> — {f.error}</li>{/each}
        </ul>
      {/if}
      {#each toast.warnings as w (w)}<p class="muted">{w}</p>{/each}
      {#if toast.restart === "reboot"}<p>{t("toast.reboot")}</p>{/if}
      {#if toast.restart === "sign_out"}<p>{t("toast.sign_out")}</p>{/if}
    </div>
    {#if toast.restart === "explorer"}
      <button class="primary" onclick={restartExplorer}>{t("toast.restart_explorer")}</button>
    {/if}
    <button class="ghost" aria-label={t("close")} onclick={() => (store.toast = null)}>✕</button>
  </div>
{/if}

{#if confirmUndo}
  <div class="backdrop" role="presentation" onclick={() => (confirmUndo = false)}>
    <div class="card dialog" role="dialog" aria-modal="true" aria-labelledby="undo-title" tabindex="-1" onclick={(e) => e.stopPropagation()} onkeydown={(e) => e.key === "Escape" && (confirmUndo = false)}>
      <h2 id="undo-title">{t("undo.confirm.title")}</h2>
      <p class="muted">{t("undo.confirm.body")}</p>
      <div class="row">
        <button onclick={() => (confirmUndo = false)}>{t("cancel")}</button>
        <button class="primary" onclick={undoAll}>{t("confirm")}</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .shell {
    display: grid;
    grid-template-columns: 232px 1fr;
    height: 100%;
  }
  aside {
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    padding: 18px 12px;
    gap: 18px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
    font-weight: 700;
    font-size: 16px;
    padding: 0 8px;
  }
  .brand svg {
    width: 22px;
    height: 22px;
    fill: var(--accent);
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
  }
  .nav {
    display: flex;
    align-items: center;
    gap: 12px;
    border: none;
    background: transparent;
    text-align: left;
    padding: 9px 10px;
    position: relative;
  }
  .nav:hover {
    background: var(--surface);
  }
  .nav.active {
    background: var(--surface-2);
    font-weight: 600;
  }
  .nav.active::before {
    content: "";
    position: absolute;
    left: 0;
    top: 10px;
    bottom: 10px;
    width: 3px;
    border-radius: 3px;
    background: var(--accent);
  }
  .nav svg {
    width: 18px;
    height: 18px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
    stroke-linecap: round;
    stroke-linejoin: round;
    flex: none;
  }
  .soon {
    margin-left: auto;
    font-size: 10.5px;
    color: var(--text-2);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .foot {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .undo {
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 8px;
  }
  .count {
    background: var(--accent);
    color: var(--accent-text);
    border-radius: 999px;
    font-size: 11px;
    padding: 0 7px;
    font-weight: 700;
  }
  .lang {
    display: flex;
    justify-content: center;
    gap: 4px;
  }
  .lang button {
    padding: 3px 10px;
    font-size: 12px;
    color: var(--text-2);
  }
  .lang button.active {
    color: var(--text);
    font-weight: 700;
  }
  main {
    overflow-y: auto;
    padding: 32px 40px 120px;
  }
  .error {
    color: var(--danger);
  }
  .progress {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    height: 3px;
    background: linear-gradient(90deg, transparent, var(--accent), transparent);
    background-size: 50% 100%;
    animation: slide 1s linear infinite;
  }
  @keyframes slide {
    from {
      background-position: -50% 0;
    }
    to {
      background-position: 150% 0;
    }
  }
  .toast {
    position: fixed;
    right: 24px;
    bottom: 24px;
    max-width: 520px;
    display: flex;
    gap: 12px;
    align-items: flex-start;
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.35);
    border-left: 4px solid var(--ok);
  }
  .toast.error {
    border-left-color: var(--danger);
  }
  .toast ul {
    margin: 6px 0 0;
    padding-left: 18px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.45);
    display: grid;
    place-items: center;
  }
  .dialog {
    width: min(420px, calc(100% - 32px));
    padding: 22px;
  }
  .row {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
    margin-top: 20px;
  }
</style>
