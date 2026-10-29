<script>
  // The model the lead set for a task (task.flow.model); nothing when unset.
  let { task } = $props();
  const model = $derived(task?.flow?.model);
</script>

{#if model}<span class="chip mono" title="Runs on {model} (set by the lead)">{model}</span>{/if}

<style>
  .chip {
    font: var(--text-caption);
    color: var(--color-text-secondary);
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    padding: 0 var(--space-xs);
  }
</style>
