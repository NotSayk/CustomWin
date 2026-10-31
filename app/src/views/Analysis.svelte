<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../lib/api";
  import { i18n, loc, t } from "../lib/i18n.svelte";
  import { run, store, tweakById } from "../lib/store.svelte";
  import TweakBadges from "../lib/TweakBadges.svelte";
  import type { Analysis, DriverCheck, Goal, Recommendation } from "../lib/types";

  let analysis = $state<Analysis | null>(null);
  let scanning = $state(false);
  let error = $state<string | null>(null);

  let goals = $state<Goal[]>(["gaming", "clean"]);
  let laptop = $state<boolean | null>(null);
  let includeExpert = $state(false);
  let restorePoint = $state(true);

  let recs = $state<Recommendation[]>([]);

  // Online driver check: only runs when the user clicks.
  let drivers = $state<DriverCheck[] | null>(null);
  let checkingDrivers = $state(false);
  let driverError = $state<string | null>(null);

  async function checkDrivers() {
    checkingDrivers = true;
    driverError = null;
    try {
      drivers = await api.checkDrivers();
    } catch (e) {
      driverError = String(e);
    } finally {
      checkingDrivers = false;
    }
  }
  let unchecked = $state<Set<string>>(new Set());

  const allGoals: Goal[] = ["gaming", "performance", "privacy", "clean"];

  async function scan() {
    scanning = true;
    error = null;
    try {
      analysis = await api.analyze();
      laptop ??= analysis.info.power.has_battery;
    } catch (e) {
      error = String(e);
    } finally {
      scanning = false;
    }
  }

  onMount(scan);

  // Recompute recommendations when answers or the system state change.
  $effect(() => {
    const answers = { goals: [...goals], include_expert: includeExpert, is_laptop: laptop };
    void store.states;
    if (!store.catalog) return;
    api.recommend(answers).then((r) => (recs = r)).catch((e) => (error = String(e)));
  });

  function toggleGoal(g: Goal) {
    goals = goals.includes(g) ? goals.filter((x) => x !== g) : [...goals, g];
  }

  function toggleRec(id: string) {
    const next = new Set(unchecked);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    unchecked = next;
  }

  const selected = $derived(recs.filter((r) => !unchecked.has(r.tweak_id)).map((r) => r.tweak_id));

  const gib = (b: number) => `${Math.round(b / 2 ** 30)} ${i18n.lang === "fr" ? "Go" : "GB"}`;
  const speed = (bps: number) => (bps >= 1e9 ? `${bps / 1e9} Gb/s` : `${Math.round(bps / 1e6)} Mb/s`);
  const sevClass = { warning: "warn", suggestion: "accent", info: "" } as const;
</script>

<header>
  <div>
    <h1>{t("analysis.title")}</h1>
    <p class="muted">{t("analysis.subtitle")}</p>
  </div>
  <button onclick={scan} disabled={scanning}>{scanning ? t("analysis.scanning") : t("analysis.rescan")}</button>
</header>

{#if error}
  <p class="error">{t("error.generic")} {error}</p>
{/if}

{#if analysis}
  {@const info = analysis.info}
  <section>
    <h2>{t("analysis.machine")}</h2>
    <div class="grid">
      <div class="card spec">
        <span class="label">{t("hw.os")}</span>
        <strong>{info.os.name}</strong>
        <span class="muted">{info.os.display_version} · build {info.os.build}.{info.os.ubr}</span>
      </div>
      <div class="card spec">
        <span class="label">{t("hw.cpu")}</span>
        <strong>{info.cpu.name}</strong>
        <span class="muted">{info.cpu.logical_cores} threads · {gib(info.memory_bytes)} RAM</span>
      </div>
      {#each info.gpus as gpu (gpu.name)}
        <div class="card spec">
          <span class="label">{t("hw.gpu")}</span>
          <strong>{gpu.name}</strong>
          <span class="muted">
            {gib(gpu.vram_bytes)}
            {#if gpu.driver_display_version}· {t("hw.driver")} {gpu.driver_display_version}{/if}
            {#if gpu.driver_date}({gpu.driver_date}){/if}
          </span>
          {#if gpu.vendor === "nvidia"}
            {@const check = drivers?.find((d) => d.gpu === gpu.name)}
            <div class="driver">
              {#if check}
                {#if check.update_available}
                  <span class="warn">{t("drivers.update", { v: check.latest.version })}</span>
                  {#if check.latest.details_url}
                    <button class="link" onclick={() => api.openNvidiaPage(check.latest.details_url)}>{t("drivers.open")}</button>
                  {/if}
                {:else}
                  <span class="ok">{t("drivers.uptodate", { v: check.latest.version })}</span>
                {/if}
              {:else if drivers}
                <span class="muted">{t("drivers.not_found")}</span>
              {:else}
                <button class="small-btn" title={t("drivers.privacy")} disabled={checkingDrivers} onclick={checkDrivers}>
                  {checkingDrivers ? t("drivers.checking") : t("drivers.check")}
                </button>
              {/if}
              {#if driverError}<span class="warn">{driverError}</span>{/if}
            </div>
          {/if}
        </div>
      {/each}
      <div class="card spec">
        <span class="label">{t("hw.power")}</span>
        <strong>{info.power.active_scheme_name || info.power.active_scheme}</strong>
        <span class="muted">
          {info.power.has_battery ? t("hw.laptop") : t("hw.desktop")}
          {#if info.power.on_battery}· {t("hw.on_battery")}{/if}
        </span>
      </div>
      <div class="card spec">
        <span class="label">{t("hw.network")}</span>
        {#each info.network.filter((n) => n.connected) as n (n.name)}
          <strong>{n.name}</strong>
          <span class="muted">{n.description} · {speed(n.link_speed_bps)}</span>
        {:else}
          <strong class="muted">{t("hw.none")}</strong>
        {/each}
      </div>
    </div>
  </section>

  <section>
    <h2>{t("analysis.findings")}</h2>
    {#each analysis.insights.filter((i) => !i.tweak || store.states[i.tweak]?.state !== "applied") as insight (insight.id)}
      <div class="card insight">
        <span class="dot {sevClass[insight.severity]}"></span>
        <div class="grow">
          <strong>{loc(insight.title)}</strong>
          <p class="muted">{loc(insight.detail)}</p>
        </div>
        {#if insight.tweak}
          <button disabled={store.busy} onclick={() => run({ op: "apply", ids: [insight.tweak!] })}>{t("analysis.fix")}</button>
        {/if}
      </div>
    {:else}
      <p class="muted">{t("analysis.no_findings")}</p>
    {/each}
  </section>
{:else if scanning}
  <p class="muted">{t("analysis.scanning")}</p>
{/if}

<section>
  <h2>{t("quiz.title")}</h2>
  <p class="muted">{t("quiz.subtitle")}</p>
  <div class="chips">
    {#each allGoals as g (g)}
      <button class="chip" class:active={goals.includes(g)} aria-pressed={goals.includes(g)} onclick={() => toggleGoal(g)}>
        {t(`goal.${g}`)}
      </button>
    {/each}
  </div>
  <div class="options">
    <label><input type="checkbox" checked={laptop ?? false} onchange={(e) => (laptop = e.currentTarget.checked)} /> {t("quiz.laptop")}</label>
    <label><input type="checkbox" bind:checked={includeExpert} /> {t("quiz.expert")}</label>
  </div>
</section>

<section>
  <h2>{t("recs.title")}</h2>
  {#if goals.length === 0}
    <p class="muted">{t("recs.pick_goal")}</p>
  {:else if recs.length === 0}
    <p class="muted">{t("recs.empty")}</p>
  {:else}
    <div class="card list">
      {#each recs as r (r.tweak_id)}
        {@const tw = tweakById(r.tweak_id)}
        {#if tw}
          <label class="rec">
            <input type="checkbox" checked={!unchecked.has(r.tweak_id)} onchange={() => toggleRec(r.tweak_id)} />
            <div class="grow">
              <strong>{loc(tw.name)}</strong>
              <p class="muted">{loc(tw.description)}</p>
              {#if tw.note}<p class="note">{loc(tw.note)}</p>{/if}
            </div>
            <TweakBadges tweak={tw} />
          </label>
        {/if}
      {/each}
    </div>
    <div class="actions">
      <label><input type="checkbox" bind:checked={restorePoint} /> {t("restore_point")}</label>
      <button
        class="primary"
        disabled={store.busy || selected.length === 0}
        onclick={() => run({ op: "apply", ids: selected }, restorePoint)}
      >
        {t("recs.apply", { n: selected.length })}
      </button>
    </div>
  {/if}
</section>

<style>
  header {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 16px;
    margin-bottom: 28px;
  }
  section {
    margin-bottom: 32px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(230px, 1fr));
    gap: 12px;
  }
  .spec {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .spec strong {
    overflow-wrap: anywhere;
  }
  .driver {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
    margin-top: 10px;
    font-size: 13px;
  }
  .driver .warn {
    color: var(--warn);
  }
  .driver .ok {
    color: var(--ok);
  }
  .small-btn {
    padding: 4px 10px;
    font-size: 12.5px;
  }
  .link {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent);
    text-decoration: underline;
    font-size: 13px;
  }
  .label {
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-2);
    margin-bottom: 4px;
  }
  .insight {
    display: flex;
    align-items: center;
    gap: 14px;
    margin-bottom: 8px;
  }
  .dot {
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--info);
  }
  .dot.warn {
    background: var(--warn);
  }
  .dot.accent {
    background: var(--accent);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin: 14px 0;
  }
  .chip {
    border-radius: 999px;
    padding: 6px 16px;
  }
  .chip.active {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }
  .options,
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 20px;
    align-items: center;
  }
  label {
    display: inline-flex;
    gap: 8px;
    align-items: center;
    cursor: pointer;
  }
  .list {
    padding: 0;
  }
  .rec {
    display: flex;
    align-items: flex-start;
    gap: 14px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
  }
  .rec:last-child {
    border-bottom: none;
  }
  .rec input {
    margin-top: 3px;
  }
  .note {
    color: var(--warn);
    font-size: 13px;
    margin-top: 4px;
  }
  .actions {
    justify-content: flex-end;
    margin-top: 14px;
  }
  .error {
    color: var(--danger);
    margin-bottom: 16px;
  }
</style>
