<script>
  // Run as program (improve-intake notes §6, boards ii-plan-*): the plan of a node (or ≥ 2 siblings) as
  // features, edited on its rows before one approval. `program_plan` drafts it (nothing stored); the
  // edits stay here; `program_start` validates the edited draft and starts it, or returns the issues
  // left (the dialog stays). Keys §12.3; states §6.5.
  import { onMount, tick } from 'svelte';
  import { app, featureBySlug, openCanvas, openFeature, openUrl, projectName, toast } from '../lib/app.svelte.js';
  import { plural } from '../lib/format.js';
  import { staleOf } from '../lib/intake.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { isTypingTarget } from '../lib/keyboard.js';
  import * as P from '../lib/program.js';
  import { tooltip } from '../lib/tooltip.js';
  import { openProgram, programTeams } from '../lib/work.js';
  import { isClosed, isContainer } from '../lib/workmodel.js';
  import FloatingWindow from './FloatingWindow.svelte';
  import Menu from './Menu.svelte';
  import GraphStrip from './program/GraphStrip.svelte';
  import Select from './Select.svelte';

  // The overlay this dialog was opened for (it is keyed on it): a late `close` leaves another one alone.
  const o = app.overlay;
  const roots = $derived(o?.roots ?? []);

  /** planning · planned · replanning · failed · stale · nothing */
  let phase = $state('planning');
  let base = $state(null); // the plan as the core drafted it
  let draft = $state(null); // the user's edited copy
  let coreIssues = $state([]);
  let error = $state(null); // plan failed
  let startError = $state(null);
  let stale = $state(null); // { message, program? }
  let starting = $state(false);
  let grouping = $state('child');
  let sequential = $state(false);
  let atOnce = $state(2);
  let root = $state(null); // the first root's WorkNode
  let teams = $state(/** @type {{ id: string, name: string }[] | null} */ (null));
  let tracker = $state(null); // the project's TrackerSettings
  let sel = $state(0);
  let briefKey = $state(null);
  let briefText = $state('');
  let ask = $state(null); // { text, yes, no, onYes }: an inline confirm
  let cutFor = $state(null); // 'cycle': the issue whose "Remove a dependency…" popover is open
  let cutPick = $state(null);
  let hoverKey = $state(null);

  const project = $derived(draft?.project ?? root?.project ?? null);
  const pname = $derived(projectName(project) ?? project ?? 'the project');
  const trackerWord = $derived(root ? P.trackerOfId(root.id) : 'the tracker');
  const issues = $derived(P.liveIssues(draft, coreIssues, teams?.map((t) => t.id) ?? null));
  const blocking = $derived(issues.filter((i) => i.blocking).length);
  const flagged = $derived(P.flaggedKeys(issues));
  const title = $derived(draft?.title ?? o?.title ?? 'program');
  const rootRef = $derived(draft?.root_ref ?? root?.ref ?? '');
  const single = $derived(roots.length === 1);
  const container = $derived(isContainer(root));
  const closedRoot = $derived(single && isClosed(root));
  const hasGrand = $derived(root?.child_nodes ? root.child_nodes.some((c) => (c.children?.total ?? 0) > 0) : true);
  const busy = $derived(phase === 'planning' || phase === 'replanning');
  const locked = $derived(busy || phase === 'stale');
  const order = $derived(sequential ? 'seq' : 'deps');
  const canStart = $derived(!!draft && !blocking && !busy && !starting && phase === 'planned' && P.liveCount(draft) > 0);
  const wbLine = $derived(P.writeBackLine(tracker, single ? rootRef : null));
  const totals = $derived(P.totalsParts(draft, pname));
  const teamOptions = $derived((teams ?? []).map((t) => ({ value: t.id, label: t.name })));
  /** "From #12 Checkout v2 · in v2 · acme/shop · GitHub" · "From 3 selected under #12 · …". */
  const fromLine = $derived.by(() => {
    const parent = root?.breadcrumb?.at(-1);
    const head = single ? `From ${[rootRef, root?.title].filter(Boolean).join(' ')}` : `From ${roots.length} selected under ${rootRef}`;
    const inPart = single && parent && !container ? `in ${[...new Set([parent.ref, parent.title].filter(Boolean))].join(' ')}` : '';
    return [head, inPart, tracker?.tracker?.repo ?? '', trackerWord].filter(Boolean).join(' · ');
  });
  const width = $derived((app.width ?? window.innerWidth) < 1512 ? 800 : 880);

  // ---- loading

  let seq = 0;
  async function plan({ replan = false } = {}) {
    const n = ++seq;
    phase = replan ? 'replanning' : 'planning';
    error = null;
    startError = null;
    try {
      const r = await invoke('program_plan', { roots, grouping, opts: { sequential, max_parallel: atOnce, cut: draft?.cut ?? [] } });
      if (n !== seq) return;
      // (the plan names the project before the root's node arrives: teams and tracker start now)
      if (!teams && r.program?.project) loadProject(r.program.project);
      base = r.program;
      draft = P.withTotals(structuredClone(r.program));
      coreIssues = r.issues ?? [];
      // (one after the other plans at 1: the chosen At once stays for By dependencies)
      if (!sequential) atOnce = r.program.max_parallel ?? atOnce;
      sel = 0;
      briefKey = null;
      phase = r.program.features?.length ? 'planned' : 'nothing';
    } catch (e) {
      if (n !== seq) return;
      failed(e, () => {
        error = errText(e);
        phase = 'failed';
      });
    }
  }

  /** A refusal: stale (banner) or `other`. */
  function failed(e, other) {
    const s = staleOf(e);
    if (!s) return other();
    stale = s;
    phase = 'stale';
  }

  onMount(() => {
    plan();
    invoke('work_node', { id: roots[0], opts: { children: true } })
      .then((n) => {
        root = n;
        if (!teams && n?.project) loadProject(n.project);
      })
      .catch(() => {});
  });

  let loadedProject = null;
  function loadProject(id) {
    if (loadedProject) return;
    loadedProject = id;
    programTeams(id).then((t) => (teams = t));
    invoke('tracker_settings', { project: id })
      .then((r) => (tracker = r?.settings ?? null))
      .catch(() => {});
  }

  // ---- the dialog

  const close = () => {
    if (app.overlay === o) app.overlay = null;
  };
  function tryClose() {
    if (ask) return (ask = null);
    if (cutFor) return (cutFor = null);
    if (briefKey) return (briefKey = null);
    if (phase === 'planned' && P.editedCount(draft, base)) return (ask = { text: 'Discard this plan?', yes: 'Discard', no: 'Keep editing', onYes: close });
    close();
  }

  /** A control that re-plans (Split into, Order): asks first when rows were edited (§6.2). */
  function replan(set) {
    const go = () => {
      set();
      plan({ replan: true });
    };
    const n = P.editedCount(draft, base);
    if (phase === 'planned' && n) return (ask = { text: `Re-plan and lose your edits to ${plural(n, 'feature')}?`, yes: 'Re-plan', no: 'Keep edits', onYes: go });
    go();
  }

  /** A radio's change: its group shows the state again until the re-plan runs (Keep edits leaves it as it was). */
  function pick(e, current, fn) {
    for (const r of e.currentTarget.closest('[role="radiogroup"]')?.querySelectorAll('input') ?? []) r.checked = r.value === current;
    fn();
  }
  const setGrouping = (v) => v !== grouping && replan(() => (grouping = v));
  const setSequential = (v) => v !== sequential && replan(() => (sequential = v));
  function setAtOnce(v) {
    atOnce = Number(v);
    if (draft) draft.max_parallel = atOnce;
  }

  // ---- rows

  const feats = $derived(draft?.features ?? []);
  const edit = (fn) => {
    draft = fn(draft);
    startError = null;
  };
  const skip = (key) => edit((p) => P.toggleSkip(p, key));
  /** A row merges into the one above unless it, or that one, is skipped (their items would run inside a skipped row, or not at all). */
  const canMerge = (p, key) => {
    const n = P.numberOf(p, key);
    return n > 1 && !p.features[n - 1].skip && !p.features[n - 2].skip;
  };
  const toggleMode = (f) => edit((p) => P.setMode(p, f.key, f.mode === 'team' ? 'solo' : 'team', teams ?? []));
  function merge(key) {
    const n = P.numberOf(draft, key);
    if (!canMerge(draft, key)) return;
    edit((p) => P.mergeIntoPrevious(p, key));
    sel = n - 2; // the row it joined
  }
  async function openBrief(f) {
    briefKey = f.key;
    briefText = f.brief ?? '';
    await tick();
    /** @type {HTMLElement | null} */ (document.querySelector('[data-brief-editor]'))?.focus();
  }
  function briefDone() {
    if (briefKey) edit((p) => P.setBrief(p, briefKey, briefText));
    briefKey = null;
  }
  function briefKeys(e) {
    const done = e.key === 'Enter' && e.metaKey;
    if (e.key !== 'Escape' && !done) return;
    e.preventDefault();
    e.stopPropagation();
    if (done) briefDone();
    else briefKey = null;
  }
  const rowMenu = (f) => [
    { id: 'skip', label: f.skip ? 'Unskip' : 'Skip', key: 'x', run: () => skip(f.key) },
    { id: 'merge', label: 'Merge into previous', key: '⇧M', disabled: !canMerge(draft, f.key), reason: P.numberOf(draft, f.key) > 1 && !f.skip ? 'The previous feature is skipped' : undefined, run: () => merge(f.key) },
    { id: 'brief', label: 'Edit brief', key: 'e', run: () => openBrief(f) },
    ...P.depsOf(draft, f.key).map((d) => ({ id: `cut-${d.key}`, label: `Remove dependency on ${d.n}`, run: () => edit((p) => P.cutEdge(p, d.key, f.key)) })),
  ];

  // ---- issue fixes (§6.4)

  function fix(i, id) {
    switch (id) {
      case 'cut':
        cutFor = i.kind;
        cutPick = 0;
        return;
      case 'sequential':
        return setSequential(true);
      case 'include':
        return edit((p) => P.includeItems(p, i.items ?? []));
      case 'feature': {
        const f = featureBySlug(project, i.feature);
        close();
        if (f) openFeature(f.id);
        else toast(`${i.feature ?? 'The feature'} is not in this app.`);
        return;
      }
      case 'solo':
        for (const k of i.features ?? []) edit((p) => P.setMode(p, k, 'solo'));
        return;
      case 'team':
        close();
        openCanvas(null, project);
        return;
    }
  }
  function applyCut() {
    const e = issues.find((x) => x.kind === cutFor)?.edges?.[cutPick];
    if (e) edit((p) => P.cutEdge(p, e[0], e[1]));
    cutFor = null;
  }
  const edgeLabel = (e) => {
    const [a, b] = [P.numberOf(draft, e[0]), P.numberOf(draft, e[1])];
    const why = P.blocksText((draft.edges ?? []).find((x) => x.from === e[0] && x.to === e[1])?.items ?? []);
    return `${b} after ${a}${why ? ` (${why})` : ''}`;
  };

  // ---- start (⌘⏎)

  async function start() {
    if (!canStart) return;
    if (briefKey) briefDone();
    starting = true;
    startError = null;
    try {
      const r = await invoke('program_start', { program: $state.snapshot({ ...draft, max_parallel: sequential ? 1 : atOnce, sequential }) });
      if (r?.program) {
        close();
        toast(`Started ${r.program.title} · ${plural(P.liveCount(r.program), 'feature')}`);
        return;
      }
      // The tree changed: the dialog stays and the block shows what is left (no toast).
      coreIssues = r?.issues ?? [];
    } catch (e) {
      failed(e, () => (startError = errText(e)));
    } finally {
      starting = false;
    }
  }

  // ---- keys (§12.3): in a text field single keys type

  function onKey(e) {
    if (e.defaultPrevented) return;
    if (e.key === 'Enter' && e.metaKey) {
      e.preventDefault();
      if (ask) return;
      return start();
    }
    if (ask) {
      if (e.key === 'Enter') {
        e.preventDefault();
        confirmAsk();
      }
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey || isTypingTarget(e.target) || phase !== 'planned') return;
    const k = e.key;
    const f = feats[sel];
    const run = (fn) => (e.preventDefault(), fn());
    if (k === 'j' || k === 'ArrowDown') return run(() => (sel = Math.min(feats.length - 1, sel + 1)));
    if (k === 'k' || k === 'ArrowUp') return run(() => (sel = Math.max(0, sel - 1)));
    if (!f) return;
    if (k === 'x') return run(() => skip(f.key));
    if (k === 'm') return run(() => toggleMode(f));
    if (k === 'M' && e.shiftKey) return run(() => merge(f.key));
    if (k === 'e') return run(() => openBrief(f));
  }

  function confirmAsk() {
    const yes = ask.onYes;
    ask = null;
    yes();
  }

  function onClose(reason) {
    if (reason === 'esc') return tryClose();
    close();
  }
</script>

<FloatingWindow title="Run as program · {title}" size={{ w: width, h: 720 }} minW={640} onclose={onClose} onkeydown={onKey}>
  <div class="pd" class:dim={!!ask} data-plan-phase={phase}>
    <p class="from muted" data-from>
      {fromLine}
      {#if root?.url && single}<button type="button" class="link" aria-label="Open {rootRef} in {trackerWord}" onclick={() => openUrl(root.url)}>↗</button>{/if}
    </p>
    {#if phase === 'stale'}
      <div class="banner" role="alert" data-stale>
        <span><span aria-hidden="true">{'\u{F0026}'}</span> {P.staleText(stale, rootRef)}</span>
        <span class="grow"></span>
        {#if stale?.program}<button type="button" class="btn" onclick={() => { const id = stale.program; close(); openProgram(id); }}>Open program</button>{/if}
        <button type="button" class="btn" onclick={close}>Close</button>
      </div>
    {/if}
    {#if closedRoot && phase !== 'stale'}<p class="muted" data-closed-root>{rootRef} was closed in {trackerWord}. Its open sub-issues can still run.</p>{/if}

    <div class="controls" class:off={locked}>
      <span class="lab">Split into</span>
      <div class="radios" role="radiogroup" aria-label="Split into">
        {#each P.splitChoices(container, hasGrand) as c (c.value)}
          <label class="radio" class:disabled={c.disabled} use:tooltip={c.disabled ? c.reason : null}>
            <input type="radio" name="grouping" value={c.value} checked={grouping === c.value} disabled={c.disabled || locked} onchange={(e) => pick(e, grouping, () => setGrouping(c.value))} />
            {c.label}
          </label>
        {/each}
      </div>
      <span class="lab">Order</span>
      <div class="radios" role="radiogroup" aria-label="Order">
        <label class="radio"><input type="radio" name="order" value="deps" checked={!sequential} disabled={locked} onchange={(e) => pick(e, order, () => setSequential(false))} /> By dependencies</label>
        <label class="radio"><input type="radio" name="order" value="seq" checked={sequential} disabled={locked} onchange={(e) => pick(e, order, () => setSequential(true))} /> One after the other, in tree order</label>
      </div>
      <span class="lab">At once</span>
      <div class="atonce">
        <span class="sel" use:tooltip={sequential ? 'One after the other runs one at a time' : null}>
          <Select value={sequential ? 1 : atOnce} options={P.AT_ONCE} width="hug" disabled={sequential || locked} aria-label="Features running at most" onchange={setAtOnce} />
        </span>
        <span class="muted">{sequential ? 'One after the other runs one at a time' : 'features running at most · Each running feature uses its own team.'}</span>
      </div>
    </div>

    {#if phase === 'planning'}
      <div class="mid center muted" aria-busy="true" data-planning><span class="spin" aria-hidden="true">{'\u{F0450}'}</span> Planning {o?.title || title}…</div>
    {:else if phase === 'failed'}
      <div class="mid" data-plan-failed>
        <p class="err">Could not plan {o?.title || title}: {error}</p>
        <button type="button" class="btn" onclick={() => plan()}>Retry</button>
      </div>
    {:else if phase === 'nothing'}
      {@const lo = base?.left_out ?? { closed: [], running: [] }}
      <div class="mid" data-nothing>
        <p>Nothing left to run under {rootRef}: {[lo.closed?.length && `${lo.closed.length} closed`, lo.running?.length && `${lo.running.length} running`].filter(Boolean).join(', ') || 'no open items'}.</p>
      </div>
    {:else if draft}
      {#if issues.length}
        <section class="issues" aria-label="Plan issues" data-issues>
          <b class:block={blocking > 0}>{P.issuesHeader(issues)}</b>
          {#each issues as i, n (n)}
            <div class="issue" class:blocking={i.blocking} data-issue={i.kind}>
              <span class="itext"><span aria-hidden="true">{'\u{F0026}'}</span> {P.issueText(i, draft, { tracker: trackerWord, project: pname })}</span>
              <span class="fixes">
                {#each P.issueFixes(i, draft) as x (x.id)}
                  <button type="button" class="btn small" data-fix={x.id} use:tooltip={x.key ? { label: x.label, key: x.key } : null} onclick={() => fix(i, x.id)}>{x.label}</button>
                {/each}
              </span>
              {#if cutFor === i.kind && i.kind === 'cycle'}
                <div class="popover" role="dialog" aria-label="Cut one edge of the loop" data-cut>
                  <b>Cut one edge of the loop</b>
                  {#each i.edges as e, k (k)}
                    <label class="radio"><input type="radio" name="cut" checked={cutPick === k} onchange={() => (cutPick = k)} /> {edgeLabel(e)}</label>
                  {/each}
                  <div class="row-end">
                    <button type="button" class="btn ghost small" onclick={() => (cutFor = null)}>Cancel</button>
                    <button type="button" class="btn primary small" onclick={applyCut}>Apply</button>
                  </div>
                </div>
              {/if}
            </div>
          {/each}
        </section>
      {/if}

      <div class="table" class:dimrows={phase === 'replanning'} role="grid" aria-label="Features of the program" data-rows>
        <div class="th" role="row">
          <span role="columnheader">#</span><span role="columnheader">Feature</span><span role="columnheader">From</span><span role="columnheader">Mode</span><span role="columnheader">Team</span><span role="columnheader">Estimate</span><span role="columnheader">Order</span><span></span>
        </div>
        {#each feats as f, i (f.key)}
          {@const note = P.orderNote(draft, f)}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_interactive_supports_focus -->
          <div class="tr" class:sel={sel === i} class:skipped={f.skip} class:hover={hoverKey === f.key} role="row" aria-selected={sel === i} data-row={f.key} onclick={() => (sel = i)}>
            <span class="num muted">{i + 1}</span>
            <span class="feat"><b class="mono">{f.slug}</b><span class="muted">{f.title}</span></span>
            <span class="mono" use:tooltip={P.fromTip(f) || null}>{P.fromText(f)}</span>
            <span><Select value={f.mode} options={P.MODES} width="hug" bare aria-label="Mode of {f.slug}" onchange={(v) => edit((p) => P.setMode(p, f.key, v, teams ?? []))} /></span>
            <span>
              {#if f.mode === 'solo'}<span class="muted">—</span>
              {:else}<Select value={f.team} options={teamOptions} placeholder="— none" width="hug" bare noun="team" aria-label="Team of {f.slug}" onchange={(v) => edit((p) => P.setTeam(p, f.key, v))} />{/if}
            </span>
            <span class:struck={f.skip}>{P.estText(f.estimate)}</span>
            <span class="order" use:tooltip={P.orderTip(draft, f)}>
              <span>{#if flagged.has(f.key)}<span class="mark" aria-label="Has an issue">{'\u{F0026}'}</span> {/if}{P.orderText(draft, f)}</span>
              {#if note}<span class="muted note">{note}</span>{/if}
            </span>
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
            <span class="menuwrap" onclick={(e) => e.stopPropagation()}><Menu items={rowMenu(f)} tabindex={-1} label="More for {f.slug}" /></span>
          </div>
          {#if briefKey === f.key}
            <div class="brief" data-brief>
              <textarea class="field mono" rows="10" data-brief-editor bind:value={briefText} onkeydown={briefKeys} aria-label="Brief of {f.slug}"></textarea>
              <span class="muted">Done ⌘⏎ · Esc cancels the edit</span>
            </div>
          {/if}
        {/each}
      </div>

      <GraphStrip program={draft} onhover={(k) => (hoverKey = k)} />
    {/if}

    {#if draft && phase !== 'planning'}
      <div class="totals">
        <p data-totals>{totals.total}{#if totals.room}<span class="muted">{` · ${totals.room}`}</span>{/if}{#if totals.over}<span class="warn">{` · ${totals.over}`}</span>{/if}</p>
        <p class="muted">Each feature still asks you to approve its spec and its PR.</p>
        {#if wbLine}<p class="muted" data-writeback>{wbLine}</p>{/if}
      </div>
    {/if}
    {#if startError}<p class="err" role="alert" data-start-error>Could not start {title}: {startError}</p>{/if}

    <div class="foot">
      <span class="muted left" data-foot>{draft && phase !== 'planning' && phase !== 'nothing' ? P.footLine(draft) : ''}</span>
      <button type="button" class="btn ghost" onclick={tryClose} use:tooltip={{ label: 'Cancel', key: 'esc' }}>Cancel</button>
      {#if phase === 'nothing'}
        <button type="button" class="btn primary" onclick={close}>Close</button>
      {:else}
        <button type="button" class="btn primary" data-start disabled={!canStart} use:tooltip={blocking ? `Fix ${plural(blocking, 'thing')} first` : { label: 'Start program', key: '⌘⏎' }} onclick={start}>{starting ? 'Starting…' : P.startLabel(draft)}</button>
      {/if}
    </div>
  </div>
  {#if ask}
    <div class="ask" role="alertdialog" aria-label={ask.text} data-ask>
      <p>{ask.text}</p>
      <div class="row-end">
        <button type="button" class="btn ghost" onclick={() => (ask = null)} use:tooltip={{ label: ask.no, key: 'esc' }}>{ask.no}</button>
        <!-- svelte-ignore a11y_autofocus -->
        <button type="button" class="btn primary" autofocus onclick={confirmAsk} use:tooltip={{ label: ask.yes, key: '⏎' }}>{ask.yes}</button>
      </div>
    </div>
  {/if}
</FloatingWindow>

<style>
  .pd {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    font: var(--text-label);
  }
  .pd.dim {
    opacity: 0.5;
  }
  .from {
    margin: 0;
  }
  .controls {
    display: grid;
    grid-template-columns: 96px 1fr;
    align-items: center;
    row-gap: var(--space-xs);
    padding-bottom: var(--space-sm);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  .controls.off {
    opacity: 0.6;
  }
  .lab {
    color: var(--color-text-secondary);
  }
  .radios {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-md);
  }
  .radio {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .radio.disabled {
    color: var(--color-text-disabled);
  }
  .atonce {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .mid {
    min-height: 120px;
    padding: var(--space-md) 0;
  }
  .center {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-xs);
  }
  .issues {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding-bottom: var(--space-sm);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  .issues b.block {
    color: var(--color-status-error);
  }
  .issue {
    position: relative;
    display: flex;
    align-items: flex-start;
    gap: var(--space-sm);
    color: var(--color-pacing-amber);
  }
  .issue.blocking {
    color: var(--color-status-error);
  }
  .itext {
    flex: 1;
    min-width: 0;
  }
  .fixes {
    display: flex;
    gap: var(--space-xs);
    flex: none;
  }
  .btn.small {
    padding: 0 var(--space-sm);
    min-height: 22px;
  }
  .popover {
    position: absolute;
    right: 0;
    top: 100%;
    z-index: 2;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    min-width: 260px;
    padding: var(--space-sm) var(--space-md);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
    box-shadow: var(--shadow-overlay);
  }
  .row-end {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-xs);
    margin-top: var(--space-xs);
  }
  .table {
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    max-height: 320px;
  }
  .table.dimrows {
    opacity: 0.5;
  }
  .th,
  .tr {
    display: grid;
    grid-template-columns: 24px minmax(160px, 2.2fr) 70px 80px minmax(90px, 1.2fr) 90px minmax(110px, 1.3fr) 24px;
    gap: var(--space-sm);
    align-items: start;
    padding: var(--space-xs) var(--space-xs);
  }
  .th {
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .tr {
    border: var(--border-width-default) solid transparent;
    border-radius: var(--radius-sm);
  }
  .tr.sel {
    border-color: var(--color-focus);
  }
  .tr.hover {
    background: var(--color-surface-raised);
  }
  .tr.skipped {
    color: var(--color-text-muted);
  }
  .tr.skipped b {
    color: var(--color-text-muted);
  }
  .feat {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .feat b,
  .feat span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .struck {
    text-decoration: line-through;
  }
  .order {
    display: flex;
    flex-direction: column;
  }
  .order .note {
    font: var(--text-caption);
  }
  .mark {
    color: var(--color-status-error);
  }
  .brief {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    margin: 0 var(--space-xs) var(--space-sm) 32px;
  }
  .totals p {
    margin: 0;
  }
  .totals {
    padding-top: var(--space-sm);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
  .warn {
    color: var(--color-pacing-amber);
  }
  p.err {
    margin: 0;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-sm);
    border: var(--border-width-default) solid var(--color-status-error);
    border-radius: var(--radius-sm);
    color: var(--color-status-error);
    background: color-mix(in srgb, var(--color-status-error) 15%, transparent);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .foot .left {
    flex: 1;
    min-width: 0;
  }
  .ask {
    position: absolute;
    left: 50%;
    top: 40%;
    transform: translate(-50%, -50%);
    z-index: 3;
    min-width: 360px;
    padding: var(--space-md);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-surface-raised);
    box-shadow: var(--shadow-overlay);
  }
  .ask p {
    margin: 0 0 var(--space-sm);
  }
</style>
