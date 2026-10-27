<script>
  // Role color + role initial + agent name (§9.1 principle 7).
  import { roleOf } from '../lib/status.js';

  let { id, compact = false } = $props();
  const r = $derived(roleOf(id));
</script>

<span class="chip">
  <span class="mark" style:color={r.color} style:border-color={r.color}>{r.letter}</span>
  {#if !compact}<span class="name">{id}</span>{/if}
</span>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    min-width: 0;
  }
  .mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 16px;
    flex: none;
    border: var(--border-width-default) solid;
    border-radius: var(--radius-sm);
    font: var(--text-caption);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
