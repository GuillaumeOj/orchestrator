<script>
  // Project settings (Penpot 04 Project settings · Linear, J13 · 3–5): a
  // project's sections. P3 builds Task backend (Local / GitHub Issues /
  // Linear with its team, project and state mapping, Sync now, the D8
  // confirm), Notion (the parent page for Publish, FR-505) and the MCP
  // overrides. Keys: 1–3 backend, j/k mapping rows, ⏎ change a row,
  // r sync, ⌘⏎ save, esc cancel.
  import { onMount } from 'svelte';
  import Kbd from '../components/Kbd.svelte';
  import McpSettings from '../components/McpSettings.svelte';
  import { app, openSettings, projects, saveLayout, toast } from '../lib/app.svelte.js';
  import { connStatus } from '../lib/ecosystem.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { BACKENDS, backendForm, COLUMN_LABEL, mappingRows, mappingToSave, migrateAsk, syncLine } from '../lib/linear.js';

  let { paneId } = $props();
  const spec = $derived(app.panes[paneId]);
  const pid = $derived(spec?.projectId);
  const name = $derived(projects().find((p) => p.id === pid)?.name ?? pid);

  const SECTIONS = [
    { id: 'general', label: 'General' },
    { id: 'backend', label: 'Task backend' },
    { id: 'allocation', label: 'Allocation' },
    { id: 'notion', label: 'Notion' },
    { id: 'mcp', label: 'MCP overrides' },
  ];
  const section = $derived(SECTIONS.find((s) => s.id === spec?.section) ?? SECTIONS[1]);

  let saved = $state(null); // project_settings
  let error = $state(null);
  let kind = $state('local');
  let linearInt = $state(null); // the linear integration (status, account)
  let teams = $state(null); // null = loading
  let teamsError = $state(null);
  let lin = $state({ team: '', project: '', states: null });
  let states = $state(null); // linear_states
  let rows = $state([]);
  let rowSel = $state(0);
  let notionPage = $state('');
  let ask = $state(null); // D8 { open, done }
  let busy = $state(false);
  let syncing = $state(false);
  let last = $state(null); // { report, at }
  let syncErr = $state(null);
  let root = $state(null);

  const connected = $derived(linearInt?.status === 'connected');
  const dirty = $derived(
    !!saved &&
      (kind !== saved.backend?.type ||
        (kind === 'linear' && (lin.team !== saved.backend.team || (lin.project || '') !== (saved.backend.project ?? '') || JSON.stringify(mappingToSave(rows) ?? {}) !== JSON.stringify(saved.backend.states ?? {}))) ||
        notionPage !== (saved.settings?.notion_parent_page ?? '')),
  );

  async function load() {
    try {
      saved = await invoke('project_settings', { project: pid });
      error = null;
      kind = saved.backend?.type ?? 'local';
      notionPage = saved.settings?.notion_parent_page ?? '';
      if (kind === 'linear') lin = { team: saved.backend.team ?? '', project: saved.backend.project ?? '', states: saved.backend.states ?? null };
    } catch (e) {
      error = errText(e);
    }
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
    const r = rows[i];
    const def = states?.states?.find((s) => s.id === r.id)?.default_column;
    rows = rows.map((x, j) => (j === i ? { ...x, column, isDefault: column === def } : x));
  }

  function backendToSave() {
    if (kind === saved?.backend?.type && kind !== 'linear') return saved.backend;
    return backendForm(kind, { team: lin.team, project: lin.project || null, states: mappingToSave(rows) });
  }

  async function save(migrate = null) {
    if (busy || !saved) return;
    if (kind === 'linear' && (!connected || !lin.team)) return toast('Pick a Linear team first');
    busy = true;
    try {
      const settings = { ...saved.settings, notion_parent_page: notionPage.trim() || undefined };
      const args = { project: pid, form: { settings, backend: backendToSave(), track_orchestrator_dir: saved.track_orchestrator_dir } };
      saved = await invoke('project_settings_set', migrate ? { ...args, migrate } : args);
      ask = null;
      toast(`Saved ${name}'s settings`);
    } catch (e) {
      const m = migrateAsk(errText(e));
      if (m && !migrate) ask = m;
      else toast(errText(e), { kind: 'error' });
    } finally {
      busy = false;
    }
  }

  async function sync() {
    if (syncing) return;
    syncing = true;
    try {
      const r = await invoke('board_sync', { project: pid });
      last = { report: r, at: Date.now() };
      syncErr = r.errors?.length ? r.errors : null;
      toast(`Synced ${name} with ${kind === 'linear' ? 'Linear' : kind === 'github' ? 'GitHub' : 'the local store'} · ${r.pushed ?? 0} pushed`);
    } catch (e) {
      syncErr = [errText(e)];
    } finally {
      syncing = false;
    }
  }

  /** A Linear state created after the mapping was saved (decisions: keep that wording). */
  const unmapped = (m) => /isn't mapped/.test(m);

  function cancel() {
    ask = null;
    load();
  }

  function pick(id) {
    app.panes[paneId] = { ...app.panes[paneId], section: id };
    saveLayout();
  }

  function onKey(e) {
    if (ask) {
      if (e.key === 'Enter') save('push_open');
      else if (e.key === 'l') save('leave');
      else if (e.key === 'Escape') ask = null;
      else return;
      return e.preventDefault();
    }
    if (e.metaKey && e.key === 'Enter') return (e.preventDefault(), save());
    if (e.target.closest('input, textarea, select, button') || e.metaKey || e.ctrlKey || e.altKey) return;
    if (section.id !== 'backend') return;
    const k = e.key;
    if (k >= '1' && k <= '3') pickBackend(BACKENDS[Number(k) - 1].id);
    else if (k === 'j' || k === 'ArrowDown') rowSel = Math.min(rows.length - 1, rowSel + 1);
    else if (k === 'k' || k === 'ArrowUp') rowSel = Math.max(0, rowSel - 1);
    else if (k === 'Enter') root?.querySelector(`[data-row="${rowSel}"] select`)?.focus();
    else if (k === 'r') sync();
    else if (k === 'Escape') cancel();
    else return;
    e.preventDefault();
  }

  onMount(load);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="ps" tabindex="0" data-pane-focus bind:this={root} onkeydown={onKey} role="region" aria-label="Project settings">
  <nav class="nav" aria-label="Project settings sections">
    <span class="muted">{name}</span>
    {#each SECTIONS as s (s.id)}<button class="sec" class:on={s.id === section.id} onclick={() => pick(s.id)}>{s.label}</button>{/each}
    <span class="grow"></span>
    <span class="muted hint"><Kbd keys="⌘+K" /> "{name} settings"</span>
  </nav>

  <div class="body">
    {#if error}
      <p class="err">Could not read {name}'s settings: {error}</p>
    {:else if !saved}
      <p class="muted">Loading…</p>
    {:else if section.id === 'backend'}
      <h2>Task backend</h2>
      <p class="muted lead">Where {name}'s Board lives. The core is the only client of the backend; agents never call it.</p>
      <div class="line">
        <span class="lab">Backend</span>
        <div class="seg" role="radiogroup" aria-label="Backend">
          {#each BACKENDS as b, i (b.id)}<button role="radio" aria-checked={kind === b.id} class:on={kind === b.id} onclick={() => pickBackend(b.id)}><Kbd keys={String(i + 1)} /> {b.label}</button>{/each}
        </div>
        {#if kind === 'linear' && linearInt}
          {@const st = connStatus(linearInt.status)}
          <span class="conn"><span style:color={st.color} aria-hidden="true">{st.icon}</span> {st.label}{linearInt.account ? ` · ${linearInt.account}` : ''}</span>
        {/if}
      </div>

      {#if kind === 'linear'}
        {#if !connected}
          <p class="box info">ⓘ Linear is not connected. Add a key in <button class="link" onclick={() => openSettings('integrations')}>Settings › Integrations</button> <Kbd keys="⌘+," /></p>
        {:else if teams == null}
          <div class="skel" aria-busy="true"><span>Loading teams…</span><i></i><i class="short"></i></div>
        {:else}
          {#if teamsError}<p class="err">{teamsError}</p>{/if}
          <label class="line"><span class="lab">Team</span>
            <select class="field auto" value={lin.team} onchange={(e) => pickTeam(e.currentTarget.value)}>
              {#each teams as t (t.id)}<option value={t.key}>{t.key} · {t.name}</option>{/each}
            </select>
            <span class="muted">{teams.length} team{teams.length === 1 ? '' : 's'}: {teams.map((t) => `${t.key} · ${t.name}`).join(', ')}</span>
          </label>
          <label class="line"><span class="lab">Project</span>
            <select class="field auto" bind:value={lin.project}>
              <option value="">Any project</option>
              {#each states?.projects ?? [] as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
            </select>
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
                  <select class="field auto" aria-label="Column for {r.name}" value={r.column} onchange={(e) => setColumn(i, e.currentTarget.value)}>
                    {#each Object.entries(COLUMN_LABEL) as [id, label] (id)}<option value={id}>{label}</option>{/each}
                  </select>
                  {#if r.isDefault}<span class="muted">default</span>{/if}
                </div>
              {/each}
            </div>
            <p class="muted note">Issues labelled orch in {lin.team} sync both ways. When both sides changed, Linear wins and you get an Inbox note. Assignee node: label orch:node:&lt;id&gt;. Dependencies: Linear "blocks" relations.</p>
          {/if}
        {/if}
      {/if}

      {#if kind !== 'local'}
        {#if syncErr}
          {#each syncErr as m (m)}
            <p class="box errbox">{unmapped(m) ? '▲' : '✕'} {m}{#if unmapped(m)} <button class="btn ghost" onclick={() => root?.querySelector('[data-row] select')?.focus()}>Map it <Kbd keys="⏎" /></button>{/if}</p>
          {/each}
        {/if}
        <div class="box syncbox">
          <span>{last ? syncLine(last.report, last.at, kind === 'linear' ? 'Linear' : 'GitHub') : 'Not synced from here yet'}</span>
          <span class="muted">· every 2 min while the app runs</span>
          <span class="grow"></span>
          <button class="btn ghost" disabled={syncing || dirty} title={dirty ? 'Save first' : ''} onclick={sync}>{syncing ? 'Syncing…' : 'Sync now'} <Kbd keys="r" /></button>
        </div>
      {/if}

      <div class="foot">
        <button class="btn primary" disabled={busy || !dirty} onclick={() => save()}>Save <Kbd keys="⌘+⏎" /></button>
        <button class="btn ghost" onclick={cancel}>Cancel <Kbd keys="esc" /></button>
      </div>
      <p class="muted hint"><Kbd keys="j" /><Kbd keys="k" /> rows · <Kbd keys="⏎" /> change · <Kbd keys="1–3" /> backend · <Kbd keys="r" /> sync</p>

      {#if ask}
        <div class="confirm" role="dialog" aria-modal="true" aria-label="Move tasks to Linear">
          <h3>Move {name}'s tasks to Linear?</h3>
          <p>{ask.open} open task{ask.open === 1 ? '' : 's'}.{ask.done ? ` ${ask.done} done tasks are not pushed.` : ''}</p>
          <button class="btn primary" onclick={() => save('push_open')}>Create {ask.open} open tasks as issues in {lin.team} <Kbd keys="⏎" /></button>
          <button class="btn ghost" onclick={() => save('leave')}>Leave them out (they stay in the app, not synced) <Kbd keys="l" /></button>
          <button class="btn ghost" onclick={() => (ask = null)}>Cancel <Kbd keys="esc" /></button>
        </div>
      {/if}
    {:else if section.id === 'notion'}
      <h2>Notion</h2>
      <p class="muted lead">Publish to Notion (on a feature's page) creates one page per feature under this parent page.</p>
      <label class="line"><span class="lab">Parent page</span><input class="field grow" bind:value={notionPage} placeholder="Notion page URL or id" spellcheck="false" /></label>
      <p class="muted note">Share the page with your Notion integration. The token is in <button class="link" onclick={() => openSettings('integrations')}>Settings › Integrations</button>.</p>
      <div class="foot"><button class="btn primary" disabled={busy || !dirty} onclick={() => save()}>Save <Kbd keys="⌘+⏎" /></button></div>
    {:else if section.id === 'mcp'}
      <McpSettings project={pid} selected={null} />
    {:else}
      <h2>{section.label}</h2>
      <p class="muted">Not designed yet. The current values are editable from <Kbd keys="⌘+K" />.</p>
    {/if}
  </div>
</div>

<style>
  .ps {
    flex: 1;
    min-height: 0;
    display: flex;
    outline: none;
    font: var(--text-label);
  }
  .nav {
    width: 200px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    padding: var(--space-md) var(--space-sm);
    background: var(--color-surface-chrome);
    border-right: var(--border-width-default) solid var(--color-border-subtle);
  }
  .sec {
    height: var(--size-row);
    padding: 0 var(--space-sm);
    background: none;
    border: 0;
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
    font: var(--text-body);
    text-align: left;
    cursor: pointer;
  }
  .sec.on {
    background: var(--color-surface-selected);
    color: var(--color-text-primary);
  }
  .body {
    position: relative;
    flex: 1;
    min-width: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-lg) var(--space-xl);
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
  .note,
  .hint {
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
  .field.auto {
    width: auto;
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
