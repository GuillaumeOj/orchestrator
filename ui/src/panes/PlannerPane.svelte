<script>
  // Week planner (§9.3, FR-802–804, FR-809, Penpot 04 Week planner): read-only
  // forecast from the Governor. Ideal vs actual vs forecast for the week and the
  // 5h window, allocations, next-action preview and the governor log with undo.
  // The actual line and the log cover the whole plan week across restarts.
  // Shadow mode (FR-812) for the first week, then the user may enforce.
  // Keys: S sprint mode, P pause all, u undo the selected log entry, a/r answer
  // the ask, e edit the plan, R the weekly report, O run the next queued turn
  // off-hours (or back in working hours). P3: model windows, off-hours bands.
  // the downgrade entries (FR-815, 816, 818, 819).
  import { onMount } from 'svelte';
  import Kbd from '../components/Kbd.svelte';
  import PressurePill from '../components/PressurePill.svelte';
  import { app, confirm, governor, openReport, queueOffHours, toast } from '../lib/app.svelte.js';
  import { clock, dayHm, dayTime, duration, pct, pp, when } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { downgradeText, nextOffHours, OFF_DISABLED, offHoursAllowed, windowLabel, windowQueued } from '../lib/pacing.js';
  import { PRESSURE, queueReason } from '../lib/status.js';

  let v = $state(null);
  let err = $state(null);
  let logSel = $state(0);
  let w = $state(720);

  async function load() {
    try {
      v = await invoke('planner_view');
      err = null;
    } catch (e) {
      err = errText(e);
    }
  }
  onMount(() => {
    load();
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  });

  const s = $derived(v?.snapshot);
  const windows = $derived(s?.model_windows ?? []);
  const off = $derived(v?.off_hours ?? null);
  const offStart = $derived(off ? nextOffHours(off, v.now_ms) : null);
  /** One "off-hours" label per day: the first band starting that day. */
  // No allowance: nights are not off-hours for the Governor, so nothing is hatched.
  const offBands = $derived(
    (offHoursAllowed(off) ? (off?.windows ?? []) : []).map((w, i, all) => {
      const a = Date.parse(w.start);
      const day = new Date(a).toDateString();
      return { a, b: Date.parse(w.end), label: all.findIndex((x) => new Date(Date.parse(x.start)).toDateString() === day) === i };
    }),
  );
  const head0 = $derived(v?.queue?.[0] ?? null);
  const head0Off = $derived(head0?.reason?.kind === 'scheduled_off_hours');

  /** O: the next queued turn to off-hours, or back (queue_off_hours). */
  function toggleOffHours() {
    if (!head0) return toast('Nothing queued');
    if (!offHoursAllowed(off)) return toast(OFF_DISABLED);
    queueOffHours({ entry: head0.id, off: !head0Off }, head0.agent, load);
  }
  const p = $derived(PRESSURE[s?.state] ?? PRESSURE.green);
  // Chart geometry
  const H = 220;
  const pad = { l: 36, r: 8, t: 10, b: 22 };
  const x = (t) => pad.l + ((t - v.week_start_ms) / (v.reset_at_ms - v.week_start_ms)) * (w - pad.l - pad.r);
  const y = (val) => pad.t + (1 - Math.min(Math.max(val, 0), 110) / 100) * (H - pad.t - pad.b);
  const path = (pts) => pts.map((q, i) => `${i ? 'L' : 'M'}${x(q[0]).toFixed(1)},${y(q[1]).toFixed(1)}`).join('');
  const ideal = $derived(v ? path(v.curve.map((c) => [c.t_ms, c.ideal])) : '');
  const forecast = $derived(v ? path(v.curve.filter((c) => c.forecast != null).map((c) => [c.t_ms, c.forecast])) : '');
  const withGov = $derived(v ? path(v.curve.filter((c) => c.with_governor != null).map((c) => [c.t_ms, c.with_governor])) : '');
  const actualPts = $derived(v ? [...v.actual.map((h) => [h.t_ms, h.governed]), [v.now_ms, s.governed]] : []);
  const actual = $derived(v ? path(actualPts) : '');
  const actualFill = $derived(v && actualPts.length ? `${path(actualPts)}L${x(actualPts.at(-1)[0])},${y(0)}L${x(actualPts[0][0])},${y(0)}Z` : '');
  const days = $derived(v ? Array.from({ length: 7 }, (_, i) => v.week_start_ms + i * 86400000) : []);
  const emptyAt = $derived(s?.eta ? Date.parse(s.eta) : null);
  const reserveTop = $derived(v ? 100 - v.reserve_pct - (s?.unattributed_forecast ?? 0) : 90);
  const WEEK_MS = 7 * 86400000;
  // Day n of the shadow week (FR-812), counted from the first start.
  const shadowDay = $derived(v?.first_run_ms ? Math.floor((v.now_ms - v.first_run_ms) / 86400000) + 1 : null);
  const shadowWeekDone = $derived(v?.first_run_ms != null && v.now_ms - v.first_run_ms >= WEEK_MS);

  function setEnforce(on) {
    const text = on ? 'Enforce the Governor?' : 'Back to shadow mode?';
    const detail = on
      ? 'From now on it holds autonomous turns over the live cap, queues work and applies its asks (FR-805–811). Your own typing is never blocked. Undo: back to shadow mode from here.'
      : 'It keeps logging what it would do, and holds nothing.';
    confirm(text, detail, async () => {
      try {
        await invoke('governor_enforce', { enforce: on });
        toast(on ? 'Governor enforced' : 'Shadow mode on');
        load();
      } catch (e) {
        toast(errText(e), { kind: 'error' });
      }
    }, on ? 'Enforce' : 'Shadow mode');
  }

  function weekNo(ms) {
    const d = new Date(ms);
    const t = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()));
    const day = t.getUTCDay() || 7;
    t.setUTCDate(t.getUTCDate() + 4 - day);
    return Math.ceil(((t.getTime() - Date.UTC(t.getUTCFullYear(), 0, 1)) / 86400000 + 1) / 7);
  }

  // A log entry is { key, action: GovernorAction, … }; the kind is GovernorAction.action.
  function logIcon(entry) {
    const a = entry.action.action;
    const k = a.kind;
    if (k === 'state_change') return PRESSURE[a.to]?.icon ?? '●';
    if (k === 'downgrade') return '↓';
    if (k === 'restore_model') return '↑';
    return { five_hour_guard: '⧗', pause_all: '‖', resume_all: '▶', limit_reached: '⊘', alert: '!', defer_after_reset: '▲' }[k] ?? '·';
  }

  function describe(entry) {
    const k = entry.action.action;
    switch (k.kind) {
      case 'state_change':
        return `${k.from} → ${k.to}`;
      case 'live_cap':
        return `live cap ${k.cap} (${k.live} live)`;
      case 'queue':
        return `queued ${k.agent}`;
      case 'release':
        return `released ${k.agent}`;
      case 'run_now':
        return `run now: ${k.agent}`;
      case 'defer_after_reset':
        return `defer ${k.allocation} until the reset`;
      case 'downgrade':
      case 'restore_model':
        return downgradeText(k);
      case 'sprint':
        return `sprint +${k.extra_pp} pp until ${clock(Date.parse(k.until))}`;
      default:
        return k.kind.replaceAll('_', ' ');
    }
  }

  async function undo(entry) {
    if (!entry?.can_undo) return toast('This action cannot be undone (the loop re-decides it)');
    await governor('undo', entry.action.id);
    load();
  }

  function sprint() {
    confirm('Sprint mode?', "Borrows 5 pp: today's target goes up and the remaining days go down. Undo from the log.", async () => {
      await governor('sprint', 5);
      load();
    }, 'Start sprint');
  }

  async function answerAsk(accept) {
    if (!v?.pending_ask) return;
    try {
      await invoke('inbox_act', { item: `ask-${v.pending_ask.id}`, action: accept ? 'approve' : 'reject', message: null });
      load();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }

  function onKey(e) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const k = e.key;
    const act = (fn) => (e.preventDefault(), fn());
    if (k === 'S') return act(sprint);
    if (k === 'P') return act(() => governor('pause_all').then(() => (toast('Paused all · Resume all from the palette'), load())));
    if (k === 'u') return act(() => undo(v?.log[logSel]));
    if (k === 'j' || k === 'ArrowDown') return act(() => (logSel = Math.min((v?.log.length ?? 1) - 1, logSel + 1)));
    if (k === 'k' || k === 'ArrowUp') return act(() => (logSel = Math.max(0, logSel - 1)));
    if (k === 'a' && v?.pending_ask) return act(() => answerAsk(true));
    if (k === 'r' && v?.pending_ask) return act(() => answerAsk(false));
    if (k === 'e') return act(() => (app.overlay = { kind: 'plan' }));
    if (k === 'R') return act(() => openReport('current'));
    if (k === 'O') return act(toggleOffHours);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="planner" tabindex="0" data-pane-focus onkeydown={onKey} role="region" aria-label="Week planner">
  {#if err}
    <p class="pad muted">Week planner unavailable: {err}</p>
  {:else if !v}
    <p class="pad muted">Loading the forecast…</p>
  {:else}
    <div class="head">
      <h2>Week {weekNo(v.week_start_ms)}</h2>
      <span class="secondary">{dayTime(v.week_start_ms)} → {dayTime(v.reset_at_ms)} · reset in {duration(v.reset_at_ms - v.now_ms)}</span>
      <PressurePill state={s.state} />
      <span class="grow"></span>
      <span class="muted">{s.provenance === 'observed' ? '● observed' : '○ estimated (low confidence)'}</span>
      {#if v.shadow}
        <span class="toggle on" title="FR-812: decisions are logged, not enforced">Shadow mode{shadowDay && !shadowWeekDone ? ` · day ${shadowDay} of 7` : ''}</span>
        {#if shadowWeekDone}<button class="btn" onclick={() => setEnforce(true)}>Shadow week done · Enforce…</button>{/if}
      {:else}
        <span class="toggle" title="The Governor holds and queues work">Enforced</span>
        <button class="btn ghost" onclick={() => setEnforce(false)}>Shadow mode…</button>
      {/if}
      <button class="btn" onclick={sprint}>Sprint mode <Kbd keys="S" /></button>
      <button class="btn danger" onclick={() => governor('pause_all').then(load)}>Pause all <Kbd keys="P" /></button>
      <button class="btn ghost" onclick={() => openReport('current')}>Weekly report <Kbd keys="R" /></button>
      <button class="btn ghost" onclick={() => (app.overlay = { kind: 'plan' })}>{v.plan_set ? 'Edit plan' : 'Set this week’s plan'} <Kbd keys="e" /></button>
    </div>

    <div class="grid">
      <section class="card week" bind:clientWidth={w}>
        <div class="ch">
          <b>Week</b><span class="muted">working hours shaded{offHoursAllowed(off) ? ' · off-hours hatched' : ''}</span>
          <span class="grow"></span>
          <span class="leg"><i class="l actual"></i>agents</span>
          <span class="leg"><i class="l ideal"></i>ideal</span>
          <span class="leg"><i class="l fc"></i>forecast</span>
          <span class="leg"><i class="l gov"></i>w/ governor</span>
        </div>
        <svg width={w - 2} height={H} role="img" aria-label="Agents' usage against the ideal curve">
          <defs><pattern id="offhatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><line x1="0" y1="0" x2="0" y2="6" class="hatch" /></pattern></defs>
          {#each v.working as [a, b] (a)}<rect x={x(a)} y={pad.t} width={Math.max(1, x(b) - x(a))} height={H - pad.t - pad.b} class="work" />{/each}
          {#each offBands as o (o.a)}
            <rect x={x(o.a)} y={pad.t} width={Math.max(1, x(Math.min(o.b, v.reset_at_ms)) - x(o.a))} height={H - pad.t - pad.b} class="off" />
            {#if o.label}<text x={x(o.a) + 4} y={H - pad.b - 6} class="ax">off-hours</text>{/if}
          {/each}
          {#if off?.queued && offStart}<text x={Math.min(x(offStart) + 4, w - 80)} y={y(30)} class="tag okq">☾ {off.queued} queued</text>{/if}
          <rect x={pad.l} y={y(100)} width={w - pad.l - pad.r} height={Math.max(0, y(reserveTop) - y(100))} class="reserve" />
          {#each [0, 25, 50, 75, 100] as g (g)}
            <line x1={pad.l} x2={w - pad.r} y1={y(g)} y2={y(g)} class="grid-l" /><text x={pad.l - 6} y={y(g) + 4} class="ax" text-anchor="end">{g}%</text>
          {/each}
          {#each days as d (d)}<text x={x(d) + 4} y={H - 6} class="ax">{new Date(d).toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric' })}</text>{/each}
          <path d={actualFill} class="fill" />
          <path d={ideal} class="ideal" />
          <path d={withGov} class="gov" />
          <path d={forecast} class="fc" />
          <path d={actual} class="actual" />
          <line x1={x(v.now_ms)} x2={x(v.now_ms)} y1={pad.t} y2={H - pad.b} class="now" />
          <text x={x(v.now_ms) + 6} y={y(s.governed) - 8} class="tag">now {clock(v.now_ms)} · agents {Math.round(s.governed)}% (plan {Math.round(s.ideal)}%, {pp(s.error)}) · total {Math.round(s.used)}%</text>
          {#if emptyAt && emptyAt < v.reset_at_ms}<text x={Math.min(x(emptyAt), w - 180)} y={y(100) - 2} class="tag warn">⚠ empty {clock(emptyAt)} at this rate</text>{/if}
          <text x={w - pad.r - 4} y={y(reserveTop) - 4} class="ax" text-anchor="end">reserve {v.reserve_pct}% + outside app {Math.round(s.unattributed_forecast)}%</text>
        </svg>
      </section>

      <section class="card five">
        <div class="ch"><b>5h window</b><span class="muted">guard at 85%</span></div>
        <div class="big">{s.five_hour == null ? '—' : pct(s.five_hour)}<span class="muted">{s.five_hour == null ? ' no reading' : ' used'}{v.five_hour_resets_at_ms ? ` · resets ${when(v.five_hour_resets_at_ms)}` : ''}</span></div>
        <span class="bar wide"><i style:width="{Math.min(100, s.five_hour ?? 0)}%"></i><em style:left="85%"></em></span>
        <p>
          {#if s.five_hour_projected != null}Projected {pct(s.five_hour_projected)} at the reset{s.five_hour_guard ? ' (> 85%) → 5h guard on: live cap 1 until the reset. Interactive and critical work stay admitted.' : '.'}{:else}No 5h reading yet: it arrives with the first response.{/if}
        </p>
        {#if windows.length}
          <div class="mw">
            <span class="muted">Model windows</span>
            {#each windows as mw (mw.name)}
              <div class="mwrow"><b>{windowLabel(mw)} {Math.round(mw.used_pct)} %</b><PressurePill state={mw.state} /><span class="muted">resets {dayHm(Date.parse(mw.resets_at))}</span></div>
              {#if mw.state === 'red' || mw.state === 'exhausted'}<p class="muted">{windowQueued(mw)}{mw.state === 'red' ? '. Roles with a downgrade switch at their next task.' : ''}</p>{/if}
            {/each}
          </div>
        {/if}
        <dl>
          <dt>Interactive</dt><dd>{s.interactive_used.toFixed(1)} pp of the week</dd>
          <dt>Live</dt><dd>{s.live_autonomous}/{s.live_cap} autonomous</dd>
          <dt>Burn</dt><dd>{s.burn_pct_per_hour.toFixed(2)} pp per working hour</dd>
          <dt>Calibration</dt><dd>confidence {Math.round(s.calibration.confidence * 100)}%</dd>
        </dl>
      </section>

      <section class="card alloc">
        <div class="ch"><b>Allocations</b><span class="muted">weights · tiers · used vs plan this week</span></div>
        <table>
          <thead><tr><th>Allocation</th><th>Weight</th><th>Tier</th><th>Used / plan</th><th></th><th>Status</th></tr></thead>
          <tbody>
            {#each v.allocations as al (al.id)}
              {@const over = al.used_pct - al.plan_pct}
              <tr>
                <td>{al.target || al.id}</td>
                <td><span class="bar" style:width="48px"><i style:width="{Math.min(100, al.weight)}%"></i></span> {al.weight}</td>
                <td>{al.tier}</td>
                <td>{al.used_pct.toFixed(0)}% / {al.plan_pct.toFixed(0)}%</td>
                <td><span class="bar" style:width="60px"><i style:width="{Math.min(100, al.used_pct)}%" style:background={over > 2 ? 'var(--color-pacing-red)' : 'var(--color-pacing-green)'}></i><em style:left="{Math.min(100, al.plan_pct)}%"></em></span></td>
                <td>{al.queued ? `queued (${al.queued})` : over > 2 ? `over +${over.toFixed(0)} pp` : over < -2 ? `under −${Math.abs(over).toFixed(0)} pp` : 'on plan'}{al.live ? ` · ${al.live} live` : ''}</td>
              </tr>
            {/each}
            <tr><td>reserve (interactive)</td><td>{v.reserve_pct}</td><td>—</td><td>{s.interactive_used.toFixed(0)}%</td><td></td><td>{s.reserve_left.toFixed(0)} pp left</td></tr>
            <tr><td>outside the app</td><td>—</td><td>—</td><td>{s.unattributed_so_far.toFixed(0)}%</td><td></td><td class="muted">claude.ai, other terminals</td></tr>
          </tbody>
        </table>
        <p class="muted">Agents {Math.round(s.governed)}% vs plan {Math.round(s.ideal)}% ({pp(s.error)}) · total {Math.round(s.used)}% with you ({s.interactive_used.toFixed(0)}%) and outside the app ({s.unattributed_so_far.toFixed(0)}%). Weights are relative; unused weight is redistributed; aging +1 tier per 2 h.</p>
      </section>

      <section class="card next">
        <div class="ch"><b>Next action</b><span class="muted">preview</span></div>
        {#if v.queue.length}
          <p>Next release: <b>{v.queue[0].agent}</b> ({v.queue[0].tier}) · {queueReason(v.queue[0])}{v.queue[0].estimated_start ? ` · ≈ ${clock(Date.parse(v.queue[0].estimated_start))}` : ''}</p>
          <div class="row"><button class="btn ghost" disabled={!offHoursAllowed(off)} title={offHoursAllowed(off) ? '' : OFF_DISABLED} onclick={() => toggleOffHours()}>{head0Off ? 'Run in working hours' : 'Run off-hours'} <Kbd keys="O" /></button></div>
        {:else}
          <p class="muted">Nothing queued.</p>
        {/if}
        <p class="muted">State {s.state === v.next.target_state ? 'stays' : `→ ${v.next.target_state}`} · earliest change {clock(Date.parse(v.next.earliest_state_change))}</p>
        {#if v.pending_ask}
          <div class="ask">
            <p><span style:color="var(--color-pacing-amber)">▲</span> Ask: {v.pending_ask.reason}</p>
            <p class="muted">{v.pending_ask.expected_effect} · default: {v.pending_ask.ask.default} at {clock(Date.parse(v.pending_ask.ask.deadline))}</p>
            <div class="row">
              <button class="btn primary" onclick={() => answerAsk(true)}>Apply <Kbd keys="a" /></button>
              <button class="btn" onclick={() => answerAsk(false)}>Keep running <Kbd keys="r" /></button>
            </div>
          </div>
        {/if}
      </section>

      <section class="card log">
        <div class="ch"><b>Governor log</b><span class="muted">reason + undo for every action</span></div>
        <ul>
          {#each v.log as entry, i (entry.key)}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
            <li class:sel={i === logSel} onclick={() => (logSel = i)}>
              <span class="muted when" title={entry.past ? 'an earlier start of the app' : ''}>{entry.past ? dayTime(Date.parse(entry.action.ts)) : clock(Date.parse(entry.action.ts))}</span>
              <span class="ic" style:color={p.color}>{logIcon(entry)}</span>
              <span class="what"><b>{describe(entry)}</b> · {entry.action.reason}{entry.action.expected_effect ? ` → ${entry.action.expected_effect}` : ''}{entry.action.mode === 'shadow' ? ' · shadow' : ''}</span>
              {#if entry.can_undo}<button class="btn ghost" onclick={() => undo(entry)}>Undo <Kbd keys="u" /></button>
              {:else if entry.action.undone_at}<span class="muted">undone</span>{/if}
            </li>
          {:else}
            <li class="muted">No governor action yet this week.</li>
          {/each}
        </ul>
      </section>
    </div>
  {/if}
</div>

<style>
  .planner {
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
  .toggle {
    padding: var(--space-xxs) var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-full);
  }
  .toggle.on {
    border-color: var(--color-accent-default);
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 2.4fr) minmax(260px, 1fr);
    gap: var(--space-md);
  }
  .log {
    grid-column: 1 / -1;
  }
  .ch {
    display: flex;
    align-items: baseline;
    gap: var(--space-sm);
    margin-bottom: var(--space-sm);
    font: var(--text-title);
  }
  .ch .muted {
    font: var(--text-label);
  }
  .leg {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    font: var(--text-label);
  }
  .l {
    display: inline-block;
    width: 14px;
    height: 0;
    border-top: 2px solid;
  }
  .l.actual {
    border-color: var(--color-pacing-red);
  }
  .l.ideal {
    border-color: var(--color-text-secondary);
    border-top-style: dashed;
  }
  .l.fc {
    border-color: var(--color-pacing-red);
    border-top-style: dashed;
  }
  .l.gov {
    border-color: var(--color-accent-default);
    border-top-style: dashed;
  }
  svg {
    display: block;
  }
  .work {
    fill: var(--color-surface-raised);
    opacity: 0.5;
  }
  .reserve {
    fill: var(--color-surface-selected);
    opacity: 0.5;
  }
  .grid-l {
    stroke: var(--color-border-subtle);
  }
  .ax {
    fill: var(--color-text-muted);
    font: var(--text-caption);
  }
  .tag {
    fill: var(--color-text-primary);
    font: var(--text-caption);
  }
  .tag.warn {
    fill: var(--color-pacing-red);
  }
  .tag.okq {
    fill: var(--color-status-done);
  }
  .off {
    fill: url(#offhatch);
  }
  .hatch {
    stroke: var(--color-border-subtle);
    stroke-width: 2;
  }
  .mw {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding-top: var(--space-sm);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
  .mw p {
    margin: 0;
    font: var(--text-caption);
  }
  .mwrow {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .ideal {
    fill: none;
    stroke: var(--color-text-secondary);
    stroke-width: 1.5;
    stroke-dasharray: 6 4;
  }
  .actual {
    fill: none;
    stroke: var(--color-pacing-red);
    stroke-width: 2;
  }
  .fill {
    fill: var(--color-pacing-red);
    opacity: 0.15;
  }
  .fc {
    fill: none;
    stroke: var(--color-pacing-red);
    stroke-width: 1.5;
    stroke-dasharray: 6 4;
  }
  .gov {
    fill: none;
    stroke: var(--color-accent-default);
    stroke-width: 1.5;
    stroke-dasharray: 6 4;
  }
  .now {
    stroke: var(--color-text-primary);
  }
  .big {
    font: var(--text-display);
  }
  .big .muted {
    font: var(--text-label);
  }
  .bar.wide {
    width: 100%;
  }
  .bar em {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--color-text-primary);
  }
  dl {
    display: grid;
    grid-template-columns: 100px 1fr;
    gap: var(--space-xs) var(--space-md);
  }
  dt {
    color: var(--color-text-muted);
  }
  dd {
    margin: 0;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th {
    text-align: left;
    font-weight: var(--font-weight-regular);
    color: var(--color-text-muted);
  }
  td,
  th {
    padding: var(--space-xs) var(--space-sm) var(--space-xs) 0;
    white-space: nowrap;
  }
  td:last-child {
    white-space: normal;
  }
  .alloc {
    overflow-x: auto;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-sm);
    border-radius: var(--radius-sm);
  }
  li.sel {
    background: var(--color-surface-selected);
  }
  .when {
    width: 72px;
    flex: none;
  }
  .what {
    flex: 1;
    min-width: 0;
  }
  .ask .row {
    display: flex;
    gap: var(--space-sm);
  }
  .pad {
    padding: var(--space-xl);
  }
  @media (max-width: 1400px) {
    .grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
