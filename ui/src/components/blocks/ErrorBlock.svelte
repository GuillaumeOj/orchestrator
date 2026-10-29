<script>
  // `error` block (notes §3.3): "Error", the text in the error color, the detail
  // in mono behind "Show detail".
  let { block } = $props();
  let open = $state(false);
</script>

<div class="bh">Error</div>
<div class="err">{block.text}</div>
{#if block.detail}
  {#if open}<pre class="mono detail">{block.detail}</pre>{/if}
  <button class="link" onclick={() => (open = !open)}>{open ? 'Hide detail' : 'Show detail'}</button>
{/if}

<style>
  .detail {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font: var(--text-mono-sm);
    user-select: text;
    -webkit-user-select: text;
  }
</style>
