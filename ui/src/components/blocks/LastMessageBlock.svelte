<script>
  // `last_message` block (notes §3.3): "Last message · 14:02", the text, 6 lines
  // then "Show more".
  import { clock } from '../../lib/format.js';
  import { collapse } from '../../lib/inbox.js';

  let { block } = $props();
  let open = $state(false);
  const shown = $derived(open ? { text: block.text, more: 0 } : collapse(block.text ?? ''));
</script>

<div class="bh">Last message{block.at_ms ? ` · ${clock(block.at_ms)}` : ''}</div>
<p class="text">{shown.text}</p>
{#if shown.more}<button class="link" onclick={() => (open = true)}>Show more</button>{/if}

<style>
  .text {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }
</style>
