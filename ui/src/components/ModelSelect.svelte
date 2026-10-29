<script>
  // A model picker: the default, an alias, or a custom id typed in a mono input.
  // `value` is the stored model (null for default); `onsave` gets the value to store.
  import { MODEL_ALIASES, modelOption, modelValue } from '../lib/canvas.js';

  let { label, value, emptyLabel = "default (Claude Code's)", onsave } = $props();
  let picking = $state(false); // "custom…" picked, text not typed yet
  const option = $derived(picking ? 'custom' : modelOption(value));

  // A new stored value (another node selected, a save landed) ends the pending pick.
  $effect(() => {
    value;
    picking = false;
  });
</script>

<label><span>{label}</span>
  <select class="field" value={option} onchange={(e) => ((picking = e.currentTarget.value === 'custom'), !picking && onsave(modelValue(e.currentTarget.value)))}>
    <option value="">{emptyLabel}</option>
    {#each MODEL_ALIASES as v (v)}<option value={v}>{v}</option>{/each}
    <option value="custom">custom…</option>
  </select>
</label>
{#if option === 'custom'}
  <input class="field mono" aria-label="Custom {label}" placeholder="claude-…" value={value ?? ''} onchange={(e) => onsave(modelValue('custom', e.currentTarget.value))} />
{/if}
