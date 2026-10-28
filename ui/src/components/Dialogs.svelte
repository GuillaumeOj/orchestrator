<script>
  // Dialogs: New session (⌘T, Penpot 06 J2 step 1), Add project (⌘N: a folder
  // or a git URL to clone, J1), Launch a team on a feature (Penpot 04 Launch
  // on feature, J3 step 2), the weekly plan, deleting an agent or a feature
  // (FR-202: killed, checked, hidden; worktrees on request), confirmations.
  // ⌘⏎ submits; esc cancels.
  import { untrack } from 'svelte';
  import { activeAgents, agent, app, contextProject, deleteAgent, deleteFeature, features, openAgent, openFeature, openProject, openSettings, projects, refresh, toast } from '../lib/app.svelte.js';
  import { connStatus, designTools, effectiveServers, nodeList } from '../lib/ecosystem.js';
  import { clock, expandHome, shortPath } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { nextOffHours, OFF_DISABLED, offHoursAllowed } from '../lib/pacing.js';
  import { PRESSURE } from '../lib/status.js';
  import { runFinished, slugify } from '../lib/teams.js';
  import Kbd from './Kbd.svelte';
  import PlanEditor from './PlanEditor.svelte';

  const o = $derived(app.overlay);
  let busy = $state(false);
  let error = $state(null);
  // New session
  let form = $state({ project_id: '', name: '', model: 'haiku', effort: '', permission_mode: '', base: '', message: '' });
  // Add project
  let path = $state('');
  let cloneInto = $state('');
  const isUrl = (x) => /^(https?:\/\/|git@|ssh:\/\/)/.test(x.trim());

  // Roles for the New session dialog (a role's settings for a solo session).
  let roles = $state([]);
  let roleId = $state('');
  // Launch on feature
  let launch = $state({ project_id: '', team_id: '', existing: '', slug: '', title: '', brief: '', base: '', off_hours: false });
  let offHours = $state(null); // planner_view.off_hours: the next window for "Run off-hours"
  let teamsView = $state(null);
  let registry = $state([]); // MCP servers the project sees (the Design tools line)
  /** The teams seen from `project_id` (its own and the global ones). A team
   *  that isn't among them, or an existing feature of another project, is
   *  dropped from the form. */
  function loadTeams(project_id) {
    error = null;
    invoke('teams_view', { project: project_id || null })
      .then((v) => {
        if (launch.project_id !== project_id) return;
        teamsView = v;
        if (!v.teams.some((t) => t.team.id === launch.team_id)) launch.team_id = v.teams[0]?.team.id ?? '';
      })
      .catch((e) => (error = errText(e)));
    invoke('mcp_registry_view', { project: project_id || null })
      .then((r) => launch.project_id === project_id && (registry = effectiveServers(r, project_id || null)))
      .catch(() => (registry = []));
  }
  function setLaunchProject(project_id) {
    launch.project_id = project_id;
    launch.existing = '';
    loadTeams(project_id);
  }
  $effect(() => {
    // Once per opening: only the overlay is tracked (the form must not reset
    // with the poll, nor depend on what it writes).
    const ov = o;
    if (ov?.kind !== 'launch') return;
    untrack(() => {
      const project_id = ov.projectId ?? contextProject()?.id ?? '';
      launch = { project_id, team_id: ov.teamId ?? '', existing: '', slug: '', title: '', brief: '', base: '', off_hours: false };
      invoke('planner_view')
        .then((pv) => (offHours = pv.off_hours ?? null))
        .catch(() => (offHours = null));
      loadTeams(project_id);
    });
  });
  const launchTeam = $derived(teamsView?.teams.find((t) => t.team.id === launch.team_id));
  const launchTools = $derived(designTools(launchTeam?.team, teamsView?.roles, registry));
  const free = $derived(features().filter((f) => f.project_id === launch.project_id && (!f.run || runFinished(f.run.status))));
  const launchSlug = $derived(launch.existing ? (free.find((f) => f.id === launch.existing)?.slug ?? '') : launch.slug || slugify(launch.title));

  async function launchTeamOnFeature() {
    if (busy) return;
    busy = true;
    error = null;
    try {
      const run = await invoke('team_launch', {
        form: { project_id: launch.project_id, team_id: launch.team_id, slug: launchSlug, title: launch.title, brief: launch.brief, base: launch.base || null, off_hours: launch.off_hours || undefined },
      });
      close();
      await refresh();
      openFeature(run.feature_id);
      toast(`Launched ${launchTeam?.team.name ?? launch.team_id} on ${launchSlug} · the lead starts when the Governor admits it`);
    } catch (e) {
      error = errText(e);
      busy = false;
    }
  }

  // Delete an agent or a feature. A feature removes its worktrees by
  // default (unmerged ones are kept unless discarded); an agent's worktree
  // (merged? dirty? shared?) decides its default once its plan is loaded.
  let del = $state({ plan: null, remove: false, discard: false });
  $effect(() => {
    const ov = o;
    if (!ov?.kind.startsWith('delete-')) return;
    untrack(() => {
      del = { plan: null, remove: ov.kind === 'delete-feature', discard: false };
      error = null;
      if (ov.kind !== 'delete-agent') return;
      invoke('agent_action', { id: ov.agentId, action: 'delete_plan', message: null })
        .then((plan) => {
          if (app.overlay !== ov) return;
          del = { plan, remove: !!plan.worktree && !plan.shared_with.length && plan.worktree.safe_to_remove, discard: false };
        })
        .catch((e) => (error = errText(e)));
    });
  });
  const delWt = $derived(del.plan?.worktree ?? null);
  const delAgent = $derived(o?.kind === 'delete-agent' ? agent(o.agentId) : null);

  /** Runs a delete; a failure seen after the dialog was dismissed (the kill
   *  takes seconds) becomes a toast instead of an error nobody sees. */
  async function runDelete(what, fn) {
    if (busy) return;
    const ov = app.overlay;
    busy = true;
    error = null;
    try {
      await fn();
      if (app.overlay === ov) close();
    } catch (e) {
      if (app.overlay !== ov) return void toast(`${what}: ${errText(e)}`, { kind: 'error' });
      error = errText(e);
      busy = false;
    }
  }

  // The core ignores `discard` unless the worktree is removed.
  const confirmDeleteAgent = () => runDelete(o.agentId, () => deleteAgent(o.agentId, del.remove, del.discard));

  const delFeature = $derived(o?.kind === 'delete-feature' ? features().find((f) => f.id === o.featureId) : null);
  const delFeatureAgents = $derived(delFeature ? activeAgents().filter((a) => a.team?.feature_id === delFeature.id) : []);

  const confirmDeleteFeature = () => runDelete(delFeature?.slug ?? o.featureId, () => deleteFeature(o.featureId, del.remove, del.discard));

  $effect(() => {
    if (o?.kind !== 'new-session') return;
    untrack(() => {
      if (!roles.length)
        invoke('teams_view', { project: null })
          .then((v) => (roles = v.roles))
          .catch(() => {});
    });
  });

  function pickRole(id) {
    roleId = id;
    const r = roles.find((x) => x.id === id);
    if (!r) return;
    form.model = r.model ?? '';
    form.effort = r.effort ?? '';
    form.permission_mode = { strict: '', gated: 'acceptEdits', autonomous: 'auto' }[r.permission_profile] ?? '';
  }

  $effect(() => {
    if (o?.kind === 'new-session') {
      const p = contextProject();
      form.project_id = form.project_id || p?.id || '';
      const used = new Set((app.data?.agents ?? []).map((a) => a.id));
      let n = 1;
      while (used.has(`solo-${n}`)) n++;
      form.name = form.name || `solo-${n}`;
      error = null;
    }
  });

  const project = $derived(projects().find((p) => p.id === form.project_id));
  const snap = $derived(app.data?.usage?.snapshot);
  const live = $derived((app.data?.agents ?? []).filter((a) => a.live).length);

  function close() {
    app.overlay = null;
    error = null;
    busy = false;
  }

  async function startSession() {
    if (busy) return;
    busy = true;
    error = null;
    try {
      const role = roles.find((x) => x.id === roleId);
      const id = await invoke('agent_start', { req: { ...$state.snapshot(form), name: form.name.trim(), prompt: role?.prompt || null } });
      close();
      form = { ...form, name: '', message: '' };
      await refresh();
      openAgent(id);
      toast(`Started ${id} · ${project?.name ?? ''}`);
    } catch (e) {
      error = errText(e);
      busy = false;
    }
  }

  async function addProject() {
    if (busy) return;
    busy = true;
    try {
      const expand = (x) => expandHome(x, app.data?.core?.user_home ?? '~');
      const p = isUrl(path)
        ? await invoke('project_clone', { url: path.trim(), parent: cloneInto.trim() ? expand(cloneInto) : null, name: null })
        : await invoke('project_add', { path: expand(path) });
      close();
      path = '';
      await refresh();
      openProject(p.id);
      toast(`Added ${p.name} · fetching in the background`);
    } catch (e) {
      error = errText(e);
      busy = false;
    }
  }

  function onKey(e, submit) {
    if (e.key === 'Escape') return (e.preventDefault(), close());
    if (e.key === 'Enter' && (e.metaKey || e.target.tagName === 'INPUT')) return (e.preventDefault(), submit());
  }
</script>

{#if o && ['new-session', 'add-project', 'launch', 'plan', 'delete-agent', 'delete-feature', 'confirm'].includes(o.kind)}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={close}></div>
  {#if o.kind === 'new-session'}
    <div class="dlg" role="dialog" aria-modal="true" aria-label="New session" tabindex="-1" onkeydown={(e) => onKey(e, startSession)}>
    <form onsubmit={(e) => (e.preventDefault(), startSession())}>
      <h3>New session</h3>
      <label>Project
        <select class="field" bind:value={form.project_id}>
          {#each projects() as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
        </select>
      </label>
      <label>Role
        <select class="field" value={roleId} onchange={(e) => pickRole(e.currentTarget.value)}>
          <option value="">plain Claude</option>
          {#each roles.filter((r) => r.id !== 'lead') as r (r.id)}<option value={r.id}>{r.name}</option>{/each}
        </select>
      </label>
      <label>Task <span class="muted">(first message, optional)</span>
        <textarea class="field" rows="3" bind:value={form.message} placeholder="What should Claude do?"></textarea>
      </label>
      <div class="row">
        <label>Name <!-- svelte-ignore a11y_autofocus --><input class="field" bind:value={form.name} autofocus spellcheck="false" /></label>
        <label>Base branch <input class="field" bind:value={form.base} placeholder={project?.git?.branch ?? 'default'} spellcheck="false" /></label>
      </div>
      <p class="muted">Worktree: <span class="mono">orch/solo/{form.name || '…'}</span> in <span class="mono">{project ? `${project.name}.worktrees/${form.name}` : '…'}</span> (sibling folder, locked while in use)</p>
      <div class="row">
        <label>Model
          <select class="field" bind:value={form.model}>
            <option value="">default</option><option value="haiku">haiku</option><option value="sonnet">sonnet</option><option value="opus">opus</option>
          </select>
        </label>
        <label>Effort
          <select class="field" bind:value={form.effort}>
            <option value="">default</option><option>low</option><option>medium</option><option>high</option>
          </select>
        </label>
        <label>Permissions
          <select class="field" bind:value={form.permission_mode}>
            <option value="">default (ask)</option><option value="acceptEdits">accept edits</option><option value="plan">plan</option>
          </select>
        </label>
      </div>
      {#if snap}
        <p class="note"><span style:color={PRESSURE[snap.state]?.color}>{PRESSURE[snap.state]?.icon}</span> Governor {PRESSURE[snap.state]?.label}{app.data?.usage?.shadow ? ' (shadow mode)' : ''} · {live}/{snap.live_cap} live · your own typing is never blocked; app-delivered turns go through the Governor.</p>
      {/if}
      {#if error}<p class="err">{error}</p>{/if}
      <div class="foot">
        <button type="button" class="btn ghost" onclick={close}>Cancel <Kbd keys="esc" /></button>
        <button type="submit" class="btn primary" disabled={busy || !form.project_id}>{busy ? 'Starting…' : 'Start session'} <Kbd keys="⌘+⏎" /></button>
      </div>
    </form>
    </div>
  {:else if o.kind === 'launch'}
    <div class="dlg" role="dialog" aria-modal="true" aria-label="Launch on feature" tabindex="-1" onkeydown={(e) => onKey(e, launchTeamOnFeature)}>
    <form onsubmit={(e) => (e.preventDefault(), launchTeamOnFeature())}>
      <h3>Launch {launchTeam?.team.name ?? 'a team'}</h3>
      <div class="seg" role="radiogroup" aria-label="Feature">
        <button type="button" class:on={!launch.existing} onclick={() => (launch.existing = '')}>New feature</button>
        {#each free as f (f.id)}<button type="button" class:on={launch.existing === f.id} onclick={() => (launch.existing = f.id)}>Existing: {f.slug}</button>{/each}
      </div>
      <div class="row">
        <label>Project
          <select class="field" value={launch.project_id} onchange={(e) => setLaunchProject(e.currentTarget.value)}>
            {#each projects() as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
          </select>
        </label>
        <label>Team
          <select class="field" bind:value={launch.team_id}>
            {#each teamsView?.teams ?? [] as t (t.team.id)}<option value={t.team.id} disabled={t.issues.length > 0}>{t.team.name}{t.issues.length ? ' (invalid)' : ''}</option>{/each}
          </select>
        </label>
      </div>
      {#if !launch.existing}
        <div class="row">
          <label>Slug <input class="field mono" bind:value={launch.slug} placeholder={slugify(launch.title) || 'search-v2'} spellcheck="false" /></label>
          <label>Base <input class="field mono" bind:value={launch.base} placeholder={projects().find((p) => p.id === launch.project_id)?.default_branch ?? 'main'} spellcheck="false" /></label>
        </div>
        <!-- svelte-ignore a11y_autofocus -->
        <label>Title <input class="field" bind:value={launch.title} autofocus placeholder="Search v2" /></label>
      {/if}
      <label>Brief <span class="muted">(3 lines)</span>
        <textarea class="field" rows="3" bind:value={launch.brief} placeholder="What to build, the constraints, where the designs are."></textarea>
      </label>
      <div class="note what">
        <span>• Creates <span class="mono">orch/{launchSlug || '…'}/integration</span> and the lead's worktree on it.</span>
        <span>• The lead drafts spec.md (and ux-brief.md, api-contract.md) in <span class="mono">.orchestrator/features/{launchSlug || '…'}/</span>, then asks for the Spec gate.</span>
        <span>• Specialists start after the gate, as the Governor and the live-agent cap allow.</span>
      </div>
      <label class="offh" title={offHoursAllowed(offHours) ? '' : OFF_DISABLED}><input type="checkbox" bind:checked={launch.off_hours} disabled={!offHoursAllowed(offHours)} /> Run off-hours{nextOffHours(offHours) ? ` (starts ${clock(nextOffHours(offHours))})` : ''}</label>
      {#each launchTools as d (d.server.id)}
        {@const st = connStatus(d.server.status, d.server.kind === 'connector')}
        {#if d.server.status === 'needs_auth'}
          <p class="note tools warn">{d.server.name} MCP needs authentication: {nodeList(d.nodes, true)} will fail its tools. <button type="button" class="linkbtn" onclick={() => openSettings('mcp', d.server.id, launch.project_id || null)}>Open Settings</button> <Kbd keys="⌘+," /> · Launch anyway <Kbd keys="⌘+⏎" /></p>
        {:else}
          <p class="note tools">Design tools: {nodeList(d.nodes)} use <b>{d.server.name}</b> ({d.server.kind === 'connector' ? 'claude.ai connector' : d.server.kind} · <span style:color={st.color} aria-hidden="true">{st.icon}</span> {st.label})</p>
        {/if}
      {/each}
      {#if snap}
        <p class="note"><span style:color={PRESSURE[snap.state]?.color}>{PRESSURE[snap.state]?.icon}</span> Governor {PRESSURE[snap.state]?.label}{app.data?.usage?.shadow ? ' (shadow mode)' : ''} · {live}/{snap.live_cap} live: the lead {live >= snap.live_cap ? 'queues for a live slot' : 'starts when its first turn is admitted'}.</p>
      {/if}
      {#if error}<p class="err">{error}</p>{/if}
      <div class="foot">
        <button type="button" class="btn ghost" onclick={close}>Cancel <Kbd keys="esc" /></button>
        <button type="submit" class="btn primary" disabled={busy || !launch.team_id || !launchSlug || !launch.project_id}>{busy ? 'Launching…' : 'Launch'} <Kbd keys="⌘+⏎" /></button>
      </div>
    </form>
    </div>
  {:else if o.kind === 'add-project'}
    <div class="dlg" role="dialog" aria-modal="true" aria-label="Add project" tabindex="-1" onkeydown={(e) => onKey(e, addProject)}>
    <form onsubmit={(e) => (e.preventDefault(), addProject())}>
      <h3>Add project</h3>
      <!-- svelte-ignore a11y_autofocus -->
      <label>Folder or git URL <input class="field mono" bind:value={path} autofocus placeholder="~/dev/my-project or git@github.com:me/repo.git" spellcheck="false" /></label>
      {#if isUrl(path)}
        <label>Clone into <input class="field mono" bind:value={cloneInto} placeholder="~/dev" spellcheck="false" /></label>
        <p class="muted">Cloned with your own git credentials (no prompt; 10 min at most), then registered.</p>
      {:else}
        <p class="muted">A git repository (a worktree registers its main checkout). The app reads its branch, remotes and status, then fetches in the background.</p>
      {/if}
      {#if error}<p class="err">{error}</p>{/if}
      <div class="foot">
        <button type="button" class="btn ghost" onclick={close}>Cancel <Kbd keys="esc" /></button>
        <button type="submit" class="btn primary" disabled={busy || !path.trim()}>{busy ? (isUrl(path) ? 'Cloning…' : 'Adding…') : isUrl(path) ? 'Clone' : 'Add'} <Kbd keys="⏎" /></button>
      </div>
    </form>
    </div>
  {:else if o.kind === 'delete-agent'}
    <div class="dlg small" role="alertdialog" aria-modal="true" aria-label="Delete {o.agentId}" tabindex="-1" onkeydown={(e) => onKey(e, confirmDeleteAgent)}>
    <form onsubmit={(e) => (e.preventDefault(), confirmDeleteAgent())}>
      <h3>Delete {o.agentId}?</h3>
      <div class="note what">
        <span>• {delAgent?.live ? 'Its process' : 'Any process it left'} and every process left in its worktree get SIGTERM, then SIGKILL after 5 s. The app scans again: nothing is deleted while one survives.</span>
        <span>• The agent leaves the sidebar. Its session transcript, usage and audit log stay.</span>
      </div>
      {#if !del.plan && !error}
        <p class="muted">Checking its worktree…</p>
      {:else if delWt}
        {#if del.plan.shared_with.length}
          <p class="muted">Its worktree <span class="mono">{shortPath(delWt.path, app.data?.core?.user_home ?? '')}</span> is also used by {del.plan.shared_with.join(', ')}: it stays.</p>
        {:else}
          <label class="check"><input type="checkbox" bind:checked={del.remove} /> Remove its worktree and branch <span class="mono">{delWt.branch ?? ''}</span></label>
          {#if del.remove && !delWt.safe_to_remove}
            <label class="check"><input type="checkbox" bind:checked={del.discard} /> Discard {[delWt.dirty && `${delWt.dirty} uncommitted file(s)`, delWt.unmerged_commits && `${delWt.unmerged_commits} unmerged commit(s)`].filter(Boolean).join(' and ') || 'unmerged work'} (otherwise the worktree is kept)</label>
          {/if}
        {/if}
      {:else if del.plan}
        <p class="muted">Its worktree is not one of the app's: it stays.</p>
      {/if}
      {#if error}<p class="err">{error}</p>{/if}
      <div class="foot">
        <button type="button" class="btn ghost" onclick={close}>Cancel <Kbd keys="esc" /></button>
        <!-- svelte-ignore a11y_autofocus -->
        <button type="submit" class="btn danger" autofocus disabled={busy}>{busy ? 'Deleting…' : 'Kill and delete'} <Kbd keys="⏎" /></button>
      </div>
    </form>
    </div>
  {:else if o.kind === 'delete-feature'}
    <div class="dlg small" role="alertdialog" aria-modal="true" aria-label="Delete feature" tabindex="-1" onkeydown={(e) => onKey(e, confirmDeleteFeature)}>
    <form onsubmit={(e) => (e.preventDefault(), confirmDeleteFeature())}>
      <h3>Delete feature {delFeature?.slug ?? ''}?</h3>
      <div class="note what">
        <span>• Its team run ends (failed, deleted by the user).</span>
        <span>• {delFeatureAgents.length ? `Killed (checked) and deleted: ${delFeatureAgents.map((a) => a.id).join(', ')}.` : 'No agent left.'}</span>
        <span>• The feature leaves the sidebar. Tasks, timeline, usage and audit log stay.</span>
      </div>
      <label class="check"><input type="checkbox" bind:checked={del.remove} /> Remove its worktrees and branches, <span class="mono">orch/{delFeature?.slug ?? '…'}/integration</span> included</label>
      {#if del.remove}
        <label class="check"><input type="checkbox" bind:checked={del.discard} /> Discard unmerged work (otherwise unmerged worktrees are kept)</label>
      {/if}
      {#if error}<p class="err">{error}</p>{/if}
      <div class="foot">
        <button type="button" class="btn ghost" onclick={close}>Cancel <Kbd keys="esc" /></button>
        <!-- svelte-ignore a11y_autofocus -->
        <button type="submit" class="btn danger" autofocus disabled={busy || !delFeature}>{busy ? 'Deleting…' : 'Delete feature'} <Kbd keys="⏎" /></button>
      </div>
    </form>
    </div>
  {:else if o.kind === 'plan'}
    <div class="dlg" role="dialog" aria-modal="true" aria-label="Weekly plan" tabindex="-1" onkeydown={(e) => e.key === 'Escape' && (e.preventDefault(), close())}>
      <h3>Weekly plan</h3>
      <PlanEditor onSaved={close} onCancel={close} />
    </div>
  {:else}
    <div class="dlg small" role="alertdialog" aria-modal="true" aria-label={o.text} tabindex="-1" onkeydown={(e) => onKey(e, () => (close(), o.onYes()))}>
    <form onsubmit={(e) => (e.preventDefault(), close(), o.onYes())}>
      <h3>{o.text}</h3>
      <p class="secondary">{o.detail}</p>
      <div class="foot">
        <button type="button" class="btn ghost" onclick={close}>Cancel <Kbd keys="esc" /></button>
        <!-- svelte-ignore a11y_autofocus -->
        <button type="submit" class="btn primary" autofocus>{o.yes} <Kbd keys="⏎" /></button>
      </div>
    </form>
    </div>
  {/if}
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: var(--color-surface-chrome);
    opacity: var(--opacity-backdrop);
  }
  .dlg {
    position: fixed;
    top: 80px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 51;
    width: min(560px, 92vw);
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    padding: var(--space-lg);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
    font: var(--text-label);
  }
  .dlg form {
    display: contents;
  }
  .dlg.small {
    width: min(420px, 92vw);
  }
  h3 {
    margin: 0;
    font: var(--text-title);
  }
  label {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    flex: 1;
    color: var(--color-text-secondary);
  }
  .row {
    display: flex;
    gap: var(--space-md);
  }
  p {
    margin: 0;
  }
  .note {
    padding: var(--space-sm);
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
  }
  .offh {
    display: flex;
    flex-direction: row;
    align-items: center;
    gap: var(--space-xs);
  }
  .tools.warn {
    border-left: var(--border-width-focus) solid var(--color-status-waiting-for-you);
  }
  .linkbtn {
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
  .err {
    white-space: pre-wrap;
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-sm);
  }
  .seg {
    display: flex;
    flex-wrap: wrap;
  }
  .seg button {
    padding: var(--space-xxs) var(--space-sm);
    background: none;
    border: var(--border-width-default) solid var(--color-border-subtle);
    color: var(--color-text-secondary);
    font: var(--text-label);
    cursor: pointer;
  }
  .seg button + button {
    border-left: 0;
  }
  .seg button.on {
    background: var(--color-surface-selected);
    color: var(--color-text-primary);
  }
  .what {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    background: var(--color-surface-chrome);
    font: var(--text-caption);
  }
</style>
