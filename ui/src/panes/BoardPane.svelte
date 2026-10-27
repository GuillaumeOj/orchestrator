<script>
  // Board (FR-502, Penpot 04 Board · 1512): a project's tasks in five columns,
  // the same over every backend (Local, GitHub Issues). Keys: h/l column, j/k
  // card, ⇧h/⇧l move the card to the previous / next status, ⏎ open its
  // feature, a assign to a node, n new task, r sync with the backend.
  import { onMount } from 'svelte';
  import AgentChip from '../components/AgentChip.svelte';
  import Kbd from '../components/Kbd.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import { app, openFeature, refresh, toast } from '../lib/app.svelte.js';
  import { ago } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { blockedBy, COLUMNS, handle, moveCursor, shiftStatus } from '../lib/teams.js';

  let { paneId } = $props();
  const spec = $derived(app.panes[paneId]);
  let v = $state(null);
  let error = $state(null);
  let feature = $state('all');
  let cur = $state({ col: 1, row: 0 });
  let form = $state(null); // { title, node, run_id, body } for n; { task, node } for a
  let syncing = $state(false);
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

  async function sync() {
    syncing = true;
    try {
      const r = await invoke('board_sync', { project: spec.projectId });
      toast(
        v?.backend === 'github'
          ? `Synced with GitHub · ${r.imported} new · ${r.updated} updated${r.conflicts ? ` · ${r.conflicts} conflict(s): GitHub's version kept` : ''}`
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
      <span class="muted">{v.backend === 'github' ? `GitHub Issues${v.repo ? ` · ${v.repo}` : ''} · remote wins on conflict` : 'Local tasks'}</span>
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
              <div class="tt"><span class="muted mono">{handle(t)}</span><span class="grow"></span><span class="muted">{t.backend === 'github' ? 'GitHub' : 'Local'}</span></div>
              <div class="title">{t.title}</div>
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
      <span><Kbd keys="h/l" /> column</span><span><Kbd keys="j/k" /> card</span><span><Kbd keys="⇧+h/l" /> move card</span><span><Kbd keys="⏎" /> open</span><span><Kbd keys="a" /> assign</span><span><Kbd keys="n" /> new</span><span><Kbd keys="r" /> sync</span>
      {#if selected}<span class="grow"></span><span class="muted">{handle(selected)} · updated {ago(selected.updated_at_ms)}</span>{/if}
    </footer>
  {:else}
    <p class="muted pad">Loading…</p>
  {/if}
</div>

<style>
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
