<script>
  // Integration / MCP server status (p3-design-notes §5): glyph + label, the
  // label carries the meaning. `busy` = a test is running ("Checking…").
  import { connStatus } from '../lib/ecosystem.js';

  let { status, connector = false, busy = false, compact = false } = $props();
  const s = $derived(connStatus(busy ? 'checking' : status, connector));
</script>

<span class="conn" class:compact title={s.label}>
  <span class="icon" style:color={s.color} aria-hidden="true">{s.icon}</span>
  {#if !compact}<span class="label">{s.label}</span>{/if}
</span>

<style>
  .conn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    white-space: nowrap;
  }
  .icon {
    width: var(--size-icon);
    text-align: center;
  }
  .label {
    color: var(--color-text-secondary);
  }
</style>
