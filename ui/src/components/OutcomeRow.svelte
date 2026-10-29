<script>
  // An outcome in the digest (notes §5, Penpot P4.1 Outcome row): kind icon
  // (muted), the core's text, a task outcome's agent in muted parentheses, a
  // PR's link, the time on the right. The text truncates, the time never does.
  import { openUrl } from '../lib/app.svelte.js';
  import { clock } from '../lib/format.js';
  import { OUTCOME } from '../lib/status.js';

  /** @type {{ outcome: any, selected?: boolean, onselect?: () => void }} */
  let { outcome, selected = false, onselect = () => {} } = $props();
  const failed = $derived(outcome.kind === 'run_phase' && outcome.to === 'failed');
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="row" class:sel={selected} role="option" tabindex="-1" aria-selected={selected} onclick={onselect} title={outcome.text}>
  <!-- A run that failed is the one outcome marked (Penpot outcome row specimen). -->
  <span class="icon" class:muted={!failed} style:color={failed ? 'var(--color-phase-failed)' : null} aria-hidden="true">{OUTCOME[outcome.kind] ?? OUTCOME.task_done}</span>
  <span class="text">
    {#if outcome.url}<button class="link" onclick={(e) => (e.stopPropagation(), openUrl(outcome.url))}>{outcome.text}</button>{:else}{outcome.text}{/if}{#if outcome.kind === 'task_done' && outcome.agent}{' '}<span class="muted">({outcome.agent})</span>{/if}
  </span>
  <span class="muted at">{clock(outcome.at_ms)}</span>
</div>

<style>
  .row {
    display: flex;
    align-items: baseline;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-md);
    min-width: 0;
    cursor: default;
  }
  .row.sel {
    background: var(--color-surface-selected);
  }
  .text {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .at {
    flex: none;
  }
</style>
