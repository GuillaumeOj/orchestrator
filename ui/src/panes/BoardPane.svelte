<script>
  // Board (FR-502, Penpot 04 Board · 1512): a project's tasks in five columns,
  // the same over every backend (Local, GitHub Issues). Keys: h/l column, j/k
  // card, ⇧h/⇧l move the card to the previous / next status, ⏎ open its
  // feature, a assign to a node, n new task, e edit title/body, g open the issue on GitHub/Linear, r sync with the backend, ⇧O run
  // the card off-hours (or back in working hours), space the card menu (P3).
  // Linear-backed boards mark the cards where Linear's change won (FR-503).
  import { onMount } from 'svelte';
  import AgentChip from '../components/AgentChip.svelte';
  import Menu from '../components/Menu.svelte';
  import Select from '../components/Select.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import TaskModelChip from '../components/TaskModelChip.svelte';
  import { app, openFeature, openUrl, queueOffHours, refresh, toast } from '../lib/app.svelte.js';
  import { ago, clock } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { isTypingTarget } from '../lib/keyboard.js';
  import { backendLabel, remoteName, syncCounts } from '../lib/linear.js';
  import { onTopics } from '../lib/now.svelte.js';
  import { nextOffHours, OFF_DISABLED, offHoursAllowed } from '../lib/pacing.js';
  import { blockedBy, COLUMNS, editPatch, handle, moveCursor, remoteLinkLabel, shiftStatus } from '../lib/teams.js';
  import { tooltip } from '../lib/tooltip.js';

  let { paneId } = $props();
  const spec = $derived(app.panes[paneId]);
  let v = $state(null);
  let error = $state(null);
  let feature = $state('all');
  let cur = $state({ col: 1, row: 0 });
  let form = $state(null); // { title, node, run_id, body } for n; { task, node } for a; { task, title, body } for e
  let syncing = $state(false);
  /** @type {any} the selected card's `⋯` Menu (space opens it, m on its Move to). */
  let cardMenu = $state();
  let offHours = $state(null); // planner_view.off_hours (the allowance and the next window)
  let root;

  const tasks = $derived((v?.tasks ?? []).filter((t) => feature === 'all' || t.feature_id === feature));
  const cols = $derived(COLUMNS.map((c) => tasks.filter((t) => t.status === c.id)));
  const selected = $derived(cols[cur.col]?.[cur.row] ?? null);
  const featureOptions = $derived([{ value: '', label: 'no feature' }, ...(v?.runs ?? []).map((r) => ({ value: r.id, label: r.slug }))]);
  /** A run's assignable nodes (never the lead), with an optional empty choice first. */
  const nodeOptions = (run, none) => [...(none ? [{ value: '', label: none }] : []), ...(run?.nodes ?? []).filter((n) => !n.lead).map((n) => ({ value: n.id, label: n.id }))];
  /** Puts the keyboard on the picker inside `node` (a Select has no autofocus). */
  const focusBox = (node) => node.querySelector('button')?.focus();
  const runOf = (t) => v?.runs.find((r) => r.id === t?.team_run_id);
  const nodeRole = (t) => runOf(t)?.nodes.find((n) => n.id === t.node)?.role ?? t.node;

  async function load() {
    try {
      v = await invoke('board_view', { project: spec.projectId });
      // Opened on a task (a conflict note's card): select its card once.
      if (spec.taskId != null) {
        const want = String(spec.taskId);
        feature = 'all';
        const ci = COLUMNS.findIndex((c) => v.tasks.some((t) => t.status === c.id && String(t.id) === want));
        if (ci >= 0) cur = { col: ci, row: v.tasks.filter((t) => t.status === COLUMNS[ci].id).findIndex((t) => String(t.id) === want) };
        app.panes[paneId] = { ...app.panes[paneId], taskId: null };
      }
      error = null;
    } catch (e) {
      error = errText(e);
    }
  }

  async function update(t, patch) {
    try {
      await invoke('task_update', {
        id: t.id,
        status: patch.status ?? null,
        node: patch.node ?? null,
        note: patch.note ?? null,
        title: patch.title ?? null,
        body: patch.body ?? null,
      });
      await load();
      return true;
    } catch (e) {
      toast(errText(e), { kind: 'error' });
      return false;
    }
  }

  async function create() {
    const f = form;
    if (!f?.title?.trim()) return;
    try {
      await invoke('task_create', {
        form: { project_id: spec.projectId, title: f.title.trim(), body: f.body ?? '', node: f.node || null, run_id: f.run_id || null, depends_on: [] },
      });
      form = null;
      toast('Task created');
      await load();
      root?.focus();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }

  function startEdit(t) {
    if (t) form = { kind: 'edit', task: t, title: t.title ?? '', body: t.body ?? '' };
  }

  async function saveEdit() {
    const f = form;
    if (f?.kind !== 'edit') return;
    if (!f.title.trim()) return toast('The task needs a title', { kind: 'error' });
    const patch = editPatch(f.task, f);
    if (patch.title !== null || patch.body !== null) {
      // A failed save keeps the form (and what was typed) open.
      if (!(await update(f.task, patch))) return;
    }
    if (form === f) form = null;
    root?.focus();
  }

  function openRemote(t) {
    if (t?.remote_url) openUrl(t.remote_url);
  }

  /** The task, or its run, is marked "Run off-hours" (board_view). */
  const isOff = (t) => !!t?.off_hours;

  function toggleOffHours(t) {
    if (!t) return;
    if (!offHoursAllowed(offHours)) return toast(OFF_DISABLED);
    queueOffHours({ task: t.id, off: !isOff(t) }, handle(t), load);
  }

  async function sync() {
    syncing = true;
    try {
      const r = await invoke('board_sync', { project: spec.projectId });
      const remote = remoteName(v?.backend);
      toast(v?.backend === 'local' ? 'Local tasks: nothing to sync' : `Synced ${v.project.name} with ${remote} · ${syncCounts(r, remote)}`);
      await load();
      await refresh();
    } catch (e) {
      toast(`Sync: ${errText(e)}`, { kind: 'error' });
    } finally {
      syncing = false;
    }
  }

  // The selected card's menu (04-board-pane): keys right-aligned, the ones that work on the board.
  const menuItems = (t) => [
    { id: 'open', label: 'Open feature', key: '⏎', disabled: !t.feature_id, reason: 'Not part of a feature', run: () => openFeature(t.feature_id) },
    { id: 'off', label: isOff(t) ? 'Run in working hours' : 'Run off-hours', key: '⇧O', disabled: !offHoursAllowed(offHours), reason: OFF_DISABLED, run: () => toggleOffHours(t) },
    { separator: true },
    { id: 'edit', label: 'Edit', key: 'e', run: () => startEdit(t) },
    ...(remoteLinkLabel(t) ? [{ id: 'remote', label: remoteLinkLabel(t), key: 'g', run: () => openRemote(t) }] : []),
    {
      id: 'move',
      label: 'Move to',
      key: 'm',
      items: COLUMNS.map((col, i) => ({ id: col.id, label: col.label, key: String(i + 1), disabled: col.id === t.status, reason: 'Already there', run: () => update(t, { status: col.id }) })),
    },
    { id: 'assign', label: 'Assign', key: 'a', disabled: !runOf(t), reason: 'No active run to assign in', run: () => (form = { kind: 'assign', task: t, node: t.node ?? '' }) },
  ];

  function onKey(e) {
    if (e.defaultPrevented) return; // the card's Menu trigger took the key
    if (isTypingTarget(e.target)) {
      if (e.key === 'Enter' && e.metaKey && form?.kind === 'edit') return (saveEdit(), e.preventDefault());
      if (e.key === 'Enter' && e.metaKey && form?.kind === 'new') return (create(), e.preventDefault());
      if (e.key === 'Escape' && !e.defaultPrevented) ((form = null), root?.focus(), e.preventDefault());
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === 'O' || (e.shiftKey && e.key === 'o')) return (toggleOffHours(selected), e.preventDefault());
    if (e.key === ' ' && selected) return (cardMenu?.show(), e.preventDefault());
    if (e.key === 'm' && selected) return (cardMenu?.show('move'), e.preventDefault());
    const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    const arrows = { ArrowLeft: 'h', ArrowRight: 'l', ArrowUp: 'k', ArrowDown: 'j' };
    const key = arrows[k] ?? k;
    if (e.shiftKey && (key === 'h' || key === 'l') && selected) {
      const s = shiftStatus(selected.status, key === 'h' ? -1 : 1);
      if (s) {
        update(selected, { status: s });
        cur = { col: cur.col + (key === 'h' ? -1 : 1), row: 0 };
      }
    } else if (['h', 'j', 'k', 'l'].includes(key)) cur = moveCursor(cols, cur, key);
    else if (key === 'Enter' && selected?.feature_id) openFeature(selected.feature_id);
    else if (key === 'a' && selected && runOf(selected)) form = { kind: 'assign', task: selected, node: selected.node ?? '' };
    else if (key === 'n') form = { kind: 'new', title: '', node: '', run_id: v?.runs[0]?.id ?? '', body: '' };
    else if (key === 'e' && selected) startEdit(selected);
    else if (key === 'g' && selected?.remote_url) openRemote(selected);
    else if (key === 'r') sync();
    else if (key === 'Escape') form = null;
    else return;
    e.preventDefault();
    queueMicrotask(() => root?.querySelector('.task.sel')?.scrollIntoView({ block: 'nearest' }));
  }

  // Opening the Board (or switching project) syncs a remote backend that is
  // stale; the core throttles it (60 s) and keeps the project in the background sync.
  async function openSync(project) {
    syncing = true;
    try {
      const r = await invoke('board_open_sync', { project });
      if (project !== spec.projectId) return;
      if (r?.synced) await load();
    } catch (e) {
      if (project === spec.projectId) {
        await load();
        if (v) v = { ...v, last_sync_error: errText(e) };
      }
    } finally {
      if (project === spec.projectId) syncing = false;
    }
  }

  // Keyed on the id, not on `spec` (which is replaced when the pane's taskId is cleared).
  const projectId = $derived(spec.projectId);
  $effect(() => {
    const project = projectId;
    if (project) openSync(project);
  });

  onMount(() => {
    invoke('planner_view')
      .then((pv) => (offHours = pv?.off_hours ?? null))
      .catch(() => {});
    // Refetched when the core says this project's board changed (no poll).
    return onTopics([`board:${spec.projectId}`], load);
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="board" tabindex="0" data-pane-focus bind:this={root} onkeydown={onKey} role="application" aria-label="Board">
  {#if error}
    <p class="muted pad">Could not load the board: {error}</p>
  {:else if v}
    <header class="bh">
      <span class="pname">{v.project.name}</span>
      <span class="muted">{v.backend === 'github' ? `${backendLabel('github')}${v.repo ? ` · ${v.repo}` : ''} · remote wins on conflict` : v.backend === 'linear' ? `${backendLabel('linear')}${v.linear_team ? ` · ${v.linear_team}` : ''}${v.last_sync_at_ms ? ` · synced ${clock(v.last_sync_at_ms)}` : ''}` : 'Local tasks'}</span>
      {#if v.last_sync_error}<span class="err">backend disconnected: {v.last_sync_error}</span>{/if}
      <span class="grow"></span>
      <div class="chips">
        <button class="chip" class:on={feature === 'all'} onclick={() => (feature = 'all')}>All features</button>
        {#each v.features as f (f.id)}<button class="chip" class:on={feature === f.id} onclick={() => (feature = f.id)}>{f.slug}</button>{/each}
      </div>
      <button type="button" class="icon-btn" aria-label="Sync" disabled={syncing} use:tooltip={{ label: syncing ? 'Syncing…' : 'Sync', key: 'r' }} onclick={sync}>{'\u{F0450}'}</button>
      <button type="button" class="btn primary" use:tooltip={{ label: 'New task', key: 'n' }} onclick={() => (form = { kind: 'new', title: '', node: '', run_id: v.runs[0]?.id ?? '', body: '' })}>New task</button>
    </header>

    {#if form?.kind === 'new'}
      <div class="form">
        <!-- svelte-ignore a11y_autofocus -->
        <input class="field grow" autofocus bind:value={form.title} placeholder="Task title" onkeydown={(e) => e.key === 'Enter' && !e.metaKey && (e.preventDefault(), create())} />
        <textarea class="field body" rows="4" bind:value={form.body} placeholder="Description (optional) — ⌘⏎ to create" aria-label="Description"></textarea>
        <Select bind:value={form.run_id} options={featureOptions} aria-label="Feature" noun="feature" />
        {#if form.run_id}
          <Select bind:value={form.node} options={nodeOptions(v.runs.find((r) => r.id === form.run_id), 'unassigned')} aria-label="Node" noun="node" />
        {/if}
        <button class="btn primary" onclick={create}>Create</button>
        <button class="btn ghost" onclick={() => (form = null)}>Cancel</button>
      </div>
    {:else if form?.kind === 'edit'}
      <div class="form">
        <span class="mono">Edit {handle(form.task)}</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input class="field grow" autofocus bind:value={form.title} placeholder="Task title" onkeydown={(e) => e.key === 'Enter' && !e.metaKey && (e.preventDefault(), saveEdit())} />
        <textarea class="field body" rows="4" bind:value={form.body} placeholder="Description — ⌘⏎ to save" aria-label="Description"></textarea>
        <button class="btn primary" onclick={saveEdit}>Save</button>
        <button class="btn ghost" onclick={() => ((form = null), root?.focus())}>Cancel</button>
      </div>
    {:else if form?.kind === 'assign'}
      <div class="form">
        <span>Assign {handle(form.task)} to</span>
        <span use:focusBox>
          <Select value={form.node} options={nodeOptions(runOf(form.task))} placeholder="node…" aria-label="Node" noun="node" onchange={(node) => (update(form.task, { node }), (form = null), root?.focus())} />
        </span>
        <button class="btn ghost" onclick={() => (form = null)}>Cancel</button>
      </div>
    {/if}

    <div class="cols">
      {#each COLUMNS as c, ci (c.id)}
        <section class="col" aria-label={c.label}>
          <div class="colh"><StatusBadge state={c.state} compact /> <b>{c.label}</b> <span class="muted">{cols[ci].length}</span></div>
          {#each cols[ci] as t, ri (t.id)}
            {@const blocked = c.id === 'ready' ? blockedBy(t, v.tasks) : []}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
            <div class="task" class:sel={cur.col === ci && cur.row === ri} onclick={() => ((cur = { col: ci, row: ri }), root?.focus())} ondblclick={() => t.feature_id && openFeature(t.feature_id)}>
              <div class="tt">
                <span class="muted mono">{handle(t)}</span><span class="grow"></span><span class="muted">{remoteName(t.backend)}</span>
                {#if cur.col === ci && cur.row === ri}
                  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                  <span class="cmenu" onclick={(e) => e.stopPropagation()}><Menu bind:this={cardMenu} items={menuItems(t)} label="Task menu" /></span>
                {/if}
              </div>
              <div class="title">{t.title} <TaskModelChip task={t} /></div>
              {#if t.remote_won}<span class="won" use:tooltip={`Changed in ${remoteName(t.backend)} and in the app since the last sync; the app now shows ${remoteName(t.backend)}'s.`}>↓ changed in {remoteName(t.backend)}</span>{/if}
              {#if isOff(t)}<span class="offtag">☾ Off-hours{nextOffHours(offHours) ? ` · starts ${clock(nextOffHours(offHours))}` : ''}</span>{/if}
              <div class="tt">
                {#if t.node}<AgentChip id={t.node} role={nodeRole(t)} />{:else}<span class="muted">unassigned</span>{/if}
                <span class="grow"></span>
                {#if blocked.length}<span class="blocked">{'\u{F051F}'} blocked by {blocked.map((d) => `T-${d}`).join(', ')}</span>
                {:else if c.id === 'done' && t.branch}<span class="muted">merged</span>
                {:else if t.branch}<span class="muted mono">{t.branch.split('/').pop()}</span>
                {:else}<span class="muted">—</span>{/if}
              </div>
            </div>
          {:else}
            <p class="muted hint">{c.id === 'ready' ? 'Tasks whose dependencies are done start by themselves.' : 'Nothing here.'}</p>
          {/each}
        </section>
      {/each}
    </div>
    <footer class="keys">
      {#if selected}<span class="grow"></span><span class="muted">{handle(selected)} · updated {ago(selected.updated_at_ms)}</span>{/if}
    </footer>
  {:else}
    <p class="muted pad">Loading…</p>
  {/if}
</div>

<style>

  .won {
    color: var(--color-status-waiting-for-you);
    font: var(--text-caption);
  }
  .offtag {
    color: var(--color-status-done);
    font: var(--text-caption);
  }
  .board {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    padding: var(--space-lg);
    outline: none;
    font: var(--text-label);
  }
  .bh,
  .form,
  .tt,
  .colh,
  .keys {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .keys {
    gap: var(--space-md);
    color: var(--color-text-secondary);
    font: var(--text-caption);
  }
  .form {
    flex-wrap: wrap;
  }
  .body {
    flex-basis: 100%;
    resize: vertical;
    field-sizing: content;
    min-height: 4lh;
  }
  .pname {
    font: var(--text-heading);
    color: var(--color-text-primary);
  }
  .chips {
    display: flex;
    gap: var(--space-xs);
  }
  .chip {
    padding: var(--space-xxs) var(--space-sm);
    background: none;
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
    font: var(--text-caption);
    cursor: pointer;
  }
  .chip.on {
    border-color: var(--color-focus);
    color: var(--color-text-primary);
  }
  .err {
    color: var(--color-status-error);
    font: var(--text-caption);
  }
  .cols {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: var(--space-md);
  }
  .col {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-sm);
    background: var(--color-surface-chrome);
    border-radius: var(--radius-md);
    overflow: auto;
  }
  .colh {
    padding: var(--space-xs) var(--space-sm);
  }
  .task {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    cursor: pointer;
  }
  .task.sel {
    background: var(--color-surface-selected);
    border: var(--border-width-focus) solid var(--color-focus);
  }
  .title {
    font: var(--text-body-strong);
    color: var(--color-text-primary);
  }
  .blocked {
    color: var(--color-status-queued);
    font: var(--text-caption);
  }
  .cmenu {
    display: contents;
  }
  .hint {
    padding: 0 var(--space-sm);
    font: var(--text-caption);
  }
  .pad {
    padding: var(--space-xl);
  }
</style>
