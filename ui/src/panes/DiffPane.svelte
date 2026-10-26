<script>
  // Diff pane (FR-107): a worktree's changes against its base (from their merge
  // base, uncommitted and untracked included), a file list with change markers,
  // and the user's own Create PR (push + gh pr create, ADR-009).
  // Keys (pane-local): j/k next/previous file, r refresh, e open in editor, c create PR.
  import { onMount } from 'svelte';
  import { app, openEditor, toast, confirm } from '../lib/app.svelte.js';
  import { invoke, errText } from '../lib/ipc.js';
  import { patchByFile, lineKind, STATUS_WORD } from '../lib/diff.js';
  import Kbd from '../components/Kbd.svelte';

  let { paneId } = $props();
  const spec = $derived(app.panes[paneId]);

  let d = $state(null);
  let err = $state(null);
  let sel = $state(null); // selected file path
  let busy = $state(false);
  let pr = $state(null); // { title, body, draft } while the form is open

  const sections = $derived(patchByFile(d?.patch));
  const files = $derived(d?.files ?? []);
  const current = $derived(files.find((f) => f.path === sel) ?? files[0]);
  const lines = $derived(current ? (sections[current.path] ?? []) : []);
  const totals = $derived({ added: files.reduce((n, f) => n + (f.added ?? 0), 0), deleted: files.reduce((n, f) => n + (f.deleted ?? 0), 0) });
  const canPr = $derived(!!d?.branch && d.branch !== d.against);

  async function load() {
    try {
      d = await invoke('worktree_diff', { project: spec.projectId, path: spec.path });
      err = null;
    } catch (e) {
      err = errText(e);
    }
  }
  onMount(() => {
    load();
    const t = setInterval(load, 15000);
    return () => clearInterval(t);
  });

  function move(delta) {
    if (!files.length) return;
    const i = Math.max(0, files.findIndex((f) => f.path === current?.path));
    sel = files[Math.min(files.length - 1, Math.max(0, i + delta))].path;
  }

  function startPr() {
    if (!canPr) return toast(`${d?.branch ?? 'This worktree'} is the base branch: nothing to open a PR from`);
    pr = { title: (d.branch ?? '').split('/').pop().replace(/[-_]/g, ' '), body: '', draft: false };
  }

  function submitPr() {
    const { title, body, draft } = pr;
    if (!title.trim()) return toast('A PR needs a title', { kind: 'error' });
    const dirty = files.length ? ' Uncommitted changes are not part of the branch: they are not pushed.' : '';
    confirm(`Push ${d.branch} and open a PR into ${d.against}?`, `This pushes the branch to origin with your credentials and opens ${draft ? 'a draft ' : 'a '}PR.${dirty}`, async () => {
      busy = true;
      try {
        const url = await invoke('worktree_create_pr', { project: spec.projectId, path: spec.path, title, body, draft });
        toast(`Opened ${url}`, { ms: 12000 });
        pr = null;
      } catch (e) {
        toast(`Could not open the PR: ${errText(e)}`, { kind: 'error', ms: 12000 });
      } finally {
        busy = false;
      }
    }, 'Push and open PR');
  }

  function onKey(e) {
    if (e.metaKey || e.ctrlKey || e.altKey || e.target.closest('input, select, textarea')) return;
    const k = { j: () => move(1), k: () => move(-1), r: load, c: startPr, e: () => current && openEditor(`${spec.path}/${current.path}`, null, { projectId: spec.projectId }) }[e.key];
    if (k) {
      e.preventDefault();
      k();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="diff" tabindex="0" data-pane-focus onkeydown={onKey} role="region" aria-label="Diff {spec?.title}">
  <header>
    <b class="mono">{d?.branch ?? spec?.title}</b>
    <span class="muted">against {d?.against ?? '…'} · {files.length} file{files.length === 1 ? '' : 's'} · <span class="add">+{totals.added}</span> <span class="del">−{totals.deleted}</span></span>
    {#if d?.truncated}<span class="warn">patch cut: the file list is complete</span>{/if}
    <span class="grow"></span>
    <button class="btn ghost" onclick={load}>Refresh <Kbd keys="r" /></button>
    <button class="btn" onclick={startPr} disabled={busy || !canPr}>Create PR… <Kbd keys="c" /></button>
  </header>
  {#if err}<p class="err">{err}</p>{/if}
  {#if pr}
    <form class="prform" onsubmit={(e) => { e.preventDefault(); submitPr(); }}>
      <input placeholder="Title" bind:value={pr.title} aria-label="PR title" />
      <textarea rows="3" placeholder="Description (optional)" bind:value={pr.body} aria-label="PR description"></textarea>
      <label class="check"><input type="checkbox" bind:checked={pr.draft} /> Draft</label>
      <span class="grow"></span>
      <button type="button" class="btn ghost" onclick={() => (pr = null)}>Cancel</button>
      <button class="btn primary" disabled={busy}>Review…</button>
    </form>
  {/if}
  <div class="cols">
    <ul class="files" aria-label="Changed files">
      {#each files as f (f.path)}
        <li>
          <button class:on={f.path === current?.path} onclick={() => (sel = f.path)} title={STATUS_WORD[f.status] ?? f.status}>
            <span class="st st-{f.status}" aria-label={STATUS_WORD[f.status] ?? f.status}>{f.status}</span>
            <span class="mono path">{f.path}</span>
            {#if f.added != null}<span class="add">+{f.added}</span> <span class="del">−{f.deleted}</span>{/if}
          </button>
        </li>
      {:else}
        <li class="muted pad">{d ? 'No changes against the base.' : 'Loading…'}</li>
      {/each}
    </ul>
    <pre class="patch" aria-label="Patch of {current?.path ?? 'no file'}">{#each lines as l, i (i)}<span class="ln {lineKind(l)}">{l}{'\n'}</span>{:else}{#if current}<span class="muted">{current.status === '?' ? 'Untracked file: no patch until it is added.' : 'No textual patch (binary file).'}</span>{/if}{/each}</pre>
  </div>
</div>

<style>
  .diff {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    outline: none;
    font: var(--text-label);
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-sm);
    flex-wrap: wrap;
  }
  .grow {
    flex: 1;
  }
  .cols {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(180px, 32%) 1fr;
  }
  .files {
    list-style: none;
    margin: 0;
    padding: 0;
    overflow: auto;
    border-right: var(--border-width-default) solid var(--color-border-subtle);
  }
  .files button {
    display: flex;
    gap: var(--space-sm);
    width: 100%;
    border: 0;
    background: none;
    text-align: left;
    padding: var(--space-xxs) var(--space-sm);
  }
  .files button:hover,
  .files button.on {
    background: var(--color-surface-raised);
  }
  .path {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .st {
    width: 1.2em;
    text-align: center;
    font-weight: bold;
  }
  .st-A,
  .st-\? {
    color: var(--color-status-done);
  }
  .st-D {
    color: var(--color-status-error);
  }
  .patch {
    margin: 0;
    padding: var(--space-sm);
    overflow: auto;
    font-family: var(--font-family-mono);
    font-size: inherit;
    line-height: 1.45;
  }
  .ln {
    display: block;
    white-space: pre;
  }
  .ln.add {
    background: color-mix(in srgb, var(--color-status-done) 16%, transparent);
  }
  .ln.del {
    background: color-mix(in srgb, var(--color-status-error) 16%, transparent);
  }
  .ln.hunk,
  .ln.meta {
    color: var(--color-text-secondary);
  }
  .add {
    color: var(--color-status-done);
  }
  .del {
    color: var(--color-status-error);
  }
  .prform {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-sm);
    padding: 0 var(--space-sm) var(--space-sm);
  }
  .prform input[aria-label='PR title'],
  .prform textarea {
    flex: 1 1 100%;
  }
  .check {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .pad {
    padding: var(--space-sm);
  }
  .warn {
    color: var(--color-status-queued);
  }
  .err {
    color: var(--color-status-error);
    padding: 0 var(--space-sm);
  }
</style>
