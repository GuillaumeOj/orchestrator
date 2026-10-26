<script>
  // §9.5: one icon, one color and one label per state; the label carries the meaning.
  import { STATUS } from '../lib/status.js';
  let { state, compact = false, label = null } = $props();
  const s = $derived(STATUS[state] ?? STATUS.idle);
</script>

<span class="badge" class:compact title={s.label}>
  <span class="icon" style:color={s.color} aria-hidden="true">{s.icon}</span>
  {#if !compact}<span class="label">{label ?? s.label}</span>{/if}
</span>

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    height: var(--size-control-sm);
    padding: 0 var(--space-sm);
    border-radius: var(--radius-sm);
    background: var(--color-surface-raised);
    white-space: nowrap;
  }
  .badge.compact {
    padding: 0;
    background: none;
    width: var(--size-icon);
    justify-content: center;
  }
  .label {
    color: var(--color-text-secondary);
  }
  .icon {
    width: var(--size-icon);
    text-align: center;
  }
</style>
