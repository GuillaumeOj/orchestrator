<script>
  // Global banners (§9.3, Penpot 05 copy): one at a time, highest attention first;
  // the rest collapse to "+n". Error banners: a 4 px left bar in the status color.
  import { app, claudeBusy, claudeUpdate, governor, openPlanner, openSettings, openSingleton } from '../lib/app.svelte.js';
  import { bannerClaude } from '../lib/claude.js';
  import { when } from '../lib/format.js';
  import { counts } from '../lib/now.svelte.js';
  import { tooltip } from '../lib/tooltip.js';
  import { isRestoreWarning } from '../lib/update.js';

  let dismissed = $state({});
  // Each `claude update` run is its own state: a dismissal made before it does not cover its outcome.
  let claudeRuns = $state(0);
  let wasUpdating = false;
  $effect(() => {
    const now = app.data?.setup?.claude?.update?.state === 'updating';
    if (now && !wasUpdating) claudeRuns++;
    wasUpdating = now;
  });
  const banners = $derived.by(() => {
    const d = app.data;
    const out = [];
    if (!d) return out;
    if (d.core?.error) out.push({ id: 'core', color: 'var(--color-status-error)', title: 'No agent core.', text: d.core.error });
    const c = d.setup?.claude;
    if (c?.problem) {
      // A too-old Claude Code offers the update; a dismissal only covers the state it was made in.
      const v = bannerClaude(c);
      out.push({ id: v ? `claude|${v.key}|${claudeRuns}` : 'claude', color: 'var(--color-status-error)', title: 'Read-only:', text: c.problem, fail: v?.fail, update: v?.button, setup: !v?.details, claudeDetails: v?.details });
    }
    const u = d.usage;
    if (u?.weekly?.used_pct >= 100)
      out.push({ id: 'week', color: 'var(--color-status-blocked-by-limit)', title: 'Weekly limit reached.', text: `All agents are blocked by limit; autonomous work resumes in priority order at the reset, ${when(u.weekly.resets_at * 1000)}. Nothing is lost.`, planner: true });
    else if (u?.five_hour?.used_pct >= 100)
      out.push({ id: '5h', color: 'var(--color-status-blocked-by-limit)', title: '5-hour limit reached.', text: `Agents are blocked until ${when(u.five_hour.resets_at * 1000)}. Interactive sessions resume first.`, planner: true });
    const s = u?.snapshot;
    const tally = counts();
    if (s?.state === 'red' && !u?.shadow)
      out.push({ id: 'red', color: 'var(--color-pacing-red)', title: `Governor in Red: week +${Math.round(s.error)} pp over plan.`, text: `Live autonomous agents capped to ${tally.live_cap ?? s.live_cap}; queued ${tally.queued}.`, planner: true, pause: true });
    // FR-816: off-hours work waits while the Mac is on battery (one banner, D6).
    if ((d.banners ?? []).some((b) => b.kind === 'not_on_ac')) {
      const n = d.power?.off_hours_pending ?? 0;
      out.push({ id: 'ac', color: 'var(--color-pacing-amber)', title: 'Off-hours work needs AC power.', text: `${n} off-hours task${n === 1 ? '' : 's'} won't run while the Mac sleeps. Plug in to keep it awake, or they wait for the next off-hours window.`, planner: true });
    }
    for (const w of (d.warnings ?? []).filter((w) => !isRestoreWarning(w))) out.push({ id: `w:${w}`, color: 'var(--color-status-queued)', title: 'Check:', text: w });
    return out.filter((b) => !dismissed[b.id]);
  });
  const top = $derived(banners[0]);
</script>

{#if top}
  <div class="banner" style:box-shadow="inset 4px 0 0 {top.color}" role="status">
    <b>{top.title}</b>
    <span class="text" class:one={top.update}>{top.text}{#if top.fail} <span class="amber">{top.fail}</span>{/if}</span>
    {#if top.update}<button class="btn" disabled={top.update.disabled || claudeBusy()} data-claude-update onclick={claudeUpdate}>{#if top.update.spinner}<span class="spinning" aria-hidden="true">◐</span> {/if}{top.update.label}</button>{/if}
    {#if top.claudeDetails}<button class="btn" onclick={() => openSettings('claude')}>Details</button>{/if}
    {#if top.planner}<button type="button" class="btn" use:tooltip={{ label: 'Week planner', key: '⌘; g' }} onclick={() => openPlanner()}>Week planner</button>{/if}
    {#if top.setup}<button class="btn" onclick={() => openSingleton('setup')}>Set up</button>{/if}
    {#if top.pause}<button type="button" class="btn danger" onclick={() => governor('pause_all')}>Pause all</button>{/if}
    {#if banners.length > 1}<span class="muted">+{banners.length - 1}</span>{/if}
    <button type="button" class="icon-btn" aria-label="Dismiss" use:tooltip={'Dismiss'} onclick={() => (dismissed[top.id] = true)}>{'\u{F0156}'}</button>
  </div>
{/if}

<style>
  .banner {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    min-height: 36px;
    padding: var(--space-xs) var(--space-md) var(--space-xs) var(--space-lg);
    background: var(--color-surface-raised);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-label);
  }
  .text {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .text.one {
    white-space: nowrap;
  }
  .amber {
    color: var(--color-pacing-amber);
  }

</style>
