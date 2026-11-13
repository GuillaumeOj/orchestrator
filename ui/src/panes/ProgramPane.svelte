<script>
  // The Program view (improve-intake notes §7.2, boards ii-program-view-*): a program's header (status,
  // Pause / Resume, Cancel), its graph strip with live states, one row per feature, the items new
  // upstream (Add to program with a row editor, Ignore) and At once. Read-only once done or
  // cancelled. Refetched on the `programs` topic (no poll). Keys §12.4.
  import { onMount, tick, untrack } from 'svelte';
  import Menu from '../components/Menu.svelte';
  import GraphStrip from '../components/program/GraphStrip.svelte';
  import Select from '../components/Select.svelte';
  import AppChip from '../components/work/AppChip.svelte';
  import HintRow from '../components/work/HintRow.svelte';
  import { app, findPane, openUrl, showPane, toast } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { isTypingTarget } from '../lib/keyboard.js';
  import { onTopics } from '../lib/now.svelte.js';
  import * as P from '../lib/program.js';
  import { tooltip } from '../lib/tooltip.js';
  import {
    cancelProgram,
    openNodeLink,
    openProgramFeature,
    programTabTitle,
    programTeams,
    skipFeature,
    togglePause,
    updateProgram,
  } from '../lib/work.js';

  let { paneId } = $props();
  const spec = $derived(app.panes[paneId]);
  const id = $derived(spec?.programId);

  let p = $state(null);
  let loadError = $state(null);
  let sel = $state(0);
  let hoverKey = $state(null);
  let nodes = $state({}); // root / new item id → its WorkNode (title, ref, parent, url)
  let teams = $state(/** @type {{ id: string, name: string }[]} */ ([]));
  let teamsLoad = Promise.resolve(); // the row editor's prefill waits for the teams
  let adding = $state(null); // { item, mode, team, after: [keys] }: the row editor
  let busy = $state(false);

  const live = $derived(P.isLive(p));
  const chip = $derived(p ? P.statusChip(p) : null);
  const root = $derived(p ? nodes[p.root?.[0]] : null);
  const tracker = $derived(p?.root?.[0] ? P.trackerOfId(p.root[0]) : 'the tracker');
  const liveCap = $derived(P.liveCapText(app.now.counts?.global));
  const project = $derived(p?.project ?? null);
  const feats = $derived(p?.features ?? []);
  const cur = $derived(feats[sel] ?? null);

  let loadSeq = 0;
  async function load() {
    if (!id) return;
    const n = ++loadSeq;
    try {
      const r = await invoke('program', { id });
      if (n !== loadSeq) return; // a newer load answers
      p = r;
      loadError = null;
      // A tab opened from a card or an outcome knew only the id: it takes the program's title.
      const s = app.panes[paneId];
      if (s && s.programTitle !== p.title) app.panes[paneId] = { ...s, programTitle: p.title, title: programTabTitle(p.title) };
      if (sel >= p.features.length) sel = Math.max(0, p.features.length - 1);
      untrack(fetchNodes);
    } catch (e) {
      loadError = errText(e);
    }
  }

  /** The roots' and the new items' nodes (titles, links), once each. */
  function fetchNodes() {
    for (const id of [...(p?.root ?? []), ...(p?.new_items ?? [])])
      if (!nodes[id])
        invoke('work_node', { id, opts: { children: false } })
          .then((n) => n && (nodes = { ...nodes, [id]: n }))
          .catch(() => {});
  }

  // Loads on mount and on window focus; later only when a `programs` delta upserts or removes THIS program.
  onMount(() => onTopics(() => false, load));
  const NONE = Symbol('unseen');
  let seenRow = /** @type {any} */ (NONE);
  $effect(() => {
    const row = id ? app.now.programs?.[id] : undefined;
    const first = seenRow === NONE;
    const same = row === seenRow;
    seenRow = row;
    if (first || same) return;
    untrack(load);
  });

  // Teams of the program's project, for the row editor (once per project, not per reload).
  $effect(() => {
    if (!project) return;
    teamsLoad = programTeams(project).then((t) => {
      teams = t;
    });
  });

  // A card's Add to program lands here: the row editor of that item (a one-shot request).
  $effect(() => {
    const land = spec?.land;
    if (!land || !p) return;
    untrack(() => {
      app.panes[paneId] = { ...app.panes[paneId], land: undefined };
      const item = typeof land.add === 'string' ? land.add : p.new_items?.[0];
      if (item) openAdd(item);
    });
  });

  const after = (r) => r && (p = r);

  // ---- actions (§7.2)

  async function pause() {
    if (!live || busy) return;
    busy = true;
    after(await togglePause(p));
    busy = false;
  }
  const cancel = () => live && cancelProgram(p, after);
  const skip = (f) => live && P.skippable(f) && skipFeature(p, f, after);

  const openRowFeature = (f) => openProgramFeature(p, f, () => toast(`${f.slug} has not started yet.`));
  const openItem = (f) => f.items?.[0] && openNodeLink(f.items[0], P.refOfId(f.items[0]));

  async function setAtOnce(v) {
    if (!live) return;
    after(await updateProgram(p.id, { max_parallel: Number(v) }));
  }

  /** The row editor of a new upstream item, prefilled: team mode with the first team (else solo), after the features its item is blocked by. */
  async function openAdd(item) {
    const n = nodes[item] ?? (await invoke('work_node', { id: item, opts: { children: false } }).catch(() => null));
    await teamsLoad;
    const blockers = new Set((n?.upstream?.blocked_by ?? []).map((b) => b.id).filter(Boolean));
    const deps = p.features.filter((f) => !f.skip && f.items.some((x) => blockers.has(x))).map((f) => f.key);
    adding = { item, slug: '', mode: teams.length ? 'team' : 'solo', team: teams[0]?.id ?? null, after: deps };
    await tick();
    /** @type {HTMLElement | null} */ (document.querySelector(`[data-pane="${paneId}"] [data-add-editor] button`))?.focus();
  }
  async function add() {
    if (!adding || busy) return;
    busy = true;
    const a = adding;
    const r = await updateProgram(p.id, { add_item: a.item, mode: a.mode, team: a.mode === 'team' ? a.team : null, after: a.after, ...(a.slug.trim() && { slug: a.slug.trim() }) });
    busy = false;
    if (r) {
      after(r);
      adding = null;
      toast(`Added ${P.refOfId(a.item)} to ${p.title}`);
    }
  }
  async function ignore(item) {
    after(await updateProgram(p.id, { ignore_item: item }));
  }

  const rowMenu = (f) => [
    { id: 'open', label: 'Open feature', key: '⏎', disabled: !['running', 'pr', 'merged', 'failed'].includes(f.state), run: () => openRowFeature(f) },
    { id: 'skip', label: 'Skip', key: 'x', disabled: !live || !P.skippable(f), run: () => skip(f) },
    { id: 'item', label: 'Open item', key: 'o', run: () => openItem(f) },
  ];
  const featOptions = $derived(feats.map((f, i) => !f.skip && { value: f.key, label: `${f.slug} (${i + 1})` }).filter(Boolean));
  const teamOptions = $derived(teams.map((t) => ({ value: t.id, label: t.name })));

  // ---- keys (§12.4)

  function back() {
    const m = findPane((s) => s.type === 'mission');
    if (m) showPane(m);
  }

  function onKey(e) {
    if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey) return;
    if (isTypingTarget(e.target)) return;
    const k = e.key;
    const run = (fn) => (e.preventDefault(), fn());
    if (k === 'Escape') return run(() => (adding ? (adding = null) : back()));
    // ⏎ on a focused button is that button's.
    if (k === 'Enter' && e.target.closest?.('button')) return;
    if (!p) return;
    if (k === 'j' || k === 'ArrowDown') return run(() => (sel = Math.min(feats.length - 1, sel + 1)));
    if (k === 'k' || k === 'ArrowUp') return run(() => (sel = Math.max(0, sel - 1)));
    if (k === 'K' && e.shiftKey) return run(cancel);
    if (k === 'p') return run(pause);
    if (k === 'a' && live && p.new_items?.length) return run(() => openAdd(p.new_items[0]));
    if (!cur) return;
    if (k === 'Enter') return run(() => openRowFeature(cur));
    if (k === 'x') return run(() => skip(cur));
    if (k === 'o') return run(() => openItem(cur));
  }

  const hints = $derived([
    ['j k', 'rows'],
    ['⏎', 'open feature'],
    ...(live ? [['x', 'skip']] : []),
    ...(live && p?.new_items?.length ? [['a', 'add item']] : []),
    ['o', 'open item'],
    ...(live ? [['p', p?.status === 'paused' ? 'resume' : 'pause'], ['⇧K', 'cancel']] : []),
    ['Esc', 'back'],
  ]);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="pv" tabindex="0" data-pane-focus onkeydown={onKey} role="region" aria-label="Program {p?.title ?? ''}">
  {#if loadError && !p}
    <div class="empty"><p class="err">Could not load the program: {loadError}</p><button type="button" class="btn" onclick={load}>Retry</button></div>
  {:else if !p}
    <div class="empty muted" aria-busy="true">Loading the program…</div>
  {:else}
    <header class="head">
      <div class="l1">
        <h2>{p.title} · program</h2>
        <span data-status={p.status}><AppChip {chip} /></span>
        <span class="grow"></span>
        {#if live}
          <button type="button" class="btn" class:primary={p.status === 'paused'} disabled={busy} use:tooltip={{ label: p.status === 'paused' ? 'Resume' : 'Pause', key: 'p' }} onclick={pause}>{p.status === 'paused' ? 'Resume' : 'Pause'}</button>
          <button type="button" class="btn" use:tooltip={{ label: 'Cancel program', key: '⇧K' }} onclick={cancel}>Cancel program</button>
        {/if}
      </div>
      <p class="sub muted" data-subline>
        {#each P.viewSubline(p, { rootTitle: root?.title ?? '', where: '' }) as part, i (i)}{#if i}{' · '}{/if}{part}{#if i === 0 && root?.url}{' '}<button type="button" class="link" aria-label="Open {p.root_ref} in {tracker}" onclick={() => openUrl(root.url)}>↗</button>{/if}{/each}
      </p>
    </header>

    <div class="scroll">
      <div class="graph"><GraphStrip program={p} live onhover={(k) => (hoverKey = k)} /></div>

      <div class="table" role="grid" aria-label="Features of {p.title}" data-rows>
        <div class="th" role="row"><span>#</span><span>Feature</span><span>From</span><span>Mode</span><span>State</span><span>Estimate</span><span>Actual</span><span></span></div>
        {#each feats as f, i (f.key)}
          {@const st = P.viewState(f, p, { liveCap })}
          {@const flag = P.flagText(f, tracker)}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_interactive_supports_focus -->
          <div class="tr" class:sel={sel === i} class:hover={hoverKey === f.key} role="row" aria-selected={sel === i} data-row={f.key} data-state={f.state} onclick={() => (sel = i)} ondblclick={() => openRowFeature(f)}>
            <span class="muted">{i + 1}</span>
            <b class="mono">{f.slug}</b>
            <span class="mono" use:tooltip={P.fromTip(f) || null}>{P.fromText(f)}</span>
            <span>{f.mode === 'solo' ? 'Solo' : 'Team'}</span>
            <span class="state {st.tone}">{st.text}{#if flag}<span class="flag">{` · ${flag}`}</span>{/if}</span>
            <span>{P.estText(f.estimate, { unit: false })}</span>
            <span>{P.actualText(f)}</span>
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
            <span class="menuwrap" onclick={(e) => e.stopPropagation()}><Menu items={rowMenu(f)} tabindex={-1} label="More for {f.slug}" /></span>
          </div>
        {/each}
        {#if live}<p class="rowhint muted">row ⋯: Open feature ⏎ · Skip x · Open item o</p>{/if}
      </div>

      {#if live}
        {#each p.new_items ?? [] as item (item)}
          {@const n = nodes[item]}
          {#if adding?.item === item}
            <div class="editor" data-add-editor>
              <div class="eh"><b>Add {P.refOfId(item)}{n?.title ? ` ${n.title}` : ''}</b> <span class="muted">as feature {feats.length + 1}</span></div>
              <div class="fields">
                <div class="f"><span>Slug</span><input class="field mono" bind:value={adding.slug} placeholder="from the title" aria-label="Slug" /></div>
                <div class="f"><span>Mode</span><Select value={adding.mode} options={P.MODES} aria-label="Mode" onchange={(v) => (adding.mode = v)} /></div>
                <div class="f"><span>Team</span>{#if adding.mode === 'team'}<Select value={adding.team} options={teamOptions} placeholder="— none" noun="team" aria-label="Team" onchange={(v) => (adding.team = v)} />{:else}<span class="muted">—</span>{/if}</div>
                <div class="f"><span>After</span><Select value={adding.after[0] ?? ''} options={[{ value: '', label: 'first' }, ...featOptions]} aria-label="After" onchange={(v) => (adding.after = v ? [v] : [])} /></div>
                <span class="grow"></span>
                <button type="button" class="btn primary" disabled={busy} use:tooltip={{ label: 'Add', key: '⏎' }} onclick={add}>Add</button>
                <button type="button" class="btn ghost" use:tooltip={{ label: 'Cancel', key: 'esc' }} onclick={() => (adding = null)}>Cancel</button>
              </div>
            </div>
          {:else}
            <div class="newrow" data-new-item={item}>
              <span>+ New in {tracker}: <b>{P.refOfId(item)}{n?.title ? ` ${n.title}` : ''}</b> <span class="muted">({n?.parent ? `sub-issue of ${P.refOfId(n.parent)}` : `under ${p.root_ref}`}{P.addedText(p.new_items_added?.[item]) ? `, ${P.addedText(p.new_items_added[item])}` : ''})</span></span>
              <span class="grow"></span>
              <button type="button" class="btn primary" use:tooltip={{ label: 'Add to program', key: 'a' }} onclick={() => openAdd(item)}>Add to program</button>
              <button type="button" class="btn" onclick={() => ignore(item)}>Ignore</button>
            </div>
          {/if}
        {/each}
        <div class="settings">
          <span class="muted">Settings</span>
          <span>At once</span>
          <Select value={p.sequential ? 1 : p.max_parallel} options={P.AT_ONCE} width="hug" disabled={p.sequential} aria-label="Features running at most" onchange={setAtOnce} />
          <span class="muted">{p.sequential ? 'One after the other runs one at a time' : 'applies to the next start'}</span>
        </div>
      {:else}
        <p class="muted over" data-over>{P.overLine(p)}</p>
      {/if}
    </div>
    <HintRow {hints} />
  {/if}
</div>

<style>
  .pv {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    outline: none;
    font: var(--text-label);
  }
  .empty {
    margin: var(--space-3xl) auto;
    text-align: center;
  }
  .head {
    padding: var(--space-md) var(--space-lg) var(--space-sm);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  .l1 {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  h2 {
    margin: 0;
    font: var(--text-title);
  }
  .sub {
    margin: var(--space-xs) 0 0;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 0 var(--space-lg) var(--space-md);
  }
  .graph {
    padding: var(--space-sm) 0;
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  .graph:empty {
    display: none;
  }
  .table {
    display: flex;
    flex-direction: column;
    padding-top: var(--space-sm);
  }
  .th,
  .tr {
    display: grid;
    grid-template-columns: 24px minmax(140px, 1.4fr) 60px 56px minmax(220px, 3fr) 80px 100px 24px;
    gap: var(--space-sm);
    align-items: center;
    min-height: var(--size-row);
    padding: 0 var(--space-xs);
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
  .state.working {
    color: var(--color-status-working);
  }
  .state.gate {
    color: var(--color-status-at-gate);
  }
  .state.done {
    color: var(--color-status-done);
  }
  .state.queued {
    color: var(--color-status-queued);
  }
  .state.error {
    color: var(--color-status-error);
  }
  .state.muted {
    color: var(--color-text-muted);
  }
  .flag {
    color: var(--color-pacing-amber);
  }
  .rowhint {
    margin: var(--space-xs) 0 0 32px;
    font: var(--text-caption);
  }
  .newrow,
  .editor {
    margin-top: var(--space-md);
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-sm);
    background: var(--color-surface-raised);
  }
  .newrow {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .editor {
    border: var(--border-width-default) solid var(--color-focus);
  }
  .fields {
    display: flex;
    align-items: flex-end;
    gap: var(--space-md);
    margin-top: var(--space-xs);
  }
  .fields .f {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    color: var(--color-text-secondary);
    font: var(--text-caption);
  }
  .settings {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    margin-top: var(--space-md);
  }
  .over {
    margin: var(--space-md) 0 0;
  }
</style>
