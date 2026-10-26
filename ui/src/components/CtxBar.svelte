<script>
  let { value, width = 32 } = $props();
</script>

{#if value == null}<span class="muted">—</span>{:else}
  <span class="ctx"><span class="bar" style:width="{width}px"><i style:width="{Math.min(100, value)}%"></i></span> {Math.round(value)}%</span>
{/if}

<style>
  .ctx {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    white-space: nowrap;
  }
</style>
