<script>
  // A permission prompt's call (FR-705, ux-brief touch points 1–2), in the Inbox
  // detail and the agent pane strip: the Parked marker, the tool, the cwd (its
  // worktree name, the full path on hover) and the full input, collapsed past 6 lines.
  import { collapse, cwdName, inputText, parked } from '../lib/inbox.js';
  import StatusBadge from './StatusBadge.svelte';

  let { item } = $props();
  let openFor = $state(null); // the item shown in full
  const p = $derived(item?.payload ?? {});
  const text = $derived(inputText(p.tool_input));
  const cut = $derived(openFor === item?.id ? { text, more: 0 } : collapse(text));
</script>

<div class="perm">
  {#if parked(item)}
    <div class="parked"><StatusBadge state="suspended" label="Parked" /> <span class="secondary">the agent stopped and resumes on your answer</span></div>
  {/if}
  <div class="call">
    <span class="mono">{p.tool ?? '?'}</span>
    {#if p.cwd}<span class="muted">in</span> <span class="mono" title={p.cwd}>{cwdName(p.cwd)}</span>{/if}
  </div>
  {#if text}
    <pre class="mono input">{cut.text}</pre>
    {#if cut.more}
      <button class="btn ghost more" onclick={() => (openFor = item.id)}>Show all · {cut.more} more line{cut.more === 1 ? '' : 's'}</button>
    {:else if openFor === item?.id}
      <button class="btn ghost more" onclick={() => (openFor = null)}>Show less</button>
    {/if}
  {/if}
</div>

<style>
  .perm {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    min-width: 0;
  }
  .parked,
  .call {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
  }
  .input {
    margin: 0;
    padding: var(--space-md);
    background: var(--color-surface-chrome);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
    max-height: 50vh;
    overflow: auto;
  }
  .more {
    align-self: flex-start;
  }
</style>
