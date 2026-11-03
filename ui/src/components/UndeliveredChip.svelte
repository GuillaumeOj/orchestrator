<script>
  // Undelivered chip (ux-brief §2): "2 undelivered · held: off hours". Muted
  // under 10 min, warning after. A click opens the delivery decision, else the
  // feature timeline filtered to Delivery. Shows nothing when nothing waits.
  import { now30, openDelivery } from '../lib/app.svelte.js';
  import { undeliveredChip } from '../lib/delivery.js';

  let { rec } = $props();
  const now = $derived(now30());
  const chip = $derived(undeliveredChip(rec, now));
</script>

{#if chip}
  <button class="udel" class:warn={chip.warn} title={chip.since ? `Waiting ${chip.since}` : null} onclick={(e) => (e.stopPropagation(), openDelivery(rec))}>{chip.text}</button>
{/if}

<style>
  .udel {
    flex: none;
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-full);
    background: transparent;
    color: var(--color-text-muted);
    font: var(--text-caption);
    white-space: nowrap;
    cursor: pointer;
  }
  .udel.warn {
    border-color: var(--color-attention-needs-you);
    color: var(--color-attention-needs-you);
  }
</style>
