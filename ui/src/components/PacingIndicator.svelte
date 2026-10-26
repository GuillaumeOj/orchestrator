<script>
  // Two-window indicator (J8 step 2, Penpot 02 "Pacing indicator"): "Week" is the
  // total the plan reports, "agents" the governed share; the plan and the pp delta
  // refer to the agents. Bars: fill = agents used, 2 px tick = plan.
  import { app, openSingleton } from '../lib/app.svelte.js';
  import { PRESSURE } from '../lib/status.js';
  import { clock, when, pp, pct } from '../lib/format.js';
  import Kbd from './Kbd.svelte';
  import PressurePill from './PressurePill.svelte';

  let open = $state(false);
  const u = $derived(app.data?.usage);
  const s = $derived(u?.snapshot);
  const est = $derived(s?.provenance === 'estimated' ? '≈' : '');
  const p = $derived(PRESSURE[s?.state] ?? PRESSURE.green);
  const five = $derived(u?.five_hour?.used_pct ?? s?.five_hour);
  // "+9": the delta in pp, as in "plan 38% (+9)".
  const sign = (e) => pp(e).replace(' pp', '');
</script>

{#if s}
  <button class="ind" onclick={() => (open = !open)} aria-expanded={open} aria-label="Usage pacing">
    <span style:color={p.color} aria-hidden="true">{p.icon}</span>
    <b>Week {est}{Math.round(s.used)}%</b>
    <span>agents {Math.round(s.governed)}%</span>
    <span class="muted">·</span>
    <span>plan {Math.round(s.ideal)}% ({sign(s.error)})</span>
    <span class="bar" style:width="40px"><i style:width="{Math.min(100, s.governed)}%" style:background={p.color}></i><em style:left="{Math.min(100, s.ideal)}%"></em></span>
    <span class="muted">·</span>
    <b>5h {pct(five)}</b>
    <span class="bar" style:width="24px"><i style:width="{Math.min(100, five ?? 0)}%"></i></span>
    <span class="muted">·</span>
    <span class="muted">resets {clock(u.reset_at_ms, 0)}</span>
  </button>
  {#if open}
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
    <div class="backdrop" onclick={() => (open = false)}></div>
    <div class="pop" role="dialog" aria-label="Usage pacing">
      <div class="row"><b>Usage pacing</b><PressurePill state={s.state} shadow={u.shadow} /></div>
      <div class="row"><span>Week {est}{Math.round(s.used)}%</span><span>agents {Math.round(s.governed)}% · plan {Math.round(s.ideal)}% ({sign(s.error)})</span></div>
      <span class="bar wide"><i style:width="{Math.min(100, s.governed)}%" style:background={p.color}></i><em style:left="{Math.min(100, s.ideal)}%"></em></span>
      <div class="muted">you {Math.round(s.interactive_used)}% · outside app {Math.round(s.unattributed_so_far)}% · projected {Math.round(s.projected)}% at the reset</div>
      <div class="row"><span>5h window</span><span>{five == null ? 'no reading yet' : `${Math.round(five)}% used`}</span></div>
      <span class="bar wide"><i style:width="{Math.min(100, five ?? 0)}%"></i></span>
      <div class="muted">
        {u.five_hour ? `resets ${when(u.five_hour.resets_at * 1000)}` : ''} · guard at 85%{s.five_hour_guard ? ' · guard on' : ''}
      </div>
      <div class="muted">live {s.live_autonomous}/{s.live_cap} · queued {s.queued} · {s.provenance === 'observed' ? 'observed' : 'estimated (low confidence)'}</div>
      <button class="btn ghost" onclick={() => ((open = false), openSingleton('planner'))}>open Week planner <Kbd keys="⌘+; g" /></button>
    </div>
  {/if}
{:else}
  <span class="muted">{app.data?.core?.ok === false ? 'no core: pacing unavailable' : 'pacing: waiting for the core'}</span>
{/if}

<style>
  .ind {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    background: none;
    border: 0;
    padding: 0 var(--space-sm);
    height: 24px;
    border-radius: var(--radius-sm);
    white-space: nowrap;
  }
  .ind:hover {
    background: var(--color-surface-raised);
  }
  .bar em {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--color-text-primary);
  }
  .bar.wide {
    width: 100%;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 40;
  }
  .pop {
    position: absolute;
    top: calc(var(--size-titlebar) - 2px);
    left: 50%;
    transform: translateX(-50%);
    z-index: 41;
    width: 320px;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-md);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
    font: var(--text-label);
  }
  .row {
    display: flex;
    justify-content: space-between;
    gap: var(--space-sm);
  }
</style>
