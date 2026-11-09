<script>
  // Project settings › Files (FR-112): files and folders outside the repository
  // that every agent of the project can use. Edits are commands applied at once
  // (add, access, note on blur, remove): no Save. Add by dropping from Finder
  // (the webview drag-drop event carries real paths), by clicking the zone
  // (⌘O, the native panel), or by pasting paths. Single keys act when no field
  // has focus (the pane forwards its key events to onKey).
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { onMount, tick } from 'svelte';
  import { app, openEditor, toast } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { ACCESS_OPTIONS } from '../lib/options.js';
  import { accessLabel, addedToast, ICON, inRect, items, midTruncate, pathLines, tilde } from '../lib/projectfiles.js';
  import Kbd from './Kbd.svelte';
  import Select from './Select.svelte';

  let { project, name } = $props();
  let view = $state(null); // { files, live_agents }; null = loading
  let loadError = $state(null);
  let refusals = $state([]); // [{ path, error }] of the last add
  let dragging = $state(false);
  let adding = $state(false);
  let text = $state(''); // the paste field
  let sel = $state(0);
  let confirming = $state(null); // the path whose remove awaits its confirm
  let noteErr = $state(null); // { path, text }
  let fresh = $state([]); // paths just added (tinted ~2 s)
  let announce = $state('');
  let root = $state(null);
  let rev = $state(0); // a refused access change remounts its picker on the access still applied
  let browsing = false; // the native panel is open
  let freshTimer = null;

  const home = $derived(app.data?.core?.user_home ?? '');
  const files = $derived(view?.files ?? []);
  const anyWrite = $derived(files.some((f) => f.access === 'write'));
  const short = (p) => tilde(p, home);

  const refocus = () => root?.closest('[data-pane-focus]')?.focus();

  async function load() {
    loadError = null;
    try {
      view = await invoke('project_files', { project });
    } catch (e) {
      loadError = errText(e);
    }
  }

  async function add(paths) {
    if (adding || !paths.length) return;
    adding = true;
    try {
      const r = await invoke('project_files_add', { project, paths });
      view = { ...view, files: r.files };
      refusals = r.refused ?? [];
      const n = r.added?.length ?? 0;
      if (n) {
        toast(addedToast(n, name));
        announce = `Added ${items(n)}`;
        fresh = r.added;
        clearTimeout(freshTimer);
        freshTimer = setTimeout(() => (fresh = []), 2000);
      }
      return n;
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    } finally {
      adding = false;
    }
  }

  async function browse() {
    if (adding || browsing) return;
    browsing = true;
    try {
      await add((await invoke('pick_paths')) ?? []);
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    } finally {
      browsing = false;
    }
  }

  async function addTyped() {
    const lines = pathLines(text);
    if (!lines.length) return;
    // Refused lines stay in the field so they can be fixed; the block says why.
    if ((await add(lines)) !== undefined) text = refusals.map((r) => r.path).join('\n');
  }

  /** Applies one change to an entry; the error text comes back, null when it worked. */
  async function update(f, change) {
    try {
      const r = await invoke('project_file_update', { project, path: f.path, ...change });
      view = { ...view, files: r.files };
      return null;
    } catch (e) {
      return errText(e);
    }
  }

  async function setAccess(f, access) {
    if (f.access === access) return;
    const err = await update(f, { access });
    if (err) rev++;
    toast(err ?? `${short(f.path)} is now ${accessLabel(access)}`, err ? { kind: 'error' } : {});
  }

  const toggle = (f) => setAccess(f, f.access === 'write' ? 'read' : 'write');

  async function saveNote(f, el) {
    const next = el.value.trim();
    if (next === (f.note ?? '')) return;
    noteErr = null;
    // `""` clears the note; leaving it out would keep it.
    const err = await update(f, { note: next });
    // Failed: the typed text stays in the field so the user can fix it and retry.
    if (err) noteErr = { path: f.path, text: err };
  }

  function noteKey(e, f) {
    if (e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      e.currentTarget.blur();
      refocus();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      e.currentTarget.value = f.note ?? '';
      noteErr = null;
      e.currentTarget.blur();
      refocus();
    }
  }

  async function startRemove(f) {
    confirming = f.path;
    await tick();
    root?.querySelector('[data-confirm] .btn.danger')?.focus();
  }

  function cancelRemove() {
    confirming = null;
    refocus();
  }

  async function remove(f) {
    try {
      const r = await invoke('project_file_remove', { project, path: f.path });
      view = { ...view, files: r.files };
      sel = Math.min(sel, Math.max(0, r.files.length - 1));
      toast(`Removed ${short(f.path)} from ${name}`);
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    } finally {
      confirming = null;
      refocus();
    }
  }

  const reveal = (f) => invoke('reveal_path', { path: f.path }).catch((e) => toast(errText(e), { kind: 'error' }));
  const edit = (f) => openEditor(f.path, null, { projectId: project });

  /** Finder drag and drop: the event is window-wide, so only drops over this section count. */
  export function onDragDrop(ev) {
    const { type, paths, position } = ev.payload ?? ev;
    const box = (root?.closest('.body') ?? root)?.getBoundingClientRect();
    const inside = type !== 'leave' && inRect(position, box, window.devicePixelRatio || 1);
    dragging = inside && type !== 'drop';
    if (type === 'drop' && inside && view) add(paths ?? []);
  }

  /** Called by the pane for every key event while this section is shown. */
  export function onKey(e) {
    if (e.metaKey && !e.ctrlKey && !e.altKey && e.key.toLowerCase() === 'o') {
      e.preventDefault();
      return browse();
    }
    if (e.target.closest('input, textarea, select, [role="combobox"]') || e.metaKey || e.ctrlKey || e.altKey) return;
    const k = e.key;
    if (loadError && !view) {
      if (k !== 'r') return;
      load();
      return e.preventDefault();
    }
    if (!view) return;
    const onButton = !!e.target.closest('button');
    const cur = files[sel];
    if (confirming) {
      if (k === 'Enter' && !onButton) {
        // The row may have vanished (the list reloaded): leave confirm mode, do nothing.
        const f = files.find((f) => f.path === confirming);
        if (f) remove(f);
        else confirming = null;
      } else if (k === 'Escape') cancelRemove();
      else if (k !== 'j' && k !== 'k' && k !== 'ArrowDown' && k !== 'ArrowUp') return;
      else confirming = null;
      return e.preventDefault();
    }
    if ((k === 'Enter' || k === ' ') && onButton) return;
    if (k === 'j' || k === 'ArrowDown') sel = Math.min(files.length - 1, sel + 1);
    else if (k === 'k' || k === 'ArrowUp') sel = Math.max(0, sel - 1);
    else if (k === 'Enter' && cur) root?.querySelector(`[data-row="${sel}"] .note`)?.focus();
    else if (k === 'a' && cur) toggle(cur);
    else if ((k === 'Backspace' || k === 'Delete') && cur) startRemove(cur);
    else if (k === 'Escape' && refusals.length) refusals = [];
    else return;
    e.preventDefault();
    if (k === 'j' || k === 'k' || k.startsWith('Arrow')) tick().then(() => root?.querySelector(`[data-row="${sel}"]`)?.scrollIntoView({ block: 'nearest' }));
  }

  onMount(() => {
    load();
    let off = null;
    let gone = false;
    try {
      getCurrentWebview()
        .onDragDropEvent(onDragDrop)
        .then((un) => (gone ? un() : (off = un)))
        .catch(() => {});
    } catch {
      // Not in Tauri (a plain browser, tests): no Finder drops.
    }
    return () => {
      gone = true;
      clearTimeout(freshTimer);
      off?.();
    };
  });
</script>

<div class="files" bind:this={root}>
  <h2>Files</h2>
  <p class="muted lead">Files and folders outside the repository that every agent of {name} can use: a brand guide, a design file, shared notes.</p>
  <p class="muted lead">Agents read them in place. They aren't copied, versioned or committed.</p>

  {#if loadError && !view}
    <p class="box errbox"><span class="lerr">{ICON.error}</span> Couldn't load {name}'s files: {loadError} <span class="grow"></span><button class="btn ghost" onclick={load}>Retry <Kbd keys="r" /></button></p>
  {:else if !view}
    <div class="skel" aria-busy="true"><i></i><i></i><i class="short"></i></div>
  {:else}
    <button
      class="zone"
      class:tall={!files.length}
      class:drag={dragging}
      aria-label="Add files or folders. Opens the file browser."
      onclick={browse}
    >
      {#if !files.length && !dragging}<span class="muted">No files yet. Add a brand guide, a design file or notes that live outside the repo.</span>{/if}
      <span class="zline">
        <span class="icon">{ICON.drop}</span>
        {#if dragging}Drop to add to {name}{:else}Drop files or folders here, or <span class="browse">click to browse</span>{/if}
      </span>
      {#if !files.length && !dragging}<span class="muted">Several at once is fine.</span>{/if}
    </button>

    <label class="paste">
      <span class="muted">or paste paths, one per line (~ works)</span>
      <span class="pline">
        <textarea
          class="field"
          rows="1"
          placeholder="~/Brand"
          spellcheck="false"
          bind:value={text}
          onkeydown={(e) => {
            if (e.key === 'Enter' && !e.shiftKey) (e.preventDefault(), addTyped());
            else if (e.key === 'Escape' && text) (e.preventDefault(), e.stopPropagation(), (text = ''));
          }}
        ></textarea>
        <button class="btn" disabled={adding || !pathLines(text).length} onclick={addTyped}>Add <Kbd keys="⏎" /></button>
      </span>
    </label>

    {#if refusals.length}
      <div class="box warnbox" role="status">
        <span class="warn">▲</span>
        <div class="grow">
          <b>Not added ({refusals.length})</b>
          {#each refusals as r, i (i)}<div class="mono">{short(r.error)}</div>{/each}
        </div>
        <button class="x" aria-label="Dismiss" onclick={() => (refusals = [])}>{ICON.dismiss}</button>
      </div>
    {/if}

    {#if files.length}
      <div class="list">
        {#each files as f, i (f.path)}
          {@const folder = f.kind === 'folder'}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="row" class:sel={i === sel} class:fresh={fresh.includes(f.path)} class:missing={!f.exists} data-row={i} onclick={() => (sel = i)} title={f.exists ? '' : 'Not found on disk. Agents are told it is missing.'}>
            {#if confirming === f.path}
              <div class="confirm" data-confirm role="alertdialog" aria-label="Remove {short(f.path)}">
                <span>Remove from the project? The {f.kind} stays on disk.</span>
                <span class="grow"></span>
                <button class="btn danger" onclick={() => remove(f)}>Remove <Kbd keys="⏎" /></button>
                <button class="btn ghost" onclick={cancelRemove}>Cancel <Kbd keys="esc" /></button>
              </div>
            {:else}
              <span class="icon kind" role="img" aria-label={f.kind}>{folder ? ICON.folder : ICON.file}</span>
              <span class="path mono" title={f.path}>{midTruncate(short(f.path))}{#if !f.exists}<span class="badge">▲ missing</span>{/if}</span>
              <span class="notecell">
                <input class="field note" aria-label="Note for {short(f.path)}" placeholder="What is it for? Agents see this" value={f.note ?? ''} spellcheck="false" onblur={(e) => saveNote(f, e.currentTarget)} onkeydown={(e) => noteKey(e, f)} />
                {#if noteErr?.path === f.path}<span class="nerr">{noteErr.text}</span>{/if}
              </span>
              <span class="access">
                {#key `${f.path}:${rev}`}
                  <Select aria-label="Access for {short(f.path)}" value={f.access} options={ACCESS_OPTIONS} onchange={(access) => setAccess(f, access)} />
                {/key}
              </span>
              <span class="acts">
                <button class="act" aria-label="Reveal in Finder" title="Reveal in Finder" disabled={!f.exists} onclick={() => reveal(f)}>{ICON.reveal}</button>
                {#if !folder}<button class="act" aria-label="Open in editor" title="Open in editor" disabled={!f.exists} onclick={() => edit(f)}>{ICON.editor}</button>{/if}
                <button class="act" aria-label="Remove" title="Remove ⌫" onclick={() => startRemove(f)}>{ICON.remove}</button>
              </span>
            {/if}
          </div>
        {/each}
      </div>
      {#if anyWrite}<p class="muted note">Agents may change these files. Two agents writing the same file: the last write wins, nothing undoes it.</p>{/if}
      {#if view.live_agents > 0}<p class="muted note">Running agents get changes when they next start · {view.live_agents} live now</p>{/if}
    {/if}
    <p class="muted note keys">Changes apply as you make them. <Kbd keys="j" /> <Kbd keys="k" /> rows · <Kbd keys="⏎" /> note · <Kbd keys="a" /> access · <Kbd keys="⌫" /> remove · <Kbd keys="⌘+O" /> browse</p>
    <p class="sr" aria-live="polite">{announce}</p>
  {/if}
</div>

<style>
  /* One width for every row, so the column lines up (04-project-settings-files). */
  .access {
    flex: none;
    width: 136px;
  }
  .files {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  .lead,
  .note {
    margin: 0;
    font: var(--text-caption);
  }
  .keys {
    margin-top: var(--space-xs);
  }
  .mono {
    font: var(--text-mono);
  }
  .zone {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-xs);
    min-height: var(--size-control);
    padding: var(--space-sm) var(--space-md);
    background: none;
    border: var(--border-width-default) dashed var(--color-border-strong);
    border-radius: var(--radius-sm);
    color: var(--color-text-primary);
    font: var(--text-body);
    cursor: pointer;
  }
  .zone.tall {
    min-height: 120px;
  }
  .zone .browse {
    color: var(--color-accent-default);
  }
  .zone:not(.tall):not(.drag) {
    flex-direction: row;
    justify-content: flex-start;
  }
  .zone:hover,
  .zone:focus-visible {
    border-color: var(--color-accent-default);
    outline: none;
  }
  .zone:hover .browse,
  .zone:focus-visible .browse {
    text-decoration: underline;
  }
  .zone.drag {
    border-style: solid;
    border-color: var(--color-accent-default);
    background: var(--color-accent-subtle);
  }
  .paste,
  .pline {
    display: flex;
    gap: var(--space-xs);
  }
  .paste {
    flex-direction: column;
    font: var(--text-caption);
  }
  .pline {
    align-items: flex-start;
    gap: var(--space-sm);
    max-width: 480px;
  }
  textarea.field {
    resize: none;
    font: var(--text-mono);
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
  .errbox {
    border-color: var(--color-status-error);
  }
  .warnbox {
    align-items: flex-start;
    border-color: var(--color-pacing-amber);
  }
  .x,
  .act {
    background: none;
    border: 0;
    padding: 0 var(--space-xs);
    color: var(--color-text-muted);
    font: var(--text-icon);
    cursor: pointer;
  }
  .x:hover,
  .act:hover:not(:disabled) {
    color: var(--color-text-primary);
  }
  .act:disabled {
    color: var(--color-text-disabled);
    cursor: default;
  }
  .list {
    display: flex;
    flex-direction: column;
    padding: var(--space-xs);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .row {
    display: grid;
    grid-template-columns: 20px minmax(160px, 280px) minmax(160px, 1fr) auto 80px;
    align-items: center;
    gap: var(--space-sm);
    min-height: var(--size-row-dense);
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid transparent;
    border-radius: var(--radius-sm);
  }
  .row.sel {
    background: var(--color-surface-selected);
  }
  .row.fresh {
    background: color-mix(in srgb, var(--color-accent-default) 14%, transparent);
  }
  .row:has(.confirm) {
    display: flex;
  }
  .confirm {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .row.missing .kind,
  .row.missing .path {
    color: var(--color-text-muted);
  }
  .warn {
    color: var(--color-pacing-amber);
  }
  .lerr {
    color: var(--color-status-error);
    font: var(--text-icon);
  }
  .kind {
    color: var(--color-text-secondary);
  }
  .path {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    white-space: nowrap;
    overflow: hidden;
  }
  .badge {
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-pacing-amber);
    border-radius: var(--radius-full);
    color: var(--color-pacing-amber);
    font: var(--text-caption);
  }
  .notecell {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .nerr {
    color: var(--color-status-error);
    font: var(--text-caption);
  }
  .acts {
    display: flex;
    justify-content: flex-end;
    opacity: 0;
  }
  .row:hover .acts,
  .row.sel .acts,
  .row:focus-within .acts {
    opacity: 1;
  }
  .skel {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .skel i {
    height: var(--size-row);
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
  }
  .skel i.short {
    width: 70%;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
</style>
