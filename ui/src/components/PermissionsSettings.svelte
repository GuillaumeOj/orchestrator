<script>
  // Settings → Permissions (P4.4 notes §9.2, api-contract §3.3, §5): the policy
  // files in effect with Open in editor, a parse error or the warnings of each,
  // the default effects for team roles and solo sessions, and the hold length.
  // Rules (allow-fine-tune-sandbox-permissions, ux-flows §4): the rules of each file with Remove (and
  // Undo), the Add rule form (the core builds the pattern: `rule_preview`) and Test a call (`policy_test`).
  import { onMount } from 'svelte';
  import { app, openPolicy, openSettings, projects } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { onTopics } from '../lib/now.svelte.js';
  import { overwatchOf } from '../lib/overwatch.js';
  import {
    EFFECT_HELP,
    EFFECT_LABEL,
    fileName,
    fileRows,
    fileTitle,
    midCut,
    EFFECT_OPTIONS as RULE_EFFECTS,
    TOOL_FIELDS,
    TOOL_OPTIONS,
    testResult,
    WARNINGS_SHOWN,
  } from '../lib/policy.js';
  import { fixFirst, reason } from '../lib/sandbox.js';
  import Select from './Select.svelte';

  const EFFECTS = [
    { id: 'overwatch', label: 'The Overwatch decides' },
    { id: 'user', label: 'Ask me' },
  ];

  const EFFECT_OPTIONS = EFFECTS.map((x) => ({ value: x.id, label: x.label }));
  let rev = $state(0); // a refused save remounts the pickers on the value still applied

  let files = $state(/** @type {any[] | null} */ (null));
  let loadError = $state('');
  let defaults = $state(/** @type {{ team: string, solo: string } | null} */ (null));
  let hold = $state(/** @type {number | null} */ (null));
  let errors = $state(/** @type {Record<string, string>} */ ({}));
  let more = $state(/** @type {Set<string>} */ (new Set()));
  const ow = $derived(overwatchOf(app.now));
  const rows = $derived(fileRows(files ?? [], projects()));

  function load() {
    invoke('policy_files')
      .then((v) => ((files = v), (loadError = '')))
      .catch((e) => (loadError = errText(e)));
  }
  $effect(() => {
    invoke('policy_defaults')
      .then((v) => (defaults = v))
      .catch((e) => (errors = { ...errors, defaults: `Could not read the defaults: ${errText(e)}` }));
    invoke('settings_get', { key: 'permissions.hold_secs' })
      .then((v) => (hold = Number.isInteger(v) && v >= 0 ? v : 300))
      .catch(() => (hold = 300));
  });
  // A file edited in the editor, or a rule added from a card, changes its rules and errors: read them
  // again on the `policy` topic and when the window returns.
  onMount(() => onTopics(['policy'], () => Promise.all([load(), loadRules()])));

  // ------------------------------------------------------------ rules
  const UNDO_MS = 6000;
  let rules = $state(/** @type {any[] | null} */ (null));
  let rulesError = $state('');
  let ghosts = $state(/** @type {Record<string, any[]>} */ ({}));
  let rowErr = $state(/** @type {Record<string, string>} */ ({}));
  let highlight = $state(/** @type {Set<string>} */ (new Set()));
  const timers = new Set();
  const later = (fn, ms) => {
    const t = setTimeout(() => (timers.delete(t), fn()), ms);
    timers.add(t);
  };
  onMount(() => () => timers.forEach(clearTimeout));
  const key = (r) => `${r.file}:${r.id}`;
  const rulesOf = (f) => (rules ?? []).filter((r) => r.file === f.file);
  const scopeOf = (file) => (file === 'global' ? { scope: 'global', scope_id: null } : { scope: 'project', scope_id: file.replace(/^project:/, '') });

  async function loadRules() {
    try {
      rules = await invoke('policy_rules', {});
      rulesError = '';
    } catch (e) {
      rulesError = errText(e);
    }
  }

  async function removeRule(r, index) {
    rowErr = { ...rowErr, [key(r)]: '' };
    try {
      await invoke('revoke_rule', { id: r.id, file: r.file });
      ghosts = { ...ghosts, [r.file]: [...(ghosts[r.file] ?? []), { ...r, index }] };
      later(() => (ghosts = { ...ghosts, [r.file]: (ghosts[r.file] ?? []).filter((g) => g.id !== r.id) }), UNDO_MS);
      await Promise.all([loadRules(), load()]);
    } catch (e) {
      rowErr = { ...rowErr, [key(r)]: `Could not remove it: ${reason(e)}.` };
    }
  }
  async function undoRule(g) {
    try {
      const sc = g.agent ? { scope: 'agent', scope_id: g.agent } : scopeOf(g.file);
      const r = await invoke('add_rule', { ...sc, pattern: g.match, effect: g.effect, role: g.role ?? null, note: g.note ?? null });
      ghosts = { ...ghosts, [g.file]: (ghosts[g.file] ?? []).filter((x) => x.id !== g.id) };
      flash(`${g.file}:${r?.rule?.id}`);
      await Promise.all([loadRules(), load()]);
    } catch (e) {
      rowErr = { ...rowErr, [key(g)]: `Could not undo it: ${reason(e)}.` };
    }
  }
  function flash(k) {
    highlight = new Set([...highlight, k]);
    later(() => (highlight = new Set([...highlight].filter((x) => x !== k))), 2000);
  }
  /** A file's rules with the removed ones back in place, as Undo lines. */
  function listed(f) {
    /** @type {{ r?: any, ghost?: any }[]} */
    const rows = rulesOf(f).map((r) => ({ r }));
    for (const g of ghosts[f.file] ?? []) rows.splice(Math.min(g.index, rows.length), 0, { ghost: g });
    return rows;
  }

  // ------------------------------------------------------------ add rule
  let form = $state(/** @type {any} */ (null));
  let preview = $state({ state: 'idle', pattern: '', summary: '', error: '' });
  let roleList = $state(/** @type {Record<string, { id: string, name: string }[]>} */ ({}));
  const scopeOptions = $derived([{ value: 'global', label: 'All projects' }, ...projects().map((p) => ({ value: `project:${p.id}`, label: p.name }))]);
  const rolesFor = (project) => roleList[project ?? ''] ?? [];
  const roleOptionsOf = (project) => [{ value: '', label: 'Any role' }, ...rolesFor(project).map((r) => ({ value: r.id, label: r.name ?? r.id }))];
  function wantRoles(project) {
    const k = project ?? '';
    if (roleList[k]) return;
    roleList = { ...roleList, [k]: [] };
    invoke('teams_view', { project: project ?? null })
      .then((v) => (roleList = { ...roleList, [k]: v?.roles ?? [] }))
      .catch(() => {});
  }
  const projectOf = (scope) => (scope === 'global' ? null : scope.replace(/^project:/, ''));

  function openForm(f) {
    form = { file: f.file, tool: 'bash', value: '', effect: 'allow', scope: f.file, role: '', note: '', sure: false, busy: false, error: '' };
    preview = { state: 'idle', pattern: '', summary: '', error: '' };
  }
  $effect(() => {
    if (form) wantRoles(projectOf(form.scope));
  });

  // The live preview: the core builds the pattern, 300 ms after the last keystroke.
  let seq = 0;
  $effect(() => {
    if (!form) return;
    const { tool } = form;
    const val = form.value.trim();
    const mine = ++seq;
    preview = { state: 'idle', pattern: '', summary: '', error: '' };
    if (TOOL_FIELDS[tool] && !val && tool !== 'mcp') return;
    const t = setTimeout(() => {
      preview = { state: 'checking', pattern: '', summary: '', error: '' };
      invoke('rule_preview', { tool, value: TOOL_FIELDS[tool] ? val : null })
        .then((r) => mine === seq && (preview = { state: 'ok', pattern: r.pattern, summary: r.summary, error: '' }))
        .catch((e) => mine === seq && (preview = { state: 'bad', pattern: '', summary: '', error: errText(e) }));
    }, 300);
    return () => clearTimeout(t);
  });
  const everything = $derived(!!form && form.tool === 'any' && form.effect === 'allow');
  const canAdd = $derived(!!form && !form.busy && preview.state === 'ok' && (!everything || form.sure));

  async function addRule() {
    if (!canAdd) return;
    const f = form;
    form.busy = true;
    form.error = '';
    try {
      const r = await invoke('add_rule', {
        ...scopeOf(f.scope),
        pattern: preview.pattern,
        effect: f.effect,
        role: f.role || null,
        note: f.note.trim() || null,
      });
      form = null;
      flash(`${f.scope}:${r?.rule?.id}`);
      await Promise.all([loadRules(), load()]);
    } catch (e) {
      if (form) {
        form.busy = false;
        form.error = `Could not add the rule: ${reason(e)}.`;
      }
    }
  }

  // ------------------------------------------------------------ test a call
  let test = $state({ role: '', tool: 'bash', value: '', project: projects()[0]?.id ?? '' });
  let testOut = $state(/** @type {any} */ (null));
  $effect(() => wantRoles(test.project || null));
  const testRoles = $derived([
    { value: '', label: 'Any role' },
    ...rolesFor(test.project || null).map((r) => ({ value: r.id, label: r.name ?? r.id })),
    { value: '__solo', label: 'Solo' },
  ]);
  const projectChoices = $derived([{ value: '', label: 'None' }, ...projects().map((p) => ({ value: p.id, label: p.name }))]);
  let testing = $state(false);
  async function runTest() {
    if (!test.value.trim() || testing) return;
    testing = true;
    try {
      const r = await invoke('policy_test', {
        project: test.project || null,
        role: test.role && test.role !== '__solo' ? test.role : null,
        solo: test.role === '__solo',
        tool: test.tool,
        value: test.value.trim(),
      });
      testOut = { ok: true, ...testResult(r, { rules: rules ?? [], projects: projects(), solo: test.role === '__solo', tool: test.tool, value: test.value }) };
    } catch (e) {
      testOut = { ok: false, error: `Could not test the call: ${reason(e)}.` };
    } finally {
      testing = false;
    }
  }
  function jumpTo(file, id) {
    document.querySelector(`[data-rule="${file}:${id}"]`)?.scrollIntoView({ block: 'nearest' });
    flash(`${file}:${id}`);
  }

  async function setDefault(which, value) {
    const before = defaults?.[which];
    errors = { ...errors, defaults: '' };
    try {
      defaults = await invoke('set_policy_defaults', { ...defaults, [which]: value });
    } catch (err) {
      rev++;
      errors = { ...errors, defaults: `${errText(err)} Still using “${EFFECTS.find((x) => x.id === before)?.label ?? before}”.` };
    }
  }

  async function setHold(e) {
    const box = e.currentTarget;
    const v = box.value.trim() === '' ? Number.NaN : Number(box.value);
    const before = hold;
    const still = ` Still using ${before} s.`;
    if (!Number.isInteger(v) || v < 0) {
      errors = { ...errors, hold: `A whole number of seconds.${still}` };
      box.value = String(before);
      return;
    }
    try {
      // The Overwatch's permission time stays below the hold (api-contract §5).
      const s = await invoke('overwatch_settings').catch(() => null);
      if (v > 0 && s && v <= s.permission_secs) {
        errors = { ...errors, hold: `Must be above the Overwatch's permission time (${s.permission_secs} s).${still}` };
        box.value = String(before);
        return;
      }
      await invoke('settings_set', { key: 'permissions.hold_secs', value: v });
      hold = v;
      errors = { ...errors, hold: '' };
    } catch (err) {
      errors = { ...errors, hold: `${errText(err)}${still}` };
      box.value = String(before);
    }
  }
</script>

<section class="perm" aria-label="Permissions settings">
  <div class="h">
    <h2>Permissions</h2>
    <p class="muted sub">Which permissions are settled without you.</p>
  </div>
  <div class="explainer" data-explainer>
    <p>Permissions decide which calls need approval. The sandbox decides what an agent's shell commands can reach. A rule can't widen the sandbox.</p>
    <p>A command fails with "Operation not permitted", or a host is blocked? That's the sandbox, not a permission. <button class="link" onclick={() => openSettings('sandbox')}>Open Sandbox settings</button></p>
  </div>

  <h3>Policy files</h3>
  {#if loadError}
    <p class="err" role="alert">Could not read the policy files: {loadError}</p>
  {:else if !files}
    <p class="muted" role="status">Loading…</p>
  {:else}
    {#each rows as f (f.key)}
      <div class="file" data-policy-file={f.scope === 'global' ? 'global' : f.project}>
        <div class="line">
          <div class="what">
            <span class="lab">{fileName(f)}</span>
            <span class="muted path" title={f.path}>{f.exists ? midCut(f.path, 46) : 'no policy file yet'}</span>
            {#if f.error}
              <span class="err" role="alert" data-policy-error>Parse error at line {f.error.line}: {f.error.message}. Using the last good version.</span>
            {:else}
              <span class="muted">{f.exists ? fileTitle(f) : 'created with your first rule'}</span>
            {/if}
          </div>
          <button class="btn" onclick={() => openPolicy(f.scope, f.project ?? null)}>{f.exists ? 'Open in editor' : 'Create and open'}</button>
        </div>
        {#if f.warnings?.length}
          <ul class="warns">
            {#each more.has(f.key) ? f.warnings : f.warnings.slice(0, WARNINGS_SHOWN) as w, i (`${w.id}:${i}`)}
              <li class="warn">Rule {w.id}: {w.message}</li>
            {/each}
          </ul>
          {#if f.warnings.length > WARNINGS_SHOWN && !more.has(f.key)}
            <button class="link" onclick={() => (more = new Set([...more, f.key]))}>Show {f.warnings.length - WARNINGS_SHOWN} more</button>
          {/if}
        {/if}
      </div>
    {/each}
    {#if rows.length === 1}<p class="muted">No projects yet. Policy files appear when you add one.</p>{/if}
  {/if}

  <h3>Rules</h3>
  {#if rulesError}
    <p class="err" role="alert">Could not read the rules: {rulesError} <button class="btn" onclick={loadRules}>Retry</button></p>
  {:else if !rules || !files}
    <p class="muted" role="status">Loading…</p>
  {:else}
    {#each rows as f (f.key)}
      {@const list = listed(f)}
      <div class="file" data-rules-file={f.file}>
        <div class="line">
          <div class="what">
            <span class="lab">{fileName(f)}</span>
            {#if f.error}<span class="err" role="alert">Parse error at line {f.error.line}: {f.error.message}. Using the last good version.</span>{/if}
          </div>
          <button class="btn" disabled={!!f.error} title={f.error ? fixFirst(f) : undefined} onclick={() => openForm(f)}>Add rule</button>
        </div>
        {#if !list.length}<p class="muted help" data-no-rules>No rules here. Anything not matched follows the defaults below.</p>{/if}
        {#each list as x, i (x.ghost ? `g:${x.ghost.id}` : x.r.id)}
          {#if x.ghost}
            <div class="rule removed" data-removed={x.ghost.id}>
              <span class="muted">Removed {x.ghost.summary} ·</span>
              <button class="link" aria-label="Undo removing {x.ghost.summary}" onclick={() => undoRule(x.ghost)}>Undo</button>
              {#if rowErr[key(x.ghost)]}<span class="err" role="alert">{rowErr[key(x.ghost)]}</span>{/if}
            </div>
          {:else}
            {@const r = x.r}
            <div class="rule" class:hl={highlight.has(key(r))} data-rule={key(r)}>
              <div class="top">
                <span class="sentence">{r.summary}</span>
                <span class="chip e-{r.effect}">{EFFECT_LABEL[r.effect] ?? r.effect}</span>
                {#if r.role || r.agent}<span class="chip">{r.role ?? r.agent} only</span>{/if}
                {#if r.note}<span class="muted note" title={r.note}>{r.note}</span>{/if}
                <span class="grow"></span>
                <span class="muted id">{r.id}</span>
                <button class="btn ghost" aria-label="Remove {r.summary}" disabled={!!f.error} title={f.error ? fixFirst(f) : undefined} onclick={() => removeRule(r, i)}>Remove</button>
              </div>
              <code class="match muted">{r.match}</code>
              {#if rowErr[key(r)]}<p class="err help" role="alert">{rowErr[key(r)]}</p>{/if}
            </div>
          {/if}
        {/each}
        {#if form && form.file === f.file}
          {@const tf = TOOL_FIELDS[form.tool]}
          <form class="form" onsubmit={(e) => (e.preventDefault(), addRule())} aria-label="Add a rule">
            <label class="fld"><span class="lab">When an agent wants to…</span>
              <Select value={form.tool} options={TOOL_OPTIONS} aria-label="When an agent wants to" onchange={(v) => ((form.tool = v), (form.value = ''), (form.sure = false))} />
            </label>
            {#if tf}
              <label class="fld"><span class="lab">{tf.label}</span>
                <input class="field mono" bind:value={form.value} placeholder={tf.placeholder} />
              </label>
              <p class="muted help">{tf.help}</p>
            {/if}
            {#if preview.state === 'checking'}<p class="muted help" role="status">Checking…</p>
            {:else if preview.state === 'bad'}<p class="err help" role="alert">{preview.error}</p>
            {:else if preview.state === 'ok'}
              <div class="preview" role="status" data-preview>
                <span class="muted help">This rule matches</span>
                <code>{preview.pattern}</code>
                <span class="help">{preview.summary}</span>
              </div>
            {/if}
            <label class="fld"><span class="lab">…then</span>
              <Select value={form.effect} options={RULE_EFFECTS} aria-label="Then" onchange={(v) => ((form.effect = v), (form.sure = false))} />
            </label>
            <p class="muted help">{EFFECT_HELP[form.effect]}</p>
            {#if everything}
              <p class="warn help" role="alert">This allows every call.</p>
              <label class="check"><input type="checkbox" bind:checked={form.sure} /> I understand</label>
            {/if}
            <label class="fld"><span class="lab">Applies to</span>
              <Select value={form.scope} options={scopeOptions} aria-label="Applies to" onchange={(v) => ((form.scope = v), (form.role = ''))} />
            </label>
            {#if rolesFor(projectOf(form.scope)).length}
              <label class="fld"><span class="lab">Only for role</span>
                <Select value={form.role} options={roleOptionsOf(projectOf(form.scope))} aria-label="Only for role" onchange={(v) => (form.role = v)} />
              </label>
            {/if}
            <label class="fld"><span class="lab">Why</span>
              <input class="field" bind:value={form.note} placeholder="Optional" />
            </label>
            {#if form.error}<p class="err" role="alert">{form.error}</p>{/if}
            <div class="acts">
              <button class="btn primary" type="submit" disabled={!canAdd}>Add rule</button>
              <button class="btn" type="button" onclick={() => (form = null)}>Cancel</button>
            </div>
          </form>
        {/if}
      </div>
    {/each}
  {/if}

  <h3>Test a call</h3>
  <div class="file" data-test>
    <p class="muted help">Check what would happen, without running anything.</p>
    {#key test.tool}
      <form class="testrow" onsubmit={(e) => (e.preventDefault(), runTest())}>
        <label class="fld"><span class="lab">Role</span><Select value={test.role} options={testRoles} aria-label="Role" onchange={(v) => (test.role = v)} /></label>
        <label class="fld"><span class="lab">Tool</span><Select value={test.tool} options={TOOL_OPTIONS.filter((o) => o.value !== 'any')} aria-label="Tool" onchange={(v) => ((test.tool = v), (test.value = ''), (testOut = null))} /></label>
        <label class="fld grow"><span class="lab">{TOOL_FIELDS[test.tool].test.label}</span><input class="field mono" bind:value={test.value} placeholder={TOOL_FIELDS[test.tool].test.placeholder} /></label>
        <label class="fld"><span class="lab">Project</span><Select value={test.project} options={projectChoices} aria-label="Project" onchange={(v) => (test.project = v)} /></label>
        <button class="btn primary" type="submit" disabled={!test.value.trim() || testing}>Test</button>
      </form>
    {/key}
    {#if testOut?.ok}
      <div class="result" role="status" data-test-result>
        <p><strong>{testOut.head}</strong> · {testOut.rest}</p>
        {#if testOut.rule}<button class="link" onclick={() => jumpTo(testOut.rule.file, testOut.rule.id)}>Show rule {testOut.rule.id}</button>{/if}
        {#if testOut.hint}<p class="muted help">Passing this test doesn't mean the command can reach it. Check the sandbox: <button class="link" onclick={() => openSettings('sandbox')}>Open Sandbox settings</button>.</p>{/if}
      </div>
    {:else if testOut}
      <p class="err" role="alert">{testOut.error}</p>
    {/if}
  </div>

  <h3>When no rule matches</h3>
  {#each [['team', 'Team roles'], ['solo', 'Solo sessions']] as [which, label] (which)}
    <div class="row" data-row="default-{which}">
      <div class="what">
        <label class="lab" for="perm-{which}">{label}</label>
        {#if which === 'team'}<p class="muted help">When no rule matches a permission. The Overwatch's decisions are lines you can undo; Ask me makes a card.</p>{/if}
      </div>
      {#key rev}<Select id="perm-{which}" value={defaults?.[which] ?? 'user'} options={EFFECT_OPTIONS} disabled={!defaults} onchange={(v) => setDefault(which, v)} />{/key}
    </div>
  {/each}
  {#if ow.status === 'off'}<p class="muted help" data-ow-off>The Overwatch is off: every unmatched permission asks you.</p>{/if}
  {#if errors.defaults}<p class="err help" role="alert">{errors.defaults}</p>{/if}

  <h3>Hold</h3>
  <div class="row" data-row="hold">
    <div class="what">
      <label class="lab" for="perm-hold">Hold a permission for</label>
      <p class="muted help">How long a permission waits for you before the agent parks. 0 does not hold: the prompt stays in the agent's pane.</p>
      {#if errors.hold}<p class="err help" role="alert">{errors.hold}</p>{/if}
    </div>
    <span class="num">
      <input id="perm-hold" class="field" type="number" min="0" value={hold ?? ''} disabled={hold == null} aria-invalid={errors.hold ? 'true' : undefined} onchange={setHold} />
      <span class="muted">s</span>
    </span>
  </div>
  <p class="muted help">Rules are also added from cards and removed from chat lines. The files are plain text.</p>
</section>

<style>
  .perm {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    max-width: 720px;
  }
  h2,
  h3 {
    margin: 0;
    font: var(--text-heading);
  }
  h3 {
    margin-top: var(--space-md);
    font: var(--text-body-strong);
  }
  .sub,
  .help {
    margin: 0;
    font: var(--text-caption);
  }
  .file,
  .row {
    display: flex;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .file {
    flex-direction: column;
  }
  .row,
  .line {
    align-items: center;
    justify-content: space-between;
    gap: var(--space-lg);
  }
  .line {
    display: flex;
  }
  .what {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font: var(--text-caption);
  }
  .row .what {
    gap: 0;
  }
  .lab {
    font: var(--text-body-strong);
  }
  .path {
    overflow-wrap: anywhere;
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
  .num {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-xs);
  }
  .num .field {
    width: 96px;
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
  .result p {
    margin: 0;
  }
  .rule {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    padding: var(--space-xs);
    border: var(--border-width-default) solid transparent;
    border-radius: var(--radius-sm);
  }
  .rule.hl {
    background: color-mix(in srgb, var(--color-accent-default) 12%, transparent);
    border-color: var(--color-accent-default);
  }
  .rule .top,
  .removed {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .match {
    font: var(--text-mono-sm);
  }
  .note {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: var(--text-caption);
  }
  .id {
    font: var(--text-caption);
    color: var(--color-text-disabled);
  }
  .chip {
    flex: none;
    padding: 0 var(--space-xs);
    border: var(--border-width-default) solid var(--color-accent-default);
    border-radius: var(--radius-full);
    color: var(--color-accent-default);
    font: var(--text-caption);
  }
  .chip.e-allow {
    color: var(--color-status-done);
    border-color: var(--color-status-done);
  }
  .chip.e-overwatch {
    color: var(--color-text-muted);
    border-color: var(--color-border-strong);
  }
  .chip.e-user {
    color: var(--color-decision-ask);
    border-color: var(--color-decision-ask);
  }
  .chip.e-deny {
    color: var(--color-status-error);
    border-color: var(--color-status-error);
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
    display: grid;
    grid-template-columns: 180px minmax(0, 1fr);
    align-items: center;
    gap: var(--space-sm);
  }
  .form .help {
    padding-left: calc(180px + var(--space-sm));
  }
  .acts,
  .check {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .testrow {
    display: flex;
    align-items: flex-end;
    flex-wrap: wrap;
    gap: var(--space-sm);
  }
  .preview {
    display: flex;
    flex-direction: column;
  }
  .mono {
    font: var(--text-mono);
  }
  .result {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
  }
  .field[aria-invalid='true'] {
    border-color: var(--color-status-error);
  }
</style>
