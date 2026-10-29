<script>
  // Two-window indicator (J8 step 2, Penpot 02 "Pacing indicator"): "Week" is the
  // total the plan reports, "agents" the governed share; the plan and the pp delta
  // refer to the agents. Week bars: fill = total W stacked outside app / you /
  // agents (spec fix-week-and-daily-limit), 2 px tick = the total plan.
  import { app, openSingleton } from '../lib/app.svelte.js';
  import { clock, pct, signed, when } from '../lib/format.js';
  import { hotWindows, weekSegments } from '../lib/pacing.js';
  import { PRESSURE } from '../lib/status.js';
  import Kbd from './Kbd.svelte';
  import PressurePill from './PressurePill.svelte';

  let open = $state(false);
  const u = $derived(app.data?.usage);
  const s = $derived(u?.snapshot);
  const est = $derived(s?.provenance === 'estimated' ? '≈' : '');
  const p = $derived(PRESSURE[s?.state] ?? PRESSURE.green);
  const five = $derived(u?.five_hour?.used_pct ?? s?.five_hour);
  // FR-815: a model window shows here only when it holds work (Red or exhausted).
  const hot = $derived(hotWindows(s?.model_windows));
  const seg = $derived(s && weekSegments(s));
  const weekLabel = $derived(
    s &&
      `Week ${est}${Math.round(s.used)}%: agents ${Math.round(s.governed)}%, you ${Math.round(s.interactive_used)}%, outside app ${Math.round(s.unattributed_so_far)}%; plan ${Math.round(s.plan)}%`,
  );
</script>

{#snippet weekBar(cls)}
  <span class="bar {cls}" role="img" aria-label={weekLabel} title={weekLabel}>
    <span class="fill">
      {#each ['outside', 'you', 'agents'] as k (k)}
        {#if seg[k] > 0}<span data-seg={k} class={k} style:width="{seg[k]}%" style:background={k === 'agents' ? p.color : undefined}></span>{/if}
      {/each}
    </span>
    <em style:left="min({seg.tick}%, 100% - 2px)"></em>
  </span>
{/snippet}

{#if s}
  <button class="ind" onclick={() => (open = !open)} aria-expanded={open} aria-label="Usage pacing: {weekLabel}">
    <span style:color={p.color} aria-hidden="true">{p.icon}</span>
    <b>Week {est}{Math.round(s.used)}%</b>
    <span>agents {Math.round(s.governed)}%</span>
    <span class="muted">·</span>
    <span>plan {Math.round(s.ideal)}% ({signed(s.error)})</span>
    {@render weekBar('strip')}
    <span class="muted">·</span>
    <b>5h {pct(five)}</b>
    <span class="bar" style:width="24px"><i style:width="{Math.min(100, five ?? 0)}%"></i></span>
    <span class="muted">·</span>
    <span class="muted">resets {clock(u.reset_at_ms, 0)}</span>
    {#each hot as w (w.name)}<span class="hot">· {w.family} {Math.round(w.used_pct)} %</span>{/each}
  </button>
  {#if open}
    <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
    <div class="backdrop" onclick={() => (open = false)}></div>
    <div class="pop" role="dialog" aria-label="Usage pacing">
      <div class="row"><b>Usage pacing</b><PressurePill state={s.state} shadow={u.shadow} /></div>
      <div class="row"><span>Week {est}{Math.round(s.used)}%</span><span>agents {Math.round(s.governed)}% · plan {Math.round(s.ideal)}% ({signed(s.error)})</span></div>
      {@render weekBar('wide')}
      <div class="muted">
        <span class="sw" style:background={p.color}></span>agents {Math.round(s.governed)}% · <span class="sw you"></span>you {Math.round(s.interactive_used)}% · <span class="sw outside"></span>outside app {Math.round(s.unattributed_so_far)}% · projected {Math.round(s.projected)}% at the reset
      </div>
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

  .hot {
    color: var(--color-pacing-red);
  }
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
  .bar.strip {
    width: 40px;
  }
  .fill {
    position: absolute;
    inset: 0;
    display: flex;
  }
  .fill > span {
    flex: none;
  }
  .outside {
    background: var(--color-text-disabled);
  }
  .you {
    background: var(--color-text-secondary);
  }
  .sw {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: var(--space-xs);
    border-radius: var(--radius-sm);
    vertical-align: middle;
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
