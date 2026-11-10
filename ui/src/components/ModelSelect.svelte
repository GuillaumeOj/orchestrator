<script>
  // A model picker: the default, an alias, or a custom id typed in a mono input.
  // `value` is the stored model (null for default); `onsave` gets the value to store.
  import { modelOption, modelValue } from '../lib/canvas.js';
  import { MODELS } from '../lib/options.js';
  import Select from './Select.svelte';

  let { label, value, emptyLabel = "default (Claude Code's)", onsave } = $props();
  let picking = $state(false); // "custom…" picked, text not typed yet
  let rev = $state(0); // a refused save remounts the picker on the stored value
  const option = $derived(picking ? 'custom' : modelOption(value));
  const options = $derived(MODELS.map((m) => (m.value === '' ? { ...m, label: emptyLabel } : m)));

  // A new stored value (a save landed, an undo) ends the pending pick.
  $effect(() => {
    value;
    picking = false;
  });

  // `onsave` may resolve false: not saved, so the picker goes back to the stored value.
  async function save(model) {
    if ((await onsave(model)) === false) {
      picking = false;
      rev++;
    }
  }

  function pick(v) {
    picking = v === 'custom';
    if (!picking) save(modelValue(v));
  }
</script>

<!-- A refused save remounts both: the Select and the custom field show the saved value again. -->
{#key rev}
  <label><span>{label}</span>
    <Select value={option} {options} noun="model" onchange={pick} />
  </label>
  {#if option === 'custom'}
    <input class="field mono custom" aria-label="Custom {label}" placeholder="claude-…" value={modelOption(value) === 'custom' ? value : ''} onchange={(e) => save(modelValue('custom', e.currentTarget.value))} />
  {/if}
{/key}

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
