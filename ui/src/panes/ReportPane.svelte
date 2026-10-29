<script>
  // Weekly report (FR-818, Penpot 04 Weekly report, J8 · 4): used at reset,
  // plan vs actual per allocation, use by project / feature / role / model in
  // pp of the week (≈ API $ muted, D7), time in each state, governor actions,
  // model windows and the past reports. The current week reads "so far".
  // Keys: 1–4 breakdown, R reports list, [ ] previous / next report,
  // e copy as markdown, ⏎ plan next week, esc back to the Week planner.
  import { onMount, untrack } from 'svelte';
  import Kbd from '../components/Kbd.svelte';
  import PressurePill from '../components/PressurePill.svelte';
  import { app, openSingleton, saveLayout, toast } from '../lib/app.svelte.js';
  import { dayHm, dayTime } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { onTopics } from '../lib/now.svelte.js';
  import { ACTION_LABEL, API_TIP, delta, kpis, minutes, noData, reportMarkdown, stateBar, TABS, usd, weekNo } from '../lib/pacing.js';
  import { PRESSURE } from '../lib/status.js';

  let { paneId } = $props();
  const spec = $derived(app.panes[paneId]);
  const id = $derived(spec?.reportId ?? 'current');

  let r = $state(null);
  let list = $state([]);
  let err = $state(null);
  let tab = $state('model');
  let showList = $state(false);
  let root = $state(null);

  const week = $derived(r ? weekNo(Date.parse(r.week_start)) : null);
  const rows = $derived(r ? (r[TABS.find((t) => t.id === tab).rows] ?? []) : []);
  const maxPp = $derived(Math.max(1, ...rows.map((x) => x.pp)));
  const bar = $derived(r ? stateBar(r.states) : []);
  const estimated = $derived(r?.provenance === 'estimated');
  const idx = $derived(list.findIndex((x) => x.id === id));

  async function load() {
    try {
      const [rep, l] = await Promise.all([invoke('report_view', { id }), invoke('reports_list')]);
      r = rep;
      list = l.reports ?? [];
      err = null;
      const t = `Week ${weekNo(Date.parse(rep.week_start))} report`;
      if (app.panes[paneId] && app.panes[paneId].title !== t) app.panes[paneId] = { ...app.panes[paneId], title: t };
    } catch (e) {
      err = errText(e);
    }
  }

  function go(reportId) {
    if (!reportId || reportId === id) return;
    app.panes[paneId] = { ...app.panes[paneId], reportId };
    saveLayout();
    showList = false;
  }

  async function copyMarkdown() {
    if (!r) return;
    try {
      await navigator.clipboard?.writeText(reportMarkdown(r));
      toast(`Copied week ${week}'s report as Markdown`);
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }

  function onKey(e) {
    if (e.metaKey || e.ctrlKey || e.altKey || e.target.closest('input, textarea, select')) return;
    const k = e.key;
    const n = Number(k);
    if (n >= 1 && n <= 4) tab = TABS[n - 1].id;
    else if (k === 'R') showList = !showList;
    else if (k === '[') go(list[idx + 1]?.id);
    else if (k === ']') go(list[idx - 1]?.id);
    else if (k === 'e') copyMarkdown();
    else if (k === 'Enter' && !e.target.closest('button')) app.overlay = { kind: 'plan' };
    else if (k === 'Escape') showList ? (showList = false) : openSingleton('planner');
    else return;
    e.preventDefault();
  }

  const stateLabel = (s) => PRESSURE[s]?.label ?? s;

  // Opening another report in this pane (Inbox digest, palette) reloads it.
  $effect(() => {
    id;
    untrack(load);
  });

  // The current week moves on: refetched when the core says the report changed.
  onMount(() => onTopics(['report'], () => id === 'current' && load(), { now: false }));
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="report" tabindex="0" data-pane-focus bind:this={root} onkeydown={onKey} role="region" aria-label="Weekly report">
  {#if err}
    <p class="pad muted">Could not build the report: {err}</p>
  {:else if !r}
    <p class="pad muted">Loading the report…</p>
  {:else}
    <header class="head">
      <h2>Week {week}</h2>
      <span class="secondary">{r.in_progress ? `so far · ${dayTime(Date.parse(r.week_start))} → now` : `${dayTime(Date.parse(r.week_start))} → ${dayTime(Date.parse(r.reset_at))} · ${Math.round(r.totals.weekly_pct)} % at reset`}</span>
      {#if estimated}<span class="tag est">≈ estimated</span>{/if}
      <span class="grow"></span>
      <button class="btn ghost" onclick={copyMarkdown}>Copy as markdown <Kbd keys="e" /></button>
      <button class="btn primary" onclick={() => (app.overlay = { kind: 'plan' })}>Plan week {weekNo(Date.parse(r.reset_at))} <Kbd keys="⏎" /></button>
    </header>
    {#if estimated}<p class="warnbox">▲ Estimated: fewer than 20 usage samples this week; pp per row are approximate.</p>{/if}

    {#if noData(r)}
      <p class="empty">No usage was recorded this week. The app was not running, or no agents ran.</p>
    {:else}
      <div class="kpis">
        {#each kpis(r) as k (k.id)}
          <section class="kpi"><span class="muted">{k.label}</span><b>{k.value}</b><span class="muted note">{k.note}</span></section>
        {/each}
      </div>
    {/if}

    <div class="grid">
      <div class="col">
        <section class="card">
          <div class="ch"><b>Plan vs actual</b><span class="muted">per allocation · pp of the week{r.in_progress ? ' · so far' : ''}</span></div>
          <table>
            <thead><tr><th>Allocation</th><th>Weight</th><th>Planned</th><th>Used</th><th>Delta</th><th>Queued p50 / max</th></tr></thead>
            <tbody>
              {#each r.allocations as a (a.id)}
                {@const d = delta(a.delta_pp)}
                <tr>
                  <td>{a.target || a.id}</td><td>{a.weight}</td>
                  <td>{Math.round(a.planned_pp)} pp{r.in_progress ? ' so far' : ''}</td>
                  <td>{Math.round(a.used_pp)} pp</td>
                  <td class:over={d.glyph === '▲'}>{d.glyph} {d.text}</td>
                  <td>{minutes(a.queued_p50_min)} / {minutes(a.queued_max_min)}</td>
                </tr>
              {/each}
              <tr><td>reserve (you)</td><td>—</td><td>{Math.round(r.totals.reserve_pct)} pp</td><td>{Math.round(r.totals.interactive_pp)} pp</td><td>{delta(r.totals.interactive_pp - r.totals.reserve_pct).text}</td><td>—</td></tr>
              <tr><td>outside the app</td><td>—</td><td>—</td><td>{Math.round(r.totals.outside_pp)} pp</td><td>—</td><td>—</td></tr>
            </tbody>
          </table>
          {#if r.in_progress}<p class="muted note">Totals say "so far"; suggestions for the next plan come at the reset.</p>{/if}
        </section>

        <section class="card">
          <div class="ch"><b>Breakdown</b><span class="muted">by {tab} · pp of the week</span></div>
          <div class="tabs" role="tablist">
            {#each TABS as t, i (t.id)}<button class="tab" class:on={tab === t.id} role="tab" aria-selected={tab === t.id} onclick={() => (tab = t.id)}><Kbd keys={String(i + 1)} /> {t.label}</button>{/each}
          </div>
          <table class="rows">
            <thead><tr><th>{TABS.find((t) => t.id === tab).label}</th><th>Used</th><th class="share">Share</th><th class="usd" title={API_TIP}>≈ API $</th></tr></thead>
            <tbody>
              {#each rows as x (x.key)}
                <tr>
                  <td>{x.key === 'outside' ? 'outside the app' : x.label}</td>
                  <td>{estimated ? '≈ ' : ''}{Math.round(x.pp)} pp</td>
                  <td class="share"><span class="sbar"><i style:width="{(x.pp / maxPp) * 100}%"></i></span></td>
                  <td class="usd" title={API_TIP}>{usd(x)}</td>
                </tr>
              {:else}
                <tr><td colspan="4" class="muted">Nothing used this week.</td></tr>
              {/each}
            </tbody>
          </table>
          <p class="muted note">≈ API $: {API_TIP}</p>
        </section>
      </div>

      <div class="col side">
        <section class="card">
          <div class="ch"><b>Time in each state</b><span class="muted">{Math.round(r.states.reduce((a, s) => a + s.minutes, 0) / 60)} h</span></div>
          <div class="stbar" role="img" aria-label="Time in each state">
            {#each bar as s (s.id)}<i style:flex={s.share} style:background={PRESSURE[s.id]?.color}></i>{/each}
          </div>
          <div class="legend">{#each bar as s (s.id)}<span><span style:color={PRESSURE[s.id]?.color}>{PRESSURE[s.id]?.icon}</span> {stateLabel(s.id)} {s.hours} h</span>{/each}</div>
        </section>

        <section class="card">
          <div class="ch"><b>Governor actions</b><span class="muted">{r.actions.reduce((a, x) => a + x.count, 0)} · {r.actions.reduce((a, x) => a + x.undone, 0)} undone</span></div>
          {#each r.actions as a (a.kind)}
            <div class="kv"><span>{ACTION_LABEL[a.kind] ?? a.kind.replaceAll('_', ' ')}</span><span>{a.count}</span><span class="muted">{a.undone ? `${a.undone} undone` : '—'}</span></div>
          {:else}<p class="muted">No governor action.</p>{/each}
          {#if r.alerts.length || r.limit_hits.length}
            <p class="muted note">Alerts and limit hits: {[...r.limit_hits.map((h) => `${h.window} limit hit ${dayHm(Date.parse(h.ts))}`), ...r.alerts.map((a) => `${a.text} (${dayHm(Date.parse(a.ts))})`)].join(' · ')}</p>
          {/if}
        </section>

        {#if r.model_windows.length}
          <section class="card">
            <div class="ch"><b>Model windows</b><span class="muted">peak this week</span></div>
            {#each r.model_windows as w (w.name)}
              {@const st = w.peak_pct >= 100 ? 'exhausted' : w.peak_pct >= 95 ? 'red' : w.peak_pct >= 85 ? 'amber' : 'green'}
              <div class="kv"><span>{w.family} week {Math.round(w.peak_pct)} %</span><PressurePill state={st} /></div>
            {/each}
          </section>
        {/if}

        <section class="card" class:open={showList}>
          <div class="ch"><b>Past reports</b><span class="grow"></span><span class="muted"><Kbd keys="R" /> list · <Kbd keys="[" /><Kbd keys="]" /> prev/next</span></div>
          {#each showList ? list : list.slice(0, 4) as x (x.id)}
            <button class="past" class:on={x.id === id} onclick={() => go(x.id)}>
              <span>Week {weekNo(Date.parse(x.week_start))}{x.in_progress ? ' · so far' : ''}&nbsp; {x.final_weekly_pct == null ? 'no data' : `${Math.round(x.final_weekly_pct)} %`}</span>
              <span class="muted">{x.in_progress ? 'now' : dayTime(Date.parse(x.reset_at)).replace(/ \d\d:\d\d$/, '')}</span>
            </button>
          {/each}
        </section>
      </div>
    </div>

    {#if !r.in_progress && r.suggestions?.length}
      <section class="card sugg">
        <div class="ch"><b>What to change for week {weekNo(Date.parse(r.reset_at))}</b><span class="muted">suggestions, nothing applied</span></div>
        <ul>{#each r.suggestions as t (t)}<li>{t}</li>{/each}</ul>
      </section>
    {/if}
  {/if}
</div>

<style>

  .sugg {
    margin-top: var(--space-md);
  }
  .sugg ul {
    margin: 0;
    padding-left: var(--space-lg);
    font: var(--text-body);
  }
  .report {
    flex: 1;
    min-height: 0;
    overflow: auto;
    outline: none;
    padding: var(--space-md);
    font: var(--text-label);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    flex-wrap: wrap;
    margin-bottom: var(--space-md);
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  .tag {
    padding: 0 var(--space-xs);
    border: var(--border-width-default) solid var(--color-pacing-amber);
    border-radius: var(--radius-sm);
  }
  .warnbox {
    margin: 0 0 var(--space-md);
    padding: var(--space-sm) var(--space-md);
    border: var(--border-width-default) solid var(--color-pacing-amber);
    border-radius: var(--radius-sm);
  }
  .empty {
    padding: var(--space-md);
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
    margin: 0 0 var(--space-md);
  }
  .kpis {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: var(--space-md);
    margin-bottom: var(--space-md);
  }
  .kpi,
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-md) var(--space-lg);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .kpi b {
    font: var(--text-display);
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 2fr) minmax(280px, 1fr);
    gap: var(--space-md);
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    min-width: 0;
  }
  .ch {
    display: flex;
    align-items: baseline;
    gap: var(--space-sm);
  }
  .ch b {
    font: var(--text-title);
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th {
    text-align: left;
    color: var(--color-text-muted);
    font: var(--text-caption);
    padding: var(--space-xs) var(--space-sm) var(--space-xs) 0;
  }
  td {
    padding: var(--space-xs) var(--space-sm) var(--space-xs) 0;
    white-space: nowrap;
  }
  .over {
    color: var(--color-pacing-amber);
  }
  .share {
    width: 40%;
  }
  .sbar {
    display: block;
    height: 6px;
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }
  .sbar i {
    display: block;
    height: 100%;
    background: var(--color-accent-default);
  }
  .usd {
    text-align: right;
    color: var(--color-text-muted);
  }
  .note {
    margin: 0;
    font: var(--text-caption);
  }
  .tabs {
    display: flex;
    gap: var(--space-xs);
  }
  .tab {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    padding: var(--space-xxs) var(--space-sm);
    background: none;
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
    font: var(--text-label);
    cursor: pointer;
  }
  .tab.on {
    border-color: var(--color-focus);
    background: var(--color-surface-selected);
    color: var(--color-text-primary);
  }
  .stbar {
    display: flex;
    height: 10px;
    gap: 2px;
    border-radius: var(--radius-sm);
    overflow: hidden;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-md);
    font: var(--text-caption);
  }
  .kv {
    display: grid;
    grid-template-columns: 1fr auto auto;
    gap: var(--space-md);
    align-items: center;
  }
  .past {
    display: flex;
    justify-content: space-between;
    padding: var(--space-xs) var(--space-sm);
    background: none;
    border: 0;
    border-radius: var(--radius-sm);
    color: var(--color-text-primary);
    font: var(--text-label);
    text-align: left;
    cursor: pointer;
  }
  .past.on {
    background: var(--color-surface-selected);
  }
  .pad {
    padding: var(--space-xl);
  }
</style>
