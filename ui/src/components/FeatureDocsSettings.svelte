<script>
  // Project settings › Feature docs (change-how-specs-are-saved §3.1): where team runs save each
  // feature's spec and designs. Two rows (Docs, Designs), each a radio group; a folder template
  // under "In the repository", checked as the user types (the backend's rules) and again by the
  // backend on save (`feature_docs.<row>.path: <message>` goes under that field). Saved with
  // Save / ⌘⏎ like the other form sections, the rest of the project's settings untouched.
  import { onMount } from 'svelte';
  import { toast } from '../lib/app.svelte.js';
  import { draftErrors, fromDraft, parseFieldError, resolveSetting, sameDraft, slashed, toDraft } from '../lib/featuredocs.js';
  import { errText, invoke } from '../lib/ipc.js';
  import SettingsFooter from './SettingsFooter.svelte';

  let { project, name } = $props();
  let saved = $state(null); // the stored draft; null while loading
  let draft = $state(toDraft(null));
  let loadError = $state(null);
  let busy = $state(false);
  /** @type {{ docs?: string, designs?: string }} */
  let serverErr = $state({}); // the backend's message, until the field changes
  let root = $state(null);

  const EXAMPLE_SLUG = 'checkout';
  /** @type {{ docs?: string, designs?: string }} */
  const errors = $derived({ ...draftErrors(draft), ...Object.fromEntries(Object.entries(serverErr).filter(([, v]) => v)) });
  const invalid = $derived(Object.keys(errors).length > 0);
  const dirty = $derived(!!saved && !sameDraft(draft, saved));
  // The example only while the folder is valid: an error says what is wrong instead.
  const resolved = $derived(resolveSetting(fromDraft(draft), EXAMPLE_SLUG));
  const docsExample = $derived(errors.docs ? null : `→ ${slashed(resolved.docs.dir)}`);
  const designsExample = $derived.by(() => {
    if (draft.designs.kind === 'repo') return errors.designs ? null : `→ ${slashed(resolved.designs.dir)}`;
    if (draft.designs.kind !== 'with_docs') return null;
    if (draft.docs.kind === 'app') return '→ Not saved either';
    return errors.docs ? null : `→ ${slashed(resolved.designs.dir)}`;
  });

  async function load() {
    loadError = null;
    try {
      const r = await invoke('project_settings', { project });
      saved = toDraft(r.settings?.feature_docs);
      draft = toDraft(r.settings?.feature_docs);
    } catch (e) {
      loadError = errText(e);
    }
  }

  const edit = (row) => {
    // Typing again clears the backend's message for that field.
    if (serverErr[row]) serverErr = { ...serverErr, [row]: undefined };
  };
  /** Choosing another option also drops the backend's message: its path field is gone, so it could never be fixed. */
  const pick = (row, kind) => {
    draft[row].kind = kind;
    // A designs folder starts as a suggestion (the docs' `design/`), not an empty, failing field.
    if (row === 'designs' && kind === 'repo' && !draft.designs.path) draft.designs.path = draft.docs.kind === 'repo' && !errors.docs ? `${draft.docs.path.trim().replace(/\/+$/, '')}/design` : 'docs/design/{slug}';
    edit(row);
  };

  export function hasChanges() {
    return dirty;
  }

  export function revert() {
    if (!saved) return;
    draft = toDraft(fromDraft(saved));
    serverErr = {};
  }

  export async function save() {
    if (busy || !dirty || invalid) return;
    busy = true;
    try {
      // The rest of the settings as stored now (another section may have saved since this one opened).
      const now = await invoke('project_settings', { project });
      const settings = { ...now.settings, feature_docs: fromDraft(draft) };
      const r = await invoke('project_settings_set', { project, form: { settings, backend: now.backend, track_orchestrator_dir: now.track_orchestrator_dir } });
      // Not the stored settings (e.g. a confirmation the backend asks for): nothing was saved.
      if (!r?.settings) throw new Error(`Could not save ${name}'s settings`);
      saved = toDraft(r.settings.feature_docs);
      draft = toDraft(fromDraft(saved));
      serverErr = {};
      toast(`Saved ${name}'s settings`);
    } catch (e) {
      const text = errText(e);
      const fe = parseFieldError(text);
      if (fe) {
        serverErr = { ...serverErr, [fe.row]: fe.message };
        root?.querySelector(`[data-row="${fe.row}"] input.field`)?.focus();
      } else toast(text, { kind: 'error' });
    } finally {
      busy = false;
    }
  }

  /** The pane forwards its key events: ⌘⏎ saves. */
  export function onKey(e) {
    if (e.metaKey && e.key === 'Enter') {
      e.preventDefault();
      save();
    }
  }

  onMount(load);
</script>

<div class="fd" bind:this={root}>
  <h2>Feature docs</h2>
  <p class="muted lead">Where team runs save each feature's spec and designs.</p>

  {#if loadError}
    <p class="err">Could not read {name}'s settings: {loadError}</p>
  {:else if !saved}
    <p class="muted">Loading…</p>
  {:else}
    <div class="row" data-row="docs" role="radiogroup" aria-labelledby="fd-docs">
      <span class="lab" id="fd-docs">Docs</span>
      <div class="opts">
        <label class="opt" class:on={draft.docs.kind === 'repo'}><input type="radio" name="fd-docs" checked={draft.docs.kind === 'repo'} onchange={() => pick('docs', 'repo')} /> In the repository</label>
        {#if draft.docs.kind === 'repo'}
          <div class="sub">
            <input class="field mono" class:bad={errors.docs} aria-label="Docs folder" aria-invalid={!!errors.docs} aria-describedby={errors.docs ? 'fd-docs-err' : undefined} bind:value={draft.docs.path} oninput={() => edit('docs')} spellcheck="false" />
            {#if errors.docs}<span class="ferr" id="fd-docs-err" role="alert">{errors.docs}</span>{:else}<span class="muted mono">{docsExample}</span>{/if}
            <span class="muted hint">{'{slug}'} is the feature's short name</span>
          </div>
        {/if}
        <label class="opt" class:on={draft.docs.kind === 'app'}><input type="radio" name="fd-docs" checked={draft.docs.kind === 'app'} onchange={() => pick('docs', 'app')} /> Not saved in the repository</label>
        {#if draft.docs.kind === 'app'}<p class="muted hint sub">Kept on this Mac only, never committed. Deleted with the feature.</p>{/if}
      </div>
    </div>

    <hr />

    <div class="row" data-row="designs" role="radiogroup" aria-labelledby="fd-designs">
      <span class="lab" id="fd-designs">Designs</span>
      <div class="opts">
        <label class="opt" class:on={draft.designs.kind === 'with_docs'}><input type="radio" name="fd-designs" checked={draft.designs.kind === 'with_docs'} onchange={() => pick('designs', 'with_docs')} /> With the docs</label>
        {#if draft.designs.kind === 'with_docs' && designsExample}<span class="muted mono sub">{designsExample}</span>{/if}
        <label class="opt" class:on={draft.designs.kind === 'repo'}><input type="radio" name="fd-designs" checked={draft.designs.kind === 'repo'} onchange={() => pick('designs', 'repo')} /> In the repository</label>
        {#if draft.designs.kind === 'repo'}
          <div class="sub">
            <input class="field mono" class:bad={errors.designs} aria-label="Designs folder" aria-invalid={!!errors.designs} aria-describedby={errors.designs ? 'fd-designs-err' : undefined} bind:value={draft.designs.path} oninput={() => edit('designs')} spellcheck="false" />
            {#if errors.designs}<span class="ferr" id="fd-designs-err" role="alert">{errors.designs}</span>{:else}<span class="muted mono">{designsExample}</span>{/if}
          </div>
        {/if}
        <label class="opt" class:on={draft.designs.kind === 'app'}><input type="radio" name="fd-designs" checked={draft.designs.kind === 'app'} onchange={() => pick('designs', 'app')} /> Not saved in the repository</label>
        {#if draft.designs.kind === 'app'}<p class="muted hint sub">Kept on this Mac only, never committed. Deleted with the feature.</p>{/if}
      </div>
    </div>

    <hr />
    <p class="muted foot-note">Applies to features you start from now on. Running features keep their folders.</p>
    <SettingsFooter {dirty} {busy} blocked={invalid} hint={invalid ? 'Fix the folders above to save.' : ''} onrevert={revert} onsave={() => save()} />
  {/if}
</div>

<style>
  .fd {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  .lead,
  .foot-note,
  .hint {
    margin: 0;
    font: var(--text-caption);
  }
  .row {
    display: flex;
    gap: var(--space-md);
    align-items: flex-start;
  }
  .lab {
    width: 120px;
    flex: none;
    color: var(--color-text-muted);
  }
  .opts {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    min-width: 0;
    flex: 1;
  }
  .opt {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    cursor: pointer;
  }
  .opt.on {
    font-weight: 600;
  }
  .sub {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    max-width: 360px;
    margin: 0 0 var(--space-xs) var(--space-lg);
  }
  .mono {
    font: var(--text-mono);
  }
  .field.bad {
    border-color: var(--color-status-error);
  }
  .ferr {
    color: var(--color-status-error);
    font: var(--text-mono);
  }
  hr {
    width: 100%;
    margin: var(--space-xs) 0;
    border: 0;
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
  .err {
    color: var(--color-status-error);
  }
</style>
