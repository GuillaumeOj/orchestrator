<script>
  // `pty_tail` block (notes §3.3): the pane's last 12 lines, mono on the
  // chrome surface, read-only.
  import { tailLines } from '../../lib/inbox.js';

  let { block } = $props();
</script>

<div class="bh">In its pane · last 12 lines</div>
<pre class="mono tail">{tailLines(block.text)}</pre>

<style>
  .tail {
    margin: 0;
    padding: var(--space-sm);
    font: var(--text-mono-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    background: var(--color-surface-chrome);
    border-radius: var(--radius-sm);
    user-select: text;
    -webkit-user-select: text;
  }
</style>
