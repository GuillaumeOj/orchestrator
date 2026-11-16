<script>
  // Run row (notes §1.3, Penpot P4.2 Run row): project › feature, the phase
  // stepper and phase text; tasks, PR and the blocker line; member chips, lead
  // first (past 6: "+n more"). The chip at `chip` is the h/l target.
  import { features, openFeature, openUrl } from '../../lib/app.svelte.js';
  import { waited } from '../../lib/format.js';
  import { originOf } from '../../lib/intake.js';
  import { phaseFull, phaseText, prText, tasksText } from '../../lib/mission.js';
  import { attention } from '../../lib/now.svelte.js';
  import { isPerIssue, prsDot, prsSummary, prsTooltip } from '../../lib/pr.js';
  import MemberChip from './MemberChip.svelte';
  import OriginChip from './OriginChip.svelte';
  import PhaseStepper from './PhaseStepper.svelte';

  const MAX_CHIPS = 6;

  /** @type {{ run: any, now: number, target: string | null, busy?: string | null, onchip?: (i: number) => void }} */
  let { run, now, target, busy = null, onchip = () => {} } = $props();
  const b = $derived(run.blocker);
  const error = $derived(run.level === 'error');
  // allow-multiple-prs: per issue, the summary of every PR; a click opens the feature pane (ux-brief §4).
  const perIssue = $derived(isPerIssue(run.prs_view));
  const feature = $derived(perIssue ? features().find((f) => f.run?.id === run.run) : null);
  const issueOf = (ref) => feature?.issues?.find((i) => i.ref === ref);
  // Past six chips the selected one is swapped in for the sixth (notes §1.3 Overflow).
  const shown = $derived.by(() => {
    const m = run.members.map((id, i) => ({ id, i }));
    if (m.length <= MAX_CHIPS) return m;
    const sel = m.findIndex((x) => x.id === target);
    const head = m.slice(0, MAX_CHIPS);
    return sel >= MAX_CHIPS ? [...head.slice(0, MAX_CHIPS - 1), m[sel]] : head;
  });
  const more = $derived(Math.max(0, run.members.length - MAX_CHIPS));
</script>

<div class="l1">
  <span class="where"><span class="muted">{run.project} ›</span> <b>{run.feature}</b></span>
  {#if originOf(run)}<OriginChip origin={originOf(run)} project={run.project} />{/if}
  <PhaseStepper {run} title={phaseFull(run, now)} />
  <span class="grow"></span>
  <span class="phase" class:failed={run.phase === 'failed'} title={phaseFull(run, now)}>{phaseText(run, now)}</span>
</div>
<div class="l2">
  <span>{tasksText(run.tasks)}</span>
  {#if perIssue}
    {@const dot = prsDot(run.prs_view)}
    <span class="muted">·</span><button class="link" tabindex="-1" data-prs title={prsTooltip(run.prs_view, issueOf)} onclick={(e) => (e.stopPropagation(), feature && openFeature(feature.id))}>{#if dot}<span class="dot" style:background={dot} aria-hidden="true"></span>{/if}{prsSummary(run.prs_view)}</button>
  {:else if run.pr}<span class="muted">·</span><button class="link" tabindex="-1" onclick={(e) => (e.stopPropagation(), openUrl(run.pr.url))}>{prText(run.pr)}</button>{/if}
  {#if b}
    <span class="muted">·</span>
    <span class="blocker"><b class:error>{error ? 'error:' : 'blocker:'}</b> {b.agent} · {b.reason}{b.waiting_on?.since_ms ? ` · ${waited(b.waiting_on.since_ms, now)}` : ''}</span>
  {/if}
</div>
<div class="chips">
  {#each shown as m (m.id)}
    <MemberChip id={m.id} rec={attention(m.id)} {now} selected={m.id === target} busy={busy === m.id} onclick={(e) => (e.stopPropagation(), onchip(m.i))} />
  {/each}
  {#if more}<span class="more">+{more} more</span>{/if}
</div>

<style>
  .l1,
  .l2 {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
    white-space: nowrap;
  }
  .l2 {
    margin-top: var(--space-xxs);
  }
  .where {
    flex: none;
  }
  .phase {
    color: var(--color-text-secondary);
    flex: none;
  }
  .phase.failed,
  .error {
    color: var(--color-status-error);
  }
  .blocker {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .blocker b {
    color: var(--color-attention-needs-you);
  }
  .blocker b.error {
    color: var(--color-status-error);
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
  .link[data-prs] {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .dot {
    width: var(--size-status-dot, 8px);
    height: var(--size-status-dot, 8px);
    border-radius: 50%;
    flex: none;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
    margin-top: var(--space-xs);
  }
  .more {
    display: inline-flex;
    align-items: center;
    height: var(--size-chip);
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    font: var(--text-label);
  }
</style>
