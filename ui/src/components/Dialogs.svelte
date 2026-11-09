<script>
  // Dialogs: New session (⌘T, Penpot 06 J2 step 1), Add project (⌘N: a folder
  // or a git URL to clone, J1), Launch a team on a feature (Penpot 04 Launch
  // on feature, J3 step 2), the weekly plan, deleting an agent or a feature
  // (FR-202: killed, checked, hidden; worktrees on request), confirmations.
  // ⌘⏎ submits; esc cancels. Each is drawn in the FloatingWindow shell
  // (design-notes §3.3): backdrop, title bar with ×, focus trap; the keys are
  // in the buttons' tooltips.
  import { untrack } from 'svelte';
  import { activeAgents, agent, app, contextProject, deleteAgent, deleteFeature, features, focusCard, openAgent, openFeature, openProject, openProjectSettings, openSettings, openUrl, projects, refresh, toast } from '../lib/app.svelte.js';
  import { designTools, effectiveServers, kindLabel, nodeList } from '../lib/ecosystem.js';
  import { clock, expandHome, shortPath } from '../lib/format.js';
  import { blockOf } from '../lib/inbox.js';
  import { basisText, dialogEstimate, originChip, SOURCE_NAME, staleBanner, staleOf } from '../lib/intake.js';
  import { endStarting } from '../lib/intake.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { counts, liveText } from '../lib/now.svelte.js';
  import { EFFORTS, MODEL_CHOICES, PERMISSION_MODES } from '../lib/options.js';
  import { nextOffHours, OFF_DISABLED, offHoursAllowed } from '../lib/pacing.js';
  import { PRESSURE } from '../lib/status.js';
  import { runFinished, slugify } from '../lib/teams.js';
  import { tooltip } from '../lib/tooltip.js';
  import ConnBadge from './ConnBadge.svelte';
  import FloatingWindow from './FloatingWindow.svelte';
  import Kbd from './Kbd.svelte';
  import PlanEditor from './PlanEditor.svelte';
  import Select from './Select.svelte';

  const o = $derived(app.overlay);
  // The dialogs keep their small sizes in the window shell (design-notes §3.3).
  const WIDE = { w: 560, h: null };
  const SMALL = { w: 420, h: null };
  const DIALOG_MIN_W = 320;
  let busy = $state(false);
  let error = $state(null);
  // P4.5: the item or brief went stale while the dialog was open (notes §5.3); the form is disabled.
  let stale = $state(null);
  let startBtn = $state();
  let briefEl = $state();
  let messageEl = $state();
  // New session
  let form = $state({ project_id: '', name: '', model: 'haiku', effort: '', permission_mode: '', base: '', message: '' });
  // Add project
  let path = $state('');
  let cloneInto = $state('');
  const isUrl = (x) => /^(https?:\/\/|git@|ssh:\/\/)/.test(x.trim());

  // Roles for the New session dialog (a role's settings for a solo session).
  let prefilled = null; // the new-session overlay whose prefill was applied
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
      const pf = ov.prefill;
      const project_id = pf?.project_id ?? ov.projectId ?? contextProject()?.id ?? '';
      stale = null;
      launch = pf
        ? { project_id, team_id: pf.team_id ?? '', existing: '', slug: pf.slug ?? '', title: pf.title ?? '', brief: pf.brief ?? '', base: pf.base ?? '', off_hours: !!pf.off_hours }
        : { project_id, team_id: ov.teamId ?? '', existing: '', slug: '', title: '', brief: '', base: '', off_hours: false };
      // The user reads an item's text first (focus on the primary button); Edit from a brief card focuses the brief.
      if (pf) queueMicrotask(() => (ov.focus === 'brief' ? briefEl : startBtn)?.focus());
      invoke('planner_view')
        .then((pv) => (offHours = pv.off_hours ?? null))
        .catch(() => (offHours = null));
      loadTeams(project_id);
    });
  });
  const projectOptions = $derived(projects().map((p) => ({ value: p.id, label: p.name })));
  const roleOptions = $derived([{ value: '', label: 'plain Claude' }, ...roles.filter((r) => r.id !== 'lead').map((r) => ({ value: r.id, label: r.name }))]);
  const teamOptions = $derived(
    (teamsView?.teams ?? []).map((t) => ({
      value: t.team.id,
      label: t.team.name,
      disabled: t.issues.length > 0,
      reason: t.issues.length ? `Invalid: ${t.issues[0].text}` : undefined,
    })),
  );
  const launchTeam = $derived(teamsView?.teams.find((t) => t.team.id === launch.team_id));
  const launchTools = $derived(designTools(launchTeam?.team, teamsView?.roles, registry));
  const free = $derived(features().filter((f) => f.project_id === launch.project_id && (!f.run || runFinished(f.run.status))));
  const launchSlug = $derived(launch.existing ? (free.find((f) => f.id === launch.existing)?.slug ?? '') : launch.slug || slugify(launch.title));

  async function launchTeamOnFeature() {
    if (busy || stale) return;
    busy = true;
    error = null;
    try {
      const run = await invoke('team_launch', {
        form: { project_id: launch.project_id, team_id: launch.team_id, slug: launchSlug, title: launch.title, brief: launch.brief, base: launch.base || null, off_hours: launch.off_hours || undefined, intake: o.intake ?? undefined },
      });
      close();
      await refresh();
      openFeature(run.feature_id);
      toast(`Launched ${launchTeam?.team.name ?? launch.team_id} on ${launchSlug} · the lead starts when the Governor admits it`);
    } catch (e) {
      failed(e);
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
    if (o?.kind === 'new-session' && o.prefill && o !== prefilled) {
      // A prefilled session (P4.5): project, name and the item's text; the rest keeps the form's.
      prefilled = o;
      const pf = o.prefill;
      untrack(() => {
        form = { ...form, project_id: pf.project_id ?? form.project_id, name: pf.name ?? '', message: pf.message ?? '' };
        stale = null;
        queueMicrotask(() => (o.focus === 'brief' ? messageEl : startBtn)?.focus());
      });
    }
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
  // The core's counts; its cap until the `now` snapshot is read (the Governor's).
  const c = $derived.by(() => {
    const n = counts();
    return { ...n, live_cap: n.live_cap ?? snap?.live_cap };
  });

  function close() {
    // Read before the overlay is cleared: `o` is derived from it.
    const prefilledSession = o?.kind === 'new-session' && !!o.prefill;
    app.overlay = null;
    error = null;
    stale = null;
    busy = false;
    // A cancelled prefilled session leaves nothing of the item's text for the next plain one.
    if (prefilledSession) form = { ...form, name: '', message: '' };
    endStarting();
  }

  /** A start that failed: `stale` (P4.5 notes §5.3) disables the form with a banner; any other error is inline. */
  function failed(e) {
    const s = o?.intake ? staleOf(e) : null;
    if (s) stale = s;
    else error = errText(e);
    busy = false;
  }

  async function startSession() {
    if (busy || stale) return;
    busy = true;
    error = null;
    try {
      const role = roles.find((x) => x.id === roleId);
      const id = await invoke('agent_start', { req: { ...$state.snapshot(form), name: form.name.trim(), prompt: role?.prompt || null, intake: o.intake ?? undefined } });
      close();
      form = { ...form, name: '', message: '' };
      await refresh();
      openAgent(id);
      toast(`Started ${id} · ${project?.name ?? ''}`);
    } catch (e) {
      failed(e);
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

  // Esc is the window's (it closes the dialog through `close`).
  function onKey(e, submit) {
    if (e.defaultPrevented) return; // an Enter a Select consumed
    if (e.key === 'Enter' && (e.metaKey || e.target.tagName === 'INPUT')) return (e.preventDefault(), submit());
  }
</script>

{#snippet fromIntake()}
  <!-- P4.5 notes §5.1–§5.3: the origin chip, the stale banner, the estimate and the untrusted-text note. -->
  {#if o.origin}
    {@const chip = originChip(o.origin)}
    <p class="chip" data-origin><span class="muted">{chip.text}</span>{#if chip.url}{' '}<button type="button" class="linkbtn" onclick={() => openUrl(chip.url)} aria-label="Open {chip.ref} at {SOURCE_NAME[chip.source]}">↗</button>{:else if chip.brief}{' '}<button type="button" class="linkbtn" onclick={() => (close(), focusCard(chip.decision))} aria-label="Show the brief card">↗</button>{/if}</p>
  {/if}
  {#if stale}<p class="err" role="alert" data-stale>{staleBanner(stale)}</p>{/if}
{/snippet}
{#snippet intakeNote(mode)}
  {#if o.item}
    {@const est = dialogEstimate(o.item, mode, projects().find((p) => p.id === o.item.project)?.name)}
    {#if est}<p class="muted note" data-estimate>{est}</p>{/if}
    <p class="muted note" data-untrusted>This text comes from {SOURCE_NAME[o.item.source]}. It goes to the agent as part of the brief you send.</p>
  {:else if o.origin?.kind === 'brief'}
    <!-- A brief's estimate is its cost block's (notes §5.2). -->
    {@const dec = app.now.decisions?.[o.origin.decision]}
    {@const est = blockOf(dec, 'cost')?.estimate}
    {@const name = projects().find((p) => p.id === (dec?.payload?.project ?? dec?.project))?.name}
    {@const line = est ? dialogEstimate({ estimate: est }, mode, name) : ''}
    {#if line}<p class="muted note" data-estimate>{line}{est[mode] ? ` · ${basisText(est[mode], mode, est.size ?? 'M', name)}` : ''}</p>{/if}
  {/if}
{/snippet}

{#if o && ['new-session', 'add-project', 'launch', 'plan', 'delete-agent', 'delete-feature', 'confirm'].includes(o.kind)}
  {#if o.kind === 'new-session'}
    <FloatingWindow title="New session" size={WIDE} minW={DIALOG_MIN_W} onclose={close} onkeydown={(e) => onKey(e, startSession)}>
    <form onsubmit={(e) => (e.preventDefault(), startSession())}>
      {@render fromIntake()}
      <label>Project
        <Select bind:value={form.project_id} options={projectOptions} noun="project" />
      </label>
      <label>Role
        <Select value={roleId} options={roleOptions} noun="role" onchange={pickRole} />
      </label>
      <label>Task <span class="muted">(first message, optional)</span>
        <textarea class="field" rows={o.prefill ? 12 : 3} bind:value={form.message} bind:this={messageEl} disabled={!!stale} placeholder="What should Claude do?"></textarea>
      </label>
      {@render intakeNote('solo')}
      <div class="row">
        <label>Name <!-- svelte-ignore a11y_autofocus --><input class="field" bind:value={form.name} autofocus spellcheck="false" /></label>
        <label>Base branch <input class="field" bind:value={form.base} placeholder={project?.git?.branch ?? 'default'} spellcheck="false" /></label>
      </div>
      <p class="muted">Worktree: <span class="mono">orch/solo/{form.name || '…'}</span> in <span class="mono">{project ? `${project.name}.worktrees/${form.name}` : '…'}</span> (sibling folder, locked while in use)</p>
      <div class="row">
        <label>Model
          <Select bind:value={form.model} options={MODEL_CHOICES} />
        </label>
        <label>Effort
          <Select bind:value={form.effort} options={EFFORTS} />
        </label>
        <label>Permissions
          <Select bind:value={form.permission_mode} options={PERMISSION_MODES} />
        </label>
      </div>
      {#if snap}
        <p class="note"><span style:color={PRESSURE[snap.state]?.color}>{PRESSURE[snap.state]?.icon}</span> Governor {PRESSURE[snap.state]?.label}{app.data?.usage?.shadow ? ' (shadow mode)' : ''} · {liveText(c)} · your own typing is never blocked; app-delivered turns go through the Governor.</p>
      {/if}
      {#if error}<p class="err">{error}</p>{/if}
      <div class="foot">
        <button type="button" class="btn ghost" onclick={close} use:tooltip={{ label: 'Cancel', key: 'esc' }}>Cancel</button>
        {#if stale}
          <button type="button" class="btn primary" bind:this={startBtn} onclick={close}>Close</button>
        {:else}
          <button type="submit" class="btn primary" bind:this={startBtn} disabled={busy || !form.project_id} use:tooltip={{ label: 'Start session', key: '⌘⏎' }}>{busy ? 'Starting…' : 'Start session'}</button>
        {/if}
      </div>
    </form>
    </FloatingWindow>
  {:else if o.kind === 'launch'}
    <FloatingWindow title="Launch {launchTeam?.team.name ?? 'a team'}" size={WIDE} minW={DIALOG_MIN_W} onclose={close} onkeydown={(e) => onKey(e, launchTeamOnFeature)}>
    <form onsubmit={(e) => (e.preventDefault(), launchTeamOnFeature())}>
      {@render fromIntake()}
      <div class="seg" role="radiogroup" aria-label="Feature">
        <button type="button" class:on={!launch.existing} onclick={() => (launch.existing = '')}>New feature</button>
        {#each free as f (f.id)}<button type="button" class:on={launch.existing === f.id} onclick={() => (launch.existing = f.id)}>Existing: {f.slug}</button>{/each}
      </div>
      <div class="row">
        <label>Project
          <Select value={launch.project_id} options={projectOptions} noun="project" onchange={setLaunchProject} />
        </label>
        <label>Team
          <Select bind:value={launch.team_id} options={teamOptions} noun="team" />
        </label>
      </div>
      {#if !launch.existing}
        <div class="row">
          <label>Slug <input class="field mono" bind:value={launch.slug} placeholder={slugify(launch.title) || 'search-v2'} spellcheck="false" /></label>
          <label>Base <input class="field mono" bind:value={launch.base} placeholder={projects().find((p) => p.id === launch.project_id)?.default_branch ?? 'main'} spellcheck="false" /></label>
        </div>
        <!-- svelte-ignore a11y_autofocus -->
        <label>Title <input class="field" bind:value={launch.title} autofocus={!o.prefill} disabled={!!stale} placeholder="Search v2" /></label>
      {/if}
      <label>Brief <span class="muted">({o.prefill ? 'the item as markdown' : '3 lines'})</span>
        <textarea class="field" rows={o.prefill ? 12 : 3} bind:value={launch.brief} bind:this={briefEl} disabled={!!stale} placeholder="What to build, the constraints, where the designs are."></textarea>
      </label>
      {@render intakeNote('team')}
      <div class="note what">
        <span>• Creates <span class="mono">orch/{launchSlug || '…'}/integration</span> and the lead's worktree on it.</span>
        <span>• The lead drafts spec.md (and ux-brief.md, api-contract.md) in <span class="mono">.orchestrator/features/{launchSlug || '…'}/</span>, then asks for the Spec gate.</span>
        <span>• Specialists start after the gate, as the Governor and the live-agent cap allow.</span>
      </div>
      <label class="offh" title={offHoursAllowed(offHours) ? '' : OFF_DISABLED}><input type="checkbox" bind:checked={launch.off_hours} disabled={!offHoursAllowed(offHours)} /> Run off-hours{nextOffHours(offHours) ? ` (starts ${clock(nextOffHours(offHours))})` : ''}</label>
      {#each launchTools as d (d.server.id)}
        {#if d.server.status === 'needs_auth'}
          <p class="note tools warn">{d.server.name} MCP needs authentication: {nodeList(d.nodes, true)} will fail its tools. <button type="button" class="linkbtn" onclick={() => (launch.project_id ? openProjectSettings(launch.project_id, 'mcp', d.server.id) : openSettings('mcp', d.server.id))}>Open Settings</button> <Kbd keys="⌘+," /> · Launch anyway <Kbd keys="⌘+⏎" /></p>
        {:else}
          <p class="note tools">Design tools: {nodeList(d.nodes)} use <b>{d.server.name}</b> ({kindLabel(d.server.kind)} · <ConnBadge status={d.server.status} connector={d.server.kind === 'connector'} />)</p>
        {/if}
      {/each}
      {#if snap}
        <p class="note"><span style:color={PRESSURE[snap.state]?.color}>{PRESSURE[snap.state]?.icon}</span> Governor {PRESSURE[snap.state]?.label}{app.data?.usage?.shadow ? ' (shadow mode)' : ''} · {liveText(c)}: the lead {c.live >= c.live_cap ? 'queues for a live slot' : 'starts when its first turn is admitted'}.</p>
      {/if}
      {#if error}<p class="err">{error}</p>{/if}
      <div class="foot">
        <button type="button" class="btn ghost" onclick={close} use:tooltip={{ label: 'Cancel', key: 'esc' }}>Cancel</button>
        {#if stale}
          <button type="button" class="btn primary" bind:this={startBtn} onclick={close}>Close</button>
        {:else}
          <button type="submit" class="btn primary" bind:this={startBtn} disabled={busy || !launch.team_id || !launchSlug || !launch.project_id} use:tooltip={{ label: 'Launch', key: '⌘⏎' }}>{busy ? 'Launching…' : 'Launch'}</button>
        {/if}
      </div>
    </form>
    </FloatingWindow>
  {:else if o.kind === 'add-project'}
    <FloatingWindow title="Add project" size={WIDE} minW={DIALOG_MIN_W} onclose={close} onkeydown={(e) => onKey(e, addProject)}>
    <form onsubmit={(e) => (e.preventDefault(), addProject())}>
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
        <button type="button" class="btn ghost" onclick={close} use:tooltip={{ label: 'Cancel', key: 'esc' }}>Cancel</button>
        <button type="submit" class="btn primary" disabled={busy || !path.trim()} use:tooltip={{ label: isUrl(path) ? 'Clone' : 'Add', key: '⏎' }}>{busy ? (isUrl(path) ? 'Cloning…' : 'Adding…') : isUrl(path) ? 'Clone' : 'Add'}</button>
      </div>
    </form>
    </FloatingWindow>
  {:else if o.kind === 'delete-agent'}
    <FloatingWindow title="Delete {o.agentId}?" role="alertdialog" size={SMALL} minW={DIALOG_MIN_W} onclose={close} onkeydown={(e) => onKey(e, confirmDeleteAgent)}>
    <form onsubmit={(e) => (e.preventDefault(), confirmDeleteAgent())}>
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
        <button type="button" class="btn ghost" onclick={close} use:tooltip={{ label: 'Cancel', key: 'esc' }}>Cancel</button>
        <!-- svelte-ignore a11y_autofocus -->
        <button type="submit" class="btn danger" autofocus disabled={busy} use:tooltip={{ label: 'Kill and delete', key: '⏎' }}>{busy ? 'Deleting…' : 'Kill and delete'}</button>
      </div>
    </form>
    </FloatingWindow>
  {:else if o.kind === 'delete-feature'}
    <FloatingWindow title="Delete feature {delFeature?.slug ?? ''}?" role="alertdialog" size={SMALL} minW={DIALOG_MIN_W} onclose={close} onkeydown={(e) => onKey(e, confirmDeleteFeature)}>
    <form onsubmit={(e) => (e.preventDefault(), confirmDeleteFeature())}>
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
        <button type="button" class="btn ghost" onclick={close} use:tooltip={{ label: 'Cancel', key: 'esc' }}>Cancel</button>
        <!-- svelte-ignore a11y_autofocus -->
        <button type="submit" class="btn danger" autofocus disabled={busy || !delFeature} use:tooltip={{ label: 'Delete feature', key: '⏎' }}>{busy ? 'Deleting…' : 'Delete feature'}</button>
      </div>
    </form>
    </FloatingWindow>
  {:else if o.kind === 'plan'}
    <FloatingWindow title="Weekly plan" size={WIDE} minW={DIALOG_MIN_W} onclose={close}>
      <PlanEditor onSaved={close} onCancel={close} />
    </FloatingWindow>
  {:else}
    <FloatingWindow title={o.text} role="alertdialog" size={SMALL} minW={DIALOG_MIN_W} onclose={close} onkeydown={(e) => onKey(e, () => (close(), o.onYes()))}>
    <form onsubmit={(e) => (e.preventDefault(), close(), o.onYes())}>
      <p class="secondary detail">{o.detail}</p>
      <div class="foot">
        <button type="button" class="btn ghost" onclick={close} use:tooltip={{ label: o.no ?? 'Cancel', key: 'esc' }}>{o.no ?? 'Cancel'}</button>
        <!-- svelte-ignore a11y_autofocus -->
        <button type="submit" class="btn primary" autofocus use:tooltip={{ label: o.yes, key: '⏎' }}>{o.yes}</button>
      </div>
    </form>
    </FloatingWindow>
  {/if}
{/if}

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    font: var(--text-label);
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
  .chip {
    display: inline-block;
    margin: 0;
    padding: 0 var(--space-sm);
    border-radius: var(--radius-full, 999px);
    background: var(--color-surface-raised);
    font: var(--text-caption);
  }
  .err {
    white-space: pre-wrap;
  }
  .detail {
    white-space: pre-line;
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
