<script>
  // The Work pane's hint row (notes §2.1, P4.2 §1.9): only the keys the focused row allows.
  import Kbd from '../Kbd.svelte';

  /** `hints`: [key, words] pairs. */
  let { hints = [] } = $props();
</script>

<div class="hints" aria-hidden="true">
  {#each hints as [k, words] (k + words)}<span class="h"><Kbd keys={k} /> {words}</span>{/each}
</div>

<style>
  .hints {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-md);
    min-height: var(--size-strip);
    padding: 0 var(--space-md);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-caption);
    color: var(--color-text-muted);
    overflow: hidden;
  }
  .h {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    white-space: nowrap;
  }
</style>
