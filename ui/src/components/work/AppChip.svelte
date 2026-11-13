<script>
  // App-state chip (notes §2.5, page 02): the only coloured thing on a row (status tokens); "ready" keeps
  // the P4.5 dashed mark. Clickable (feature, program, PR) but never a Tab stop.
  import { tooltip } from '../../lib/tooltip.js';

  /** `chip` from chipOf(); `onopen(click)` runs its target. */
  let { chip, onopen = null } = $props();
</script>

{#if chip}
  {#if chip.click && onopen}
    <button type="button" class="chip {chip.tone}" tabindex="-1" data-chip={chip.tone} use:tooltip={chip.tip} onclick={(e) => (e.stopPropagation(), onopen(chip.click))}>{chip.text}</button>
  {:else}
    <span class="chip {chip.tone}" data-chip={chip.tone} use:tooltip={chip.tip}>{chip.text}</span>
  {/if}
{/if}

<style>
  .chip {
    --tone: var(--color-text-muted);
    display: inline-flex;
    align-items: center;
    height: 18px;
    max-width: 100%;
    padding: 0 var(--space-xs);
    border: var(--border-width-default) solid var(--tone);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--tone) 20%, transparent);
    color: var(--tone);
    font: var(--text-mono-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  button.chip {
    cursor: pointer;
  }
  .ready {
    border-style: dashed;
    background: none;
    color: var(--color-text-muted);
    border-color: var(--color-text-muted);
  }
  .waiting {
    --tone: var(--color-status-queued);
    background: none;
  }
  .queued {
    --tone: var(--color-status-queued);
  }
  .working {
    --tone: var(--color-status-working);
  }
  .pr {
    --tone: var(--color-status-at-gate);
  }
  .done {
    --tone: var(--color-status-done);
  }
  .program {
    --tone: var(--color-status-working);
    background: none;
  }
</style>
