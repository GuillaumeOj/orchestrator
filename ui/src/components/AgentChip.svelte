<script>
  // Role color + role initial + agent name (§9.1 principle 7). The role comes
  // from the agent's data; `role` / `color` / `name` override it (canvas
  // nodes, tasks assigned to a node).
  import { agent } from '../lib/app.svelte.js';
  import { roleOf } from '../lib/status.js';

  let { id, compact = false, role = null, color = null, name = null } = $props();
  const r = $derived(roleOf(id, role ?? agent(id)?.role ?? null, color));
</script>

<span class="chip" class:compact>
  <span class="mark" style:color={r.color} style:border-color={r.color}>{r.letter}</span>
  {#if !compact}<span class="name">{name ?? id}</span>{/if}
</span>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    min-width: 0;
  }
  /* A compact chip is one fixed-width mark: it never shrinks in a flex row. */
  .chip.compact {
    flex: none;
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
