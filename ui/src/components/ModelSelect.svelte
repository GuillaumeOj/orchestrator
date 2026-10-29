<script>
  // A model picker: the default, an alias, or a custom id typed in a mono input.
  // `value` is the stored model (null for default); `onsave` gets the value to store.
  import { MODEL_ALIASES, modelOption, modelValue } from '../lib/canvas.js';

  let { label, value, emptyLabel = "default (Claude Code's)", onsave } = $props();
  let picking = $state(false); // "custom…" picked, text not typed yet
  const option = $derived(picking ? 'custom' : modelOption(value));

  // A new stored value (a save landed, an undo) ends the pending pick.
  $effect(() => {
    value;
    picking = false;
  });

  function pick(e) {
    const v = e.currentTarget.value;
    picking = v === 'custom';
    if (!picking) onsave(modelValue(v));
  }
</script>

<label><span>{label}</span>
  <select class="field" value={option} onchange={pick}>
    <option value="">{emptyLabel}</option>
    {#each MODEL_ALIASES as v (v)}<option value={v}>{v}</option>{/each}
    <option value="custom">custom…</option>
  </select>
</label>
{#if option === 'custom'}
  <input class="field mono custom" aria-label="Custom {label}" placeholder="claude-…" value={modelOption(value) === 'custom' ? value : ''} onchange={(e) => onsave(modelValue('custom', e.currentTarget.value))} />
{/if}

<style>
  /* The inspector's label grid (the parent's scoped styles do not reach in here). */
  label {
    display: grid;
    grid-template-columns: 80px 1fr;
    align-items: center;
    gap: var(--space-sm);
  }
  label > span {
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .custom {
    margin-left: calc(80px + var(--space-sm));
  }
</style>
