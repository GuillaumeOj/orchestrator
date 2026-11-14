<script>
  // Settings → Sandbox (allow-fine-tune-sandbox-permissions, ux-flows §3, api-contract §3): what an
  // agent's shell commands can reach, per file (All projects, then each project). Built-in rows are
  // read-only; the inline Add form checks the value live with the core (`sandbox_check`), the core's
  // sentence is shown as is; Remove is at once with an Undo line; the lists reload on the `sandbox` topic.
  import { onMount } from 'svelte';
  import { openPolicy, openSettings, projects } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { onTopics } from '../lib/now.svelte.js';
  import { midCut, WARNINGS_SHOWN } from '../lib/policy.js';
  import { appliesLine, EXAMPLES, entryText, fixFirst, GITHUB_LINE, groups, isGithub, KIND_ORDER, KINDS, reason, scopeArgs } from '../lib/sandbox.js';
  import Select from './Select.svelte';

  const UNDO_MS = 6000;
  const HIGHLIGHT_MS = 2000;
  const CHECK_MS = 300;

  let scopes = $state(/** @type {any[] | null} */ (null));
  let kinds = $state(/** @type {string[]} */ ([]));
  let loadError = $state('');
  let limits = $state(false);
  let more = $state(/** @type {Set<string>} */ (new Set()));
  let applies = $state(/** @type {Record<string, string>} */ ({}));
  let highlight = $state(/** @type {Set<string>} */ (new Set()));
  let ghosts = $state(/** @type {Record<string, any[]>} */ ({})); // removed rows kept as an Undo line
  let rowErr = $state(/** @type {Record<string, string>} */ ({}));
  let note = $state(/** @type {Record<string, string>} */ ({})); // "Already allowed here."
  let form = $state(/** @type {any} */ (null));
  let check = $state({ state: 'idle', value: '', reason: '' });
  let roles = $state(/** @type {Record<string, { id: string, name: string }[]>} */ ({}));
  const timers = new Set();
  let seen = null; // row keys of the last load: a row that appears in a later one is highlighted

  const later = (fn, ms) => {
    const t = setTimeout(() => (timers.delete(t), fn()), ms);
    timers.add(t);
    return t;
  };
  onMount(() => () => timers.forEach(clearTimeout));

  const order = (list) => {
    const at = (f) => projects().findIndex((p) => p.id === f.project);
    return [...list].sort((a, b) => (a.scope === 'global' ? -1 : b.scope === 'global' ? 1 : at(a) - at(b)));
  };
  const blocks = $derived(order(scopes ?? []));
  const nameOf = (f) => (f.scope === 'global' ? 'All projects' : (projects().find((p) => p.id === f.project)?.name ?? f.project));
  const rowKey = (f, id) => `${f.file}:${id}`;

  async function load() {
    try {
      const [list, k] = await Promise.all([invoke('sandbox_list', {}), invoke('sandbox_kinds').catch(() => [])]);
      const keys = new Set(list.flatMap((f) => (f.entries ?? []).map((e) => rowKey(f, e.id))));
      if (seen) flash([...keys].filter((x) => !seen.has(x)));
      seen = keys;
      scopes = list;
      kinds = Array.isArray(k) ? k : [];
      loadError = '';
    } catch (e) {
      loadError = errText(e);
    }
  }
  onMount(() => onTopics(['sandbox'], load));

  function flash(keys) {
    if (!keys.length) return;
    highlight = new Set([...highlight, ...keys]);
    later(() => (highlight = new Set([...highlight].filter((k) => !keys.includes(k)))), HIGHLIGHT_MS);
  }
  function applied(f, r) {
    applies = { ...applies, [f.file]: appliesLine(r) };
    later(() => applies[f.file] === appliesLine(r) && (applies = { ...applies, [f.file]: '' }), 10000);
  }

  // ------------------------------------------------------------ add form
  const needsValue = (kind) => KINDS[kind]?.field != null;
  const value = $derived(form?.value.trim() ?? '');
  const scopeOptions = $derived([
    { value: 'global', label: 'All projects' },
    ...projects().map((p) => ({ value: `project:${p.id}`, label: p.name })),
  ]);
  const kindOptions = $derived(kinds.filter((k) => KINDS[k]).sort((a, b) => KIND_ORDER.indexOf(a) - KIND_ORDER.indexOf(b)).map((k) => ({ value: k, label: KINDS[k].label })));
  const roleOptions = $derived([{ value: '', label: 'Any role' }, ...(roles[form?.scope] ?? []).map((r) => ({ value: r.id, label: r.name ?? r.id }))]);

  function openForm(f, kind = kinds[0], val = '') {
    form = { file: f.file, kind, value: val, scope: f.file, role: '', note: '', touched: false, busy: false, error: '' };
    check = { state: 'idle', value: '', reason: '' };
  }

  // The roles of the chosen scope's team (all of them for All projects); a solo project has none.
  $effect(() => {
    const s = form?.scope;
    if (!s || roles[s]) return;
    invoke('teams_view', { project: scopeArgs(s).project })
      .then((v) => (roles = { ...roles, [s]: v?.roles ?? [] }))
      .catch(() => (roles = { ...roles, [s]: [] }));
  });

  // The live check: 300 ms after the last keystroke, and on a kind or scope change.
  let seq = 0;
  $effect(() => {
    if (!form) return;
    const { kind, scope } = form;
    const val = value;
    const mine = ++seq;
    check = { state: 'idle', value: '', reason: '' };
    if (needsValue(kind) && !val) return;
    const t = setTimeout(() => {
      check = { state: 'checking', value: '', reason: '' };
      invoke('sandbox_check', { kind, value: needsValue(kind) ? val : null, ...scopeArgs(scope) })
        .then((r) => {
          if (mine === seq) check = r?.ok ? { state: 'ok', value: r.value ?? val, reason: '' } : { state: 'bad', value: '', reason: r?.reason ?? 'Not allowed.' };
        })
        .catch((e) => mine === seq && (check = { state: 'bad', value: '', reason: errText(e) }));
    }, CHECK_MS);
    return () => clearTimeout(t);
  });

  const canAdd = $derived(!!form && !form.busy && check.state === 'ok');

  async function add() {
    const f = form;
    if (!canAdd) return;
    form.busy = true;
    form.error = '';
    const target = scopes.find((s) => s.file === f.scope);
    try {
      const r = await invoke('sandbox_add', {
        ...scopeArgs(f.scope),
        kind: f.kind,
        value: needsValue(f.kind) ? value : null,
        role: f.role || null,
        note: f.note.trim() || null,
      });
      form = null;
      const block = target ?? { file: f.scope };
      if (r?.created === false) note = { ...note, [block.file]: 'Already allowed here.' };
      else note = { ...note, [block.file]: '' };
      applied(block, r);
      flash([rowKey(block, r?.entry?.id)]);
      await load();
    } catch (e) {
      form.busy = false;
      form.error = `Could not add it: ${reason(e)}.`;
    }
  }

  // ------------------------------------------------------------ remove
  async function remove(f, e, index) {
    const key = rowKey(f, e.id);
    rowErr = { ...rowErr, [key]: '' };
    try {
      const r = await invoke('sandbox_remove', { id: e.id, file: f.file });
      ghosts = { ...ghosts, [f.file]: [...(ghosts[f.file] ?? []), { ...e, index }] };
      later(() => (ghosts = { ...ghosts, [f.file]: (ghosts[f.file] ?? []).filter((g) => g.id !== e.id) }), UNDO_MS);
      applied(f, r);
      await load();
    } catch (err) {
      rowErr = { ...rowErr, [key]: `Could not remove it: ${reason(err)}.` };
    }
  }
  async function undo(f, g) {
    const key = rowKey(f, g.id);
    try {
      const r = await invoke('sandbox_add', { ...scopeArgs(f.file), kind: g.kind, value: g.value ?? null, role: g.role ?? null, note: g.note ?? null });
      ghosts = { ...ghosts, [f.file]: (ghosts[f.file] ?? []).filter((x) => x.id !== g.id) };
      applied(f, r);
      flash([rowKey(f, r?.entry?.id)]);
      await load();
    } catch (err) {
      rowErr = { ...rowErr, [key]: `Could not undo it: ${reason(err)}.` };
    }
  }
  /** A group's rows with the removed ones put back, as Undo lines, where they were. */
  function withGhosts(f, g) {
    const rows = g.rows.map((r) => ({ r }));
    for (const x of (ghosts[f.file] ?? []).filter((x) => x.kind === g.kind)) rows.splice(Math.min(x.index, rows.length), 0, { ghost: x });
    return rows;
  }
  // A group with only removed rows is still shown for its Undo line.
  const shownGroups = (f) => {
    const gs = groups(f, kinds);
    for (const x of ghosts[f.file] ?? []) if (!gs.some((g) => g.kind === x.kind)) gs.push({ kind: x.kind, title: KINDS[x.kind]?.group ?? x.kind, rows: [] });
    return gs.sort((a, b) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind));
  };
  const entryIndex = (g, r) => g.rows.indexOf(r);
</script>

<section class="sb" aria-label="Sandbox settings">
  <div class="h">
    <h2>Sandbox</h2>
    <p class="muted sub">What an agent's shell commands can reach: internet hosts, folders, local ports, sockets.</p>
  </div>

  <div class="explainer" data-explainer>
    <p>Permissions decide which calls need approval. The sandbox decides what an agent's shell commands can reach. A rule can't widen the sandbox.</p>
    <p class="links">
      <button class="link" aria-expanded={limits} onclick={() => (limits = !limits)}>What can't be allowed</button>
      <span class="muted">·</span>
      <span class="muted">Want fewer approval prompts instead? That's a permission rule.</span>
      <button class="link" onclick={() => openSettings('permissions')}>Open Permissions settings</button>
    </p>
    {#if limits}
      <div class="limits" data-limits>
        <p>Some limits stay, whatever you allow:</p>
        <ul>
          <li>Credentials, the app's own data, the main checkout and other agents' worktrees can't be read or written.</li>
          <li>Folders that hold code run outside the sandbox (for example <code>.git</code>, <code>.claude</code>, hooks) can't be written, nor <code>/</code>, your home folder itself or system folders.</li>
          <li>"All hosts" (<code>*</code>) can't be allowed.</li>
          <li>Commands can't run outside the sandbox.</li>
          <li>Commits, pushes, pull requests and GitHub issues go through the orchestrator (<code>mcp__orch__commit</code>, <code>request_pr</code>, <code>read_issue</code>, <code>update_issue</code>). Allowing <code>api.github.com</code> only gives unauthenticated access, without your GitHub login.</li>
        </ul>
      </div>
    {/if}
  </div>

  {#if loadError}
    <p class="err" role="alert">Could not read the sandbox settings: {loadError} <button class="btn" onclick={load}>Retry</button></p>
  {:else if !scopes}
    <p class="muted" role="status">Loading…</p>
  {:else}
    {#if !kinds.length}<p class="muted" data-no-kinds>This version of Claude Code can't widen the sandbox.</p>{/if}
    {#each blocks as f (f.file)}
      {@const gs = shownGroups(f)}
      {@const broken = !!f.error}
      <div class="file" class:broken data-sandbox-file={f.file}>
        <div class="line">
          <div class="what">
            <span class="lab">{nameOf(f)}</span>
            <span class="muted path" title={f.path}>{f.exists ? midCut(f.path, 46) : 'no policy file yet'}</span>
            {#if f.error}<span class="err" role="alert" data-sandbox-error>Parse error at line {f.error.line}: {f.error.message}. Using the last good version.</span>{/if}
          </div>
          <button class="btn" onclick={() => openPolicy(f.scope, f.project ?? null)}>{f.exists ? 'Open in editor' : 'Create and open'}</button>
        </div>

        {#if f.warnings?.length}
          <ul class="warns">
            {#each more.has(f.file) ? f.warnings : f.warnings.slice(0, WARNINGS_SHOWN) as w, i (`${w.id}:${i}`)}
              <li class="warn">Allowance {w.id}: {w.message}. Skipped.</li>
            {/each}
          </ul>
          {#if f.warnings.length > WARNINGS_SHOWN && !more.has(f.file)}
            <button class="link" onclick={() => (more = new Set([...more, f.file]))}>Show {f.warnings.length - WARNINGS_SHOWN} more</button>
          {/if}
          <p class="muted help">Edit the file to fix it.</p>
        {/if}

        {#if f.scope === 'global' && f.defaults?.length}<p class="muted help">Every agent gets these.</p>{/if}

        {#each gs as g (g.kind)}
          <div class="group" data-group={g.kind}>
            <h4>{g.title}</h4>
            {#each withGhosts(f, g) as x (x.ghost ? `g:${x.ghost.id}` : x.r.builtin ? `b:${x.r.source}:${x.r.value}` : x.r.id)}
              {#if x.ghost}
                <div class="item removed" data-removed={x.ghost.id}>
                  <span class="muted">Removed {entryText(x.ghost)}</span>
                  <button class="link" aria-label="Undo removing {entryText(x.ghost)}" onclick={() => undo(f, x.ghost)}>Undo</button>
                  {#if rowErr[rowKey(f, x.ghost.id)]}<span class="err" role="alert">{rowErr[rowKey(f, x.ghost.id)]}</span>{/if}
                </div>
              {:else}
                {@const r = x.r}
                <div class="item" class:builtin={r.builtin} class:hl={highlight.has(rowKey(f, r.id))} data-row={r.builtin ? 'builtin' : r.id}>
                  <span class="val" class:mono={r.kind !== 'local_ports'}>{entryText(r)}</span>
                  {#if r.builtin}
                    <span class="chip outline">built in{r.role ? ` · ${r.role}` : ''}</span>
                  {:else if r.role}
                    <span class="chip">{r.role} only</span>
                  {/if}
                  {#if r.note}<span class="muted note" title={r.note}>{r.note}</span>{/if}
                  <span class="grow"></span>
                  {#if !r.builtin}
                    <button class="btn ghost" aria-label="Remove {entryText(r)}" disabled={broken} title={broken ? fixFirst(f) : undefined} onclick={() => remove(f, r, entryIndex(g, r))}>Remove</button>
                  {/if}
                </div>
                {#if rowErr[rowKey(f, r.id)]}<p class="err help" role="alert">{rowErr[rowKey(f, r.id)]}</p>{/if}
              {/if}
            {/each}
          </div>
        {/each}

        {#if !f.entries?.length && !(ghosts[f.file] ?? []).length}
          <p class="muted help" data-empty>{f.scope === 'global' ? (f.defaults?.length ? 'Nothing added for all projects.' : 'Nothing extra: agents get the built-in hosts only.') : 'Nothing extra: agents get the built-in hosts only.'}</p>
        {/if}
        {#if f.scope === 'global' && blocks.length === 1}<p class="muted help">No projects yet. Add one to give it its own allowances.</p>{/if}
        {#if note[f.file]}<p class="muted help" role="status">{note[f.file]}</p>{/if}

        {#if form && form.file === f.file}
          {@const k = KINDS[form.kind]}
          <form class="form" onsubmit={(e) => (e.preventDefault(), add())} aria-label="Add an allowance">
            <label class="fld"><span class="lab">What to allow</span>
              <Select value={form.kind} options={kindOptions} aria-label="What to allow" onchange={(v) => ((form.kind = v), (form.value = ''))} />
            </label>
            {#if EXAMPLES[form.kind] || form.kind === 'local_ports'}
              <div class="chips" data-examples>
                {#each EXAMPLES[form.kind] ?? [] as ex (ex)}
                  <button type="button" class="chip btnchip" onclick={() => ((form.value = ex), (form.touched = true))}>{ex}</button>
                {/each}
                {#if form.kind === 'local_ports'}<button type="button" class="chip btnchip" onclick={() => (form.kind = 'local_ports')}>Allow local ports</button>{/if}
              </div>
            {/if}
            {#if k.field}
              <label class="fld"><span class="lab">{k.field}</span>
                <input class="field mono" bind:value={form.value} placeholder={k.placeholder} aria-invalid={check.state === 'bad' || (form.touched && !value) ? 'true' : undefined} oninput={() => (form.touched = true)} />
              </label>
              <p class="muted help">{k.help}</p>
            {:else}
              <p class="muted help">{k.help}</p>
            {/if}
            {#if form.touched && !value && k.empty}<p class="err help" role="alert">{k.empty}</p>{/if}
            {#if check.state === 'checking'}<p class="muted help" role="status">Checking…</p>
            {:else if check.state === 'bad'}<p class="err help" role="alert">{check.reason}</p>
            {:else if check.state === 'ok' && k.field && check.value !== value}<p class="muted help" role="status">Will be saved as <code>{check.value}</code></p>
            {:else if check.state === 'ok'}<p class="ok help" role="status" aria-label="Valid">✓</p>{/if}
            <label class="fld"><span class="lab">Applies to</span>
              <Select value={form.scope} options={scopeOptions} aria-label="Applies to" onchange={(v) => ((form.scope = v), (form.role = ''))} />
            </label>
            {#if (roles[form.scope] ?? []).length}
              <label class="fld"><span class="lab">Only for role</span>
                <Select value={form.role} options={roleOptions} aria-label="Only for role" onchange={(v) => (form.role = v)} />
              </label>
              <p class="muted help">Leave on Any role unless only one role needs it.</p>
            {/if}
            <label class="fld"><span class="lab">Why</span>
              <input class="field" bind:value={form.note} placeholder="Optional, for whoever reads the file" />
            </label>
            {#if form.kind === 'network' && isGithub(value)}<p class="muted help" data-github>{GITHUB_LINE}</p>{/if}
            {#if form.error}<p class="err" role="alert">{form.error}</p>{/if}
            <div class="acts">
              <button class="btn primary" type="submit" disabled={!canAdd}>Add</button>
              <button class="btn" type="button" onclick={() => (form = null)}>Cancel</button>
            </div>
            <p class="muted help">Permissions decide which calls need approval. The sandbox decides what an agent's shell commands can reach. A rule can't widen the sandbox.</p>
          </form>
        {:else if kinds.length}
          <div class="acts"><button class="btn" disabled={broken} title={broken ? fixFirst(f) : undefined} onclick={() => openForm(f)}>Add</button></div>
        {/if}
        {#if applies[f.file]}<p class="muted help" role="status" data-applies>{applies[f.file]}</p>{/if}
      </div>
    {/each}
  {/if}
</section>

<style>
  .sb {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    max-width: 720px;
  }
  h2,
  h4 {
    margin: 0;
    font: var(--text-heading);
  }
  h4 {
    font: var(--text-label);
    color: var(--color-text-muted);
  }
  .sub,
  .help {
    margin: 0;
    font: var(--text-caption);
  }
  .explainer {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-raised);
    border-left: 3px solid var(--color-decision-sandbox);
    border-radius: var(--radius-sm);
    font: var(--text-caption);
  }
  .explainer p,
  .limits p {
    margin: 0;
  }
  .links {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }
  .limits ul {
    margin: var(--space-xs) 0 0;
    padding-left: var(--space-lg);
  }
  .file {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .file.broken {
    border-color: var(--color-status-error);
  }
  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-lg);
  }
  .what {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font: var(--text-caption);
  }
  .lab {
    font: var(--text-body-strong);
  }
  .path {
    overflow-wrap: anywhere;
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: 30px;
    padding: 0 var(--space-xs);
    border: var(--border-width-default) solid transparent;
    border-radius: var(--radius-sm);
  }
  .item.hl {
    background: color-mix(in srgb, var(--color-accent-default) 12%, transparent);
    border-color: var(--color-accent-default);
  }
  .item.builtin .val {
    color: var(--color-text-muted);
  }
  .val.mono,
  .mono {
    font: var(--text-mono);
  }
  .note {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: var(--text-caption);
  }
  .grow {
    flex: 1;
  }
  .chip {
    flex: none;
    padding: 0 var(--space-xs);
    color: var(--color-accent-default);
    border: var(--border-width-default) solid var(--color-accent-default);
    border-radius: var(--radius-full);
    font: var(--text-caption);
  }
  .chip.outline {
    color: var(--color-text-disabled);
    border-color: var(--color-border-strong);
  }
  .btnchip {
    background: none;
    cursor: pointer;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }
  .warns {
    margin: 0;
    padding-left: var(--space-lg);
    font: var(--text-caption);
  }
  .warn {
    color: var(--color-status-waiting-for-you);
  }
  .err {
    color: var(--color-status-error);
  }
  .ok {
    color: var(--color-status-done);
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-md);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-accent-default);
    border-radius: var(--radius-md);
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
  }
  .acts {
    display: flex;
    gap: var(--space-sm);
  }
  .field[aria-invalid='true'] {
    border-color: var(--color-status-error);
  }
</style>
