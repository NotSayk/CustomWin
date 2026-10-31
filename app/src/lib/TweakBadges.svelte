<script lang="ts">
  import { t } from "./i18n.svelte";
  import type { Tweak } from "./types";

  let { tweak }: { tweak: Tweak } = $props();

  const riskClass = { safe: "ok", low: "", medium: "warn", expert: "danger" } as const;
</script>

<span class="badges">
  <span class="badge {riskClass[tweak.risk]}">{t(`risk.${tweak.risk}`)}</span>
  {#if tweak.evidence !== "functional"}
    <span class="badge accent">{t(`evidence.${tweak.evidence}`)}</span>
  {/if}
  {#if !tweak.fully_reversible}
    <span class="badge warn" title={t("badge.partial.hint")}>{t("badge.partial")}</span>
  {/if}
  {#if tweak.requires_admin}
    <span class="badge">{t("badge.admin")}</span>
  {/if}
  {#if tweak.restart !== "none"}
    <span class="badge">{t(`badge.${tweak.restart}`)}</span>
  {/if}
</span>

<style>
  .badges {
    display: inline-flex;
    flex-wrap: wrap;
    gap: 6px;
  }
</style>
