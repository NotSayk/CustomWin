<script lang="ts">
  import { t, type Key } from "../lib/i18n.svelte";

  let { title, body }: { title: Key; body: Key } = $props();
</script>

<h1>{t(title)}</h1>
<div class="card">
  <span class="badge accent">{t("soon.label")}</span>
  <p>{t(body)}</p>
</div>

<style>
  .card {
    margin-top: 20px;
    max-width: 640px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    align-items: flex-start;
  }
</style>
