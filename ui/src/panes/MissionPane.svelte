<script>
  // Mission control (FR-1302, P4.2 spec §3.2–3.4, notes p4-2, Penpot 04):
  // what runs, waits on what, is queued why and was done today, on one
  // screen. A render of `now` (§9.1 principle 10): it fetches nothing, and
  // its one timer is the 30 s relative-time clock. Rows and lanes come from
  // `lanes()` (lib/mission.js); this pane keeps the selection, the open
  // lanes and details, and the keys (notes §3.1).
  import Kbd from '../components/Kbd.svelte';
  import ProjectFilter from '../components/mission/ProjectFilter.svelte';
  import QueueRow from '../components/mission/QueueRow.svelte';
  import RowDetails from '../components/mission/RowDetails.svelte';
  import RunRow from '../components/mission/RunRow.svelte';
  import SoloRow from '../components/mission/SoloRow.svelte';
  import OutcomeRow from '../components/OutcomeRow.svelte';
  import {
    agentAction,
    app,
    cancelQueued,
    confirm,
    now30,
    openBoard,
    openInbox,
    openReport,
    openSession,
    openUrl,
    projects,
    runNow,
    select,
    setMissionFilter,
  } from '../lib/app.svelte.js';
  import { mb } from '../lib/format.js';
  import { DONE_ROWS, decisionOf, defaultTarget, filterLabel, footerText, hiddenText, hints, lanes, looseText, rowIndex, stops, target } from '../lib/mission.js';
  import { attention } from '../lib/now.svelte.js';
  import { LEVEL } from '../lib/status.js';

  let { paneId } = $props();

  let selKey = $state(null);
  let chip = $state(null); // index of the selected member chip of a run row (null: the run)
  let collapsed = $state({});
  let idleOpen = $state(false);
  let showAll = $state(false);
  let details = $state({});
  let busy = $state(null); // the agent an action waits on
  let lastAt = 0; // the selection's last stop index (a row that leaves gives its place to the next)

  const now = $derived(Math.max(now30(), Date.now()));
  const filter = $derived(app.mission.projects);
  const ps = $derived(projects());
  const model = $derived(lanes(app.now, filter, now));
  const all = $derived(filter ? lanes(app.now, null, now) : model);
  const stopList = $derived(stops(model, { collapsed, idleOpen, showAll }));
  const index = $derived(rowIndex(model));
  const row = $derived(index.get(selKey) ?? null);
  const t = $derived(target(row, chip));
  const trec = $derived(t ? attention(t) : null);
  const dec = $derived(decisionOf(row, trec));
  const hintList = $derived(hints(row, { target: t, rec: trec, decision: dec }));
  const loaded = $derived(app.now.rev > 0);
  const work = (m) => m.lanes.some((l) => l.id !== 'done' && l.rows.length) || !!m.idle;
  const moving = $derived(model.lanes.some((l) => l.id !== 'needs_you' && l.id !== 'done') || !!model.idle);
  const done = $derived(model.lanes.find((l) => l.id === 'done'));
  const filterOpen = $derived(app.missionFilterOpen);

  // The selection follows its row across lanes (keys are the run or agent);
  // a row that left gives its place to the one now there.
  $effect(() => {
    const list = stopList;
    if (selKey && list.includes(selKey)) {
      lastAt = list.indexOf(selKey);
      return;
    }
    if (list.length) setSel(list[Math.min(lastAt, list.length - 1)], { quiet: true });
    else selKey = null;
  });

  /** Selects a stop and tells the Inspector (D10); `quiet`: no scroll. */
  function setSel(key, { c = null, quiet = false } = {}) {
    selKey = key;
    chip = c;
    const r = index.get(key);
    if (r?.kind === 'run') {
      if (c == null) select('run', r.run.run);
      else select('agent', r.run.members[c]);
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
    const next = Math.max(0, Math.min(m.length - 1, (chip ?? def) + d));
    setSel(selKey, { c: next === def ? null : next, quiet: true });
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
    if (o.run && index.has(`run:${o.run}`)) return setSel(`run:${o.run}`);
    if (o.agent) return openSession(o.agent);
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
    if (row?.kind === 'decisions') return openInbox(row.ids[0]);
    if (row?.kind === 'outcome') return openOutcome(row.outcome);
    if (t) openSession(t);
  }

  function onKey(e) {
    if (filterOpen || e.defaultPrevented) return;
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.target.closest?.('input, textarea, select')) return;
    const k = e.key;
    const run = (fn) => {
      e.preventDefault();
      fn();
    };
    // ⇧K kills; k (and a caps-lock K) moves up (notes §3.2).
    if (k === 'K' && e.shiftKey) return t && trec && run(() => kill(t));
    if (k === 'j' || k === 'ArrowDown') return run(() => move(1));
    if (k === 'k' || k === 'K' || k === 'ArrowUp') return run(() => move(-1));
    if (k === 'h' || k === 'ArrowLeft') return run(() => moveChip(-1));
    if (k === 'l' || k === 'ArrowRight') return run(() => moveChip(1));
    if (k === 'f') return run(() => (app.missionFilterOpen = true));
    if (k === 'Enter') return run(onEnter);
    if (k === ' ') {
      if (!selKey) return;
      if (selKey.startsWith('lane:') || selKey === 'showall') return run(() => expand(selKey));
      if (row && row.kind !== 'outcome' && row.kind !== 'decisions') return run(() => (details = toggle(details, selKey)));
      return;
    }
    if (k === 'd') return dec && run(() => openInbox(dec));
    if (k === 'o') {
      if (row?.kind === 'outcome') return run(() => openOutcome(row.outcome));
      if (row?.kind === 'run' && row.run.pr) return run(() => openUrl(row.run.pr.url));
      return;
    }
    if (row?.kind === 'queue') {
      if (k === 'n' && trec?.queue) return run(() => runNow(trec));
      if (k === 'Backspace' && trec?.queue) return run(() => cancelQueued(trec));
    }
    if (!t || !trec) return;
    if (k === 'i' && trec.live) return run(() => act(t, 'interrupt'));
    if (k === 'p') return run(() => act(t, trec.status === 'paused' ? 'unpause' : 'pause'));
    if (k === 's' && trec.live) return run(() => act(t, 'suspend'));
    if (k === 'r' && ['paused', 'suspended', 'error'].includes(trec.status)) return run(() => act(t, trec.status === 'paused' ? 'unpause' : 'resume'));
  }

  function applyFilter(next) {
    app.missionFilterOpen = false;
    if (JSON.stringify(next) !== JSON.stringify(filter)) setMissionFilter(next);
    queueMicrotask(() => /** @type {HTMLElement | null} */ (document.querySelector(`[data-pane="${paneId}"] .mc`))?.focus());
  }

  function clickRow(r) {
    setSel(r.key);
    // A click on a solo row opens its session (spec §3.3).
    if (r.kind === 'solo') openSession(r.agent.agent);
  }

  function clickChip(r, i) {
    const def = Math.max(0, r.run.members.indexOf(defaultTarget(r.run)));
    setSel(r.key, { c: i === def ? null : i, quiet: true });
    openSession(r.run.members[i]);
  }

  const ids = (r) => (r.kind === 'run' ? r.run.members : r.kind === 'solo' || r.kind === 'queue' ? [r.agent.agent] : []);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="mc" tabindex="0" data-pane-focus onkeydown={onKey} role="region" aria-label="Mission control">
  <div class="toolbar">
    <b>Mission control</b>
    <span class="fwrap">
      <button class="fbtn" class:on={!!filter} onclick={() => (app.missionFilterOpen = !filterOpen)} title={hiddenText(app.now.counts, filter, ps) || 'Filter by project'}>
        <span aria-hidden="true">{'\u{F0232}'}</span> {filterLabel(filter, ps)} <Kbd keys="f" />
      </button>
      {#if filterOpen}<ProjectFilter {filter} projects={ps} counts={app.now.counts} onapply={applyFilter} />{/if}
    </span>
    <span class="grow"></span>
    {#if loaded && ps.length}
      <span class="hints">
        {#each hintList as [keys, label] (keys)}<span class="hint"><Kbd {keys} /> {label}</span>{/each}
      </span>
    {/if}
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
    {:else if filter && !work(model) && !done && (work(all) || all.lanes.some((l) => l.id === 'done'))}
      <div class="empty">
        <h2>Nothing in the {filter.length === 1 ? filterLabel(filter, ps) : `${filter.length} projects`} selected</h2>
        <button class="linkbtn" onclick={() => (app.missionFilterOpen = true)}>Show all projects <Kbd keys="f" /></button>
      </div>
    {:else}
      {#if !work(model)}
        <div class="empty">
          <h2>Nothing is running</h2>
          <p class="secondary">Start a session or a feature and it shows up here.</p>
          <button class="btn primary" onclick={() => (app.overlay = { kind: 'new-session' })}>New session <Kbd keys="⌘+T" /></button>
        </div>
      {/if}
      {#each model.lanes as lane (lane.id)}
        {#if work(model) || lane.id === 'done'}
          {@const shut = !!collapsed[lane.id]}
          {@const shown = lane.id === 'done' && !showAll ? lane.rows.slice(0, DONE_ROWS) : lane.rows}
          <section class="lane" aria-label={lane.label}>
            <h3 class="lh" class:sel={selKey === `lane:${lane.id}`} data-key="lane:{lane.id}">
              <button class="lbtn" tabindex="-1" aria-expanded={!shut} onclick={() => (collapsed = toggle(collapsed, lane.id))}>
                <span class="chev" aria-hidden="true">{shut ? '›' : '⌄'}</span>
                <span style:color={LEVEL[lane.level].color} aria-hidden="true">{LEVEL[lane.level].icon}</span>
                <b>{lane.rows.length || lane.id !== 'needs_you' ? lane.label : 'Nothing needs you'}</b>
                {#if lane.count}<span class="muted">· {lane.count}</span>{/if}
              </button>
            </h3>
            {#if !shut}
              {#each shown as r (r.key)}
                {@const sel = selKey === r.key}
                {#if r.kind === 'outcome'}
                  <div role="row" aria-selected={sel} data-key={r.key}>
                    <div role="gridcell"><OutcomeRow outcome={r.outcome} selected={sel} onselect={() => setSel(r.key)} /></div>
                  </div>
                {:else}
                  <!-- svelte-ignore a11y_click_events_have_key_events -->
                  <div class="row {r.kind}" class:sel class:err={r.kind === 'run' ? r.run.level === 'error' : r.agent?.level === 'error'} role="row" tabindex="-1" aria-selected={sel} data-key={r.key} onclick={() => clickRow(r)}>
                    <div role="gridcell" class="cell">
                      {#if r.kind === 'run'}
                        <RunRow run={r.run} {now} target={sel ? t : defaultTarget(r.run)} {busy} onchip={(i) => clickChip(r, i)}>
                          {#if details[r.key]}<RowDetails ids={ids(r)} {now} />{/if}
                        </RunRow>
                      {:else if r.kind === 'solo'}
                        <SoloRow rec={r.agent}>{#if details[r.key]}<RowDetails ids={ids(r)} {now} />{/if}</SoloRow>
                      {:else if r.kind === 'queue'}
                        <QueueRow rec={r.agent} run={r.run} onrun={() => runNow(r.agent)} oncancel={() => cancelQueued(r.agent)} onresume={() => act(r.agent.agent, 'unpause')}>
                          {#if details[r.key]}<RowDetails ids={ids(r)} {now} />{/if}
                        </QueueRow>
                      {:else if r.kind === 'decisions'}
                        <div class="loose">
                          <span style:color="var(--color-attention-needs-you)" aria-hidden="true">{LEVEL.needs_you.icon}</span>
                          {looseText(r.count)} · <button class="linkbtn" tabindex="-1" onclick={(e) => (e.stopPropagation(), openInbox(r.ids[0]))}>Open in Inbox</button> <Kbd keys="d" />
                        </div>
                      {/if}
                    </div>
                  </div>
                {/if}
              {/each}
              {#if lane.id === 'done' && lane.rows.length > DONE_ROWS}
                <button class="more" class:sel={selKey === 'showall'} data-key="showall" tabindex="-1" onclick={() => (showAll = !showAll)}>{showAll ? 'Show fewer' : `Show all ${lane.rows.length}`} <Kbd keys="⏎" /></button>
              {/if}
            {/if}
          </section>
          {#if lane.id === 'needs_you' && lane.rows.length && !moving}<p class="muted still">Nothing else is moving</p>{/if}
        {/if}
      {/each}
      {#if !work(model) && !done}<p class="muted still">Nothing done yet today.</p>{/if}
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
            {#each model.idle.rows as r (r.key)}
              {@const sel = selKey === r.key}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div class="row solo" class:sel role="row" tabindex="-1" aria-selected={sel} data-key={r.key} onclick={() => clickRow(r)}>
                <div role="gridcell" class="cell">
                  {#if r.kind === 'run'}
                    <RunRow run={r.run} {now} target={sel ? t : defaultTarget(r.run)} {busy} onchip={(i) => clickChip(r, i)}>{#if details[r.key]}<RowDetails ids={ids(r)} {now} />{/if}</RunRow>
                  {:else}
                    <SoloRow rec={r.agent}>{#if details[r.key]}<RowDetails ids={ids(r)} {now} />{/if}</SoloRow>
                  {/if}
                </div>
              </div>
            {/each}
          {/if}
        </section>
      {/if}
    {/if}
  </div>

  <div class="foot">
    <span>{filter && !work(model) && work(all) ? hiddenText(app.now.counts, filter, ps) : footerText(app.now.counts, filter, ps)}</span>
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
    padding: var(--space-xs) var(--space-md);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
    white-space: nowrap;
  }
  .fwrap {
    position: relative;
  }
  .fbtn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    height: var(--size-control-sm);
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    background: none;
    color: var(--color-text-secondary);
    font: var(--text-label);
  }
  .fbtn.on {
    border-color: var(--color-accent-default);
    color: var(--color-accent-default);
  }
  .hints {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-xs) var(--space-sm);
    min-width: 0;
    overflow: hidden;
    color: var(--color-text-secondary);
  }
  .hint {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
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
    box-shadow: inset 2px 0 0 var(--color-accent-default);
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
