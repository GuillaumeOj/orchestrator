<script>
  // `countdown` block (notes §3.3): the default's verb and the time left ("Keeps
  // queued in 18 min", "Applies in 18 min"); past the deadline "Applying the
  // default…". On the 30 s clock.
  import { now30 } from '../../lib/app.svelte.js';
  import { duration } from '../../lib/format.js';

  let { block, kind = null } = $props();
  const left = $derived(block.deadline_ms - Math.max(now30(), Date.now()));
  // The default's verb: an ask keeps the agent queued, a budget skips.
  const verb = $derived(block.default !== 'reject' ? 'Applies' : kind === 'ask' ? 'Keeps queued' : 'Skips');
</script>

<div class="cd">
  <span class="muted" aria-hidden="true">{'\u{F051B}'}</span>
  {left > 0 ? `${verb} in ${duration(left)}` : 'Applying the default…'}
</div>

<style>
  .cd {
    display: flex;
    gap: var(--space-sm);
  }
</style>
