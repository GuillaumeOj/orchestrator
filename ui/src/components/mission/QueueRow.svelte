<script>
  // Queue row (notes §1.5, Penpot P4.2 Queue row): the agent, its run or
  // project, the badge and the core's reason, a tier tag when not normal,
  // since, then Run now / Start now with its cost in the binding window (or
  // Resume for a paused one) and Cancel. Everything from the record (D7).
  import { now30 } from '../../lib/app.svelte.js';
  import { waited } from '../../lib/format.js';
  import { runNowCost } from '../../lib/mission.js';
  import AgentChip from '../AgentChip.svelte';
  import Kbd from '../Kbd.svelte';
  import StatusBadge from '../StatusBadge.svelte';

  let { rec, run = null, onrun = () => {}, oncancel = () => {}, onresume = () => {}, children } = $props();
  const now = $derived(Math.max(now30(), Date.now()));
  const fresh = $derived(rec.status === 'not_started');
  const cost = $derived(runNowCost(rec.queue));
  const tier = $derived(rec.queue?.tier && rec.queue.tier !== 'normal' ? rec.queue.tier : null);
  const since = $derived(waited(rec.waiting_on?.since_ms ?? rec.since_ms, now));
</script>

<div class="l1">
  <span class="who"><AgentChip id={rec.agent} compact /> <b>{rec.agent}</b> <span class="muted">· {run ? `${run.project} › ${run.feature}` : (rec.project ?? '')}</span></span>
  <StatusBadge state={rec.status} />
  <span class="reason" title={rec.reason}>{rec.reason}</span>
  {#if tier}<span class="tier" title="Allocation tier: {tier}">{tier}</span>{/if}
  <span class="grow"></span>
  {#if since}<span class="muted since">{since}</span>{/if}
  {#if rec.queue}
    <button class="btn" tabindex="-1" onclick={(e) => (e.stopPropagation(), onrun())}>{fresh ? 'Start now' : 'Run now'}{cost ? ` ${cost}` : ''} <Kbd keys="n" /></button>
  {:else if rec.status === 'paused'}
    <button class="btn" tabindex="-1" onclick={(e) => (e.stopPropagation(), onresume())}>Resume <Kbd keys="r" /></button>
  {/if}
  {#if rec.queue}<button class="btn ghost" tabindex="-1" title="Removes it from the queue; the agent stays as it is" onclick={(e) => (e.stopPropagation(), oncancel())}>Cancel</button>{/if}
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
  .who {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    flex: none;
  }
  .reason {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--color-text-secondary);
  }
  .tier {
    flex: none;
    padding: 0 var(--space-xs);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .since,
  .btn {
    flex: none;
  }
</style>
