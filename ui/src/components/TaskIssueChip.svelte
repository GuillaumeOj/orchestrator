<script>
  // The issue a task is tagged with (allow-multiple-prs ux-brief §5): `#12`, mono,
  // muted border, next to the card title; the title in the tooltip. Untagged tasks
  // show nothing.
  import { features } from '../lib/app.svelte.js';
  import { issueLabel } from '../lib/issues.js';

  let { task } = $props();
  const ref = $derived(task?.issue ?? null);
  const title = $derived(ref ? (features().find((f) => f.id === task.feature_id)?.issues ?? []).find((i) => i.ref === ref)?.title : null);
</script>

{#if ref}<span class="chip mono" data-issue-chip title={issueLabel(ref, title)}>{ref}</span>{/if}

<style>
  .chip {
    flex: none;
    font: var(--text-caption);
    color: var(--color-text-secondary);
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    padding: 0 var(--space-xs);
  }
</style>
