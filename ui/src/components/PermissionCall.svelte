<script>
  // A `tool_call` block (FR-705, notes §3.3): "<tool> in <worktree name>" (the
  // full cwd on hover) and the input in mono, collapsed past 6 lines. In the
  // Inbox detail and the agent pane's permission strip.
  import { baseName } from '../lib/format.js';
  import { collapse, inputText } from '../lib/inbox.js';

  let { block } = $props();
  let open = $state(false);
  const text = $derived(inputText(block?.input));
  const shown = $derived(open ? { text, more: 0 } : collapse(text));
</script>

<div class="perm">
  <div class="call bh">
    <span class="mono">{block?.tool ?? '?'}</span>
    {#if block?.cwd}<span class="muted">in</span> <span class="mono" title={block.cwd}>{baseName(block.cwd)}</span>{/if}
  </div>
  {#if text}
    <pre class="mono input">{shown.text}</pre>
    {#if shown.more}
      <button class="link more" onclick={() => (open = true)}>Show {shown.more} more line{shown.more === 1 ? '' : 's'}</button>
    {:else if open}
      <button class="link more" onclick={() => (open = false)}>Show less</button>
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
  .call {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
  }
  .input {
    margin: 0;
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
