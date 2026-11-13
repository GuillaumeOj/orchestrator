<script>
  // Feature pane › Tasks (notes §9.2, §9.3): the moved Board. Five columns, a card per team task, and the
  // Board's moves on the focused task while this section has focus (`b` focuses it, `Esc` leaves):
  // j k h l move, ⏎ menu, a assign, m move to, 1–5 column, e edit, o open upstream, n new, ⇧H ⇧L shift.
  import { untrack } from 'svelte';
  import { openUrl, toast } from '../../lib/app.svelte.js';
  import { errText, invoke } from '../../lib/ipc.js';
  import { isTypingTarget } from '../../lib/keyboard.js';
  import { remoteName } from '../../lib/linear.js';
  import { taskMarks, tasksSummary } from '../../lib/source.js';
  import { blockedBy, COLUMNS, editPatch, handle, moveCursor, remoteLinkLabel, runFinished, shiftStatus } from '../../lib/teams.js';
  import { tooltip } from '../../lib/tooltip.js';
  import AgentChip from '../AgentChip.svelte';
  import Menu from '../Menu.svelte';
  import Select from '../Select.svelte';
  import TaskModelChip from '../TaskModelChip.svelte';

  /**
   * `v` = feature_view; `land` = { task, conflict, tasks, seq } (one-shot, by `seq`); `onleave` gives the focus back to the pane;
   * `hints` (bindable): the keys of the focused card, for the pane's hint row.
   */
  let { v, reload, land = null, onleave = () => {}, hints = $bindable([]) } = $props();

  let root = $state();
  let cur = $state({ col: 1, row: 0 });
  let form = $state(null); // { kind: 'new', title, body, node } | { kind: 'edit', task, title, body } | { kind: 'assign', task, node }
  let flash = $state(null);
  /** @type {any} */
  let cardMenu = $state();
  let landed = 0;

  const tasks = $derived(v.tasks ?? []);
  const cols = $derived(COLUMNS.map((c) => tasks.filter((t) => t.status === c.id)));
  const selected = $derived(cols[cur.col]?.[cur.row] ?? null);
  /** The run tasks are created and assigned in: a finished run takes no new work. */
  const run = $derived(v.run && !runFinished(v.run.status) ? v.run : null);
  const source = $derived(v.run?.source ?? null);
  const nodes = $derived((v.team?.nodes ?? []).filter((n) => !n.is_lead));
  const nodeOptions = (none) => [...(none ? [{ value: '', label: none }] : []), ...nodes.map((n) => ({ value: n.id, label: n.id }))];
  const focusBox = (node) => node.querySelector('button')?.focus();
  /** The full title in a tooltip, only when the card cuts it. */
  const ifCut = (text) => (/** @type {HTMLElement} */ node) => (node.scrollWidth > node.clientWidth ? text : null);
  const roleOf = (t) => v.team?.nodes?.find((n) => n.id === t.node)?.role_id ?? t.node;

  export function focus() {
    if (!selected) cur = firstCard();
    root?.focus();
    root?.scrollIntoView?.({ block: 'nearest' });
  }

  function firstCard() {
    const col = cols.findIndex((c) => c.length);
    return { col: col < 0 ? 1 : col, row: 0 };
  }

  async function update(t, patch) {
    try {
      await invoke('task_update', { id: t.id, status: patch.status ?? null, node: patch.node ?? null, note: patch.note ?? null, title: patch.title ?? null, body: patch.body ?? null });
      await reload();
      return true;
    } catch (e) {
      toast(errText(e), { kind: 'error' });
      return false;
    }
  }

  async function create() {
    const f = form;
    if (!f?.title?.trim() || !run) return;
    try {
      await invoke('task_create', { form: { project_id: v.feature.project_id, title: f.title.trim(), body: f.body ?? '', node: f.node || null, run_id: run.id, depends_on: [] } });
      form = null;
      toast('Task created');
      await reload();
      root?.focus();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }

  async function saveEdit() {
    const f = form;
    if (f?.kind !== 'edit') return;
    if (!f.title.trim()) return toast('The task needs a title', { kind: 'error' });
    const patch = editPatch(f.task, f);
    if ((patch.title !== null || patch.body !== null) && !(await update(f.task, patch))) return;
    if (form === f) form = null;
    root?.focus();
  }

  const closeForm = () => ((form = null), root?.focus());
  const openRemote = (t) => t?.remote_url && openUrl(t.remote_url);
  const startEdit = (t) => t && (form = { kind: 'edit', task: t, title: t.title ?? '', body: t.body ?? '' });
  const startAssign = (t) => t && (form = { kind: 'assign', task: t, node: t.node ?? '' });
  const startNew = () => (form = { kind: 'new', title: '', body: '', node: '' });

  /** Moves the card to column `ci` (`update`), the cursor following it. */
  async function moveTo(t, ci) {
    const col = COLUMNS[ci];
    if (!t || !col || col.id === t.status) return;
    if (await update(t, { status: col.id })) {
      const row = cols[ci].findIndex((x) => x.id === t.id);
      cur = { col: ci, row: Math.max(0, row) };
    }
  }

  /** ⇧H / ⇧L: the card one column left or right. */
  function shift(t, d) {
    const s = t && shiftStatus(t.status, d);
    if (s) moveTo(t, COLUMNS.findIndex((c) => c.id === s));
  }

  const menuItems = (t) => {
    const marks = taskMarks(t);
    const link = remoteLinkLabel(t);
    return [
      { id: 'assign', label: 'Assign…', key: 'a', disabled: !run, reason: 'No active run to assign in', run: () => startAssign(t) },
      {
        id: 'move',
        label: 'Move to…',
        key: 'm',
        items: COLUMNS.map((col, i) => ({ id: col.id, label: col.label, key: String(i + 1), disabled: col.id === t.status, reason: 'Already there', run: () => moveTo(t, i) })),
      },
      { id: 'edit', label: 'Edit title and body', key: 'e', run: () => startEdit(t) },
      ...(link ? [{ id: 'remote', label: `Open ${handle(t)} in ${remoteName(t.backend)}`, key: 'o', run: () => openRemote(t) }] : []),
      {
        id: 'shift',
        label: 'Move left / right',
        key: '⇧H ⇧L',
        items: [
          { id: 'left', label: 'Move left', key: '⇧H', disabled: !shiftStatus(t.status, -1), reason: 'Already in the first column', run: () => shift(t, -1) },
          { id: 'right', label: 'Move right', key: '⇧L', disabled: !shiftStatus(t.status, 1), reason: 'Already in the last column', run: () => shift(t, 1) },
        ],
      },
      ...(marks.conflict ? [{ separator: true }, { id: 'resolve', label: 'Resolve conflict…', key: '⏎', run: () => resolveConflict(t) }] : []),
    ];
  };

  /** The conflict is settled upstream: the task's page opens (the app already shows the remote's side, Linear won). */
  function resolveConflict(t) {
    if (t.remote_url) return openRemote(t);
    toast(`${handle(t)} changed in ${remoteName(t.backend)} and in the app since the last sync; the app now shows ${remoteName(t.backend)}'s.`);
  }

  function onKey(e) {
    if (e.defaultPrevented) return; // the card Menu's trigger took the key
    if (isTypingTarget(e.target)) {
      if (e.key === 'Enter' && e.metaKey && form?.kind === 'edit') return (saveEdit(), e.preventDefault());
      if (e.key === 'Enter' && e.metaKey && form?.kind === 'new') return (create(), e.preventDefault());
      if (e.key === 'Escape') ((form = null), root?.focus(), e.preventDefault(), e.stopPropagation());
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    // A focused button / link (Create, Cancel, Save) keeps its own Enter.
    if (e.key === 'Enter' && e.target !== root && e.target.closest?.('button, a, summary')) return;
    const arrows = { ArrowLeft: 'h', ArrowRight: 'l', ArrowUp: 'k', ArrowDown: 'j' };
    const k = arrows[e.key] ?? (e.key.length === 1 ? e.key.toLowerCase() : e.key);
    const t = selected;
    if (e.shiftKey && (k === 'h' || k === 'l')) shift(t, k === 'h' ? -1 : 1);
    else if (['h', 'j', 'k', 'l'].includes(k)) cur = moveCursor(cols, cur, k);
    else if (k === 'Enter') {
      if (t) cardMenu?.show();
    } else if (/^[1-5]$/.test(k)) moveTo(t, Number(k) - 1);
    else if (k === 'a') run && startAssign(t);
    else if (k === 'm') t && cardMenu?.show('move');
    else if (k === 'e') startEdit(t);
    else if (k === 'o') openRemote(t);
    else if (k === 'n') run && startNew();
    else if (k === 'Escape') {
      if (form) form = null;
      else onleave();
    } else return;
    e.preventDefault();
    e.stopPropagation();
    queueMicrotask(() => root?.querySelector('.task.sel')?.scrollIntoView?.({ block: 'nearest' }));
  }

  // The pane's hint row while Tasks has focus (notes §9.2, board ii-feature-*).
  $effect(() => {
    const t = selected;
    hints = [
      ['j k h l', 'move'],
      ['⏎', 'menu'],
      ...(run ? [['a', 'assign']] : []),
      ['m', 'move to'],
      ['1–5', 'column'],
      ['e', 'edit'],
      ...(run ? [['n', 'new']] : []),
      ...(t && remoteLinkLabel(t) ? [['o', `open ${handle(t)}`]] : []),
      ['⇧H ⇧L', 'move card'],
      ['Esc', 'back to pane'],
    ];
  });

  // Landing (§9.3): scroll Tasks in, focus the task, flash it once (not under reduced motion); a conflict opens its menu.
  $effect(() => {
    const l = land;
    if (!l?.seq || l.seq === untrack(() => landed) || !tasks.length) return;
    landed = l.seq;
    untrack(() => {
      const want = l.task != null ? String(l.task) : null;
      const ci = want ? cols.findIndex((c) => c.some((t) => String(t.id) === want)) : -1;
      if (ci >= 0) cur = { col: ci, row: cols[ci].findIndex((t) => String(t.id) === want) };
      focus();
      if (ci < 0) return;
      const id = cols[ci][cur.row].id;
      if (!globalThis.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches) {
        flash = id;
        setTimeout(() => (flash = null), 200);
      }
      if (l.conflict && taskMarks(cols[ci][cur.row]).conflict) queueMicrotask(() => cardMenu?.show());
    });
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<section class="card tasks" tabindex="-1" data-tasks bind:this={root} onkeydown={onKey} aria-label="Tasks">
  <div class="cardh"><b>Tasks</b><span class="muted">{tasksSummary(source, tasks)}</span></div>

  {#if form?.kind === 'new'}
    <div class="form">
      <!-- svelte-ignore a11y_autofocus -->
      <input class="field grow" autofocus bind:value={form.title} placeholder="Task title" onkeydown={(e) => e.key === 'Enter' && !e.metaKey && (e.preventDefault(), create())} />
      <textarea class="field body" rows="3" bind:value={form.body} placeholder="Description (optional) — ⌘⏎ to create" aria-label="Description"></textarea>
      <Select bind:value={form.node} options={nodeOptions('unassigned')} aria-label="Node" noun="node" />
      <button class="btn primary" onclick={create}>Create</button>
      <button class="btn ghost" onclick={() => closeForm()}>Cancel</button>
    </div>
  {:else if form?.kind === 'edit'}
    <div class="form">
      <span class="mono">Edit {handle(form.task)}</span>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="field grow" autofocus bind:value={form.title} placeholder="Task title" onkeydown={(e) => e.key === 'Enter' && !e.metaKey && (e.preventDefault(), saveEdit())} />
      <textarea class="field body" rows="3" bind:value={form.body} placeholder="Description — ⌘⏎ to save" aria-label="Description"></textarea>
      <button class="btn primary" onclick={saveEdit}>Save</button>
      <button class="btn ghost" onclick={() => closeForm()}>Cancel</button>
    </div>
  {:else if form?.kind === 'assign'}
    <div class="form">
      <span>Assign {handle(form.task)} to</span>
      <span use:focusBox>
        <Select value={form.node} options={nodeOptions()} placeholder="node…" aria-label="Node" noun="node" onchange={(node) => (update(form.task, { node }), (form = null), root?.focus())} />
      </span>
      <button class="btn ghost" onclick={() => closeForm()}>Cancel</button>
    </div>
  {/if}

  <div class="taskcols">
    {#each COLUMNS as c, ci (c.id)}
      <div class="tcol" role="group" aria-label={c.label}>
        <div class="colh"><b>{c.label}</b> <span class="muted">{cols[ci].length}</span><span class="grow"></span><span class="colkey muted" aria-hidden="true">{ci + 1}</span></div>
        {#each cols[ci] as t, ri (t.id)}
          {@const blocked = c.id === 'ready' ? blockedBy(t, tasks) : []}
          {@const marks = taskMarks(t)}
          {@const sel = cur.col === ci && cur.row === ri}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="task" class:sel class:flash={flash === t.id} data-task={t.id} onclick={() => ((cur = { col: ci, row: ri }), root?.focus())}>
            <div class="tt">
              <span class="muted mono">T-{t.id}{t.external_id ? ` · ${handle(t)}` : ''}</span>
              <span class="grow"></span>
              {#if marks.conflict}<span class="conflict" use:tooltip={`Changed in ${remoteName(t.backend)} and in the app since the last sync; the app now shows ${remoteName(t.backend)}'s.`}>⚠ conflict</span>
              {:else if marks.notSynced}<span class="muted" use:tooltip={'Write-back is waiting: the change reaches the tracker at the next sync.'}>not synced</span>{/if}
              {#if sel}
                <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                <span class="cmenu" onclick={(e) => e.stopPropagation()}><Menu bind:this={cardMenu} items={menuItems(t)} label="Task menu" bounds={root} /></span>
              {/if}
            </div>
            <div class="title" use:tooltip={ifCut(t.title)}>{t.title}</div>
            <div class="chipsrow">
              <TaskModelChip task={t} />
              {#if t.node}<AgentChip id={t.node} role={roleOf(t)} />{/if}
            </div>
            {#if blocked.length}<div class="blocked">{'\u{F051F}'} blocked by {blocked.map((d) => `T-${d}`).join(', ')}</div>{/if}
          </div>
        {:else}<span class="muted none">No tasks</span>{/each}
      </div>
    {/each}
  </div>
</section>

<style>
  .tasks {
    outline: none;
  }
  .tasks:focus-within {
    border-color: var(--color-focus);
  }
  .cardh,
  .tt,
  .colh,
  .form {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .cardh b {
    font: var(--text-title);
  }
  .form {
    flex-wrap: wrap;
  }
  .body {
    flex-basis: 100%;
    resize: vertical;
  }
  .taskcols {
    display: flex;
    gap: var(--space-sm);
    align-items: flex-start;
  }
  .tcol {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .colkey {
    padding: 0 var(--space-xs);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    font: var(--text-caption);
  }
  .none {
    padding: 0 var(--space-sm);
    font: var(--text-caption);
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
  .tasks:focus-within .task.sel {
    background: var(--color-surface-selected);
    border: var(--border-width-focus) solid var(--color-focus);
  }
  .task.flash {
    background: var(--color-surface-selected);
  }
  .title {
    font: var(--text-body);
    color: var(--color-text-primary);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .chipsrow {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }
  .blocked {
    color: var(--color-status-queued);
    font: var(--text-caption);
  }
  .conflict {
    color: var(--color-pacing-amber);
    font: var(--text-caption);
  }
  .cmenu {
    display: contents;
  }
  /* The Feature pane's content is the container: five columns down to the 1280 board (≈ 860 px), two-by-two below. */
  @container (max-width: 720px) {
    .taskcols {
      flex-wrap: wrap;
    }
    .tcol {
      flex-basis: 40%;
    }
  }
</style>
