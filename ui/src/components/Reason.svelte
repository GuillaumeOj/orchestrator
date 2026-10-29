<script>
  // An attention record's reason (or a running agent's activity) with how long
  // it has held: "→ lead · review of T-4 · 3m" (ux-brief §2: the core writes the
  // reason, the UI appends since). Plain text until F2's waiting-on chip.
  import { now30 } from '../lib/app.svelte.js';
  import { ago } from '../lib/format.js';
  import { since } from '../lib/now.svelte.js';

  let { rec } = $props();
  const what = $derived(rec?.reason || rec?.activity || '');
  const text = $derived(rec && what ? `${what} · ${ago(since(rec), Math.max(now30(), Date.now()))}` : '');
</script>

{#if text}<span class="reason" title={text}>{text}</span>{/if}

<style>
  .reason {
    color: var(--color-text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
</style>
