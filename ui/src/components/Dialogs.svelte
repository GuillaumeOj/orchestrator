<script>
  // Dialogs: New session (⌘T, Penpot 06 J2 step 1), Add project (⌘N: a folder
  // or a git URL to clone, J1), the weekly plan, confirmations.
  // ⌘⏎ submits; esc cancels.
  import { app, projects, contextProject, openAgent, openProject, refresh, toast } from '../lib/app.svelte.js';
  import PlanEditor from './PlanEditor.svelte';
  import { invoke, errText } from '../lib/ipc.js';
  import { PRESSURE } from '../lib/status.js';
  import Kbd from './Kbd.svelte';

  const o = $derived(app.overlay);
  let busy = $state(false);
  let error = $state(null);
  // New session
  let form = $state({ project_id: '', name: '', model: 'haiku', effort: '', permission_mode: '', base: '', message: '' });
  // Add project
  let path = $state('');
  let cloneInto = $state('');
  const isUrl = (x) => /^(https?:\/\/|git@|ssh:\/\/)/.test(x.trim());

  $effect(() => {
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
  const live = $derived((app.data?.agents ?? []).filter((a) => a.live).length);

  function close() {
    app.overlay = null;
    error = null;
    busy = false;
  }

  async function startSession() {
    if (busy) return;
    busy = true;
    error = null;
    try {
      const id = await invoke('agent_start', { req: { ...$state.snapshot(form), name: form.name.trim() } });
      close();
      form = { ...form, name: '', message: '' };
      await refresh();
      openAgent(id);
      toast(`Started ${id} · ${project?.name ?? ''}`);
    } catch (e) {
      error = errText(e);
      busy = false;
    }
  }

  async function addProject() {
    if (busy) return;
    busy = true;
    try {
      const home = app.data?.core?.user_home ?? '~';
      const expand = (x) => x.trim().replace(/^~(?=\/)/, home);
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

  function onKey(e, submit) {
    if (e.key === 'Escape') return (e.preventDefault(), close());
    if (e.key === 'Enter' && (e.metaKey || e.target.tagName === 'INPUT')) return (e.preventDefault(), submit());
  }
</script>

{#if o && ['new-session', 'add-project', 'plan', 'confirm'].includes(o.kind)}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={close}></div>
  {#if o.kind === 'new-session'}
    <div class="dlg" role="dialog" aria-modal="true" aria-label="New session" tabindex="-1" onkeydown={(e) => onKey(e, startSession)}>
    <form onsubmit={(e) => (e.preventDefault(), startSession())}>
      <h3>New session</h3>
      <label>Project
        <select class="field" bind:value={form.project_id}>
          {#each projects() as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
        </select>
      </label>
      <label>Role <select class="field" disabled><option>plain Claude (roles come with teams)</option></select></label>
      <label>Task <span class="muted">(first message, optional)</span>
        <textarea class="field" rows="3" bind:value={form.message} placeholder="What should Claude do?"></textarea>
      </label>
      <div class="row">
        <label>Name <!-- svelte-ignore a11y_autofocus --><input class="field" bind:value={form.name} autofocus spellcheck="false" /></label>
        <label>Base branch <input class="field" bind:value={form.base} placeholder={project?.git?.branch ?? 'default'} spellcheck="false" /></label>
      </div>
      <p class="muted">Worktree: <span class="mono">orch/solo/{form.name || '…'}</span> in <span class="mono">{project ? `${project.name}.worktrees/${form.name}` : '…'}</span> (sibling folder, locked while in use)</p>
      <div class="row">
        <label>Model
          <select class="field" bind:value={form.model}>
            <option value="">default</option><option value="haiku">haiku</option><option value="sonnet">sonnet</option><option value="opus">opus</option>
          </select>
        </label>
        <label>Effort
          <select class="field" bind:value={form.effort}>
            <option value="">default</option><option>low</option><option>medium</option><option>high</option>
          </select>
        </label>
        <label>Permissions
          <select class="field" bind:value={form.permission_mode}>
            <option value="">default (ask)</option><option value="acceptEdits">accept edits</option><option value="plan">plan</option>
          </select>
        </label>
      </div>
      {#if snap}
        <p class="note"><span style:color={PRESSURE[snap.state]?.color}>{PRESSURE[snap.state]?.icon}</span> Governor {PRESSURE[snap.state]?.label}{app.data?.usage?.shadow ? ' (shadow mode)' : ''} · {live}/{snap.live_cap} live · your own typing is never blocked; app-delivered turns go through the Governor.</p>
      {/if}
      {#if error}<p class="err">{error}</p>{/if}
      <div class="foot">
        <button type="button" class="btn ghost" onclick={close}>Cancel <Kbd keys="esc" /></button>
        <button type="submit" class="btn primary" disabled={busy || !form.project_id}>{busy ? 'Starting…' : 'Start session'} <Kbd keys="⌘+⏎" /></button>
      </div>
    </form>
    </div>
  {:else if o.kind === 'add-project'}
    <div class="dlg" role="dialog" aria-modal="true" aria-label="Add project" tabindex="-1" onkeydown={(e) => onKey(e, addProject)}>
    <form onsubmit={(e) => (e.preventDefault(), addProject())}>
      <h3>Add project</h3>
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
        <button type="button" class="btn ghost" onclick={close}>Cancel <Kbd keys="esc" /></button>
        <button type="submit" class="btn primary" disabled={busy || !path.trim()}>{busy ? (isUrl(path) ? 'Cloning…' : 'Adding…') : isUrl(path) ? 'Clone' : 'Add'} <Kbd keys="⏎" /></button>
      </div>
    </form>
    </div>
  {:else if o.kind === 'plan'}
    <div class="dlg" role="dialog" aria-modal="true" aria-label="Weekly plan" tabindex="-1" onkeydown={(e) => e.key === 'Escape' && (e.preventDefault(), close())}>
      <h3>Weekly plan</h3>
      <PlanEditor onSaved={close} onCancel={close} />
    </div>
  {:else}
    <div class="dlg small" role="alertdialog" aria-modal="true" aria-label={o.text} tabindex="-1" onkeydown={(e) => onKey(e, () => (close(), o.onYes()))}>
    <form onsubmit={(e) => (e.preventDefault(), close(), o.onYes())}>
      <h3>{o.text}</h3>
      <p class="secondary">{o.detail}</p>
      <div class="foot">
        <button type="button" class="btn ghost" onclick={close}>Cancel <Kbd keys="esc" /></button>
        <!-- svelte-ignore a11y_autofocus -->
        <button type="submit" class="btn primary" autofocus>{o.yes} <Kbd keys="⏎" /></button>
      </div>
    </form>
    </div>
  {/if}
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: var(--color-surface-chrome);
    opacity: var(--opacity-backdrop);
  }
  .dlg {
    position: fixed;
    top: 80px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 51;
    width: min(560px, 92vw);
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    padding: var(--space-lg);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
    font: var(--text-label);
  }
  .dlg form {
    display: contents;
  }
  .dlg.small {
    width: min(420px, 92vw);
  }
  h3 {
    margin: 0;
    font: var(--text-title);
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
  .err {
    color: var(--color-status-error);
    white-space: pre-wrap;
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-sm);
  }
</style>
