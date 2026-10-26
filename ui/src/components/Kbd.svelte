<script>
  // One key-hint container per shortcut (§9.7): "⌘+K", "⌘+; z".
  let { keys } = $props();
</script>

{#if keys}<span class="kbd">{keys}</span>{/if}
