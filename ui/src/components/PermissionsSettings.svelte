<script>
  // Settings → Permissions (P4.4 notes §9.2, api-contract §3.3, §5): the policy
  // files in effect with Open in editor, a parse error or the warnings of each,
  // the default effects for team roles and solo sessions, and the hold length.
  // No rule editor and no rule list: rules are added from cards, removed from lines.
  import { app, openPolicy, projects } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { overwatchOf } from '../lib/overwatch.js';
  import { fileName, fileRows, fileTitle, midCut, WARNINGS_SHOWN } from '../lib/policy.js';

  const EFFECTS = [
    { id: 'overwatch', label: 'The Overwatch decides' },
    { id: 'user', label: 'Ask me' },
  ];

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
      .then((v) => (files = v))
      .catch((e) => (loadError = errText(e)));
  }
  $effect(() => {
    load();
    invoke('policy_defaults')
      .then((v) => (defaults = v))
      .catch((e) => (errors = { ...errors, defaults: `Could not read the defaults: ${errText(e)}` }));
    invoke('settings_get', { key: 'permissions.hold_secs' })
      .then((v) => (hold = Number.isInteger(v) && v >= 0 ? v : 300))
      .catch(() => (hold = 300));
  });
  // A file edited in the editor changes its rules and errors: read them again when the window returns.
  $effect(() => {
    const again = () => document.visibilityState === 'visible' && load();
    document.addEventListener('visibilitychange', again);
    return () => document.removeEventListener('visibilitychange', again);
  });

  async function setDefault(which, e) {
    const box = e.currentTarget;
    const before = defaults?.[which];
    errors = { ...errors, defaults: '' };
    try {
      defaults = await invoke('set_policy_defaults', { ...defaults, [which]: box.value });
    } catch (err) {
      box.value = before ?? 'user';
      errors = { ...errors, defaults: `${errText(err)} Still using “${EFFECTS.find((x) => x.id === before)?.label ?? before}”.` };
    }
  }

  async function setHold(e) {
    const box = e.currentTarget;
    const v = Number(box.value);
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

  <h3>When no rule matches</h3>
  {#each [['team', 'Team roles'], ['solo', 'Solo sessions']] as [which, label] (which)}
    <div class="row" data-row="default-{which}">
      <div class="what">
        <label class="lab" for="perm-{which}">{label}</label>
        {#if which === 'team'}<p class="muted help">When no rule matches a permission. The Overwatch's decisions are lines you can undo; Ask me makes a card.</p>{/if}
      </div>
      <select id="perm-{which}" class="field" value={defaults?.[which] ?? 'user'} disabled={!defaults} onchange={(e) => setDefault(which, e)}>
        {#each EFFECTS as x (x.id)}<option value={x.id}>{x.label}</option>{/each}
      </select>
    </div>
  {/each}
  {#if ow.status === 'off'}<p class="muted help" data-ow-off>The Overwatch is off: every unmatched permission asks you.</p>{/if}
  {#if errors.defaults}<p class="err help" role="alert">{errors.defaults}</p>{/if}

  <h3>Hold</h3>
  <div class="row" data-row="hold">
    <div class="what">
      <label class="lab" for="perm-hold">Hold a permission for</label>
      <p class="muted help">How long a permission waits for you before the agent parks. 0 holds it without limit.</p>
      {#if errors.hold}<p class="err help" role="alert">{errors.hold}</p>{/if}
    </div>
    <span class="num">
      <input id="perm-hold" class="field" type="number" min="0" value={hold ?? ''} disabled={hold == null} aria-invalid={errors.hold ? 'true' : undefined} onchange={setHold} />
      <span class="muted">s</span>
    </span>
  </div>
  <p class="muted help">Rules are added from cards and removed from their lines. The files are plain text.</p>
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
  .field[aria-invalid='true'] {
    border-color: var(--color-status-error);
  }
</style>
