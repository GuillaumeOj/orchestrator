<script>
  // Project pane (§4.1, J1, J11): git state (FR-102), fetch (FR-103), worktrees
  // and branches with one-click cleanup (FR-104), PRs and CI via gh (FR-106),
  // workspace trust (J0), and removal with the guided cleanup (J11). Unmerged
  // work is never deleted without its own confirmation.
  // Keys (pane-local): f fetch, p load PRs, t trust, x remove project.
  import { onMount } from 'svelte';
  import { app, projects, agents, openAgent, openShell, openTrust, refresh, toast, confirm, closePane } from '../lib/app.svelte.js';
  import { invoke, errText } from '../lib/ipc.js';
  import { ago, dayTime } from '../lib/format.js';
  import { displayState } from '../lib/status.js';
  import StatusBadge from '../components/StatusBadge.svelte';
  import Kbd from '../components/Kbd.svelte';

  let { paneId } = $props();
  const spec = $derived(app.panes[paneId]);
  const p = $derived(projects().find((x) => x.id === spec?.projectId));
  const list = $derived(agents().filter((a) => a.project_id === p?.id && a.state !== 'archived'));
  const home = $derived(app.data?.core?.user_home ?? '');
  const short = (path) => (home && String(path).startsWith(home) ? `~${String(path).slice(home.length)}` : path);

  let d = $state(null);
  let err = $state(null);
  let prs = $state(null);
  let busy = $state(null);
  let removal = $state(null); // J11: { plan, agents, discard: Set, history }

  async function load() {
    if (!p) return;
    try {
      d = await invoke('project_detail', { id: p.id, withPrs: false });
      err = null;
    } catch (e) {
      err = errText(e);
    }
  }
  onMount(() => {
    load();
    // git per worktree and branch: not more often than the user can read it.
    const t = setInterval(load, 60000);
    return () => clearInterval(t);
  });

  async function act(label, fn) {
    if (busy) return;
    busy = label;
    try {
      await fn();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    } finally {
      busy = null;
      await load();
      await refresh();
    }
  }

  const fetchNow = () =>
    act('fetch', async () => {
      const r = await invoke('project_fetch', { id: p.id });
      toast(r?.error ? `Fetch failed · ${p.name}: ${r.error}` : `Fetched · ${p.name}`, r?.error ? { kind: 'error' } : {});
    });

  const loadPrs = () =>
    act('prs', async () => {
      const r = await invoke('project_detail', { id: p.id, withPrs: true });
      prs = r.prs;
    });

  const VERDICT = {
    ok: 'in use or clean',
    missing: 'folder gone (prunable)',
    prunable: 'prunable',
    unknown: 'not created by the app',
    main: 'main checkout',
    external: 'outside the sibling folder',
  };
  const verdict = (w) => VERDICT[w.verdict] ?? w.verdict;
  const ci = (c) => (!c ? '—' : c.failing ? `${c.failing} failing` : c.pending ? `${c.pending} pending` : c.passing ? 'passing' : 'none');
  // The app manages its sibling folder; the user's own worktrees are listed, never cleaned up.
  const managed = $derived((d?.worktrees ?? []).filter((w) => !['main', 'external'].includes(w.verdict)));
  const external = $derived((d?.worktrees ?? []).filter((w) => w.verdict === 'external'));
  const orchBranches = $derived((d?.branches ?? []).filter((b) => b.orch));
  const unmerged = (w) => !w.safe_to_remove && (w.dirty > 0 || w.unmerged_commits > 0 || w.in_progress?.length > 0);

  function removeWorktree(w) {
    const risky = !w.safe_to_remove;
    const run = (discard) => act('wt', async () => {
      await invoke('worktree_remove', { project: p.id, path: w.path, deleteBranch: true, discard });
      toast(`Removed ${short(w.path)}`);
    });
    if (!risky) return confirm(`Remove worktree ${w.branch ?? ''}?`, `${short(w.path)} and its branch. Its work is merged into ${p.default_branch}.`, () => run(false), 'Remove');
    confirm(`Discard unmerged work in ${w.branch ?? short(w.path)}?`, `This worktree has work ${p.default_branch} does not have (commits, uncommitted or untracked files). It is deleted for good.`, () => run(true), 'Discard and remove');
  }

  async function startRemoval() {
    try {
      const r = await invoke('project_removal_plan', { id: p.id });
      removal = { ...r, discard: {}, history: 'archive', removeMerged: true };
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }

  function doRemove() {
    const discard = Object.entries(removal.discard).filter(([, v]) => v).map(([k]) => k);
    const what = removal.history === 'delete' ? 'Its rules, worktree rows and transcript index are deleted; events, usage and the audit log are kept.' : 'It is hidden; everything is kept.';
    confirm(`Remove project ${p.name}?`, `${removal.agents.length} agent(s) are stopped and archived. ${discard.length ? `${discard.length} worktree(s) with unmerged work are discarded. ` : ''}${what} .orchestrator/ in the repo is left untouched.`, () =>
      act('remove', async () => {
        const r = await invoke('project_remove', { id: p.id, choices: { remove_merged: removal.removeMerged, discard, history: removal.history } });
        removal = null;
        toast(`Removed ${p.name}${r.kept?.length ? ` · kept ${r.kept.length} worktree(s) with unmerged work` : ''}`, { ms: 10000 });
        closePane(paneId);
      }), 'Remove project');
  }

  function onKey(e) {
    if (e.metaKey || e.ctrlKey || e.altKey || e.target.closest('input, select, textarea')) return;
    const k = { f: fetchNow, p: loadPrs, t: () => openTrust(p.id), x: startRemoval }[e.key];
    if (k) {
      e.preventDefault();
      k();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="proj" tabindex="0" data-pane-focus onkeydown={onKey} role="region" aria-label="Project {p?.name ?? ''}">
  {#if !p}
    <p class="muted">This project is no longer registered.</p>
  {:else}
    {@const g = p.git}
    <div class="head">
      <h2>{p.name}</h2>
      <span class="secondary mono">{short(p.path)}</span>
      <span class="grow"></span>
      <button class="btn" onclick={fetchNow} disabled={!!busy}>{busy === 'fetch' ? 'Fetching…' : 'Fetch'} <Kbd keys="f" /></button>
      <button class="btn ghost" onclick={() => openShell({ projectId: p.id, cwd: p.path, how: 'right' })}>Shell</button>
      <button class="btn danger" onclick={startRemoval}>Remove project… <Kbd keys="x" /></button>
    </div>

    <section class="card">
      <dl>
        <dt>Branch</dt><dd>{g?.branch ?? 'detached'}{g?.upstream ? ` → ${g.upstream}` : ' (no upstream)'} · ↑{g?.ahead ?? 0} ↓{g?.behind ?? 0}</dd>
        <dt>Changes</dt><dd>{g?.dirty ?? 0} files ({g?.staged ?? 0} staged, {g?.untracked ?? 0} untracked{g?.conflicted ? `, ${g.conflicted} conflicted` : ''}){g?.stash ? ` · ${g.stash} stashed` : ''}</dd>
        <dt>Last commit</dt><dd>{g?.last_commit ?? '—'}{g?.last_commit_s ? ` · ${ago(g.last_commit_s * 1000)} ago` : ''}</dd>
        <dt>Default branch</dt><dd>{p.default_branch}</dd>
        <dt>Fetched</dt><dd>{p.last_fetch_at_ms ? dayTime(p.last_fetch_at_ms) : 'not yet'}{p.last_fetch_error ? ` · ${p.last_fetch_error}` : ''} <span class="muted">(every 15 min while active; paused on low battery or offline)</span></dd>
        <dt>Trust</dt>
        <dd>
          {#if d?.trusted === true}Claude Code trusts this folder
          {:else}<span class="warn">Not trusted yet</span> · Claude Code asks once per repository <button class="btn" onclick={() => openTrust(p.id)}>Trust in a pane <Kbd keys="t" /></button>{/if}
        </dd>
      </dl>
    </section>

    {#if err}<p class="err">{err}</p>{/if}

    <section class="card">
      <div class="ch"><b>Sessions</b><span class="muted">{list.length}</span></div>
      {#each list as a (a.id)}
        <button class="row" onclick={() => openAgent(a.id)}><StatusBadge state={displayState(a)} compact /> <b>{a.id}</b> <span class="muted">{a.branch ?? ''}</span></button>
      {:else}
        <p class="muted">No session. <button class="btn ghost" onclick={() => (app.overlay = { kind: 'new-session' })}>New session <Kbd keys="⌘+T" /></button></p>
      {/each}
    </section>

    <section class="card">
      <div class="ch"><b>Worktrees</b><span class="muted">sibling folder {p.name}.worktrees · unmerged work is never deleted without confirmation</span></div>
      <table>
        <thead><tr><th>Branch</th><th>Folder</th><th>State</th><th></th></tr></thead>
        <tbody>
          {#each managed as w (w.path)}
            <tr>
              <td class="mono">{w.branch ?? '—'}</td>
              <td class="mono">{short(w.path)}</td>
              <td>{verdict(w)}{unmerged(w) ? ' · unmerged work' : ''}{w.row?.agent_id ? ` · ${w.row.agent_id}` : ''}{w.locked ? ' · locked' : ''}</td>
              <td><button class="btn ghost" disabled={!!busy} onclick={() => removeWorktree(w)}>Remove</button></td>
            </tr>
          {:else}
            <tr><td colspan="4" class="muted">{d ? 'No worktree of the app.' : 'Loading…'}</td></tr>
          {/each}
        </tbody>
      </table>
      {#if orchBranches.length}
        <p class="muted">orch/ branches: {orchBranches.map((b) => `${b.name}${b.merged ? ' (merged)' : b.ahead_of_base ? ` (+${b.ahead_of_base})` : ''}`).join(' · ')}</p>
      {/if}
      {#if external.length}
        <details>
          <summary class="muted">{external.length} other worktree{external.length > 1 ? 's' : ''} outside the sibling folder (yours, not managed by the app)</summary>
          {#each external as w (w.path)}<p class="mono muted">{w.branch ?? '—'} · {short(w.path)}</p>{/each}
        </details>
      {/if}
    </section>

    <section class="card">
      <div class="ch"><b>Pull requests</b><span class="muted">gh</span><span class="grow"></span><button class="btn ghost" onclick={loadPrs} disabled={!!busy}>{busy === 'prs' ? 'Loading…' : 'Load'} <Kbd keys="p" /></button></div>
      {#if prs?.error}<p class="muted">{prs.error}</p>
      {:else if prs}
        {#each prs as pr (pr.number)}
          <p><b>#{pr.number}</b> {pr.title}{pr.is_draft ? ' (draft)' : ''} <span class="muted">{pr.head_ref} → {pr.base_ref} · CI {ci(pr.checks)}{pr.review_decision ? ` · ${pr.review_decision.toLowerCase()}` : ''}</span></p>
        {:else}<p class="muted">No open pull request.</p>{/each}
      {:else}<p class="muted">Not loaded (a network call).</p>{/if}
    </section>

    {#if removal}
      <section class="card danger-zone" aria-label="Remove project">
        <div class="ch"><b>Remove {p.name}</b><span class="muted">J11</span></div>
        <p>{removal.agents.length ? `Agents stopped and archived: ${removal.agents.map((a) => a.id).join(', ')}.` : 'No agent.'}</p>
        <label class="check"><input type="checkbox" bind:checked={removal.removeMerged} /> Remove worktrees whose work is merged and clean</label>
        {#each removal.plan.worktrees as w (w.path)}
          {#if unmerged(w)}
            <label class="check"><input type="checkbox" bind:checked={removal.discard[w.path]} /> Discard unmerged work in <span class="mono">{w.branch ?? short(w.path)}</span> (otherwise kept)</label>
          {/if}
        {/each}
        <div class="row2">
          <label class="check"><input type="radio" bind:group={removal.history} value="archive" /> Archive: hide the project, keep everything</label>
          <label class="check"><input type="radio" bind:group={removal.history} value="delete" /> Delete its settings, rules and search index (usage and audit kept)</label>
        </div>
        <div class="row2">
          <button class="btn ghost" onclick={() => (removal = null)}>Cancel</button>
          <button class="btn danger" onclick={doRemove} disabled={!!busy}>Remove project</button>
        </div>
      </section>
    {/if}
  {/if}
</div>

<style>
  .proj {
    flex: 1;
    min-height: 0;
    overflow: auto;
    outline: none;
    padding: var(--space-md);
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    font: var(--text-label);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    flex-wrap: wrap;
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  .card {
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    padding: var(--space-md);
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }
  .danger-zone {
    border-color: var(--color-status-error);
  }
  .ch {
    display: flex;
    align-items: baseline;
    gap: var(--space-sm);
    font: var(--text-title);
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-xs) var(--space-md);
    margin: 0;
  }
  dt {
    color: var(--color-text-secondary);
  }
  dd {
    margin: 0;
  }
  p {
    margin: 0;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th {
    text-align: left;
    color: var(--color-text-secondary);
    font-weight: var(--font-weight-regular);
  }
  td,
  th {
    padding: var(--space-xxs) var(--space-sm) var(--space-xxs) 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    border: 0;
    background: none;
    text-align: left;
    padding: var(--space-xxs) 0;
  }
  .row:hover {
    background: var(--color-surface-raised);
  }
  .row2 {
    display: flex;
    gap: var(--space-md);
    flex-wrap: wrap;
  }
  .check {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .warn {
    color: var(--color-status-queued);
  }
  .err {
    color: var(--color-status-error);
  }
</style>
