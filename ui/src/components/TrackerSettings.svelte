<script>
  // Project settings › Tracker (improve-intake design notes §10, FigCraft 04.7, api-contract §1):
  // the one place that says where this project's work lives. Kind (Local · GitHub · Linear), the
  // fields of the kind, the Ready filter, Write back, Refresh, Preview (a dry run), the migration
  // notices and the errors per code. ⌘⏎ saves. Replaces the Intake and Task backend sections.
  import { onMount, tick, untrack } from 'svelte';
  import { app, openSettings, projects, toast } from '../lib/app.svelte.js';
  import { GH_SCOPE_COMMAND, integrationsRow } from '../lib/intake.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { BOARD_COLUMNS } from '../lib/options.js';
  import { tooltip } from '../lib/tooltip.js';
  import {
    CYCLES,
    DEFAULT_GLOB,
    fieldCopy,
    hasParentPart,
    KINDS,
    nameOf,
    noticeInfo,
    optionList,
    parseErr,
    REFRESH,
    readySummary,
    repoOfRemote,
    problems as rules,
    STATE_TYPES,
    sampleWhere,
    stateRows,
    statesToSave,
    switchKind,
    toggleType,
    trackerError,
    trackerName,
    trackerTarget,
    writeBackLines,
  } from '../lib/tracker.js';
  import Select from './Select.svelte';

  let { project, name = project, block = null } = $props();

  const known = $derived.by(() => {
    const p = projects().find((x) => x.id === project);
    const r = (p?.remotes ?? []).find((x) => x.name === 'origin') ?? p?.remotes?.[0];
    return repoOfRemote(r?.url);
  });

  // A write-back card's Open settings: the Write back block comes into view once the form is there.
  let blockSeen = null;
  $effect(() => {
    if (block !== 'write_back' || !form || blockSeen === app.window) return;
    blockSeen = app.window;
    tick().then(() => {
      const el = /** @type {HTMLElement | null} */ (document.querySelector('[data-wb-switch]'));
      el?.scrollIntoView?.({ block: 'center' });
      el?.querySelector('input')?.focus();
    });
  });
  let form = $state(/** @type {any} */ (null)); // the TrackerSettings being edited
  let baseline = $state('null'); // JSON of the saved settings
  let notice = $state(/** @type {any} */ (null));
  let pick = $state('alternative'); // the mismatch radio
  let loadError = $state(null);
  let busy = $state(false);
  let savedAt = $state(0);
  let savedTimer = null;
  let touched = $state(false);
  let fieldErrors = $state(/** @type {Record<string, string>} */ ({}));
  let saveError = $state(null);
  let altPending = $state(false); // "Use <alternative> instead" is in the form; the notice is dismissed once it is saved
  let ask = $state(/** @type {any} */ (null)); // D8: { open, done, settings } while a switch to Linear waits for a choice

  let opts = $state(/** @type {any} */ (null)); // tracker_options for the form's tracker
  let optsError = $state(/** @type {any} */ (null));
  let autoReady = $state(false); // the Ready labels are still the default: follow the repo's `ready` label
  let rows = $state(/** @type {any[]} */ ([])); // Linear states → team task column
  let optsTick = $state(0); // Retry of the options read
  let advancedOpen = $state(false);

  let previewing = $state(false);
  let preview = $state(/** @type {any} */ (null));
  let previewJson = $state(null);
  let root = $state(/** @type {HTMLElement | null} */ (null));

  const json = $derived(JSON.stringify(form));
  const dirty = $derived(!!form && json !== baseline);
  const kind = $derived(form?.tracker.kind);
  const problems = $derived(form ? rules(form) : {});
  const valid = $derived(!Object.keys(problems).length);
  const errs = $derived({ ...(touched ? problems : {}), ...fieldErrors });
  const stale = $derived(!!preview && previewJson !== json);
  const wbName = $derived(kind === 'github' ? 'GitHub' : 'Linear');

  const boards = $derived(opts?.boards ?? []);
  const boardKey = (b) => (b ? `${b.owner}/${b.number}` : '');
  const pickedBoard = $derived(form?.tracker.project_board ? (boards.find((b) => boardKey(b) === boardKey(form.tracker.project_board)) ?? null) : null);
  const boardOptions = $derived([
    { value: '', label: 'None' },
    ...boards.map((b) => ({ value: boardKey(b), label: `${b.owner} · #${b.number} ${b.title}` })),
    ...(form?.tracker.project_board && !pickedBoard ? [{ value: boardKey(form.tracker.project_board), label: boardKey(form.tracker.project_board) }] : []),
  ]);
  const statusNames = $derived(pickedBoard?.statuses ?? []);
  const defaultStatus = $derived(statusNames.find((s) => /in progress/i.test(s)) ?? statusNames[0] ?? null);
  const started = $derived((opts?.states ?? []).filter((s) => s.type === 'started'));
  const linearTarget = $derived(started.find((s) => s.id === form?.write_back.linear_state) ?? started[0] ?? null);
  const targetName = $derived(kind === 'github' ? (form?.write_back.board_status ?? defaultStatus) : (linearTarget?.name ?? null));
  const lines = $derived(form && kind !== 'local' ? writeBackLines(form, known, targetName) : []);
  /** A tooltip param: the full text, only when the element cuts it. */
  const ifCut = (text) => (/** @type {HTMLElement} */ node) => (node.scrollWidth > node.clientWidth ? text : null);
  const projectName = (id) => (opts?.projects ?? []).find((p) => p.id === id)?.name ?? id;
  const summary = $derived(form ? readySummary(form.ready, projectName) : '');
  // A team is stored by key or id (`tracker_options` projects carry the team's key).
  const teamKeys = $derived((form?.tracker.teams ?? []).map((k) => (opts?.teams ?? []).find((t) => t.id === k)?.key ?? k));
  const teamIds = $derived(teamKeys.map((k) => (opts?.teams ?? []).find((t) => t.key === k)?.id).filter(Boolean));
  const teamOptions = $derived((opts?.teams ?? []).filter((t) => !teamKeys.includes(t.key)).map((t) => ({ value: t.key, label: `${t.key} · ${t.name}` })));
  const linProjects = $derived((opts?.projects ?? []).filter((p) => !teamKeys.length || teamKeys.includes(p.team) || teamIds.includes(p.team)));
  const projectOptions = $derived(linProjects.filter((p) => !form?.tracker.projects?.includes(p.id)).map((p) => ({ value: p.id, label: p.name })));
  const unmapped = $derived(rows.filter((x) => !x.column));
  const notSetUp = $derived(
    optsError && ['gh_missing', 'gh_auth', 'linear_key', 'linear_auth'].includes(optsError.code) ? (optsError.code.startsWith('linear') ? 'Linear' : 'GitHub') : null,
  );
  const trackerJson = $derived(form ? JSON.stringify(form.tracker) : '');
  const trkName = $derived(form ? (kind === 'linear' ? form.tracker.teams.join(', ') || null : trackerName(form.tracker, known)) : null);
  const previewErr = $derived(preview?.error ? trackerError(preview.error, kind, trkName) : null);
  const previewWarn = $derived(preview?.warning === 'gh_sub_issues' ? trackerError({ code: 'gh_sub_issues' }, kind, trkName).text : null);
  const info = $derived(notice && !altPending && form ? noticeInfo(notice, form, { project: name, known }) : null);

  // ------------------------------------------------------------ loading

  function adopt(r) {
    form = $state.snapshot(r.settings);
    baseline = JSON.stringify(form);
    notice = r.migration_notice && !r.migration_notice.dismissed ? r.migration_notice : null;
    pick = 'alternative';
    altPending = false;
    rows = [];
  }

  async function load() {
    try {
      adopt(await invoke('tracker_settings', { project }));
      loadError = null;
    } catch (e) {
      loadError = errText(e);
    }
  }
  onMount(() => {
    load();
    return () => clearTimeout(savedTimer);
  });

  // A request counter: an answer older than the latest request is dropped.
  let seq = 0;
  async function loadOptions(tracker) {
    const n = ++seq;
    let out;
    try {
      out = (await invoke('tracker_options', { project, tracker })) ?? {};
    } catch (e) {
      out = { error: { code: parseErr(e, errText).code ?? 'failed', message: errText(e) } };
    }
    return n === seq ? out : null;
  }

  // Suggestions follow what is filled so far (debounced): the repo, the teams, the glob. Keyed on the
  // tracker alone (a save or revert that leaves it as it was does not read again), and an answer for a
  // tracker that is no longer the form's is dropped.
  $effect(() => {
    optsTick;
    if (!trackerJson) return;
    const key = trackerJson; // the tracker this read is for (`trackerJson` itself is live: it follows the form)
    const t = JSON.parse(key);
    if (t.kind === 'github' && t.repo?.trim() && !/^[^/\s]+\/[^/\s]+$/.test(t.repo.trim())) return;
    if (t.kind === 'local' && (!t.backlog_glob || hasParentPart(t.backlog_glob))) {
      opts = null;
      return;
    }
    const timer = setTimeout(
      async () => {
        const o = await loadOptions(t);
        if (!o || JSON.stringify(untrack(() => form?.tracker)) !== key) return;
        opts = o;
        optsError = o.error ?? null;
        untrack(() => {
          const ready = (o.labels ?? []).map(nameOf).find((l) => l.toLowerCase() === 'ready');
          if (autoReady && form.ready.kind === 'github' && ready && !form.ready.labels.length) form.ready.labels = [ready];
          if (form.tracker.kind === 'linear' && o.states) rows = stateRows(o.states, form.task_states);
        });
      },
      untrack(() => (opts ? 400 : 0)),
    );
    return () => clearTimeout(timer);
  });

  // ------------------------------------------------------------ edits

  /** Any edit drops the last outcome of a save and the core's field errors. */
  function edit() {
    fieldErrors = {};
    saveError = null;
    ask = null;
  }

  function pickKind(k) {
    if (k === kind) return;
    form = switchKind($state.snapshot(form), k);
    opts = null;
    optsError = null;
    rows = [];
    autoReady = k === 'github';
    preview = null;
    edit();
  }

  const addTo = (list, v) => (v && !list.includes(v) ? [...list, v] : list);

  function addLabel(v) {
    const n = v.trim().replace(/,$/, '').trim();
    labelInput = '';
    if (!n) return; // a blur with nothing typed is not an edit
    form.ready.labels = addTo(form.ready.labels, n);
    autoReady = false;
    edit();
  }
  let labelInput = $state('');
  function labelKey(e) {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) addLabel(labelInput); // ⌘⏎ still saves (the window's key)
    else if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault();
      e.stopPropagation();
      addLabel(labelInput);
    } else if (e.key === 'Backspace' && !labelInput && form.ready.labels.length) form.ready.labels = form.ready.labels.slice(0, -1);
  }

  function pickBoard(v) {
    const b = boards.find((x) => boardKey(x) === v);
    form.tracker.project_board = b ? { owner: b.owner, number: b.number } : null;
    form.write_back.board_status = null;
    form.ready.board_status = null;
    edit();
  }

  function setStateColumn(i, column) {
    rows = rows.map((r, j) => (j === i ? { ...r, column } : r));
    form.task_states = statesToSave(rows);
    edit();
  }

  const any = (v) => v || null;
  /** Fields with an error line under them; any other field's error shows with the save line. */
  const SLOTS = ['repo', 'teams', 'projects', 'backlog_glob', 'ready'];
  const fail = (x, e) => {
    if (x.field && SLOTS.includes(x.field)) fieldErrors = { [x.field]: fieldCopy(x.field, x.message) };
    else saveError = x.field ? fieldCopy(x.field, x.message) : (x.message ?? errText(e));
  };

  // ------------------------------------------------------------ actions

  async function runPreview() {
    touched = true;
    if (previewing || !valid) return;
    previewing = true;
    fieldErrors = {};
    const settings = $state.snapshot(form);
    try {
      preview = await invoke('tracker_preview', { project, settings });
    } catch (e) {
      const x = parseErr(e, errText);
      if (x.field && SLOTS.includes(x.field)) fieldErrors = { [x.field]: fieldCopy(x.field, x.message) };
      else preview = { error: { code: x.code ?? 'failed', message: x.message } };
    } finally {
      previewJson = JSON.stringify(settings);
      previewing = false;
    }
    if (preview?.error) settingsError(trackerError(preview.error, kind, trkName));
  }

  /** The field a `fix: "settings"` error (not_found, bad_glob, branch_missing) is about, and its control. */
  const ERR_FIELD = { github: ['repo', '#trk-repo'], local: ['backlog_glob', '#trk-glob'], linear: ['teams', '[aria-label="Add a team"]'] };

  /** A `fix: "settings"` error is a field error (notes §10.7): message under the field, red border, focus; no Preview line. */
  async function settingsError(e) {
    if (e.fix !== 'settings' || !ERR_FIELD[kind]) return;
    const [field, sel] = ERR_FIELD[kind];
    preview = null;
    fieldErrors = { [field]: e.text };
    await tick();
    /** @type {HTMLElement | null | undefined} */ (root?.querySelector(sel))?.focus();
  }

  /**
   * Stores `settings`. A switch to Linear of a project with tasks answers `{ confirm }` and stores
   * nothing (D8): it opens the choice and resolves to null; `migrate` is that choice.
   */
  async function commit(settings, migrate = null) {
    const r = await invoke('tracker_settings_set', { project, settings, ...(migrate ? { migrate } : {}) });
    if (r?.confirm) {
      ask = { ...r.confirm, settings };
      return null;
    }
    ask = null;
    return r;
  }

  /** The saved answer of the core becomes the form (or, with none, the form is what was saved). */
  function landed(r, settings) {
    const was = notice;
    if (altPending) {
      altPending = false;
      invoke('tracker_notice_dismiss', { project }).catch(() => {});
      notice = null;
    }
    if (r?.settings) adopt(r);
    else baseline = JSON.stringify(settings);
    if (was && !r?.migration_notice) notice = null;
    savedAt = Date.now();
    clearTimeout(savedTimer);
    savedTimer = setTimeout(() => (savedAt = 0), 5000);
  }

  async function save(migrate = null) {
    touched = true;
    if (busy || !valid || !dirty) return;
    busy = true;
    saveError = null;
    fieldErrors = {};
    try {
      const settings = $state.snapshot(form);
      const r = await commit(settings, migrate);
      if (r) landed(r, settings);
    } catch (e) {
      fail(parseErr(e, errText), e);
    } finally {
      busy = false;
    }
  }

  /** D8: the choice for the tasks of a project moving to Linear: `push_open` or `leave`. */
  async function answer(migrate) {
    if (busy || !ask) return;
    busy = true;
    saveError = null;
    const { settings } = ask;
    try {
      const r = await commit(settings, migrate);
      if (r) landed(r, settings);
    } catch (e) {
      const x = parseErr(e, errText);
      saveError = x.message ?? errText(e);
    } finally {
      busy = false;
    }
  }

  export function revert() {
    if (!baseline || baseline === 'null') return;
    ask = null;
    altPending = false;
    form = JSON.parse(baseline);
    rows = opts?.states ? stateRows(opts.states, form.task_states) : [];
    fieldErrors = {};
    saveError = null;
    touched = false;
  }

  async function dismiss() {
    try {
      await invoke('tracker_notice_dismiss', { project });
      notice = null;
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }

  /** "Use acme/web instead": the alternative becomes the tracker (still to save). */
  function useAlternative() {
    form = { ...$state.snapshot(form), tracker: $state.snapshot(notice.alternative) };
    opts = null;
    altPending = true;
  }

  /** Mismatch › Use this tracker: saves the saved settings with the chosen tracker. */
  async function useTracker() {
    const choice = info.choices.find((c) => c.id === pick);
    if (!choice || busy) return;
    busy = true;
    const saved = JSON.parse(baseline);
    const t = $state.snapshot(choice.tracker);
    const settings = t.kind === saved.tracker.kind ? { ...saved, tracker: t } : { ...switchKind(saved, t.kind), tracker: t };
    try {
      const r = await commit(settings);
      if (r) adopt(r);
    } catch (e) {
      saveError = parseErr(e, errText).message ?? errText(e);
    } finally {
      busy = false;
    }
  }

  async function copyCommand() {
    try {
      await navigator.clipboard.writeText(GH_SCOPE_COMMAND);
    } catch {}
    toast('Copied. Run it in a terminal, then Preview again.');
  }

  /** The Fix of a Preview error (notes §10.7). */
  function fix(e) {
    if (e.fix === 'limit_projects') {
      if (kind === 'linear' && !form.tracker.projects) {
        form.tracker.projects = [];
        edit();
      }
      root?.querySelector('[data-projects]')?.scrollIntoView?.({ block: 'center' });
      /** @type {HTMLElement | null | undefined} */ (root?.querySelector('[data-projects] input[type="radio"]'))?.focus();
    } else if (e.fix === 'turn_off_write_back') form.write_back.enabled = false;
    else if (e.fix === 'integrations') openSettings('integrations', integrationsRow(e.code));
    else if (e.fix === 'gh_scope') copyCommand();
    else if (e.fix === 'retry') runPreview();
  }

  /** Unsaved changes (the window's Discard guard). */
  export function hasChanges() {
    return dirty;
  }

  /** Called by the window for keys typed in the section. */
  export function onKey(e) {
    if (e.metaKey && e.key === 'Enter') {
      e.preventDefault();
      save();
    }
  }
</script>

{#snippet err(key)}
  {#if errs[key]}<p class="ferr" role="alert"><span aria-hidden="true">{'\u{F0026}'}</span> {errs[key]}</p>{/if}
{/snippet}

{#snippet integrations(label)}
  <p class="box info" data-not-set-up>{label} isn't set up. <button class="link" onclick={() => openSettings('integrations', integrationsRow(optsError?.code))}>Settings › Integrations ↗</button></p>
{/snippet}

<section class="tracker" aria-label="Tracker" bind:this={root}>
  <h2>Tracker{#if dirty}<span class="dot" aria-hidden="true">●</span><span class="unsaved">Unsaved changes</span>{/if}{#if savedAt}<span class="done" role="status">Saved · updated just now</span>{/if}</h2>
  <p class="muted lead">Where this project's work lives. Work, Ready to start and team tasks all read it.</p>

  {#if loadError}
    <p class="ferr">Could not read {name}'s tracker: {loadError}</p>
    <div><button class="btn" onclick={load}>Retry</button></div>
  {:else if !form}
    <p class="muted">Loading…</p>
  {:else}
    {#if info}
      <div class="notice" class:warn={info.tone === 'warning'} data-notice={notice.kind} role="status">
        <span class="glyph" aria-hidden="true">{info.tone === 'warning' ? '▲' : 'ⓘ'}</span>
        <div class="nbody">
          <p>{info.text}</p>
          {#if info.choices}
            <div role="radiogroup" aria-label="Tracker" class="choices">
              {#each info.choices as c (c.id)}<label class="check"><input type="radio" name="tracker-pick" checked={pick === c.id} onchange={() => (pick = c.id)} /> {c.label}</label>{/each}
            </div>
            <p class="muted">{info.until}</p>
          {/if}
          <div class="acts">
            {#if info.actions.includes('got_it')}<button class="btn primary" onclick={dismiss}>Got it</button>{/if}
            {#if info.actions.includes('use_alternative')}<button class="btn" onclick={useAlternative}>{info.altLabel}</button>{/if}
            {#if info.actions.includes('use_this')}<button class="btn primary" disabled={busy} onclick={useTracker}>Use this tracker</button>{/if}
          </div>
        </div>
      </div>
    {/if}

    <div class="row">
      <span class="lab" id="trk-kind">Kind</span>
      <div class="seg" role="radiogroup" aria-labelledby="trk-kind">
        {#each KINDS as k (k.id)}<button role="radio" aria-checked={kind === k.id} class:on={kind === k.id} onclick={() => pickKind(k.id)}>{k.label}</button>{/each}
      </div>
    </div>

    {#if kind === 'local'}
      <div class="row top">
        <label class="lab" for="trk-glob">Backlog folder</label>
        <div class="col">
          <input id="trk-glob" class="field mono w360" class:bad={errs.backlog_glob} bind:value={form.tracker.backlog_glob} oninput={edit} placeholder={DEFAULT_GLOB} spellcheck="false" />
          {@render err('backlog_glob')}
          {#if !form.tracker.backlog_glob?.trim()}
            <p class="muted hint">No backlog files: only tasks you create.</p>
          {:else if !errs.backlog_glob && opts?.matches != null}
            <p class="muted hint" data-matches>{opts.matches === 0 ? 'No file matches' : opts.matches === 1 ? '1 file matches' : `${opts.matches} files match`}</p>
          {/if}
          <p class="muted hint">Local has no hierarchy: Work shows a flat list.</p>
        </div>
      </div>
    {:else if kind === 'github'}
      <div class="row top">
        <label class="lab" for="trk-repo">Repository</label>
        <div class="col">
          <input id="trk-repo" class="field mono w320" class:bad={errs.repo} value={form.tracker.repo ?? ''} oninput={(e) => ((form.tracker.repo = e.currentTarget.value.trim() ? e.currentTarget.value : null), edit())} placeholder={known || 'owner/name'} spellcheck="false" />
          {@render err('repo')}
          <p class="muted hint">Defaults to this project's repository.</p>
        </div>
      </div>
      {#if notSetUp}{@render integrations(notSetUp)}{/if}
      <div class="row top">
        <label class="lab" for="trk-board">Projects v2 board</label>
        <div class="col">
          <div class="w320"><Select id="trk-board" width="fill" value={form.tracker.project_board ? boardKey(form.tracker.project_board) : ''} options={boardOptions} noun="board" onchange={pickBoard} /></div>
          <p class="muted hint">Shows its Status in Work and lets write-back move items on it.</p>
          {#if optsError?.code === 'gh_scope'}
            <div class="scope" data-gh-scope>
              <p class="ferr" role="alert"><span aria-hidden="true">{'\u{F0026}'}</span> GitHub needs the read:project scope to read the board.</p>
              <p class="muted hint">Run <code>{GH_SCOPE_COMMAND}</code>, then Preview.</p>
              <div><button class="btn" onclick={copyCommand}>Copy command</button></div>
            </div>
          {:else if optsError && !notSetUp}
            <p class="ferr">{trackerError(optsError, 'github', trkName).text}</p>
          {/if}
        </div>
      </div>
    {:else}
      {#if notSetUp}{@render integrations(notSetUp)}{/if}
      <div class="row top">
        <span class="lab" id="trk-teams">Teams</span>
        <div class="col">
          <div class="tags w360" class:bad={errs.teams} role="group" aria-labelledby="trk-teams">
            {#each form.tracker.teams as t (t)}<span class="chip">{t}<button type="button" class="x" aria-label="Remove {t}" onclick={() => ((form.tracker.teams = form.tracker.teams.filter((x) => x !== t)), edit())}>×</button></span>{/each}
            <Select width="hug" bare value="" options={teamOptions} placeholder="add a team…" noun="team" aria-label="Add a team" onchange={(v) => ((form.tracker.teams = addTo(form.tracker.teams, v)), edit())} />
          </div>
          {@render err('teams')}
        </div>
      </div>
      <div class="row top" data-projects>
        <span class="lab" id="trk-projects">Projects</span>
        <div class="col" role="radiogroup" aria-labelledby="trk-projects">
          <label class="check"><input type="radio" name="trk-projects" checked={!form.tracker.projects} onchange={() => ((form.tracker.projects = null), edit())} /> Every project of these teams</label>
          <label class="check"><input type="radio" name="trk-projects" checked={!!form.tracker.projects} onchange={() => ((form.tracker.projects ??= []), edit())} /> Only these projects</label>
          {#if form.tracker.projects}
            <div class="tags indent" class:bad={errs.projects}>
              {#each form.tracker.projects as id (id)}<span class="chip">{projectName(id)}<button type="button" class="x" aria-label="Remove {projectName(id)}" onclick={() => ((form.tracker.projects = form.tracker.projects.filter((x) => x !== id)), edit())}>×</button></span>{/each}
              <Select width="hug" bare searchable value="" options={projectOptions} placeholder="add a project…" noun="project" aria-label="Add a project" onchange={(v) => ((form.tracker.projects = addTo(form.tracker.projects, v)), edit())} />
            </div>
          {/if}
          {@render err('projects')}
        </div>
      </div>
      <details class="adv" bind:open={advancedOpen}>
        <summary>Advanced · Team task states</summary>
        <p class="muted hint">Where team tasks go when they move in the feature.</p>
        {#if !rows.length}
          {#if optsError && !notSetUp}
            <p class="ferr" role="alert" data-states-error><span aria-hidden="true">{'\u{F0026}'}</span> {trackerError(optsError, 'linear', trkName).text} <button class="link" onclick={() => optsTick++}>Retry</button></p>
          {:else}
            <p class="muted hint">{form.tracker.teams.length ? 'Loading states…' : 'Pick a team to map its states.'}</p>
          {/if}
        {/if}
        {#each unmapped as st (st.id)}
          <p class="box warnbox" data-unmapped>▲ State "{st.name}" isn't mapped. <button class="btn ghost" onclick={() => /** @type {HTMLElement | null | undefined} */ (root?.querySelector(`[data-state="${st.id}"] [role="combobox"]`))?.focus()} use:tooltip={{ label: 'Map it', key: '⏎' }}>Map it</button></p>
        {/each}
        {#each rows as r, i (r.id)}
          <div class="mrow" data-state={r.id}><span>{r.name}</span><span class="muted" aria-hidden="true">→</span><Select width="hug" aria-label="Column for {r.name}" value={r.column ?? ''} options={BOARD_COLUMNS} placeholder="map to…" onchange={(c) => setStateColumn(i, c)} /></div>
        {/each}
      </details>
    {/if}

    <hr />
    <h3>Ready to start shows</h3>
    <p class="muted hint">Mission control lists these items. Work shows everything.</p>
    {#if form.ready.kind === 'github'}
      <div class="row top">
        <label class="lab" for="trk-labels">Labels</label>
        <div class="col">
          <div class="tags w360">
            {#each form.ready.labels as n (n)}<span class="chip">{n}<button type="button" class="x" aria-label="Remove {n}" onclick={() => ((form.ready.labels = form.ready.labels.filter((x) => x !== n)), (autoReady = false), edit())}>×</button></span>{/each}
            <input id="trk-labels" class="bare" list="trk-label-list" bind:value={labelInput} onkeydown={labelKey} onblur={() => addLabel(labelInput)} placeholder="add…" spellcheck="false" />
          </div>
          <datalist id="trk-label-list">{#each opts?.labels ?? [] as l (l)}<option value={nameOf(l)}></option>{/each}</datalist>
        </div>
      </div>
      <div class="row">
        <label class="lab" for="trk-ms">Milestone</label>
        <div class="w200"><Select id="trk-ms" width="fill" value={form.ready.milestone ?? ''} options={optionList(opts?.milestones, 'Any')} noun="milestone" onchange={(v) => ((form.ready.milestone = any(v)), edit())} /></div>
      </div>
      {#if form.tracker.project_board}
        <div class="row">
          <label class="lab" for="trk-bs">Board status</label>
          <div class="w200"><Select id="trk-bs" width="fill" value={form.ready.board_status ?? ''} options={optionList(statusNames, 'Any')} noun="status" onchange={(v) => ((form.ready.board_status = any(v)), edit())} /></div>
        </div>
      {/if}
      <div class="row">
        <label class="lab" for="trk-it">Issue type</label>
        <div class="w200"><Select id="trk-it" width="fill" value={form.ready.issue_type ?? ''} options={optionList(opts?.issue_types, 'Any')} noun="type" onchange={(v) => ((form.ready.issue_type = any(v)), edit())} /></div>
      </div>
    {:else if form.ready.kind === 'linear'}
      <div class="row">
        <span class="lab" id="trk-st">State types</span>
        <div class="checks" role="group" aria-labelledby="trk-st">
          {#each STATE_TYPES as s (s.id)}
            <label class="check"><input type="checkbox" checked={form.ready.state_types.includes(s.id)} onchange={(e) => ((form.ready.state_types = toggleType(form.ready.state_types, s.id, e.currentTarget.checked)), edit())} /> {s.label}</label>
          {/each}
        </div>
      </div>
      <div class="row">
        <label class="lab" for="trk-ll">Label</label>
        <div class="w200"><Select id="trk-ll" width="fill" value={form.ready.label ?? ''} options={optionList(opts?.labels, 'Any')} noun="label" onchange={(v) => ((form.ready.label = any(v)), edit())} /></div>
      </div>
      <div class="row">
        <label class="lab" for="trk-lp">Project</label>
        <div class="w200"><Select id="trk-lp" width="fill" searchable value={form.ready.project ?? ''} options={[{ value: '', label: 'Any' }, ...linProjects.map((p) => ({ value: p.id, label: p.name }))]} noun="project" onchange={(v) => ((form.ready.project = any(v)), edit())} /></div>
      </div>
      <div class="row">
        <span class="lab" id="trk-cy">Cycle</span>
        <div class="seg" role="radiogroup" aria-labelledby="trk-cy">
          {#each CYCLES as c (c.label)}<button role="radio" aria-checked={form.ready.cycle === c.value} class:on={form.ready.cycle === c.value} onclick={() => ((form.ready.cycle = c.value), edit())}>{c.label}</button>{/each}
        </div>
      </div>
    {/if}
    <div class="indent">
      {@render err('ready')}
      <p class="muted hint" data-ready-summary>{summary}</p>
    </div>

    {#if kind !== 'local'}
      <hr />
      <label class="switch" data-wb-switch><input type="checkbox" role="switch" class="pill" aria-checked={form.write_back.enabled} bind:checked={form.write_back.enabled} onchange={edit} /> <b>Update {wbName} as work runs</b></label>
      {#if form.write_back.enabled}
        <ul class="wb" data-write-back>
          {#each lines as l (l)}<li>{l}</li>{/each}
        </ul>
        <label class="check wbopt"><input type="checkbox" bind:checked={form.write_back.assign_me} onchange={edit} /> Assign the item to me when I start it</label>
        {#if kind === 'github' && !form.tracker.project_board}
          <div class="row wbopt">
            <label class="lab sub" for="trk-run">Running label</label>
            <div class="col"><input id="trk-run" class="field mono" bind:value={form.write_back.running_label} oninput={edit} spellcheck="false" /><p class="muted hint">Added on start, removed on merge</p></div>
          </div>
        {:else if kind === 'github'}
          <div class="row top wbopt">
            <span class="lab sub" id="trk-mv">Move to</span>
            <div class="col">
              <div class="w200"><Select width="fill" aria-labelledby="trk-mv" value={form.write_back.board_status ?? defaultStatus ?? ''} options={optionList(statusNames)} noun="status" onchange={(v) => ((form.write_back.board_status = v), edit())} /></div>
              <p class="muted hint">The board's Status option for started items</p>
            </div>
          </div>
        {:else}
          <div class="row top wbopt">
            <span class="lab sub" id="trk-mv">Move to</span>
            <div class="col">
              <div class="w200"><Select width="fill" aria-labelledby="trk-mv" value={linearTarget?.id ?? ''} options={started.map((s) => ({ value: s.id, label: s.name }))} noun="state" onchange={(v) => ((form.write_back.linear_state = v), edit())} /></div>
              <p class="muted hint">A started state of {form.tracker.teams.join(', ') || 'the team'}</p>
            </div>
          </div>
        {/if}
      {:else}
        <p class="muted hint" data-write-back-off>Read-only: {wbName} stays as it is. Team tasks stay in the app.</p>
      {/if}
    {/if}

    <hr />
    <div class="row top">
      <label class="lab" for="trk-refresh">Refresh</label>
      <div class="col">
        <div class="w160"><Select id="trk-refresh" width="fill" value={form.refresh_secs} options={REFRESH} noun="interval" onchange={(v) => ((form.refresh_secs = v), edit())} /></div>
        <p class="muted hint">Work also refreshes when you open it and on r.</p>
      </div>
    </div>

    <hr />
    <div class="row top">
      <span class="lab">Preview</span>
      <div class="col grow">
        <div><button class="btn" disabled={previewing || !valid || !!notSetUp} onclick={runPreview}>{previewing ? 'Previewing…' : 'Preview'}</button></div>
        {#if preview}
          <div class="result" class:stale data-preview>
            {#if previewErr}
              <p class="ferr" role="alert" title={previewErr.tip}><span aria-hidden="true">{'\u{F0026}'}</span> {previewErr.text}
                {#if previewErr.fix === 'gh_scope'}<button class="link" onclick={() => fix(previewErr)}>Copy command</button>
                {:else if previewErr.fix === 'integrations'}<button class="link" onclick={() => fix(previewErr)}>Settings › Integrations ↗</button>
                {:else if previewErr.fix && previewErr.fix !== 'settings'}<button class="link" onclick={() => fix(previewErr)}>{previewErr.fixLabel ?? 'Retry'}</button>{/if}
              </p>
            {:else if !preview.items}
              <p class="muted">Nothing matches this tracker right now. It may still be right: Ready to start will show items when they appear.</p>
            {:else}
              <p data-preview-count><b>{preview.items} {preview.items === 1 ? 'item' : 'items'}</b>{#if kind !== 'local'}, {preview.roots} at the top{/if} · <b>{preview.ready} ready</b> in {trackerTarget(form.tracker, known)}</p>
              {#each (preview.sample ?? []).slice(0, 5) as n (n.id)}
                {#if kind === 'local'}
                  <div class="srow local">
                    <span class="mono muted">{n.ref}</span>
                    <span use:tooltip={ifCut(n.title)}>{n.title}</span>
                  </div>
                {:else}
                  {@const where = sampleWhere(n, kind)}
                  <div class="srow">
                    <span class="mono muted">{n.ref}</span>
                    <span use:tooltip={ifCut(n.title)}>{n.title}</span>
                    <span class="muted where" use:tooltip={ifCut(where)}>{where}</span>
                  </div>
                {/if}
              {/each}
              {#if preview.more > 0}<p class="muted" data-preview-more>+ {preview.more} more</p>{/if}
            {/if}
            {#if previewWarn}<p class="muted" data-preview-warning>{previewWarn} <button class="link" onclick={runPreview}>Retry</button></p>{/if}
            {#if stale}<p class="stale-line" data-stale>Changed since this preview · <button class="link" onclick={runPreview}>Preview again</button></p>{/if}
          </div>
        {/if}
      </div>
    </div>

    {#if ask}
      <div class="notice" role="alertdialog" aria-label="Move tasks to Linear" data-confirm>
        <span class="glyph" aria-hidden="true">ⓘ</span>
        <div class="nbody">
          <p><b>Move {name}'s tasks to Linear?</b> {ask.open} open task{ask.open === 1 ? '' : 's'}.{ask.done ? ` ${ask.done} done tasks are not pushed.` : ''}</p>
          <div class="acts">
            <button class="btn primary" disabled={busy} onclick={() => answer('push_open')}>Create {ask.open} open tasks as issues in {ask.settings.tracker.teams.join(', ')}</button>
            <button class="btn" disabled={busy} onclick={() => answer('leave')}>Leave them out (they stay in the app, not synced)</button>
            <button class="btn" disabled={busy} onclick={() => (ask = null)}>Cancel</button>
          </div>
        </div>
      </div>
    {/if}
    {#if saveError}<p class="ferr" role="alert">Could not save: {saveError}</p>{/if}
    <div class="foot">
      <button class="btn primary" disabled={busy || !dirty || (touched && !valid)} use:tooltip={{ label: 'Save', key: '⌘⏎' }} onclick={() => save()}>{busy ? 'Saving…' : 'Save'}</button>
      <button class="btn" disabled={!dirty} onclick={revert}>Revert</button>
    </div>
  {/if}
</section>

<style>
  .tracker {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    max-width: 860px;
  }
  h2 {
    display: flex;
    align-items: baseline;
    gap: var(--space-sm);
    margin: 0;
    font: var(--text-heading);
  }
  h3 {
    margin: 0;
    font: var(--text-title);
  }
  p {
    margin: 0;
  }
  hr {
    width: 100%;
    margin: var(--space-sm) 0;
    border: 0;
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
  .lead,
  .hint {
    font: var(--text-caption);
  }
  /* Beats the global `.hint` (app.css): muted, one line of its own. */
  .hint {
    display: block;
    color: var(--color-text-muted);
  }
  .dot {
    color: var(--color-accent-default);
  }
  .unsaved {
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .done {
    color: var(--color-status-done);
    font: var(--text-caption);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }
  .row.top {
    align-items: flex-start;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .grow {
    flex: 1;
  }
  .lab {
    width: 180px;
    flex: none;
    color: var(--color-text-muted);
  }
  .lab.sub {
    width: 130px;
  }
  .wbopt {
    margin-left: var(--space-xl);
  }
  .w360,
  .w320,
  .w200,
  .w160 {
    box-sizing: border-box;
  }
  .w360 {
    width: 360px;
  }
  .w320 {
    width: 320px;
  }
  .w200 {
    width: 200px;
  }
  .w160 {
    width: 160px;
  }
  .indent {
    margin-left: calc(180px + var(--space-md));
  }
  .scope {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    align-items: flex-start;
  }
  .stale-line {
    margin-top: var(--space-xs);
    color: var(--color-pacing-amber);
    font: var(--text-caption);
  }
  .stale-line .link {
    color: var(--color-pacing-amber);
    text-decoration: underline;
  }
  .warnbox {
    border-color: var(--color-pacing-amber);
  }
  .pill {
    appearance: none;
    flex: none;
    position: relative;
    width: 34px;
    height: 18px;
    margin: 0;
    border-radius: 9px;
    background: var(--color-border-strong);
    cursor: pointer;
  }
  .pill::after {
    content: '';
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--color-text-primary);
  }
  .pill:checked {
    background: var(--color-accent-default);
  }
  .pill:checked::after {
    left: 18px;
  }
  .pill:focus-visible {
    outline: var(--border-width-focus) solid var(--color-focus);
    outline-offset: 2px;
  }
  .seg {
    display: inline-flex;
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }
  .seg button {
    height: var(--size-control-sm);
    padding: 0 var(--space-md);
    border: 0;
    background: none;
    color: var(--color-text-secondary);
    font: var(--text-label);
  }
  .seg button.on {
    background: var(--color-surface-selected);
    color: var(--color-text-primary);
  }
  .field.bad,
  .tags.bad {
    border-color: var(--color-status-error);
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-xs);
    min-height: var(--size-control);
    padding: 0 var(--space-xs);
    background: var(--color-surface-chrome);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
  }
  .tags.indent {
    margin-left: var(--space-lg);
  }
  .bare {
    flex: 1;
    min-width: 96px;
    border: 0;
    outline: none;
    background: none;
    color: var(--color-text-primary);
    font: var(--text-body);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xxs);
    padding: 0 var(--space-xs);
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
    font: var(--text-label);
  }
  .x {
    padding: 0;
    border: 0;
    background: none;
    color: var(--color-text-muted);
  }
  .check,
  .switch {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .checks {
    display: flex;
    gap: var(--space-lg);
  }
  .adv {
    margin-left: calc(180px + var(--space-md));
  }
  .adv summary {
    cursor: pointer;
    font: var(--text-body-strong);
  }
  .mrow {
    display: grid;
    grid-template-columns: 170px 20px auto;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-xxs) 0;
  }
  .wb {
    margin: 0;
    padding-left: calc(30px + var(--space-xl));
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .ferr {
    color: var(--color-status-error);
    font: var(--text-caption);
  }
  .box {
    padding: var(--space-xs) var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    font: var(--text-caption);
  }
  .notice {
    display: flex;
    gap: var(--space-sm);
    padding: var(--space-md);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .notice.warn {
    border-color: var(--color-pacing-amber);
  }
  .nbody {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    align-items: flex-start;
  }
  .choices {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .acts {
    display: flex;
    gap: var(--space-sm);
  }
  .result {
    container-type: inline-size;
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .srow.local {
    grid-template-columns: 160px 1fr;
  }
  .srow {
    display: grid;
    grid-template-columns: 100px 1fr 1fr;
    gap: var(--space-sm);
    padding-top: var(--space-xs);
  }
  /* One line per sample: cut with an ellipsis (the tooltip has the full text). */
  .srow > span {
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  /* The 1280 board: ref and title only. */
  @container (max-width: 560px) {
    .srow:not(.local) {
      grid-template-columns: 100px 1fr;
    }
    .srow .where {
      display: none;
    }
  }
  .foot {
    display: flex;
    gap: var(--space-sm);
    padding-top: var(--space-sm);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
</style>
