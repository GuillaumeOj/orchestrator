<script>
  // Project settings › Intake (P4.5 spec §3.6, notes p4-5 §7, Penpot 04.7): where the work you can
  // start comes from. None · GitHub · Linear · Repo docs, the fields of the chosen source, Preview
  // (a dry run, nothing stored), Save (stores, refreshes at once) and the hints. Starting is
  // always the user's action; intake only reads. ⌘⏎ saves.
  import { onMount, untrack } from 'svelte';
  import { openSettings, toast } from '../lib/app.svelte.js';
  import { errorInfo, GH_SCOPE_COMMAND, integrationsRow, SOURCE_NAME } from '../lib/intake.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { tooltip } from '../lib/tooltip.js';
  import Select from './Select.svelte';

  let { project, name = project } = $props();

  const KINDS = ['none', 'github', 'linear', 'repo_docs'];
  const FILTERS = [
    { id: 'label', label: 'Label' },
    { id: 'milestone', label: 'Milestone' },
    { id: 'project', label: 'Project' },
  ];

  let info = $state(null); // intake_source: { source, default_glob, default_branch, known_repo }
  let loadError = $state(null);
  let type = $state('none');
  let repo = $state('');
  let fkind = $state('label');
  let names = $state([]);
  let nameInput = $state('');
  let milestone = $state('');
  let pOwner = $state('');
  let pNumber = $state('');
  let pStatus = $state('');
  let team = $state('');
  let lproject = $state('');
  let llabel = $state('');
  let backlog = $state(false);
  let glob = $state('');
  let branch = $state('');

  let baseline = $state('null'); // JSON of the saved source (dirty compare)
  let busy = $state(false);
  let previewing = $state(false);
  let preview = $state(null); // { count, more, items, error }
  let previewJson = $state(null); // the source JSON the preview was run on
  let fieldErrors = $state(/** @type {Record<string, string>} */ ({})); // field → message (client rules and the core's bad_request)
  let saveError = $state(null);
  let savedAt = $state(0);
  let savedTimer = null;
  let touched = $state(false); // a Preview or Save was tried: client rules show

  let ghOptions = $state(null); // { labels, milestones, projects }
  let ghOptionsError = $state(null); // a source-error shape
  let teams = $state(null); // linear_teams, null = loading
  let linearError = $state(null);
  let linStates = $state(null); // linear_states: projects
  let linLabels = $state([]);
  let matches = $state(null); // repo docs: file count for the glob

  // ------------------------------------------------------------ the source as JSON

  /** The form as an IntakeSource (api-contract §1.1). */
  function toSource() {
    if (type === 'github') {
      const filter =
        fkind === 'label'
          ? { kind: 'label', names: [...names] }
          : fkind === 'milestone'
            ? { kind: 'milestone', title: milestone }
            : { kind: 'project', owner: pOwner.trim(), number: Number(pNumber), ...(pStatus.trim() ? { status: pStatus.trim() } : {}) };
      return { type: 'github', ...(repo.trim() ? { repo: repo.trim() } : {}), filter };
    }
    if (type === 'linear')
      return { type: 'linear', team, ...(lproject ? { project: lproject } : {}), ...(llabel ? { label: llabel } : {}), include_backlog: backlog };
    if (type === 'repo_docs') return { type: 'repo_docs', glob: glob.trim(), branch: branch.trim() || null };
    return { type: 'none' };
  }
  const json = $derived(JSON.stringify(toSource()));
  const dirty = $derived(!!info && json !== baseline);
  /** "acme/shop · label ready": what the preview counted (notes §7.3). */
  const previewLabel = $derived.by(() => {
    if (type === 'github') {
      const f = fkind === 'label' ? `label ${names.join(', ')}` : fkind === 'milestone' ? `milestone ${milestone}` : `project ${pOwner}/${pNumber}`;
      return [repo || info?.known_repo, f].filter(Boolean).join(' · ');
    }
    return type === 'repo_docs' ? glob : '';
  });

  function fromSource(s) {
    type = s?.type ?? 'none';
    repo = s?.repo ?? '';
    fkind = s?.filter?.kind ?? 'label';
    names = s?.filter?.names ? [...s.filter.names] : [];
    milestone = s?.filter?.title ?? '';
    pOwner = s?.filter?.owner ?? '';
    pNumber = s?.filter?.number != null ? String(s.filter.number) : '';
    pStatus = s?.filter?.status ?? '';
    team = s?.team ?? '';
    lproject = s?.project ?? '';
    llabel = s?.label ?? '';
    backlog = !!s?.include_backlog;
    glob = s?.glob ?? info?.default_glob ?? '.orchestrator/backlog/*.md';
    branch = s?.branch ?? '';
  }

  /** The error under a field, in the notes' words (§7.2). */
  function fieldCopy(field, message = '') {
    switch (field) {
      case 'repo':
        return 'Use owner/name, e.g. acme/shop.';
      case 'names':
        return 'Add at least one label.';
      case 'number':
        return 'Use a number of 1 or more.';
      case 'team':
        return 'Pick a team.';
      case 'glob':
        return /\.\.|leading|\//.test(message) && !/parse/i.test(message) ? 'Use a path inside the repo: no leading `/`, no `..`.' : "The glob doesn't parse.";
      case 'branch':
        return `Branch \`${branch.trim()}\` isn't in this repo.`;
      default:
        return message;
    }
  }

  /** Client-side rules, the same fields the core validates; {} when valid. */
  const problems = $derived.by(() => {
    const p = /** @type {Record<string, string>} */ ({});
    if (type === 'github') {
      if (repo.trim() ? !/^[^/\s]+\/[^/\s]+$/.test(repo.trim()) : !info?.known_repo) p.repo = repo.trim() ? fieldCopy('repo') : 'Needs a repository.';
      if (fkind === 'label' && !names.length) p.names = fieldCopy('names');
      if (fkind === 'milestone' && !milestone) p.milestone = 'Pick a milestone.';
      if (fkind === 'project') {
        if (!pOwner.trim()) p.owner = 'Needs an owner.';
        if (!(Number(pNumber) >= 1)) p.number = fieldCopy('number');
      }
    } else if (type === 'linear') {
      if (!team) p.team = fieldCopy('team');
    } else if (type === 'repo_docs') {
      const g = glob.trim();
      if (!g || g.startsWith('/') || g.includes('..')) p.glob = fieldCopy('glob', 'leading / or ..');
    }
    return p;
  });
  const valid = $derived(!Object.keys(problems).length);
  const milestoneOptions = $derived([
    ...(ghOptions?.milestones ?? []).map((m) => ({ value: m.title, label: `${m.title} · ${m.open_issues} open` })),
    // A saved milestone the list no longer has stays selectable (shown, not lost).
    ...(milestone && !(ghOptions?.milestones ?? []).some((m) => m.title === milestone) ? [{ value: milestone, label: milestone }] : []),
  ]);
  const ghProjectOptions = $derived((ghOptions?.projects ?? []).map((p) => ({ value: `${p.owner}/${p.number}`, label: `${p.owner} #${p.number} · ${p.title}` })));
  const teamOptions = $derived((teams ?? []).map((t) => ({ value: t.key, label: `${t.key} · ${t.name}` })));
  const lprojectOptions = $derived([{ value: '', label: 'Any project' }, ...(linStates?.projects ?? []).map((p) => ({ value: p.id, label: p.name }))]);
  const llabelOptions = $derived([{ value: '', label: 'Any label' }, ...linLabels.map((l) => ({ value: l, label: l }))]);
  const errs = $derived({ ...(touched ? problems : {}), ...fieldErrors });

  // ------------------------------------------------------------ loading

  async function load() {
    try {
      info = await invoke('intake_source', { project });
      loadError = null;
      fromSource(info.source);
      // The default repository is part of the form before the baseline is taken, or a saved source
      // without `repo` would open as unsaved.
      if (!repo && info.known_repo && type === 'github') repo = info.known_repo;
      baseline = JSON.stringify(toSource());
    } catch (e) {
      loadError = errText(e);
    }
  }

  /** A command error as {code, field, message}: an object, or JSON text. */
  function parseErr(e) {
    if (e && typeof e === 'object' && ('code' in e || 'field' in e)) return e;
    try {
      const j = JSON.parse(typeof e === 'string' ? e : (e?.message ?? ''));
      if (j && typeof j === 'object') return j;
    } catch {}
    return { message: errText(e) };
  }

  // A request counter per kind: an answer older than the latest request is dropped (null).
  const seq = { github: 0, linear: 0, repo_docs: 0 };
  async function loadOptions(source) {
    const n = ++seq[source.type];
    let out;
    try {
      out = (await invoke('intake_options', { project, source })) ?? {};
    } catch (e) {
      out = { error: { code: parseErr(e).code ?? 'failed', message: errText(e) } };
    }
    return n === seq[source.type] ? out : null;
  }

  // GitHub suggestions follow the repository (debounced), Linear's the team, the glob's count the glob.
  $effect(() => {
    if (!info || type !== 'github') return;
    const r = repo.trim() || info.known_repo || '';
    if (r && !/^[^/\s]+\/[^/\s]+$/.test(r)) return;
    const t = setTimeout(
      async () => {
        const o = await loadOptions({ type: 'github', ...(repo.trim() ? { repo: repo.trim() } : {}) });
        if (!o) return;
        ghOptions = o;
        ghOptionsError = o.error ?? null;
      },
      untrack(() => (ghOptions ? 400 : 0)),
    );
    return () => clearTimeout(t);
  });

  $effect(() => {
    if (!info || type !== 'linear' || teams) return;
    untrack(async () => {
      try {
        teams = (await invoke('linear_teams')).teams ?? [];
        linearError = null;
        if (!team && teams.length) team = teams[0].key;
      } catch (e) {
        teams = [];
        linearError = errText(e);
      }
    });
  });

  $effect(() => {
    if (!info || type !== 'linear' || !team || linearError) return;
    const t = team;
    untrack(async () => {
      let states = null;
      try {
        states = await invoke('linear_states', { team: t });
      } catch {}
      if (team === t) linStates = states;
      const o = await loadOptions({ type: 'linear', team: t });
      if (o && team === t) linLabels = o.labels ?? [];
    });
  });

  $effect(() => {
    if (!info || type !== 'repo_docs') return;
    const g = glob.trim();
    if (!g || g.startsWith('/') || g.includes('..')) {
      matches = null;
      return;
    }
    const b = branch.trim();
    const t = setTimeout(async () => {
      const o = await loadOptions({ type: 'repo_docs', glob: g, branch: b || null });
      if (o && glob.trim() === g) matches = o.error ? null : (o.matches ?? null);
    }, 400);
    return () => clearTimeout(t);
  });

  onMount(() => {
    load();
    return () => clearTimeout(savedTimer);
  });

  // ------------------------------------------------------------ actions

  function pickType(t) {
    if (t === type) return;
    type = t;
    if (t === 'github' && !repo && info?.known_repo) repo = info.known_repo;
    if (t === 'repo_docs' && !glob) glob = info?.default_glob ?? '.orchestrator/backlog/*.md';
    fieldErrors = {};
    saveError = null;
  }

  function addName(v = nameInput) {
    const n = v.trim().replace(/,$/, '').trim();
    if (n && !names.includes(n)) names = [...names, n];
    nameInput = '';
    edit();
  }

  function nameKey(e) {
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault();
      e.stopPropagation();
      addName();
    } else if (e.key === 'Backspace' && !nameInput && names.length) names = names.slice(0, -1);
  }

  /** Any edit drops the last outcome of a save and the core's field errors. */
  function edit() {
    fieldErrors = {};
    saveError = null;
  }

  function onPickProject(v) {
    const p = ghOptions?.projects?.find((x) => `${x.owner}/${x.number}` === v);
    if (!p) return;
    pOwner = p.owner;
    pNumber = String(p.number);
    pStatus = '';
    edit();
  }

  const pickedProject = $derived(ghOptions?.projects?.find((x) => x.owner === pOwner && String(x.number) === pNumber) ?? null);
  const statusOptions = $derived([{ value: '', label: 'All statuses' }, ...(pickedProject?.statuses ?? []).map((s) => ({ value: s, label: s }))]);

  async function runPreview() {
    touched = true;
    if (previewing || !valid) return;
    previewing = true;
    fieldErrors = {};
    const src = toSource();
    try {
      preview = await invoke('intake_preview', { project, source: src });
      previewJson = JSON.stringify(src);
    } catch (e) {
      const x = parseErr(e);
      if (x.field) fieldErrors = { [x.field]: fieldCopy(x.field, x.message) };
      else preview = { error: { code: x.code ?? 'failed', message: x.message } };
      previewJson = JSON.stringify(src);
    } finally {
      previewing = false;
    }
  }

  async function save() {
    touched = true;
    if (busy || !valid || !dirty) return;
    busy = true;
    saveError = null;
    fieldErrors = {};
    const src = toSource();
    try {
      const r = await invoke('intake_source_set', { project, source: src });
      baseline = JSON.stringify(src);
      info = { ...info, source: src.type === 'none' ? null : src };
      preview = r?.preview ?? null;
      previewJson = baseline;
      savedAt = Date.now();
      clearTimeout(savedTimer);
      savedTimer = setTimeout(() => (savedAt = 0), 5000);
    } catch (e) {
      const x = parseErr(e);
      if (x.field) fieldErrors = { [x.field]: fieldCopy(x.field, x.message) };
      else saveError = x.message ?? errText(e);
    } finally {
      busy = false;
    }
  }

  function revert() {
    if (!info) return;
    fromSource(info.source);
    fieldErrors = {};
    saveError = null;
    touched = false;
  }

  async function copyCommand() {
    try {
      await navigator.clipboard.writeText(GH_SCOPE_COMMAND);
    } catch {}
    toast('Copied. Run it in a terminal, then Preview again.');
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

  const previewErr = $derived(preview?.error ? errorInfo(preview.error, { type }) : null);
  const stale = $derived(!!preview && previewJson !== json);
  const saved = $derived(savedAt > 0);
  const optErr = $derived(ghOptionsError ? errorInfo(ghOptionsError, { type: 'github' }) : null);
  const notSetUp = $derived(
    type === 'github' && ghOptionsError && ['gh_missing', 'gh_auth'].includes(ghOptionsError.code)
      ? 'GitHub'
      : type === 'linear' && linearError
        ? 'Linear'
        : null,
  );
</script>

{#snippet err(key)}
  {#if errs[key]}<p class="ferr" role="alert"><span aria-hidden="true">{'\u{F0026}'}</span> {errs[key]}</p>{/if}
{/snippet}

{#snippet integrations(code, label)}
  <p class="box info" data-not-set-up>{label} isn't set up. <button class="link" onclick={() => openSettings('integrations', integrationsRow(code))}>Settings › Integrations ↗</button></p>
{/snippet}

<section class="intake" aria-label="Intake">
  <h2>Intake{#if dirty}<span class="dot" title="Unsaved changes" aria-hidden="true"> ●</span>{/if}</h2>
  <p class="muted lead">Where the work you can start comes from. Starting is always your action.</p>

  {#if loadError}
    <p class="ferr">Could not read {name}'s intake source: {loadError}</p>
    <div><button class="btn" onclick={load}>Retry</button></div>
  {:else if !info}
    <p class="muted">Loading…</p>
  {:else}
    <div class="line">
      <span class="lab" id="src-lab">Source</span>
      <div class="seg" role="radiogroup" aria-labelledby="src-lab">
        {#each KINDS as k (k)}<button role="radio" aria-checked={type === k} class:on={type === k} onclick={() => pickType(k)}>{k === 'none' ? 'None' : SOURCE_NAME[k]}</button>{/each}
      </div>
      {#if dirty}<span class="muted">Unsaved changes</span>{/if}
      {#if saved}<span class="done" role="status">Saved · updated just now</span>{/if}
    </div>

    {#if type === 'none'}
      <p class="muted">No source. The Ready to start lane will offer Plan with the Overwatch for this project.</p>
    {:else if type === 'github'}
      <div class="block">
        <label class="lab" for="in-repo">Repository</label>
        <input id="in-repo" class="field mono" class:bad={errs.repo} bind:value={repo} oninput={edit} placeholder={info.known_repo || 'owner/name'} spellcheck="false" />
        <p class="muted hint">Defaults to this project's repository.</p>
        {@render err('repo')}
      </div>
      {#if notSetUp}{@render integrations(ghOptionsError.code, notSetUp)}{/if}
      <div class="line">
        <span class="lab" id="flt-lab">Filter</span>
        <div class="seg" role="radiogroup" aria-labelledby="flt-lab">
          {#each FILTERS as f (f.id)}<button role="radio" aria-checked={fkind === f.id} class:on={fkind === f.id} onclick={() => ((fkind = f.id), edit())}>{f.label}</button>{/each}
        </div>
      </div>
      {#if fkind === 'label'}
        <div class="block">
          <label class="lab" for="in-label">Label</label>
          <div class="tags" class:bad={errs.names}>
            {#each names as n (n)}<span class="chip">{n}<button type="button" class="x" aria-label="Remove {n}" onclick={() => ((names = names.filter((x) => x !== n)), edit())}>×</button></span>{/each}
            <input id="in-label" class="bare" list="gh-labels" bind:value={nameInput} onkeydown={nameKey} onblur={() => addName()} placeholder={names.length ? '' : 'Add a label'} spellcheck="false" />
          </div>
          <datalist id="gh-labels">{#each ghOptions?.labels ?? [] as l (l)}<option value={l}></option>{/each}</datalist>
          <p class="muted hint">Items that have all of these labels.</p>
          {@render err('names')}
        </div>
      {:else if fkind === 'milestone'}
        <div class="block">
          <label class="lab" for="in-ms">Milestone</label>
          <Select id="in-ms" width="hug" bind:value={milestone} options={milestoneOptions} placeholder="Pick a milestone" noun="milestone" error={errs.milestone} onchange={edit} />
          <p class="muted hint">{ghOptions && !ghOptions.milestones?.length ? `No open milestones in ${repo.trim() || info.known_repo || 'the repository'}.` : 'Open issues in this milestone.'}</p>
        </div>
      {:else}
        {#if ghOptions?.projects?.length}
          <div class="block">
            <label class="lab" for="in-proj">Project</label>
            <Select id="in-proj" width="hug" searchable value={pickedProject ? `${pickedProject.owner}/${pickedProject.number}` : ''} options={ghProjectOptions} placeholder="Pick a project" noun="project" onchange={onPickProject} />
          </div>
        {/if}
        <div class="row">
          <div class="block"><label class="lab" for="in-owner">Owner</label><input id="in-owner" class="field" class:bad={errs.owner} bind:value={pOwner} oninput={edit} spellcheck="false" />{@render err('owner')}</div>
          <div class="block"><label class="lab" for="in-num">Number</label><input id="in-num" class="field num" class:bad={errs.number} type="number" min="1" bind:value={pNumber} oninput={edit} />{@render err('number')}</div>
          <div class="block">
            <label class="lab" for="in-status">Status</label>
            {#if pickedProject?.statuses?.length}
              <Select id="in-status" width="hug" bind:value={pStatus} options={statusOptions} noun="status" onchange={edit} />
            {:else}
              <input id="in-status" class="field" bind:value={pStatus} oninput={edit} placeholder="optional" spellcheck="false" />
            {/if}
          </div>
        </div>
        <p class="muted hint">The project's number is in its URL. Status: the value of the project's Status field to keep, e.g. Todo. Leave empty for all.</p>
        {#if ghOptionsError?.code === 'gh_scope'}
          <p class="ferr" role="alert" data-gh-scope>GitHub needs the read:project scope. Run <code>{GH_SCOPE_COMMAND}</code>. <button class="link" onclick={copyCommand}>Copy command</button></p>
        {/if}
      {/if}
      {#if optErr && !notSetUp && ghOptionsError.code !== 'gh_scope'}<p class="ferr">{optErr.text}</p>{/if}
    {:else if type === 'linear'}
      {#if notSetUp}
        {@render integrations(linearError?.includes('auth') ? 'linear_auth' : 'linear_key', 'Linear')}
      {:else if !teams}
        <p class="muted">Loading teams…</p>
      {:else}
        <div class="block">
          <label class="lab" for="in-team">Team</label>
          <Select id="in-team" width="hug" searchable bind:value={team} options={teamOptions} placeholder="Pick a team" noun="team" error={errs.team} onchange={() => ((lproject = ''), (llabel = ''), edit())} />
        </div>
        <div class="row">
          <div class="block">
            <label class="lab" for="in-lproj">Project</label>
            <Select id="in-lproj" width="hug" searchable bind:value={lproject} options={lprojectOptions} noun="project" onchange={edit} />
          </div>
          <div class="block">
            <label class="lab" for="in-llabel">Label</label>
            <Select id="in-llabel" width="hug" bind:value={llabel} options={llabelOptions} noun="label" onchange={edit} />
          </div>
        </div>
        <label class="check"><input type="checkbox" bind:checked={backlog} onchange={edit} /> Include backlog</label>
        <p class="muted hint">By default only issues that are ready to start (Todo, Unstarted) are listed.</p>
      {/if}
    {:else}
      <div class="block">
        <label class="lab" for="in-glob">Glob</label>
        <input id="in-glob" class="field mono" class:bad={errs.glob} bind:value={glob} oninput={edit} spellcheck="false" />
        {#if !errs.glob && matches != null}<p class="muted hint" data-matches>{matches === 0 ? 'No file matches' : matches === 1 ? '1 file matches' : `${matches} files match`}</p>{/if}
        {@render err('glob')}
      </div>
      <div class="block">
        <label class="lab" for="in-branch">Branch</label>
        <input id="in-branch" class="field mono" class:bad={errs.branch} bind:value={branch} oninput={edit} placeholder="{info.default_branch || 'main'} (base branch)" spellcheck="false" />
        <p class="muted hint">Read from the local branch, no checkout. A file committed on it shows within a minute.</p>
        {@render err('branch')}
      </div>
      <p class="muted hint">Optional front matter: <code>title</code>, <code>size</code> (S, M, L), <code>mode</code> (team, solo), <code>team</code> (a team id), <code>status</code> (done or started hides the file). Without a title, the first <code>#</code> heading is used.</p>
    {/if}

    {#if type !== 'none'}
      <div class="preview">
        <div class="line">
          <button class="btn" disabled={previewing || !valid || !!notSetUp} onclick={runPreview}>{previewing ? 'Previewing…' : 'Preview'}</button>
        </div>
        {#if preview}
          <div class="result" class:stale data-preview>
            {#if previewErr}
              <p class="ferr" role="alert" title={previewErr.tip}><span aria-hidden="true">{'\u{F0026}'}</span> {previewErr.text}
                {#if previewErr.fix === 'gh_scope'}<button class="link" onclick={copyCommand}>Copy command</button>
                {:else if previewErr.fix === 'integrations'}<button class="link" onclick={() => openSettings('integrations', integrationsRow(previewErr.code))}>Settings › Integrations ↗</button>{/if}
              </p>
            {:else if !preview.count}
              <p class="muted">Nothing matches this source right now. It may still be right: Ready to start will show items when they appear.</p>
            {:else}
              <p><b>{preview.count} {preview.count === 1 ? 'item' : 'items'}</b>{previewLabel ? ` in ${previewLabel}` : ''}</p>
              <ul>
                {#each (preview.items ?? []).slice(0, 5) as it (it.id ?? it.ref)}<li><span class="mono muted">{it.ref}</span> {it.title}</li>{/each}
              </ul>
              {#if preview.more > 0}<p class="muted">+ {preview.more} more</p>{/if}
            {/if}
            {#if stale}<p class="muted">Changed since this preview</p>{/if}
          </div>
        {/if}
      </div>
    {/if}

    {#if saveError}<p class="ferr" role="alert">Could not save: {saveError}</p>{/if}
    <div class="foot">
      <button class="btn primary" disabled={busy || !dirty || (touched && !valid)} use:tooltip={{ label: 'Save', key: '⌘⏎' }} onclick={save}>{busy ? 'Saving…' : 'Save'}</button>
      {#if dirty}<button class="btn ghost" onclick={revert}>Revert</button>{/if}
    </div>
    <p class="muted hint">Issues labelled <code>orch</code> or <code>orch:*</code> are never listed: they are tasks of the task backend. · Pull requests are not listed. · Intake only reads: starting an item changes nothing at the source.</p>
  {/if}
</section>

<style>
  .intake {
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    max-width: 720px;
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  p {
    margin: 0;
  }
  .lead {
    font: var(--text-caption);
  }
  .dot {
    color: var(--color-accent-default);
  }
  .line,
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-sm);
  }
  .row {
    align-items: flex-start;
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .lab {
    font: var(--text-body-strong);
  }
  .hint {
    font: var(--text-caption);
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
  .num {
    width: 96px;
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
  .check {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .ferr {
    color: var(--color-status-error);
    font: var(--text-caption);
  }
  .done {
    color: var(--color-status-done);
  }
  .box {
    padding: var(--space-xs) var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    font: var(--text-caption);
  }
  .preview .result {
    margin-top: var(--space-sm);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .result.stale {
    opacity: 0.6;
  }
  ul {
    margin: var(--space-xs) 0 0;
    padding-left: var(--space-md);
  }
  .foot {
    display: flex;
    gap: var(--space-sm);
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
