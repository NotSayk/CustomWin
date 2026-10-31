<script lang="ts">
  let {
    checked,
    disabled = false,
    label,
    onchange,
  }: { checked: boolean; disabled?: boolean; label: string; onchange: (value: boolean) => void } = $props();
</script>

<button
  class="toggle"
  class:on={checked}
  role="switch"
  aria-checked={checked}
  aria-label={label}
  {disabled}
  onclick={() => onchange(!checked)}
>
  <span class="knob"></span>
</button>

<style>
  .toggle {
    flex: none;
    width: 40px;
    height: 20px;
    padding: 0;
    border-radius: 999px;
    border: 1px solid var(--text-2);
    background: transparent;
    position: relative;
  }
  .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--text-2);
    transition: transform 0.15s, background 0.15s;
  }
  .on {
    background: var(--accent);
    border-color: var(--accent);
  }
  .on .knob {
    transform: translateX(20px);
    background: var(--accent-text);
  }
</style>
