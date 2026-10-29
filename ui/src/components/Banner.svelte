<script>
  // Global banners (§9.3, Penpot 05 copy): one at a time, highest attention first;
  // the rest collapse to "+n". Error banners: a 4 px left bar in the status color.
  import { app, governor, openSingleton } from '../lib/app.svelte.js';
  import { when } from '../lib/format.js';
  import { counts } from '../lib/now.svelte.js';
  import Kbd from './Kbd.svelte';

  let dismissed = $state({});
  const banners = $derived.by(() => {
    const d = app.data;
    const out = [];
    if (!d) return out;
    if (d.core?.error) out.push({ id: 'core', color: 'var(--color-status-error)', title: 'No agent core.', text: d.core.error });
    const c = d.setup?.claude;
    if (c?.problem) out.push({ id: 'claude', color: 'var(--color-status-error)', title: 'Read-only:', text: c.problem, setup: true });
    const u = d.usage;
    if (u?.weekly?.used_pct >= 100)
      out.push({ id: 'week', color: 'var(--color-status-blocked-by-limit)', title: 'Weekly limit reached.', text: `All agents are blocked by limit; autonomous work resumes in priority order at the reset, ${when(u.weekly.resets_at * 1000)}. Nothing is lost.`, planner: true });
    else if (u?.five_hour?.used_pct >= 100)
      out.push({ id: '5h', color: 'var(--color-status-blocked-by-limit)', title: '5-hour limit reached.', text: `Agents are blocked until ${when(u.five_hour.resets_at * 1000)}. Interactive sessions resume first.`, planner: true });
    const s = u?.snapshot;
    const tally = counts();
    if (s?.state === 'red' && !u?.shadow)
      out.push({ id: 'red', color: 'var(--color-pacing-red)', title: `Governor in Red: week +${Math.round(s.error)} pp over plan.`, text: `Live autonomous agents capped to ${tally.live_cap}; queued ${tally.queued}.`, planner: true, pause: true });
    // FR-816: off-hours work waits while the Mac is on battery (one banner, D6).
    if ((d.banners ?? []).some((b) => b.kind === 'not_on_ac')) {
      const n = d.power?.off_hours_pending ?? 0;
      out.push({ id: 'ac', color: 'var(--color-pacing-amber)', title: 'Off-hours work needs AC power.', text: `${n} off-hours task${n === 1 ? '' : 's'} won't run while the Mac sleeps. Plug in to keep it awake, or they wait for the next off-hours window.`, details: true, later: true });
    }
    for (const w of d.warnings ?? []) out.push({ id: `w:${w}`, color: 'var(--color-status-queued)', title: 'Check:', text: w });
    return out.filter((b) => !dismissed[b.id]);
  });
  const top = $derived(banners[0]);
</script>

{#if top}
  <div class="banner" style:box-shadow="inset 4px 0 0 {top.color}" role="status">
    <b>{top.title}</b>
    <span class="text">{top.text}</span>
    {#if top.planner}<button class="btn" onclick={() => openSingleton('planner')}>Week planner <Kbd keys="⌘+; g" /></button>{/if}
    {#if top.setup}<button class="btn" onclick={() => openSingleton('setup')}>Set up</button>{/if}
    {#if top.details}<button class="btn" onclick={() => openSingleton('planner')}>Details <Kbd keys="o" /></button>{/if}
    {#if top.later}<button class="btn ghost" onclick={() => (dismissed[top.id] = true)}>Later <Kbd keys="esc" /></button>{/if}
    {#if top.pause}<button class="btn ghost" onclick={() => governor('pause_all')}>Pause all</button>{/if}
    {#if banners.length > 1}<span class="muted">+{banners.length - 1}</span>{/if}
    <button class="x" aria-label="Dismiss" onclick={() => (dismissed[top.id] = true)}>×</button>
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
  .x {
    background: none;
    border: 0;
    color: var(--color-text-muted);
  }
</style>
