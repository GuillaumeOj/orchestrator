<script>
  // Project pane (§4.1, J1, J11): git state (FR-102), fetch (FR-103), worktrees
  // and branches with one-click cleanup (FR-104), PRs and CI via gh (FR-106),
  // workspace trust (J0), and removal with the guided cleanup (J11). Unmerged
  // work is never deleted without its own confirmation.
  // Keys (pane-local): f fetch, p load PRs, t trust, x remove project.
  import { onMount, untrack } from 'svelte';
  import Menu from '../components/Menu.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import { activeAgents, app, closePane, confirm, openAgent, openDiff, openProjectSettings, openShell, openTrust, projects, refresh, toast } from '../lib/app.svelte.js';
  import { ago, dayTime, shortPath } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { keyIgnored } from '../lib/keyboard.js';
  import { onTopics, statusOf } from '../lib/now.svelte.js';
  import { tooltip } from '../lib/tooltip.js';

  let { paneId } = $props();
  const spec = $derived(app.panes[paneId]);
  const p = $derived(projects().find((x) => x.id === spec?.projectId));
  const list = $derived(activeAgents().filter((a) => a.project_id === p?.id));
  const home = $derived(app.data?.core?.user_home ?? '');
  const short = (path) => shortPath(path, home);

  let d = $state(null);
  let err = $state(null);
  let prs = $state(null);
  let busy = $state(null);
  let removal = $state(null); // J11: { plan, agents, discard: { [path]: bool }, history, removeMerged }

  // One path for every `project_detail`: a response older than the latest request
  // (or than a local removal) is dropped, so a slow reload can't bring a removed row back.
  let seq = 0;
  async function fetchDetail(withPrs) {
    if (!p) return null;
    const mine = ++seq;
    try {
      const r = await invoke('project_detail', { id: p.id, withPrs });
      if (mine === seq) {
        d = r;
        err = null;
      }
      return r; // a stale response is not assigned to `d`, but its PRs are still shown
    } catch (e) {
      if (withPrs) throw e; // `loadPrs` runs in `act`, which toasts it
      if (mine === seq) err = errText(e);
      return null;
    }
  }
  const load = () => fetchDetail(false);
  // Refetched when the core says the project changed (no poll).
  onMount(() => onTopics([`project:${spec.projectId}`], load));

  const BUSY_NAME = { fetch: 'the fetch', prs: 'the PR load', wt: 'the worktree removal', prune: 'the prune', remove: 'the project removal' };
  /** Runs an action; then reloads the detail and the app state unless `reload` is false. */
  async function act(label, fn, reload = true) {
    if (busy) {
      toast(`Wait for ${BUSY_NAME[busy] ?? busy} to finish`);
      return;
    }
    busy = label;
    try {
      await fn();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    } finally {
      busy = null;
      if (reload) await Promise.all([load(), refresh()]);
    }
  }

  const fetchNow = () =>
    act('fetch', async () => {
      const r = await invoke('project_fetch', { id: p.id });
      toast(r?.error ? `Fetch failed · ${p.name}: ${r.error}` : `Fetched · ${p.name}`, r?.error ? { kind: 'error' } : {});
    });

  const loadPrs = () =>
    act('prs', async () => {
      const r = await fetchDetail(true);
      if (r) prs = r.prs;
    }, false);

  const VERDICT = {
    ok: 'in use or clean',
    missing: 'folder gone',
    prunable: 'folder gone',
    unknown: 'not created by the app',
    main: 'main checkout',
    external: 'yours',
  };
  const verdict = (w) => VERDICT[w.verdict] ?? w.verdict;
  const ci = (c) => (!c ? '—' : c.failing ? `${c.failing} failing` : c.pending ? `${c.pending} pending` : c.passing ? 'passing' : 'none');
  const lockText = (w) => (w.locked ? (w.lock_reason ? ` · locked: ${w.lock_reason}` : ' · locked') : '');
  const gone = (w) => w.verdict === 'prunable' || w.verdict === 'missing';
  const plural = (n, one, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
  // The sibling folder's worktrees first; the user's own (`outside`) in a second group, same actions.
  const wts = $derived((d?.worktrees ?? []).filter((w) => w.verdict !== 'main'));
  const isOutside = (w) => w.outside === true || w.verdict === 'external';
  const managed = $derived(wts.filter((w) => !isOutside(w)));
  const outside = $derived(wts.filter(isOutside));
  const prunable = $derived(wts.filter((w) => gone(w) && !w.in_use_by?.length));
  const orchBranches = $derived((d?.branches ?? []).filter((b) => b.orch));
  const unmerged = (w) => !w.safe_to_remove && (w.dirty > 0 || w.unmerged_commits > 0 || w.in_progress?.length > 0);

  /** What an outside worktree loses on discard (the branch is kept when unmerged). */
  function losses(w) {
    const out = [];
    if (w.dirty > 0) out.push(plural(w.dirty, 'uncommitted file'));
    if (w.in_progress?.length) out.push(`an operation in progress (${w.in_progress.join(', ')})`);
    if (w.locked) out.push(w.lock_reason ? `a lock (${w.lock_reason}), another tool may be using it` : 'a lock, another tool may be using it');
    if (!w.branch && w.unmerged_commits > 0) out.push(`${plural(w.unmerged_commits, 'commit')} on a detached HEAD`);
    return out;
  }

  function removeWorktree(w) {
    const risky = !w.safe_to_remove;
    const out = isOutside(w);
    const run = (discard) => act('wt', async () => {
      const r = await invoke('worktree_remove', { project: p.id, path: w.path, deleteBranch: true, discard });
      seq++; // a reload that started before the removal must not bring the row back
      if (d) d = { ...d, worktrees: d.worktrees.filter((x) => x.path !== w.path) };
      toast(`Removed ${short(w.path)}${r?.kept_branch ? ` · kept branch ${r.kept_branch} (unmerged)` : ''}`);
    });
    const where = short(w.path);
    const base = p.default_branch;
    let title = `Remove worktree ${w.branch ?? ''}?`;
    let body;
    if (out) {
      body = risky
        ? `${where}: ${losses(w).join('; ') || 'work that is not saved'}. Removing it discards all of this for good. Its branch is kept if unmerged.`
        : `${where}. Its branch is deleted if merged into ${base}, kept otherwise.`;
    } else if (risky) {
      title = `Discard unmerged work in ${w.branch ?? where}?`;
      body = `This worktree has work ${base} does not have (commits, uncommitted or untracked files). It is deleted for good.`;
    } else {
      body = `${where} and its branch. Its work is merged into ${base}.`;
    }
    confirm(title, body, () => run(risky), risky ? 'Discard and remove' : 'Remove');
  }

  function pruneGone() {
    const n = prunable.length;
    confirm(`Prune ${plural(n, 'gone worktree')}?`, 'Their folders are already gone. Their git entries go, and their branches when merged; unmerged branches are kept.', () =>
      act('prune', async () => {
        const r = await invoke('worktrees_prune', { project: p.id });
        const removed = new Set(r?.removed ?? []);
        seq++; // as for a single removal
        if (d) d = { ...d, worktrees: d.worktrees.filter((x) => !removed.has(x.path)) };
        const kept = r?.kept_branches?.length ?? 0;
        const skipped = r?.skipped?.length ?? 0;
        toast(`${removed.size ? `Pruned ${plural(removed.size, 'worktree')}` : 'Nothing pruned'}${kept ? ` · kept ${plural(kept, 'unmerged branch', 'unmerged branches')}` : ''}${skipped ? ` · skipped ${skipped} in use` : ''}`);
      }), 'Prune');
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
    const name = p.name;
    const choices = { remove_merged: removal.removeMerged, discard, history: removal.history };
    confirm(`Remove project ${name}?`, `${removal.agents.length} agent(s) are stopped and archived. ${discard.length ? `${discard.length} worktree(s) with unmerged work are discarded. ` : ''}${what} .orchestrator/ in the repo is left untouched.`, () =>
      act('remove', async () => {
        const r = await invoke('project_remove', { id: p.id, choices });
        removal = null;
        toast(`Removed ${name}${r.kept?.length ? ` · kept ${r.kept.length} worktree(s) with unmerged work` : ''}`, { ms: 10000 });
        closePane(paneId);
      }), 'Remove project');
  }

  // Asked from Project settings › General: start the removal once the project is known (one-shot).
  $effect(() => {
    if (!spec?.remove || !p) return;
    app.panes[paneId] = { ...app.panes[paneId], remove: false };
    untrack(startRemoval);
  });

  // The header's `⋯` (R4): Fetch, Shell, Diff main, Trust in a pane (untrusted only); keys that work in the pane.
  const menu = $derived([
    { id: 'fetch', icon: '\u{F06B0}', label: busy === 'fetch' ? 'Fetching…' : 'Fetch', key: 'f', disabled: !!busy, reason: 'Busy', run: fetchNow },
    { id: 'shell', icon: '\u{F018D}', label: 'Shell', run: () => openShell({ projectId: p.id, cwd: p.path }) },
    { id: 'diff', icon: '\u{F0E2B}', label: `Diff ${p?.default_branch ?? 'main'}`, run: () => openDiff(p.id, p.path, d?.status?.branch) },
    ...(d && d.trusted !== true ? [{ id: 'trust', icon: '\u{F0565}', label: 'Trust in a pane', key: 't', run: () => openTrust(p.id) }] : []),
  ]);

  function onKey(e) {
    if (keyIgnored(e)) return;
    const k = { f: fetchNow, p: loadPrs, t: () => openTrust(p.id), x: startRemoval }[e.key];
    if (k) {
      e.preventDefault();
      k();
    }
  }
</script>

{#snippet wrow(w)}
  <tr>
    <td class="mono">{w.branch ?? '—'}</td>
    <td class="mono">{short(w.path)}</td>
    <td>{verdict(w)}{unmerged(w) ? ' · unmerged work' : ''}{w.in_use_by?.length ? ` · used by ${w.in_use_by.join(', ')}` : w.row?.agent_id ? ` · ${w.row.agent_id}` : ''}{lockText(w)}</td>
    <td>
      <span class="head-acts">
        {#if !gone(w)}
          <button type="button" class="icon-btn" aria-label="Diff" use:tooltip={'Diff'} onclick={() => openDiff(p.id, w.path, w.branch)}>±</button>
        {/if}
        <button type="button" class="icon-btn danger" aria-label="Remove worktree…" disabled={!!busy || !!w.in_use_by?.length} use:tooltip={w.in_use_by?.length ? `used by ${w.in_use_by.join(', ')}: archive or kill it first` : 'Remove worktree…'} onclick={() => removeWorktree(w)}>{'\u{F01B4}'}</button>
      </span>
    </td>
  </tr>
{/snippet}

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
      <span class="head-acts">
        <button type="button" class="icon-btn" aria-label="Project settings" use:tooltip={'Project settings'} onclick={() => openProjectSettings(p.id)}>{'\u{F0493}'}</button>
        <button type="button" class="icon-btn danger" aria-label="Remove project…" use:tooltip={{ label: 'Remove project…', key: 'x' }} onclick={startRemoval}>{'\u{F01B4}'}</button>
        <Menu items={menu} label="More actions" />
      </span>
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
          {:else}<span class="warn">Not trusted yet</span> · Claude Code asks once per repository{/if}
        </dd>
      </dl>
    </section>

    {#if err}<p class="err">{err}</p>{/if}

    <section class="card">
      <div class="ch"><b>Sessions</b><span class="muted">{list.length}</span></div>
      {#each list as a (a.id)}
        <button class="row" onclick={() => openAgent(a.id)}><StatusBadge state={statusOf(a.id)} compact /> <b>{a.id}</b> <span class="muted">{a.branch ?? ''}</span></button>
      {:else}
        <p class="muted">No session. <button type="button" class="btn ghost" use:tooltip={{ label: 'New session', key: '⌘T' }} onclick={() => (app.overlay = { kind: 'new-session' })}>New session</button></p>
      {/each}
    </section>

    <section class="card">
      <div class="ch">
        <b>Worktrees</b><span class="muted">sibling folder {p.name}.worktrees · unmerged work is never deleted without confirmation</span><span class="grow"></span>
        {#if prunable.length}
          <button type="button" class="btn" disabled={!!busy} onclick={pruneGone}>Prune {prunable.length} gone…</button>
        {/if}
      </div>
      <table>
        <thead><tr><th>Branch</th><th>Folder</th><th>State</th><th></th></tr></thead>
        <tbody>
          {#each managed as w (w.path)}
            {@render wrow(w)}
          {:else}
            <tr><td colspan="4" class="muted">{d ? 'No worktree of the app.' : 'Loading…'}</td></tr>
          {/each}
          {#if outside.length}
            <tr><th colspan="4" class="sub">Not created by the app</th></tr>
            {#each outside as w (w.path)}
              {@render wrow(w)}
            {/each}
          {/if}
        </tbody>
      </table>
      {#if orchBranches.length}
        <p class="muted">orch/ branches: {orchBranches.map((b) => `${b.name}${b.merged ? ' (merged)' : b.ahead_of_base ? ` (+${b.ahead_of_base})` : ''}`).join(' · ')}</p>
      {/if}
    </section>

    <section class="card">
      <div class="ch"><b>Pull requests</b><span class="muted">gh</span><span class="grow"></span><button type="button" class="btn" onclick={loadPrs} disabled={!!busy} use:tooltip={{ label: 'Load PRs', key: 'p' }}>{busy === 'prs' ? 'Loading…' : 'Load PRs'}</button></div>
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
  .sub {
    padding-top: var(--space-sm);
  }
  .warn {
    color: var(--color-status-queued);
  }
</style>
