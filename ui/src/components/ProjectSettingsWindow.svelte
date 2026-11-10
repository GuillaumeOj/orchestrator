<script module>
  /** The Project settings window's sections, in nav order (General first, design-notes §3.1). */
  export const SECTIONS = [
    { id: 'general', label: 'General' },
    { id: 'files', label: 'Files' },
    { id: 'backend', label: 'Task backend' },
    { id: 'intake', label: 'Intake' },
    { id: 'allocation', label: 'Allocation' },
    { id: 'notion', label: 'Notion' },
    { id: 'mcp', label: 'MCP overrides' },
  ];

</script>

<script>
  // Project settings (design-notes §3, 04-project-settings-*.png; Penpot 04
  // Project settings · Linear, J13 · 3–5): a project's sections in the
  // floating window. Task backend (Local / GitHub Issues / Linear with its
  // team, project and state mapping, Sync now, the D8 confirm), Notion (the
  // parent page for Publish, FR-505), the MCP overrides, Files, Intake and
  // Allocation. Keys in the section: 1–3 backend, j/k mapping rows, ⏎ change
  // a row, r sync, ⌘⏎ save, esc cancel. A dirty footer section asks
  // "Discard changes?" before the window closes or the section changes; a
  // backdrop click on it only flashes the footer.
  import { onMount, untrack } from 'svelte';
  import { app, guardWindow, onWindowClose, openSettings, projects, toast, windowSection } from '../lib/app.svelte.js';
  import { clock } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { isTypingTarget } from '../lib/keyboard.js';
  import { BACKENDS, mappingRows, mappingToSave, remoteName, syncLine } from '../lib/linear.js';
  import { BOARD_COLUMNS } from '../lib/options.js';
  import { tooltip } from '../lib/tooltip.js';
  import AllocationSettings from './AllocationSettings.svelte';
  import ConnBadge from './ConnBadge.svelte';
  import FilesSettings from './FilesSettings.svelte';
  import FloatingWindow from './FloatingWindow.svelte';
  import IntakeSettings from './IntakeSettings.svelte';
  import McpSettings from './McpSettings.svelte';
  import Select from './Select.svelte';

  const spec = $derived(app.window);
  // The window is for one project: a new project mounts a new window.
  const pid = untrack(() => spec?.projectId);
  const name = $derived(projects().find((p) => p.id === pid)?.name ?? pid);

  const section = $derived(SECTIONS.find((s) => s.id === spec?.section) ?? SECTIONS.find((s) => s.id === 'backend'));

  let saved = $state(null); // project_settings
  let error = $state(null);
  let kind = $state('local');
  let linearInt = $state(null); // the linear integration (status, account)
  let teams = $state(null); // null = loading
  let teamsError = $state(null);
  let lin = $state({ team: '', project: '' });
  let states = $state(null); // linear_states
  let rows = $state([]);
  let rowSel = $state(0);
  let notionPage = $state('');
  let ask = $state(null); // D8 { open, done }
  let busy = $state(false);
  let syncing = $state(false);
  let last = $state(null); // { report, at }
  let syncErr = $state(null);
  let unmapped = $state([]); // board_sync.unmapped_states: Linear states with no column
  let root = $state(null);
  let alloc = $state(null); // AllocationSettings (forwards the keys)
  let files = $state(null); // FilesSettings (same)
  let intake = $state(null); // IntakeSettings (⌘⏎ saves)

  const teamOptions = $derived((teams ?? []).map((t) => ({ value: t.key, label: `${t.key} · ${t.name}` })));
  const projectOptions = $derived([{ value: '', label: 'Any project' }, ...(states?.projects ?? []).map((p) => ({ value: p.id, label: p.name }))]);
  const connected = $derived(linearInt?.status === 'connected');
  const backendDirty = $derived(
    !!saved &&
      (kind !== saved.backend?.type ||
        (kind === 'linear' && (lin.team !== saved.backend.team || (lin.project || '') !== (saved.backend.project ?? '') || JSON.stringify(mappingToSave(rows) ?? {}) !== JSON.stringify(saved.backend.states ?? {})))),
  );
  const notionDirty = $derived(!!saved && notionPage !== (saved.settings?.notion_parent_page ?? ''));
  const dirty = $derived(backendDirty || notionDirty);

  /** The shown section has changes that are not saved (the window's Discard guard). */
  function sectionDirty() {
    if (section.id === 'backend') return backendDirty || !!ask;
    if (section.id === 'notion') return notionDirty;
    if (section.id === 'intake') return !!intake?.hasChanges();
    if (section.id === 'allocation') return !!alloc?.hasChanges();
    return false;
  }
  // Discarding (closing or leaving the section): the fields go back to what is saved.
  onMount(() => guardWindow(sectionDirty, () => (dirty || ask) && cancel()));

  async function load() {
    try {
      saved = await invoke('project_settings', { project: pid });
      error = null;
      kind = saved.backend?.type ?? 'local';
      notionPage = saved.settings?.notion_parent_page ?? '';
      if (kind === 'linear') lin = { team: saved.backend.team ?? '', project: saved.backend.project ?? '' };
    } catch (e) {
      error = errText(e);
    }
    // The last sync, from the Board's view (kept by the core across restarts).
    const bv = await invoke('board_view', { project: pid }).catch(() => null);
    if (bv?.last_sync && bv.last_sync_at_ms) last = { report: bv.last_sync, at: bv.last_sync_at_ms };
    if (bv?.last_sync_error) syncErr = [`Last sync ${bv.last_sync_at_ms ? `${clock(bv.last_sync_at_ms)} ` : ''}failed: ${bv.last_sync_error}`];
    const iv = await invoke('integrations_view').catch(() => null);
    linearInt = iv?.items?.find((i) => i.id === 'linear') ?? null;
    if (kind === 'linear') loadTeams();
  }

  async function loadTeams() {
    if (!connected || teams) return;
    try {
      teams = (await invoke('linear_teams')).teams;
      teamsError = null;
      if (!lin.team && teams.length) lin.team = teams[0].key;
      if (lin.team) loadStates(lin.team);
    } catch (e) {
      teamsError = errText(e);
      teams = [];
    }
  }

  async function loadStates(team) {
    states = null;
    try {
      states = await invoke('linear_states', { team });
      rows = mappingRows(states.states, team === saved?.backend?.team ? saved.backend.states : null);
    } catch (e) {
      teamsError = errText(e);
    }
  }

  function pickBackend(id) {
    kind = id;
    if (id === 'linear') loadTeams();
  }

  function pickTeam(key) {
    lin = { ...lin, team: key, project: '' };
    loadStates(key);
  }

  function setColumn(i, column) {
    rows = rows.map((x, j) => (j === i ? { ...x, column, isDefault: column === x.default_column } : x));
  }

  function backendToSave() {
    if (kind === saved?.backend?.type && kind !== 'linear') return saved.backend;
    if (kind !== 'linear') return { type: kind };
    const states = mappingToSave(rows);
    return { type: 'linear', team: lin.team, ...(lin.project ? { project: lin.project } : {}), ...(states ? { states } : {}) };
  }

  // Each section saves itself (design-notes §9 bug 1): the Task backend's Save
  // sends the saved settings as they are, the Notion one the saved backend.
  async function send(form, migrate = null) {
    busy = true;
    try {
      const r = await invoke('project_settings_set', { project: pid, form: migrate ? { ...form, migrate } : form });
      // D8: switching to Linear with tasks: nothing is saved until you choose.
      if (r?.confirm?.kind === 'linear_migrate') {
        ask = { open: r.confirm.open, done: r.confirm.done };
        return;
      }
      saved = r;
      ask = null;
      toast(`Saved ${name}'s settings`);
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    } finally {
      busy = false;
    }
  }

  /** Task backend › Save (and the D8 choices). */
  function saveBackend(migrate = null) {
    if (busy || !saved) return;
    if (kind === 'linear' && (!connected || !lin.team)) return toast('Pick a Linear team first');
    return send({ settings: saved.settings, backend: backendToSave(), track_orchestrator_dir: saved.track_orchestrator_dir }, migrate);
  }

  /** Notion › Save: only the parent page. */
  function saveNotion() {
    if (busy || !saved) return;
    const settings = { ...saved.settings, notion_parent_page: notionPage.trim() || undefined };
    return send({ settings, backend: saved.backend, track_orchestrator_dir: saved.track_orchestrator_dir });
  }

  /** Notion › Revert: the field goes back to the saved page. */
  function revertNotion() {
    notionPage = saved?.settings?.notion_parent_page ?? '';
  }

  async function sync() {
    if (syncing || backendDirty) return;
    syncing = true;
    try {
      const r = await invoke('board_sync', { project: pid });
      last = { report: r, at: Date.now() };
      syncErr = r.errors?.length ? r.errors : null;
      unmapped = r.unmapped_states ?? [];
      toast(`Synced ${name} with ${kind === 'local' ? 'the local store' : remoteName(kind)} · ${r.pushed ?? 0} pushed`);
    } catch (e) {
      syncErr = [errText(e)];
    } finally {
      syncing = false;
    }
  }

  function cancel() {
    ask = null;
    load();
  }

  function onKey(e) {
    if (section.id === 'allocation' && !ask) return alloc?.onKey(e);
    if (section.id === 'files') return files?.onKey(e);
    if (section.id === 'intake') return intake?.onKey(e);
    if (ask) {
      if (e.key === 'Enter') saveBackend('push_open');
      else if (e.key === 'l') saveBackend('leave');
      else if (e.key === 'Escape') ask = null;
      else return;
      return e.preventDefault();
    }
    if (e.metaKey && e.key === 'Enter') {
      e.preventDefault();
      if (section.id === 'backend') saveBackend();
      else if (section.id === 'notion') saveNotion();
      return;
    }
    // Section keys work after a click on a segment or a button; not while typing or picking.
    if (isTypingTarget(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
    if (section.id !== 'backend') return;
    const k = e.key;
    if ((k === 'Enter' || k === ' ') && e.target.closest('button')) return;
    if (k >= '1' && k <= '3') pickBackend(BACKENDS[Number(k) - 1].id);
    else if (k === 'j' || k === 'ArrowDown') rowSel = Math.min(rows.length - 1, rowSel + 1);
    else if (k === 'k' || k === 'ArrowUp') rowSel = Math.max(0, rowSel - 1);
    else if (k === 'Enter') root?.querySelector(`[data-row="${rowSel}"] [role="combobox"]`)?.focus();
    else if (k === 'r' && !backendDirty) sync();
    // Esc reverts unsaved changes; with none it goes back to the nav (the window's).
    else if (k === 'Escape' && backendDirty) cancel();
    else return;
    e.preventDefault();
  }

  onMount(load);
</script>

<FloatingWindow title="Project settings" subtitle={name} size={{ w: 960, h: 760 }} nav={SECTIONS} section={section.id} onsection={windowSection} onclose={onWindowClose} restoreFocus={false}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- data-pane-focus: where a section puts the focus back when its button goes away (it keeps the keys). -->
  <div class="ps" tabindex="-1" data-pane-focus bind:this={root} onkeydown={onKey}>
    {#if section.id === 'allocation'}
      <AllocationSettings bind:this={alloc} project={pid} {name} />
    {:else if section.id === 'files'}
      <FilesSettings bind:this={files} project={pid} {name} />
    {:else if section.id === 'intake'}
      <IntakeSettings bind:this={intake} project={pid} {name} />
    {:else if error}
      <p class="err">Could not read {name}'s settings: {error}</p>
    {:else if !saved}
      <p class="muted">Loading…</p>
    {:else if section.id === 'backend'}
      <h2>Task backend</h2>
      <p class="muted lead">Where {name}'s Board lives. The core is the only client of the backend; agents never call it.</p>
      <div class="line">
        <span class="lab">Backend</span>
        <div class="seg" role="radiogroup" aria-label="Backend">
          {#each BACKENDS as b, i (b.id)}<button role="radio" aria-checked={kind === b.id} class:on={kind === b.id} onclick={() => pickBackend(b.id)} use:tooltip={{ label: b.label, key: String(i + 1) }}>{b.label}</button>{/each}
        </div>
        {#if kind === 'linear' && linearInt}
          <span class="conn"><ConnBadge status={linearInt.status} />{linearInt.account ? ` · ${linearInt.account}` : ''}</span>
        {/if}
      </div>

      {#if kind === 'linear'}
        {#if !connected}
          <p class="box info">ⓘ Linear is not connected. Add a key in <button class="link" onclick={() => openSettings('integrations')}>Settings › Integrations</button></p>
        {:else if teams == null}
          <div class="skel" aria-busy="true"><span>Loading teams…</span><i></i><i class="short"></i></div>
        {:else}
          {#if teamsError}<p class="err">{teamsError}</p>{/if}
          <label class="line"><span class="lab">Team</span>
            <Select width="hug" searchable value={lin.team} options={teamOptions} noun="team" onchange={pickTeam} />
            <span class="muted">{teams.length} team{teams.length === 1 ? '' : 's'}: {teams.map((t) => `${t.key} · ${t.name}`).join(', ')}</span>
          </label>
          <label class="line"><span class="lab">Project</span>
            <Select width="hug" searchable bind:value={lin.project} options={projectOptions} noun="project" />
            <span class="muted">optional: limit to one Linear project</span>
          </label>

          <h3>Linear state → Board column <span class="muted">from {lin.team}'s workflow, by state type</span></h3>
          {#if !states}
            <div class="skel" aria-busy="true"><i></i><i class="short"></i></div>
          {:else}
            <div class="map">
              {#each rows as r, i (r.id)}
                <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                <div class="mrow" class:sel={i === rowSel} data-row={i} onclick={() => (rowSel = i)}>
                  <b>{r.name}</b><span class="muted">→</span>
                  <Select width="hug" aria-label="Column for {r.name}" value={r.column} options={BOARD_COLUMNS} onchange={(column) => setColumn(i, column)} />
                  {#if r.isDefault}<span class="muted">default</span>{/if}
                </div>
              {/each}
            </div>
            <p class="muted note">Open issues of {lin.team} (and of the project, when one is set) show on the Board. Moving or editing a card writes it back to Linear; moving it also adds the label orch. When both sides changed, Linear wins and you get a note in today's digest. Assignee node: label orch:node:&lt;id&gt;. Dependencies: Linear "blocks" relations.</p>
          {/if}
        {/if}
      {/if}

      {#if kind !== 'local'}
        {#if syncErr}
          {#each syncErr as m (m)}<p class="box errbox">✕ {m}</p>{/each}
        {/if}
        {#each unmapped as st (st)}
          <p class="box warnbox">▲ State "{st}" isn't mapped. <button class="btn ghost" onclick={() => root?.querySelector('[data-row] [role="combobox"]')?.focus()} use:tooltip={{ label: 'Map it', key: '⏎' }}>Map it</button></p>
        {/each}
        <div class="box syncbox">
          <span>{last ? syncLine(last.report, last.at, remoteName(kind)) : 'Not synced from here yet'}</span>
          <span class="muted">· every 5 min while a team runs</span>
          <span class="grow"></span>
          <button class="btn ghost" disabled={syncing || backendDirty} use:tooltip={backendDirty ? { label: 'Save first' } : { label: 'Sync now', key: 'r' }} onclick={sync}>{syncing ? 'Syncing…' : 'Sync now'}</button>
        </div>
      {/if}

      <div class="foot">
        <button class="btn primary" disabled={busy || !backendDirty} onclick={() => saveBackend()} use:tooltip={{ label: 'Save', key: '⌘⏎' }}>Save</button>
        <button class="btn ghost" disabled={!backendDirty} onclick={cancel} use:tooltip={{ label: 'Revert', key: 'esc' }}>Revert</button>
      </div>

      {#if ask}
        <div class="confirm" role="dialog" aria-modal="true" aria-label="Move tasks to Linear">
          <h3>Move {name}'s tasks to Linear?</h3>
          <p>{ask.open} open task{ask.open === 1 ? '' : 's'}.{ask.done ? ` ${ask.done} done tasks are not pushed.` : ''}</p>
          <button class="btn primary" onclick={() => saveBackend('push_open')}>Create {ask.open} open tasks as issues in {lin.team}</button>
          <button class="btn ghost" onclick={() => saveBackend('leave')}>Leave them out (they stay in the app, not synced)</button>
          <button class="btn ghost" onclick={() => (ask = null)} use:tooltip={{ label: 'Cancel', key: 'esc' }}>Cancel</button>
        </div>
      {/if}
    {:else if section.id === 'notion'}
      <h2>Notion</h2>
      <p class="muted lead">Publish to Notion (on a feature's page) creates one page per feature under this parent page.</p>
      <label class="line"><span class="lab">Parent page</span><input class="field grow" bind:value={notionPage} placeholder="Notion page URL or id" spellcheck="false" /></label>
      <p class="muted note">Share the page with your Notion integration. The token is in <button class="link" onclick={() => openSettings('integrations')}>Settings › Integrations</button>.</p>
      <div class="foot">
        <button class="btn primary" disabled={busy || !notionDirty} onclick={() => saveNotion()} use:tooltip={{ label: 'Save', key: '⌘⏎' }}>Save</button>
        <button class="btn ghost" disabled={!notionDirty} onclick={revertNotion}>Revert</button>
      </div>
    {:else if section.id === 'mcp'}
      {#key spec?.server}<McpSettings project={pid} selected={spec?.server ?? null} />{/key}
    {:else}
      <h2>{section.label}</h2>
      <p class="muted">Not designed yet. The current values are editable from the palette (⌘K).</p>
    {/if}
  </div>
</FloatingWindow>

<style>

  .warnbox {
    border-color: var(--color-pacing-amber);
  }
  .ps {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    font: var(--text-label);
    outline: none;
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  h3 {
    margin: var(--space-md) 0 0;
    font: var(--text-title);
  }
  h3 .muted {
    font: var(--text-caption);
  }
  .lead,
  .note {
    margin: 0;
    font: var(--text-caption);
  }
  .line {
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }
  .lab {
    width: 120px;
    flex: none;
    color: var(--color-text-muted);
  }
  .seg {
    display: flex;
    gap: var(--space-xs);
  }
  .seg button {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    padding: var(--space-xxs) var(--space-sm);
    background: none;
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
    font: var(--text-label);
    cursor: pointer;
  }
  .seg button.on {
    border-color: var(--color-focus);
    background: var(--color-surface-selected);
    color: var(--color-text-primary);
  }
  .conn {
    padding: 0 var(--space-sm);
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
  }
  .map {
    display: flex;
    flex-direction: column;
  }
  .mrow {
    display: grid;
    grid-template-columns: 170px 20px auto 1fr;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-sm);
    border: var(--border-width-default) solid transparent;
    border-radius: var(--radius-sm);
  }
  .mrow.sel {
    border-color: var(--color-focus);
    background: var(--color-surface-selected);
  }
  .box {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    margin: 0;
    padding: var(--space-sm) var(--space-md);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
  }
  .info {
    border-color: var(--color-accent-default);
  }
  .errbox {
    border-color: var(--color-status-error);
  }
  .syncbox {
    background: var(--color-surface-raised);
  }
  .skel {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    max-width: 240px;
  }
  .skel i {
    height: 10px;
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
  }
  .skel i.short {
    width: 70%;
  }
  .foot {
    display: flex;
    gap: var(--space-sm);
    margin-top: var(--space-sm);
  }
  .confirm {
    position: absolute;
    top: var(--space-xl);
    right: var(--space-xl);
    width: 460px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-sm);
    padding: var(--space-lg);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
  }
  .confirm h3,
  .confirm p {
    margin: 0;
  }
  .link {
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
</style>
