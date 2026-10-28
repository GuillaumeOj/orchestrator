<script>
  // Board (FR-502, Penpot 04 Board · 1512): a project's tasks in five columns,
  // the same over every backend (Local, GitHub Issues). Keys: h/l column, j/k
  // card, ⇧h/⇧l move the card to the previous / next status, ⏎ open its
  // feature, a assign to a node, n new task, r sync with the backend, ⇧O run
  // the card off-hours (or back in working hours), space the card menu (P3).
  // Linear-backed boards mark the cards where Linear's change won (FR-503).
  import { onMount } from 'svelte';
  import AgentChip from '../components/AgentChip.svelte';
  import Kbd from '../components/Kbd.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import { app, openFeature, refresh, toast } from '../lib/app.svelte.js';
  import { ago, clock } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { nextOffHours, OFF_DISABLED, offHoursAllowed } from '../lib/pacing.js';
  import { blockedBy, COLUMNS, handle, moveCursor, shiftStatus } from '../lib/teams.js';

  let { paneId } = $props();
  const spec = $derived(app.panes[paneId]);
  let v = $state(null);
  let error = $state(null);
  let feature = $state('all');
  let cur = $state({ col: 1, row: 0 });
  let form = $state(null); // { title, node, run_id, body } for n; { task, node } for a
  let syncing = $state(false);
  let menu = $state(false); // the selected card's menu (Open task, Run off-hours, Assign node)
  let offHours = $state(null); // planner_view.off_hours (the allowance and the next window)
  let offTasks = $state({}); // task id → starts_at, for the tasks scheduled from here
  let root;

  const tasks = $derived((v?.tasks ?? []).filter((t) => feature === 'all' || t.feature_id === feature));
  const cols = $derived(COLUMNS.map((c) => tasks.filter((t) => t.status === c.id)));
  const selected = $derived(cols[cur.col]?.[cur.row] ?? null);
  const runOf = (t) => v?.runs.find((r) => r.id === t?.team_run_id);
  const nodeRole = (t) => runOf(t)?.nodes.find((n) => n.id === t.node)?.role ?? t.node;

  async function load() {
    try {
      v = await invoke('board_view', { project: spec.projectId });
      error = null;
    } catch (e) {
      error = errText(e);
    }
  }

  async function update(t, patch) {
    try {
      await invoke('task_update', { id: t.id, status: patch.status ?? null, node: patch.node ?? null, note: patch.note ?? null });
      await load();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
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

  /** Whether a task runs off-hours: the core's flag when it has one, else what was set here. */
  const isOff = (t) => !!(t?.off_hours ?? offTasks[t?.id]);

  async function toggleOffHours(t) {
    menu = false;
    if (!t) return;
    if (!offHoursAllowed(offHours)) return toast(OFF_DISABLED);
    const off = !isOff(t);
    try {
      const r = await invoke('queue_off_hours', { task: String(t.id), off });
      offTasks = { ...offTasks, [t.id]: off ? r.starts_at : null };
      toast(off ? `${handle(t)} runs off-hours · starts ${clock(Date.parse(r.starts_at))}` : `${handle(t)} runs in working hours`);
      await load();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }

  async function sync() {
    syncing = true;
    try {
      const r = await invoke('board_sync', { project: spec.projectId });
      const remote = { github: 'GitHub', linear: 'Linear' }[v?.backend];
      toast(
        v?.backend === 'linear'
          ? `Synced ${v.project.name} with Linear · ${r.pushed ?? 0} pushed${r.imported || r.updated ? ` · ${(r.imported ?? 0) + (r.updated ?? 0)} pulled` : ''}${r.conflicts ? ` · ${r.conflicts} conflict${r.conflicts === 1 ? '' : 's'} (Linear won)` : ''}`
          : remote
            ? `Synced with ${remote} · ${r.imported} new · ${r.updated} updated${r.conflicts ? ` · ${r.conflicts} conflict(s): ${remote}'s version kept` : ''}`
            : 'Local tasks: nothing to sync',
      );
      await load();
      await refresh();
    } catch (e) {
      toast(`Sync: ${errText(e)}`, { kind: 'error' });
    } finally {
      syncing = false;
    }
  }

  function onKey(e) {
    if (e.target.closest('input, textarea, select')) {
      if (e.key === 'Escape') ((form = null), root?.focus(), e.preventDefault());
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (menu && e.key === 'Escape') return ((menu = false), e.preventDefault());
    if (e.key === 'O' || (e.shiftKey && e.key === 'o')) return (toggleOffHours(selected), e.preventDefault());
    if (e.key === ' ' && selected) return ((menu = !menu), e.preventDefault());
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
    else if (key === 'r') sync();
    else if (key === 'Escape') form = null;
    else return;
    e.preventDefault();
    queueMicrotask(() => root?.querySelector('.task.sel')?.scrollIntoView({ block: 'nearest' }));
  }

  onMount(() => {
    load();
    invoke('planner_view')
      .then((pv) => (offHours = pv?.off_hours ?? null))
      .catch(() => {});
    const t = setInterval(load, 5000);
    return () => clearInterval(t);
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="board" tabindex="0" data-pane-focus bind:this={root} onkeydown={onKey} role="application" aria-label="Board">
  {#if error}
    <p class="muted pad">Could not load the board: {error}</p>
  {:else if v}
    <header class="bh">
      <span class="pname">{v.project.name}</span>
      <span class="muted">{v.backend === 'github' ? `GitHub Issues${v.repo ? ` · ${v.repo}` : ''} · remote wins on conflict` : v.backend === 'linear' ? `Linear${v.linear_team ? ` · ${v.linear_team}` : ''}${v.last_sync_at_ms ? ` · synced ${clock(v.last_sync_at_ms)}` : ''}` : 'Local tasks'}</span>
      {#if v.last_sync_error}<span class="err">backend disconnected: {v.last_sync_error}</span>{/if}
      <span class="grow"></span>
      <div class="chips">
        <button class="chip" class:on={feature === 'all'} onclick={() => (feature = 'all')}>all features</button>
        {#each v.features as f (f.id)}<button class="chip" class:on={feature === f.id} onclick={() => (feature = f.id)}>{f.slug}</button>{/each}
      </div>
      <button class="btn ghost" disabled={syncing} onclick={sync}>{syncing ? 'Syncing…' : 'Sync'} <Kbd keys="r" /></button>
      <button class="btn" onclick={() => (form = { kind: 'new', title: '', node: '', run_id: v.runs[0]?.id ?? '', body: '' })}>New task <Kbd keys="n" /></button>
    </header>

    {#if form?.kind === 'new'}
      <div class="form">
        <!-- svelte-ignore a11y_autofocus -->
        <input class="field grow" autofocus bind:value={form.title} placeholder="Task title" onkeydown={(e) => e.key === 'Enter' && (e.preventDefault(), create())} />
        <select class="field" bind:value={form.run_id} aria-label="Feature">
          <option value="">no feature</option>
          {#each v.runs as r (r.id)}<option value={r.id}>{r.slug}</option>{/each}
        </select>
        {#if form.run_id}
          <select class="field" bind:value={form.node} aria-label="Node">
            <option value="">unassigned</option>
            {#each v.runs.find((r) => r.id === form.run_id)?.nodes.filter((n) => !n.lead) ?? [] as n (n.id)}<option value={n.id}>{n.id}</option>{/each}
          </select>
        {/if}
        <button class="btn primary" onclick={create}>Create</button>
        <button class="btn ghost" onclick={() => (form = null)}>Cancel</button>
      </div>
    {:else if form?.kind === 'assign'}
      <div class="form">
        <span>Assign {handle(form.task)} to</span>
        <!-- svelte-ignore a11y_autofocus -->
        <select class="field" autofocus bind:value={form.node} onchange={() => (update(form.task, { node: form.node }), (form = null), root?.focus())}>
          <option value="" disabled>node…</option>
          {#each runOf(form.task)?.nodes.filter((n) => !n.lead) ?? [] as n (n.id)}<option value={n.id}>{n.id}</option>{/each}
        </select>
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
              <div class="tt"><span class="muted mono">{handle(t)}</span><span class="grow"></span><span class="muted">{t.backend === 'github' ? 'GitHub' : t.backend === 'linear' ? 'Linear' : 'Local'}</span></div>
              <div class="title">{t.title}</div>
              {#if t.remote_won}<span class="won" title="Changed in Linear and in the app since the last sync; the app now shows Linear's.">↓ changed in {t.backend === 'linear' ? 'Linear' : 'GitHub'}</span>{/if}
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
            {#if menu && cur.col === ci && cur.row === ri}
              <div class="menu" role="menu" aria-label="Task menu">
                <button role="menuitem" onclick={() => ((menu = false), t.feature_id && openFeature(t.feature_id))}>Open task <Kbd keys="⏎" /></button>
                <button role="menuitem" disabled={!offHoursAllowed(offHours)} title={offHoursAllowed(offHours) ? '' : OFF_DISABLED} onclick={() => toggleOffHours(t)}>{isOff(t) ? 'Run in working hours' : 'Run off-hours'} <Kbd keys="⇧+O" /></button>
                <button role="menuitem" disabled={!runOf(t)} onclick={() => ((menu = false), (form = { kind: 'assign', task: t, node: t.node ?? '' }))}>Assign node… <Kbd keys="a" /></button>
              </div>
              {#if !offHoursAllowed(offHours)}<p class="muted hint">{OFF_DISABLED}</p>{/if}
            {/if}
          {:else}
            <p class="muted hint">{c.id === 'ready' ? 'Tasks whose dependencies are done start by themselves.' : 'Nothing here.'}</p>
          {/each}
        </section>
      {/each}
    </div>
    <footer class="keys">
      <span><Kbd keys="h/l" /> column</span><span><Kbd keys="j/k" /> card</span><span><Kbd keys="⇧+h/l" /> move card</span><span><Kbd keys="⏎" /> open</span><span><Kbd keys="a" /> assign</span><span><Kbd keys="n" /> new</span><span><Kbd keys="r" /> sync</span><span><Kbd keys="space" /> menu</span><span><Kbd keys="⇧+O" /> off-hours</span>
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
  .menu {
    display: flex;
    flex-direction: column;
    padding: var(--space-xs);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
  }
  .menu button {
    display: flex;
    justify-content: space-between;
    gap: var(--space-md);
    padding: var(--space-xs) var(--space-sm);
    background: none;
    border: 0;
    border-radius: var(--radius-sm);
    color: var(--color-text-primary);
    font: var(--text-label);
    text-align: left;
    cursor: pointer;
  }
  .menu button:disabled {
    color: var(--color-text-disabled);
    cursor: default;
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
  .hint {
    padding: 0 var(--space-sm);
    font: var(--text-caption);
  }
  .pad {
    padding: var(--space-xl);
  }
</style>
