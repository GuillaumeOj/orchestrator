<script>
  // Project settings › General (settings-brief §2.1, 04-project-settings-general-*; spec §3.1,
  // api-contract §2). The project record, edited: Name and Default branch are the only dirty
  // fields (footer Save / Revert, the window's Discard guard); path actions, Fetch now, Trust,
  // Worktrees (root only: "remove merged automatically" is not built) and the danger zone.
  import { onMount } from 'svelte';
  import { app, askRemoveProject, openInKitty, openTrust, projects, refresh, toast } from '../lib/app.svelte.js';
  import { dayTime, shortPath } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { tooltip } from '../lib/tooltip.js';
  import Select from './Select.svelte';

  let { project: pid } = $props();
  const p = $derived(projects().find((x) => x.id === pid) ?? null);
  const home = $derived(app.data?.core?.user_home ?? '');

  let name = $state('');
  let branch = $state('');
  let branches = $state(/** @type {string[] | null} */ (null));
  let trusted = $state(/** @type {boolean | null} */ (null));
  let nameError = $state('');
  let branchError = $state('');
  let busy = $state(false);
  let fetching = $state(false);
  let loaded = false;

  // The saved values: the record, refreshed when the window saves or the core says it changed.
  const savedName = $derived(p?.name ?? '');
  const savedBranch = $derived(p?.default_branch ?? '');
  const nameDirty = $derived(name.trim() !== savedName);
  const branchDirty = $derived(!!branch && branch !== savedBranch);
  const dirty = $derived(nameDirty || branchDirty);

  export const hasChanges = () => dirty;
  export function revert() {
    name = savedName;
    branch = savedBranch;
    nameError = branchError = '';
  }

  const branchOptions = $derived((branches ?? (savedBranch ? [savedBranch] : [])).map((b) => ({ value: b, label: b, detail: b === savedBranch ? 'current' : undefined })));

  async function load() {
    const [b, d] = await Promise.all([invoke('project_branches', { id: pid }).catch(() => null), invoke('project_detail', { id: pid, withPrs: false }).catch(() => null)]);
    branches = b;
    trusted = d ? d.trusted === true : null;
    if (!loaded) {
      loaded = true;
      revert();
    }
  }
  onMount(load);

  /** Saves the dirty fields: the name first, then the default branch; a refusal stays under its field. */
  export async function save() {
    if (busy || !dirty) return;
    busy = true;
    nameError = branchError = '';
    try {
      if (nameDirty) {
        try {
          await invoke('rename_project', { id: pid, name: name.trim() });
        } catch (e) {
          nameError = errText(e);
          return;
        }
      }
      if (branchDirty) {
        try {
          await invoke('set_default_branch', { id: pid, branch });
        } catch (e) {
          branchError = errText(e);
          return;
        }
      }
      await refresh();
      toast('Saved');
    } finally {
      busy = false;
    }
  }

  async function fetchNow() {
    if (fetching) return;
    fetching = true;
    try {
      const r = await invoke('project_fetch', { id: pid });
      toast(r?.error ? `Fetch failed · ${p?.name}: ${r.error}` : `Fetched · ${p?.name}`, r?.error ? { kind: 'error' } : {});
      await refresh();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    } finally {
      fetching = false;
    }
  }

  const reveal = () => invoke('reveal_path', { path: p.path }).catch((e) => toast(errText(e), { kind: 'error' }));
  const copy = async (text, what) => {
    try {
      await navigator.clipboard?.writeText(text);
      toast(`Copied ${what}`);
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  };
</script>

{#if !p}
  <p class="muted">This project is no longer registered.</p>
{:else}
  <section class="sset" aria-label="General settings">
    <h2>General</h2>
    <p class="muted lead">{p.name} · the project record. The pane header keeps the quick actions.</p>

    <div class="scard" data-group="project">
      <h3>Project</h3>
      <div class="srow">
        <label class="slab" for="pg-name">Name</label>
        <input id="pg-name" class="field" bind:value={name} spellcheck="false" aria-invalid={nameError ? 'true' : undefined} aria-describedby={nameError ? 'pg-name-err' : undefined} />
        {#if nameError}<p class="serr shelp" id="pg-name-err" role="alert">{nameError}</p>{/if}
      </div>
      <div class="srow">
        <span class="slab">Path</span>
        <span class="sctl">
          <span class="mono">{shortPath(p.path, home)}</span>
          <button type="button" class="icon-btn" aria-label="Reveal in Finder" use:tooltip={'Reveal in Finder'} onclick={reveal}>{'\u{F024B}'}</button>
          <button type="button" class="icon-btn" aria-label="Open in kitty" use:tooltip={'Open in kitty'} onclick={() => openInKitty(p.path)}>{'\u{F018D}'}</button>
        </span>
      </div>
      <div class="srow">
        <span class="slab">Id</span>
        <span class="sctl">
          <span class="muted mono">{p.id}</span>
          <button type="button" class="icon-btn" aria-label="Copy the id" use:tooltip={'Copy the id'} onclick={() => copy(p.id, 'the id')}>{'\u{F018F}'}</button>
        </span>
      </div>
    </div>

    <div class="scard" data-group="git">
      <h3>Git</h3>
      <div class="srow">
        <span class="slab" id="pg-branch-l">Default branch</span>
        <Select value={branch} options={branchOptions} aria-label="Default branch" onchange={(v) => (branch = v)} />
        {#if branchError}<p class="serr shelp" role="alert">{branchError}</p>{/if}
      </div>
      <div class="srow">
        <span class="slab">Fetch</span>
        <span class="sctl">
          <span>every 15 min · last {p.last_fetch_at_ms ? dayTime(p.last_fetch_at_ms) : 'not yet'}</span>
          <button type="button" class="btn" disabled={fetching} onclick={fetchNow}>{fetching ? 'Fetching…' : 'Fetch now'}</button>
        </span>
        {#if p.last_fetch_error}<p class="shelp amber" data-fetch-error>{p.last_fetch_error}</p>{/if}
        <p class="shelp">Paused on low battery or offline.</p>
      </div>
    </div>

    <div class="scard" data-group="trust">
      <h3>Trust</h3>
      <div class="srow">
        <span class="slab">Claude Code</span>
        <span class="sctl">
          {#if trusted === null}<span class="muted">Checking…</span>
          {:else if trusted}<span class="badge ok" data-trust="trusted"><span aria-hidden="true">●</span> Trusted</span>
          {:else}
            <span class="badge warn" data-trust="untrusted"><span aria-hidden="true">●</span> Not trusted yet</span>
            <button type="button" class="btn" onclick={() => openTrust(pid)}>Trust in a pane</button>
          {/if}
        </span>
      </div>
    </div>

    <div class="scard" data-group="worktrees">
      <h3>Worktrees</h3>
      <div class="srow"><span class="slab">Root</span><span class="sctl mono">{shortPath(`${p.path}.worktrees`, home)}</span></div>
    </div>

    <div class="scard danger" data-group="danger">
      <h3>Danger zone</h3>
      <div class="srow">
        <span class="slab">Remove project</span>
        <span class="sctl">
          <button type="button" class="btn ghost danger" onclick={() => askRemoveProject(pid)}>Remove project…</button>
        </span>
        <p class="shelp">Sessions and history stay on disk unless you choose to delete them.</p>
      </div>
    </div>

    <div class="sfoot foot">
      {#if dirty}<span class="dirty" data-dirty>Unsaved changes</span>{/if}
      <button type="button" class="btn" disabled={!dirty} onclick={revert}>Revert</button>
      <button type="button" class="btn primary" disabled={busy || !dirty} use:tooltip={{ label: 'Save', key: '⌘⏎' }} onclick={save}>Save</button>
    </div>
  </section>
{/if}

<style>
  .badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    height: 20px;
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid;
    border-radius: var(--radius-full);
    font: var(--text-caption);
  }
  .badge.ok {
    color: var(--color-status-done);
  }
  .badge.warn,
  .amber {
    color: var(--color-pacing-amber);
  }
  .scard.danger {
    border-color: var(--color-border-subtle);
  }
</style>
