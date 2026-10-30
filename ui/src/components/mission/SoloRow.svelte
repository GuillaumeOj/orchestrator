<script>
  // Solo row (notes §1.4, Penpot P4.2 Solo row): project › session, the
  // status badge and the waiting-on chip (reason + since, or the activity).
  import StatusBadge from '../StatusBadge.svelte';
  import WaitingChip from '../WaitingChip.svelte';

  let { rec, children } = $props();
</script>

<div class="l1">
  <span class="where"><span class="muted">{rec.project ?? '—'} ›</span> <b>{rec.agent}</b></span>
  <StatusBadge state={rec.status} />
  <span class="why" class:error={rec.level === 'error'}><WaitingChip {rec} compact /></span>
</div>
{@render children?.()}

<style>
  .l1 {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
    white-space: nowrap;
  }
  .where {
    flex: none;
  }
  .why {
    flex: 1;
    min-width: 0;
    display: flex;
  }
  .why.error {
    color: var(--color-status-error);
  }
</style>
