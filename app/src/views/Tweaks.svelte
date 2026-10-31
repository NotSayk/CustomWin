<script lang="ts">
  import { loc, t } from "../lib/i18n.svelte";
  import { isApplied, run, store } from "../lib/store.svelte";
  import Toggle from "../lib/Toggle.svelte";
  import TweakBadges from "../lib/TweakBadges.svelte";
  import type { Category, Profile, Tweak } from "../lib/types";

  let query = $state("");
  let showExpert = $state(false);

  const order: Category[] = [
    "privacy",
    "ads",
    "interface",
    "explorer",
    "apps",
    "gaming",
    "nvidia",
    "network",
    "power",
    "updates",
    "expert",
  ];

  const groups = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const match = (tw: Tweak) =>
      !q || [tw.name.fr, tw.name.en, tw.description.fr, tw.description.en].some((s) => s.toLowerCase().includes(q));
    return order
      .filter((c) => c !== "expert" || showExpert)
      .map((c) => ({ category: c, tweaks: (store.catalog?.tweaks ?? []).filter((tw) => tw.category === c && match(tw)) }))
      .filter((g) => g.tweaks.length > 0);
  });

  function profileDone(p: Profile) {
    return p.tweaks.every(isApplied);
  }

  function toggle(tw: Tweak, on: boolean) {
    run(on ? { op: "apply", ids: [tw.id] } : { op: "revert", ids: [tw.id] });
  }
</script>

<header>
  <h1>{t("tweaks.title")}</h1>
  <p class="muted">{t("tweaks.subtitle")}</p>
</header>

<section>
  <h2>{t("tweaks.profiles")}</h2>
  <div class="profiles">
    {#each store.catalog?.profiles ?? [] as p (p.id)}
      {@const done = profileDone(p)}
      <div class="card profile">
        <strong>{loc(p.name)}</strong>
        <p class="muted grow">{loc(p.description)}</p>
        <div class="row">
          <span class="muted small">{p.tweaks.filter(isApplied).length}/{p.tweaks.length}</span>
          <button
            class:primary={!done}
            disabled={store.busy || done}
            onclick={() => run({ op: "apply", ids: p.tweaks.filter((id) => !isApplied(id)) }, true)}
          >
            {done ? t("tweaks.profile_done") : t("tweaks.apply_profile")}
          </button>
        </div>
      </div>
    {/each}
  </div>
</section>

<div class="filters">
  <input type="search" placeholder={t("tweaks.search")} bind:value={query} />
  <label><input type="checkbox" bind:checked={showExpert} /> {t("tweaks.show_expert")}</label>
</div>

{#each groups as g (g.category)}
  <section>
    <h2>{t(`cat.${g.category}`)}</h2>
    <div class="card list">
      {#each g.tweaks as tw (tw.id)}
        {@const st = store.states[tw.id]}
        {@const applied = st?.state === "applied"}
        {@const undoable = store.journaled.includes(tw.id)}
        <div class="tweak">
          <div class="grow">
            <strong>{loc(tw.name)}</strong>
            <p class="muted">{loc(tw.description)}</p>
            {#if tw.note}<p class="note">{loc(tw.note)}</p>{/if}
            <div class="meta">
              <TweakBadges tweak={tw} />
              {#if applied && !undoable}<span class="small muted">{t("tweaks.already")}</span>{/if}
              {#if st?.state === "partial"}<span class="small warn">{t("tweaks.partial")}</span>{/if}
              {#if st?.state === "unavailable"}<span class="small muted">{t("tweaks.unavailable")} — {st.reason}</span>{/if}
            </div>
          </div>
          <Toggle
            checked={applied}
            label={loc(tw.name)}
            disabled={store.busy || st?.state === "unavailable" || (applied && !undoable)}
            onchange={(on) => toggle(tw, on)}
          />
        </div>
      {/each}
    </div>
  </section>
{/each}

<style>
  header {
    margin-bottom: 28px;
  }
  section {
    margin-bottom: 28px;
  }
  .profiles {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 12px;
  }
  .profile {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .profile strong {
    font-size: 15px;
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 8px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .small {
    font-size: 12.5px;
  }
  .warn,
  .note {
    color: var(--warn);
  }
  .note {
    font-size: 13px;
    margin-top: 4px;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 16px;
    align-items: center;
    margin-bottom: 20px;
  }
  .filters input[type="search"] {
    flex: 1;
    max-width: 360px;
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
  .tweak {
    display: flex;
    align-items: center;
    gap: 20px;
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
  }
  .tweak:last-child {
    border-bottom: none;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
    align-items: center;
    margin-top: 8px;
  }
</style>
