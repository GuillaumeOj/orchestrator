<script>
  // Lifecycle marker (notes §3.4, Penpot P4.1 Lifecycle marker): one line under
  // the decision's title, never a spinner. Icon in color.lifecycle.<state>; the
  // text in text.secondary, failed in its own color, stale, dismissed and
  // interrupted muted. A PR opened links its URL.
  import { now30, openUrl } from '../lib/app.svelte.js';
  import { marker } from '../lib/inbox.js';
  import { LIFECYCLE } from '../lib/status.js';

  let { decision, pending = null } = $props();
  const m = $derived(marker(decision, pending, Math.max(now30(), Date.now())));
  const muted = $derived(['stale', 'dismissed', 'interrupted'].includes(m.state));
</script>

<div class="lc {m.state}" class:muted data-state={m.state}>
  <span class="icon" style:color={LIFECYCLE[m.state].color} aria-hidden="true">{LIFECYCLE[m.state].icon}</span>
  {#if m.url}
    <button class="link" onclick={() => openUrl(m.url)}>{m.text}</button>
  {:else}
    <span class="text">{m.text}</span>
  {/if}
</div>

<style>
  .lc {
    display: flex;
    align-items: baseline;
    gap: var(--space-xs);
    color: var(--color-text-secondary);
    min-width: 0;
  }
  .lc.failed {
    color: var(--color-lifecycle-failed);
  }
  .lc.muted {
    color: var(--color-text-muted);
  }
  .text {
    overflow-wrap: anywhere;
  }
</style>
