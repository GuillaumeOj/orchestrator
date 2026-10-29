<script>
  import AgentChip from '../components/AgentChip.svelte';
  import CtxBar from '../components/CtxBar.svelte';
  import Kbd from '../components/Kbd.svelte';
  import Reason from '../components/Reason.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  // Overview (§9.3, FR-901/903, Penpot 04 Overview): every agent with its
  // attention record (status, reason, since), elapsed time, context, usage,
  // governor reason and memory; grouped by project or by state. Chips and
  // totals are the core's counts. Keys (pane-local): j/k move, ⏎ open,
  // i interrupt, s suspend, r resume, K kill (asks first), p pause.
  import { activeAgents, agentAction, app, cancelQueued, confirm, openAgent, projects, run, runNow } from '../lib/app.svelte.js';
  import { duration, mb } from '../lib/format.js';
  import { attention, counts, governorReason, liveText, statusOf } from '../lib/now.svelte.js';
  import { runCost, STATUS, sessionPp } from '../lib/status.js';

  let { paneId } = $props();
  let groupBy = $state('project');
  let filter = $state('all');
  let selected = $state(null);
  let collapsed = $state({});

  const level = (a) => attention(a.id)?.level;
  const c = $derived(counts());
  // `count`: the core's number where it has one (it agrees with every surface).
  const FILTERS = [
    { id: 'all', label: 'All', test: () => true },
    { id: 'need', label: 'Need you', test: (a) => level(a) === 'needs_you', count: 'needs_you' },
    { id: 'live', label: 'Live', test: (a) => !!attention(a.id)?.live, count: 'live' },
    { id: 'queued', label: 'Queued', test: (a) => level(a) === 'queued', count: 'queued' },
    { id: 'suspended', label: 'Suspended', test: (a) => statusOf(a.id) === 'suspended' },
  ];
  const list = $derived(activeAgents());
  const shown = $derived(list.filter(FILTERS.find((f) => f.id === filter).test));
  const groups = $derived.by(() => {
    if (groupBy === 'state') {
      return Object.keys(STATUS)
        .map((s) => ({ id: s, title: STATUS[s].label, rows: shown.filter((a) => statusOf(a.id) === s) }))
        .filter((g) => g.rows.length);
    }
    const out = projects().map((p) => ({ id: p.id, title: p.name, project: p, rows: shown.filter((a) => a.project_id === p.id) }));
    const orphan = shown.filter((a) => !projects().some((p) => p.id === a.project_id));
    if (orphan.length) out.push({ id: '_', title: 'Other', rows: orphan });
    return out;
  });
  const flat = $derived(groups.flatMap((g) => (collapsed[g.id] ? [] : g.rows)));
  const suspended = $derived(list.filter((a) => statusOf(a.id) === 'suspended').length);
  const claudeMb = $derived(list.reduce((s, a) => s + (a.memory_mb ?? 0), 0));

  function usage(a) {
    const v = sessionPp(a, app.data?.usage?.snapshot);
    return v == null ? '—' : `${v.toFixed(1)} pp`;
  }

  const sel = () => flat.find((a) => a.id === selected) ?? null;

  function onKey(e) {
    if (e.target.closest('input, textarea, select')) return;
    const a = sel();
    const key = e.key;
    const act = (fn) => {
      e.preventDefault();
      fn();
    };
    if (key === 'j' || key === 'ArrowDown') return act(() => move(1));
    if (key === 'k' || key === 'ArrowUp') return act(() => move(-1));
    if (!a) return;
    if (key === 'Enter') return act(() => openAgent(a.id));
    if (key === 'i') return act(() => agentAction(a.id, 'interrupt'));
    if (key === 's') return act(() => agentAction(a.id, 'suspend'));
    if (key === 'r') return act(() => agentAction(a.id, 'resume'));
    if (key === 'p') return act(() => run('agent.pause'));
    if (key === 'K') return act(() => confirm(`Kill ${a.id}?`, 'SIGTERM to its process group and to leftover processes in its worktree, SIGKILL after 5 s. The session can be resumed.', () => agentAction(a.id, 'kill'), 'Kill'));
  }

  function move(d) {
    if (!flat.length) return;
    const i = flat.findIndex((a) => a.id === selected);
    selected = flat[Math.max(0, Math.min(flat.length - 1, i < 0 ? 0 : i + d))].id;
    queueMicrotask(() => document.querySelector(`[data-pane="${paneId}"] tr.sel`)?.scrollIntoView({ block: 'nearest' }));
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="ov" tabindex="0" data-pane-focus onkeydown={onKey} role="grid" aria-label="Overview">
  <div class="toolbar">
    <span class="muted">Group by</span>
    <div class="seg" role="radiogroup">
      <button class:on={groupBy === 'project'} role="radio" aria-checked={groupBy === 'project'} onclick={() => (groupBy = 'project')}>Project</button>
      <button class:on={groupBy === 'state'} role="radio" aria-checked={groupBy === 'state'} onclick={() => (groupBy = 'state')}>State</button>
    </div>
    {#each FILTERS as f (f.id)}
      <button class="filter-chip" class:on={filter === f.id} onclick={() => (filter = f.id)}>{f.label} {f.count ? c[f.count] : list.filter(f.test).length}</button>
    {/each}
    <span class="grow"></span>
    <span class="keys"><Kbd keys="⏎" /> open <Kbd keys="i" /> interrupt <Kbd keys="p" /> pause <Kbd keys="s" /> suspend <Kbd keys="r" /> resume <Kbd keys="⇧+K" /> kill</span>
  </div>
  <div class="scroll">
    <table>
      <thead>
        <tr><th>State</th><th>Agent</th><th>Doing</th><th>Elapsed</th><th>Ctx</th><th>Usage</th><th>Governor</th><th class="r">Mem</th></tr>
      </thead>
      <tbody>
        {#each groups as g (g.id)}
          <tr class="grp">
            <td colspan="8">
              <button class="gbtn" onclick={() => (collapsed[g.id] = !collapsed[g.id])}>{collapsed[g.id] ? '▸' : '▾'} <b>{g.title}</b></button>
              {#if g.project}
                <span class="muted">{g.project.git?.branch ?? ''}{g.project.git?.ahead ? ` ↑${g.project.git.ahead}` : ''}{g.project.git?.dirty ? ` ±${g.project.git.dirty}` : ''} · {g.rows.length} session{g.rows.length === 1 ? '' : 's'}</span>
              {/if}
            </td>
          </tr>
          {#if !collapsed[g.id]}
            {#each g.rows as a (a.id)}
              {@const rec = attention(a.id)}
              {@const fresh = rec?.status === 'not_started'}
              <tr class:sel={selected === a.id} onclick={() => (selected = a.id)} ondblclick={() => openAgent(a.id)}>
                <td><StatusBadge state={statusOf(a.id)} /></td>
                <td><AgentChip id={a.id} /></td>
                <td class="doing">
                  <Reason {rec} />
                  {#if a.queue_entry}
                    <button class="btn" onclick={(e) => (e.stopPropagation(), runNow(a, fresh))}>{fresh ? 'Start now' : 'Run now'} ≈ {runCost(a.queue_entry)}</button>
                    <button class="btn ghost" onclick={(e) => (e.stopPropagation(), cancelQueued(a))} title="Drop the queued turn">Cancel</button>
                  {/if}
                </td>
                <td>{duration(Date.now() - (a.created_at_ms ?? a.last_event_at_ms ?? Date.now()))}</td>
                <td><CtxBar value={a.context_pct} /></td>
                <td>{usage(a)}</td>
                <td class="gov">{governorReason(rec)}</td>
                <td class="r">{a.memory_mb ? mb(a.memory_mb) : '—'}</td>
              </tr>
            {:else}
              <tr><td colspan="8" class="muted pad">No sessions{filter !== 'all' ? ' in this filter' : ''}.</td></tr>
            {/each}
          {/if}
        {:else}
          <tr>
            <td colspan="8" class="pad">
              {#if !projects().length}
                <p>No projects yet: add one to start a session.</p>
                <button class="btn" onclick={() => (app.overlay = { kind: 'add-project' })}>Add project <Kbd keys="⌘+N" /></button>
              {:else}
                <p class="muted">Nothing running.</p>
                <button class="btn" onclick={() => (app.overlay = { kind: 'new-session' })}>New session <Kbd keys="⌘+T" /></button>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
  <div class="foot">
    <span class="totals">{list.length} agents · {liveText(c)} · queued {c.queued} · {suspended} suspended · {c.needs_you} need you</span>
    <span class="grow"></span>
    <span class="muted">claude processes {mb(claudeMb)} · app {mb(app.data?.memory?.app_mb)} · warn at 2.5 GB</span>
  </div>
</div>

<style>
  .ov {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    outline: none;
    font: var(--text-label);
  }
  .foot {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-md);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
    white-space: nowrap;
  }
  .seg {
    display: inline-flex;
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
  }
  .seg button {
    border: 0;
    background: none;
    padding: var(--space-xxs) var(--space-sm);
  }
  .seg button.on {
    background: var(--color-surface-selected);
  }
  .scroll {
    flex: 1;
    overflow: auto;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th {
    position: sticky;
    top: 0;
    text-align: left;
    font-weight: var(--font-weight-regular);
    color: var(--color-text-muted);
    background: var(--color-surface-pane);
    padding: var(--space-xs) var(--space-md);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  td {
    padding: var(--space-xxs) var(--space-md);
    height: 32px;
    white-space: nowrap;
  }
  .r {
    text-align: right;
  }
  tr.grp td {
    background: var(--color-surface-chrome);
  }
  .gbtn {
    border: 0;
    background: none;
    padding: 0 var(--space-sm) 0 0;
    font: var(--text-body);
  }
  tbody tr:not(.grp):hover {
    background: var(--color-surface-raised);
  }
  tr.sel {
    background: var(--color-surface-selected) !important;
    box-shadow: inset 0 0 0 var(--border-width-default) var(--color-accent-default);
  }
  .doing {
    max-width: 420px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .doing .btn {
    margin-left: var(--space-sm);
    height: 22px;
  }
  .pad {
    padding: var(--space-lg);
  }
</style>
