<script>
  // Mission control (FR-1302, P4.2 spec §3.2–3.4, notes p4-2, Penpot 04):
  // what runs, waits on what, is queued why and was done today, on one
  // screen. A render of `now` (§9.1 principle 10): it fetches nothing, and
  // its one timer is the 30 s relative-time clock. Rows and lanes come from
  // `lanes()` (lib/mission.js); this pane keeps the selection, the open
  // lanes and details, and the keys (notes §3.1).
  import { untrack } from 'svelte';
  import Kbd from '../components/Kbd.svelte';
  import QueueRow from '../components/mission/QueueRow.svelte';
  import ReadyDetails from '../components/mission/ReadyDetails.svelte';
  import ReadyLine from '../components/mission/ReadyLine.svelte';
  import ReadyRow from '../components/mission/ReadyRow.svelte';
  import RowDetails from '../components/mission/RowDetails.svelte';
  import RunRow from '../components/mission/RunRow.svelte';
  import SoloRow from '../components/mission/SoloRow.svelte';
  import OutcomeRow from '../components/OutcomeRow.svelte';
  import {
    agent,
    agentAction,
    app,
    cancelQueued,
    confirm,
    contextProjectId,
    focusCard,
    now30,
    openBoard,
    openEditor,
    openReport,
    openSession,
    openUrl,
    projectName,
    projects,
    runNow,
    select,
    setContext,
  } from '../lib/app.svelte.js';
  import { mb, startOfDay } from '../lib/format.js';
  import { isOpen } from '../lib/inbox.js';
  import { ageText, modeOf, rateLimitedUntil } from '../lib/intake.js';
  import { fixSource, hideItem, intakeUi, openSource, planWithOverwatchIn, refreshIntake, setUpIntake, startItem, undoHide, unhideItem } from '../lib/intake.svelte.js';
  import { isTypingTarget } from '../lib/keyboard.js';
  import { allowed, COUNT_KEYS, DONE_ROWS, decisionOf, defaultTarget, footerText, hiddenText, lanes, looseText, rowIndex, stops, target } from '../lib/mission.js';
  import { attention } from '../lib/now.svelte.js';
  import { LEVEL } from '../lib/status.js';

  let { paneId } = $props();

  let selKey = $state(null);
  let chip = $state(null); // the agent id of the selected member chip of a run row (null: the run)
  let collapsed = $state({});
  let idleOpen = $state(false);
  let showAll = $state(false);
  let details = $state({});
  let busy = $state(null); // the agent an action waits on
  let lastAt = 0; // the selection's last stop index (a row that leaves gives its place to the next)

  const now = $derived(Math.max(now30(), Date.now()));
  const ps = $derived(projects());
  // Rows follow the workspace's project context (design-notes §5.3): [id], or null in All projects.
  const ctx = $derived(contextProjectId());
  const filter = $derived(ctx ? [ctx] : null);
  // lanes() needs only the day: the 30 s clock does not rebuild it.
  const day = $derived(startOfDay(now));
  const model = $derived(lanes(app.now, filter, day, { projects: ps, showHidden: intakeUi.showHidden }));
  // Rows somewhere, by the core's counts (the filter may hide them all).
  const anyWork = $derived.by(() => {
    const c = app.now.counts?.global ?? {};
    return COUNT_KEYS.some((k) => k !== 'live' && c[k]) || Object.values(app.now.decisions ?? {}).some(isOpen);
  });
  const stopList = $derived(stops(model, { collapsed, idleOpen, showAll }));
  const index = $derived(rowIndex(model));
  const row = $derived(index.get(selKey) ?? null);
  const t = $derived(target(row, chip));
  const trec = $derived(t ? attention(t) : null);
  const dec = $derived(decisionOf(row, trec));
  const loaded = $derived(app.now.rev > 0);
  // Ready to start is not work in flight: it neither makes the board "busy" nor "moving" (P4.5 notes §1.1).
  const work = $derived(model.lanes.some((l) => l.id !== 'done' && l.id !== 'ready' && l.rows.length) || !!model.idle);
  const moving = $derived(model.lanes.some((l) => l.id !== 'needs_you' && l.id !== 'done' && l.id !== 'ready') || !!model.idle);
  const overwatch = $derived(app.now.overwatch?.status ?? 'off');
  const done = $derived(model.lanes.find((l) => l.id === 'done'));
  // The context hides every row while some exist elsewhere.
  const filteredOut = $derived(!!filter && !work && !done && anyWork && !model.lanes.some((l) => l.id === 'ready'));

  // The selection follows its row across lanes (keys are the run or agent);
  // a row that left gives its place to the one now there.
  $effect(() => {
    const list = stopList;
    if (selKey && list.includes(selKey)) {
      lastAt = list.indexOf(selKey);
      return;
    }
    // Only Mission control's own selection is repaired: one made in the sidebar,
    // the palette or the Inspector while it is not focused stays.
    if (list.length) setSel(list[Math.min(lastAt, list.length - 1)], { quiet: true, inform: app.focused === paneId });
    else selKey = null;
  });

  // Back in Mission control (Esc from a session, leader o): the Inspector shows its row again (D10).
  let wasFocused = false;
  $effect(() => {
    const on = app.focused === paneId;
    if (on && !wasFocused && selKey) untrack(() => setSel(selKey, { c: chip, quiet: true }));
    wasFocused = on;
  });

  /** Selects a stop and tells the Inspector (D10) unless `inform` is false; `quiet`: no scroll. */
  function setSel(key, { c = null, quiet = false, inform = true } = {}) {
    selKey = key;
    chip = c;
    if (!inform) return;
    const r = index.get(key);
    if (r?.kind === 'run') {
      if (c == null) select('run', r.run.run);
      else select('agent', c);
    } else if (r?.kind === 'solo' || r?.kind === 'queue') select('agent', r.agent.agent);
    else if (r?.kind === 'decisions') select('decision', r.ids[0]);
    else if (r?.kind === 'outcome') {
      const o = r.outcome;
      if (o.run && app.now.runs[o.run]) select('run', o.run);
      else if (o.agent) select('agent', o.agent);
    }
    if (!quiet) queueMicrotask(() => document.querySelector(`[data-pane="${paneId}"] [data-key="${CSS.escape(key)}"]`)?.scrollIntoView({ block: 'nearest' }));
  }

  function move(d) {
    if (!stopList.length) return;
    const i = stopList.indexOf(selKey);
    setSel(stopList[Math.max(0, Math.min(stopList.length - 1, i < 0 ? 0 : i + d))]);
  }

  /** h/l on a run row (notes §3.1): the chips; landing on the default target is the run again. */
  function moveChip(d) {
    if (row?.kind !== 'run') return;
    const m = row.run.members;
    const def = Math.max(0, m.indexOf(defaultTarget(row.run)));
    const at = chip && m.includes(chip) ? m.indexOf(chip) : def;
    const next = Math.max(0, Math.min(m.length - 1, at + d));
    setSel(selKey, { c: next === def ? null : m[next], quiet: true });
  }

  const toggle = (obj, k) => ({ ...obj, [k]: !obj[k] });

  async function act(id, action) {
    busy = id;
    try {
      await agentAction(id, action);
    } finally {
      busy = null;
    }
  }

  function kill(id) {
    confirm(`Kill ${id}?`, 'It loses its current turn. Its process group and leftover processes in its worktree get SIGTERM, then SIGKILL after 5 s.', () => act(id, 'kill'), 'Kill');
  }

  /** `o` on an outcome (notes §1.7). */
  function openOutcome(o) {
    if (o.url) return openUrl(o.url);
    if (o.kind === 'turn_done' && o.agent) return openSession(o.agent);
    if (o.kind === 'report_ready') return openReport(o.report ?? 'current');
    if (o.kind === 'sync_conflict' && o.project) return openBoard(o.project, o.task ?? null);
    if (o.run && index.has(`run:${o.run}`)) return reveal(`run:${o.run}`);
    if (o.agent) return openSession(o.agent);
  }

  /** Selects a row, opening its collapsed lane or the Idle line first (else the selection would be repaired away). */
  function reveal(key) {
    const lane = model.lanes.find((l) => l.rows.some((r) => r.key === key));
    if (lane && collapsed[lane.id]) collapsed = { ...collapsed, [lane.id]: false };
    if (!lane && model.idle?.rows.some((r) => r.key === key)) idleOpen = true;
    setSel(key);
  }

  /** ⏎ / space on a lane line, Show all or the Idle line: expand or collapse. */
  function expand(key) {
    if (key === 'showall') return (showAll = !showAll);
    if (key === 'lane:idle') return (idleOpen = !idleOpen);
    collapsed = toggle(collapsed, key.slice(5));
  }

  function onEnter() {
    if (!selKey) return;
    if (selKey.startsWith('lane:') || selKey === 'showall') return expand(selKey);
    if (row?.kind === 'decisions') return focusCard(row.ids[0]);
    if (row?.kind === 'outcome') return openOutcome(row.outcome);
    if (row?.kind === 'ready') return row.hidden ? unhideItem(row.item) : startItem(row.item, modeOf(row.item));
    if (row?.kind === 'iline') return lineAct(row);
    if (t) openSession(t);
  }

  // ---- Ready to start (P4.5): user presses only; the lane itself fetches nothing.

  const refreshLane = () => refreshIntake((app.now.intake?.sources ?? []).filter((s) => s.type !== 'none' && (!filter || filter.includes(s.project))).map((s) => s.project));

  /** A Ready item's actions, for its row and its details. */
  const itemActs = (it) => ({ onstart: (m) => startItem(it, m), onhide: () => hideItem(it), onunhide: () => unhideItem(it), onsource: () => openSource(it, openEditor) });

  /** ⏎ / a click on a line (notes §4.1, §10.1). */
  function lineAct(r) {
    if (r.line === 'nosource') return planWithOverwatchIn(r.projects);
    if (r.line === 'empty') return refreshIntake(r.merged ? r.projects : [r.project]);
    if (r.line === 'error') return fixSource(r.src);
    if (r.line === 'more') return r.src.url && openUrl(r.src.url);
    if (r.line === 'allhidden') return (intakeUi.showHidden = true);
  }

  /** `e` on a no-source or error line: Project settings › Intake (the first project of a merged line). */
  const setUp = (r) => setUpIntake(r.projects?.[0] ?? r.project);

  function onKey(e) {
    if (e.defaultPrevented) return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (isTypingTarget(e.target)) return;
    const k = e.key;
    const run = (fn) => {
      e.preventDefault();
      fn();
    };
    // ⇧K kills; k (and a caps-lock K) moves up (notes §3.2).
    if (k === 'K' && e.shiftKey) return t && trec ? run(() => kill(t)) : undefined;
    if (k === 'j' || k === 'ArrowDown') return run(() => move(1));
    if (k === 'k' || k === 'K' || k === 'ArrowUp') return run(() => move(-1));
    if (k === 'h' || k === 'ArrowLeft') return run(() => moveChip(-1));
    if (k === 'l' || k === 'ArrowRight') return run(() => moveChip(1));
    if (k === 'R' && e.shiftKey) return run(refreshLane);
    if (k === 'H' && e.shiftKey) return run(() => (intakeUi.showHidden = !intakeUi.showHidden));
    if (k === 'u' && intakeUi.lastHide) return run(undoHide);
    if (row?.kind === 'ready') {
      const it = row.item;
      if (!row.hidden && k === 't' && it.can_team !== false) return run(() => startItem(it, 'team'));
      if (!row.hidden && k === 'a') return run(() => startItem(it, 'solo'));
      if (k === 'x' || k === 'Backspace') return run(() => (row.hidden ? unhideItem(it) : hideItem(it)));
      if (k === 'o') return run(() => openSource(it, openEditor));
    }
    if (row?.kind === 'iline' && k === 'e' && (row.line === 'nosource' || row.line === 'error')) return run(() => setUp(row));
    if (k === 'Enter') return run(onEnter);
    if (k === ' ') {
      if (!selKey) return;
      if (selKey.startsWith('lane:') || selKey === 'showall') return run(() => expand(selKey));
      if (row && row.kind !== 'outcome' && row.kind !== 'decisions' && row.kind !== 'iline') return run(() => (details = toggle(details, selKey)));
      return;
    }
    if (k === 'd') return dec && run(() => focusCard(dec));
    if (k === 'o') {
      if (row?.kind === 'outcome') return run(() => openOutcome(row.outcome));
      if (row?.kind === 'run' && row.run.pr) return run(() => openUrl(row.run.pr.url));
      return;
    }
    if (row?.kind === 'queue' && trec?.queue) {
      if (k === 'n') return run(() => runNow(trec));
      if (k === 'Backspace') return run(() => cancelQueued(trec));
    }
    if (!t || !trec) return;
    const ok = allowed(trec);
    if (k === 'i' && ok.i) return run(() => act(t, 'interrupt'));
    // A paused agent mid-turn keeps its working status: app_state's flag says it is paused.
    if (k === 'p') return run(() => act(t, agent(t)?.paused || trec.status === 'paused' ? 'unpause' : 'pause'));
    if (k === 's' && ok.s) return run(() => act(t, 'suspend'));
    if (k === 'r' && ok.r) return run(() => act(t, trec.status === 'paused' ? 'unpause' : 'resume'));
  }

  function clickRow(r) {
    setSel(r.key);
    // A click on a solo row opens its session (spec §3.3).
    if (r.kind === 'solo') openSession(r.agent.agent);
  }

  function clickChip(r, i) {
    const id = r.run.members[i];
    setSel(r.key, { c: id === defaultTarget(r.run) ? null : id, quiet: true });
    openSession(id);
  }

  const ids = (r) => (r.kind === 'run' ? r.run.members : r.kind === 'solo' || r.kind === 'queue' ? [r.agent.agent] : []);
</script>

{#snippet item(r)}
  {@const sel = selKey === r.key}
  {#if r.kind === 'outcome'}
    <div role="row" aria-selected={sel} data-key={r.key}>
      <div role="gridcell"><OutcomeRow outcome={r.outcome} selected={sel} onselect={() => setSel(r.key)} /></div>
    </div>
  {:else if r.kind === 'ready' || r.kind === 'iline'}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="row ready" class:iline={r.kind === 'iline'} class:sel role="row" tabindex="-1" aria-selected={sel} data-key={r.key} onclick={() => setSel(r.key)}>
      <div role="gridcell" class="cell">
        {#if r.kind === 'ready'}
          <ReadyRow row={r} selected={sel} starting={intakeUi.starting === r.item.id} {...itemActs(r.item)} ondetails={() => (details = toggle(details, r.key))} />
          {#if details[r.key]}<ReadyDetails item={r.item} name={r.name} {now} hidden={r.hidden} {...itemActs(r.item)} />{/if}
        {:else}
          <ReadyLine line={r} {now} {overwatch} onplan={() => planWithOverwatchIn(r.projects)} onsetup={() => setUp(r)} onrefresh={() => lineAct(r)} onfix={() => fixSource(r.src)} onopen={() => lineAct(r)} onshow={() => (intakeUi.showHidden = true)} />
        {/if}
      </div>
    </div>
  {:else}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="row {r.kind}" class:sel class:err={r.kind === 'run' ? r.run.level === 'error' : r.agent?.level === 'error'} role="row" tabindex="-1" aria-selected={sel} data-key={r.key} onclick={() => clickRow(r)}>
      <div role="gridcell" class="cell">
        {#if r.kind === 'run'}
          <RunRow run={r.run} {now} target={sel ? t : defaultTarget(r.run)} {busy} onchip={(i) => clickChip(r, i)} />
        {:else if r.kind === 'solo'}
          <SoloRow rec={r.agent} />
        {:else if r.kind === 'queue'}
          <QueueRow rec={r.agent} run={r.run} {now} onrun={() => runNow(r.agent)} oncancel={() => cancelQueued(r.agent)} onresume={() => act(r.agent.agent, 'unpause')} />
        {:else if r.kind === 'decisions'}
          <div class="loose">
            <span style:color="var(--color-attention-needs-you)" aria-hidden="true">{LEVEL.needs_you.icon}</span>
            {looseText(r.count)} · <button class="linkbtn" tabindex="-1" onclick={(e) => (e.stopPropagation(), focusCard(r.ids[0]))}>Show card</button> <Kbd keys="d" />
          </div>
        {/if}
        {#if details[r.key] && r.kind !== 'decisions'}<RowDetails ids={ids(r)} {now} origin={r.kind === 'run' ? r.run.origin : r.agent?.origin} />{/if}
      </div>
    </div>
  {/if}
{/snippet}

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="mc" tabindex="0" data-pane-focus onkeydown={onKey} role="region" aria-label="Mission control">
  <div class="toolbar">
    <b>Mission control</b>
    <span class="muted">{ctx ? `in ${projectName(ctx)}` : 'every project'} · follows the project context</span>
  </div>

  <div class="scroll" role="grid" aria-label="Mission control rows" aria-readonly="true">
    {#if !loaded}
      <div class="skel" aria-busy="true" aria-label="Loading">
        <span class="bar short"></span><span class="bar"></span><span class="bar"></span><span class="bar short"></span><span class="bar"></span>
      </div>
    {:else if !ps.length}
      <div class="empty">
        <h2>No projects yet</h2>
        <p class="secondary">Add a project to see what runs, waits and queues across them.</p>
        <button class="btn primary" onclick={() => (app.overlay = { kind: 'add-project' })}>Add project <Kbd keys="⌘+N" /></button>
      </div>
    {:else if filteredOut}
      <div class="empty">
        <p class="secondary">Nothing in {projectName(ctx)}</p>
        <button class="btn" onclick={() => setContext('all')}>Show all projects</button>
      </div>
    {:else}
      {#if !work}
        <div class="empty">
          <h2>Nothing is running</h2>
          <p class="secondary">Start a session or a feature and it shows up here.</p>
          <button class="btn primary" onclick={() => (app.overlay = { kind: 'new-session' })}>New session <Kbd keys="⌘+T" /></button>
        </div>
      {/if}
      {#each model.lanes as lane (lane.id)}
        {#if work || lane.id === 'done' || lane.id === 'ready'}
          {@const shut = !!collapsed[lane.id]}
          {@const shown = lane.id === 'done' && !showAll ? lane.rows.slice(0, DONE_ROWS) : lane.rows}
          <section class="lane" class:readylane={lane.id === 'ready'} aria-label={lane.label}>
            <h3 class="lh" class:sel={selKey === `lane:${lane.id}`} data-key="lane:{lane.id}">
              <button class="lbtn" tabindex="-1" aria-expanded={!shut} onclick={() => (collapsed = toggle(collapsed, lane.id))}>
                <span class="chev" aria-hidden="true">{shut ? '›' : '⌄'}</span>
                <span style:color={LEVEL[lane.level].color} aria-hidden="true">{LEVEL[lane.level].icon}</span>
                <b class:quiet={lane.id === 'ready'}>{lane.rows.length || lane.id !== 'needs_you' ? lane.label : 'Nothing needs you'}</b>
                {#if lane.count}<span class="muted">· {lane.count}</span>{/if}
              </button>
              {#if lane.id === 'ready' && lane.status}
                {@const st = lane.status}
                <span class="grow"></span>
                <!-- A failure stays visible when the lane is collapsed (notes §1.1). -->
                <span class="rstat" class:fail={!!st.failed}>
                  {#if st.refreshing}refreshing…{:else if st.failed}{st.failed === 1 ? '1 source failed' : `${st.failed} sources failed`}{:else if st.oldest != null}updated {ageText(st.oldest, now)}{/if}
                </span>
                {#if st.withSource}
                  <button class="btn ghost rbtn" tabindex="-1" disabled={st.refreshing || (app.now.intake?.sources ?? []).some((s) => (!filter || filter.includes(s.project)) && rateLimitedUntil(s, now))} onclick={refreshLane} title="Check GitHub, Linear and repo docs now">{st.refreshing ? 'Refreshing…' : 'Refresh'} <Kbd keys="⇧R" /></button>
                {/if}
                {#if !shut && st.hidden > 0}
                  <button class="linkbtn rbtn" tabindex="-1" onclick={() => (intakeUi.showHidden = !intakeUi.showHidden)}>{intakeUi.showHidden ? `Hide the ${st.hidden}` : `${st.hidden} hidden · Show`} <Kbd keys="⇧H" /></button>
                {/if}
              {/if}
            </h3>
            {#if !shut}
              {#each shown as r (r.key)}{@render item(r)}{/each}
              {#if lane.id === 'done' && lane.rows.length > DONE_ROWS}
                <button class="more" class:sel={selKey === 'showall'} data-key="showall" tabindex="-1" onclick={() => (showAll = !showAll)}>{showAll ? 'Show fewer' : `Show all ${lane.rows.length}`} <Kbd keys="⏎" /></button>
              {/if}
            {/if}
          </section>
          {#if lane.id === 'needs_you' && lane.rows.length && !moving}<p class="muted still">Nothing else is moving</p>{/if}
        {/if}
      {/each}
      {#if !work && !done}<p class="muted still">Nothing done yet today.</p>{/if}
      {#if model.idle}
        <section class="lane" aria-label="Idle">
          <h3 class="lh" class:sel={selKey === 'lane:idle'} data-key="lane:idle">
            <button class="lbtn" tabindex="-1" aria-expanded={idleOpen} onclick={() => (idleOpen = !idleOpen)}>
              <span class="chev" aria-hidden="true">{idleOpen ? '⌄' : '›'}</span>
              <span style:color={LEVEL.idle.color} aria-hidden="true">{LEVEL.idle.icon}</span>
              <b>Idle</b> <span class="muted">· {model.idle.count}</span>
            </button>
          </h3>
          {#if idleOpen}
            {#each model.idle.rows as r (r.key)}{@render item(r)}{/each}
          {/if}
        </section>
      {/if}
    {/if}
  </div>

  <div class="foot">
    <span>{filteredOut ? hiddenText(app.now.counts, filter, ps) : footerText(app.now.counts, filter, ps)}</span>
    <span class="grow"></span>
    {#if app.data?.memory}<span class="muted">RAM {mb(app.data.memory.total_mb)} / {mb(app.data.memory.system_mb)}</span>{/if}
  </div>
</div>

<style>
  .mc {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    outline: none;
    font: var(--text-label);
  }
  .toolbar {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: var(--size-row);
    padding: var(--space-md) var(--space-lg) var(--space-xs);
    white-space: nowrap;
  }
  .toolbar b {
    font: var(--text-title);
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--space-xs) var(--space-sm) var(--space-md);
  }
  .lane {
    margin-top: var(--space-sm);
  }
  .lh {
    display: flex;
    align-items: center;
    height: var(--size-lane-header);
    margin: 0;
    font: var(--text-label);
  }
  .lh.sel,
  .more.sel {
    background: var(--color-surface-selected);
    border-radius: var(--radius-sm);
  }
  .lbtn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    padding: 0 var(--space-xs);
    border: 0;
    background: none;
    color: var(--color-text-primary);
    font: var(--text-body);
  }
  .rstat {
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .rstat.fail {
    color: var(--color-status-error);
  }
  .rbtn {
    margin-left: var(--space-sm);
  }
  /* Ready is not attention (notes p4-5): no per-row box; rows sit on the lane between hairlines,
     and one lane-wide 1 px dashed rule runs down the left through rows and lines alike. */
  .readylane {
    padding-left: var(--space-sm);
    border-left: var(--border-width-default) dashed var(--color-border-strong);
  }
  .row.ready {
    margin-top: 0;
    border: 0;
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: 0;
    background: none;
  }
  .row.ready.sel {
    background: var(--color-surface-selected);
  }
  b.quiet {
    color: var(--color-text-secondary);
  }
  .chev {
    width: var(--size-icon-sm);
    color: var(--color-text-muted);
  }
  .row {
    min-height: var(--size-row-dense);
    margin-top: var(--space-xs);
    padding: var(--space-xs) var(--space-md);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    background: var(--color-surface-pane);
    display: flex;
    align-items: center;
    cursor: default;
    outline: none;
  }
  .row .cell {
    flex: 1;
    min-width: 0;
  }
  .row:hover {
    background: var(--color-surface-raised);
  }
  .row.sel {
    background: var(--color-surface-raised);
    box-shadow: inset var(--border-width-focus) 0 0 var(--color-focus);
  }
  .loose {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .linkbtn {
    padding: 0;
    border: 0;
    background: none;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
  .more {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    margin: var(--space-xs) var(--space-md);
    padding: var(--space-xxs) var(--space-xs);
    border: 0;
    background: none;
    color: var(--color-text-secondary);
    font: var(--text-label);
  }
  .still {
    margin: var(--space-md);
  }
  .empty {
    margin: var(--space-3xl) auto;
    text-align: center;
  }
  .empty h2 {
    margin: 0 0 var(--space-sm);
    font: var(--text-title);
  }
  .empty p {
    margin: 0 0 var(--space-md);
  }
  .skel {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-sm);
  }
  .bar {
    height: var(--size-row-dense);
    border-radius: var(--radius-sm);
    background: var(--color-surface-raised);
  }
  .bar.short {
    width: 30%;
    height: var(--size-icon);
  }
  .foot {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: var(--size-strip);
    padding: var(--space-xs) var(--space-md);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
    white-space: nowrap;
  }
</style>
