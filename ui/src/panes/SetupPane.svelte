<script>
  // First run (J0): Claude Code installed, in the supported range and logged in
  // with Pro/Max (otherwise the app stays read-only); projects; workspace trust
  // per project (the user answers Claude Code's own dialog in a pane); the
  // kitty.conf import; the editor command; the first weekly plan. Shown at start
  // until "Done"; the palette reopens it.
  import { app, projects, openTrust, openProject, refresh, toast, closePane, findPane } from '../lib/app.svelte.js';
  import { invoke, errText } from '../lib/ipc.js';
  import PlanEditor from '../components/PlanEditor.svelte';
  import Kbd from '../components/Kbd.svelte';

  const s = $derived(app.data?.setup);
  const c = $derived(s?.claude);
  const trust = $derived(Object.fromEntries((s?.trust ?? []).map((t) => [t.id, t.trusted])));
  const home = $derived(app.data?.core?.user_home ?? '');
  const short = (path) => (home && String(path).startsWith(home) ? `~${String(path).slice(home.length)}` : path);

  let editor = $state(null);
  let kittyPath = $state(null);
  let editPlan = $state(false);
  $effect(() => {
    if (s && editor === null) editor = s.editor;
    if (s && kittyPath === null) kittyPath = s.kitty?.sources?.[0] ?? `${home}/.config/kitty/kitty.conf`;
  });

  const claudeOk = $derived(c && !c.problem);
  const steps = $derived([
    { id: 'claude', done: !!claudeOk },
    { id: 'projects', done: projects().length > 0 },
    { id: 'trust', done: projects().length > 0 && projects().every((p) => trust[p.id]) },
    { id: 'look', done: !!s?.kitty?.sources?.length },
    { id: 'plan', done: !!s?.plan_confirmed },
  ]);
  const left = $derived(steps.filter((x) => !x.done).length);

  async function saveEditor() {
    try {
      await invoke('set_editor', { template: editor });
      toast(`Editor: ${editor}`);
      await refresh();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }

  async function importKitty() {
    try {
      const k = await invoke('import_kitty', { path: kittyPath.trim().replace(/^~(?=\/)/, home) });
      app.kitty = { ...k, letterSpacing: app.kitty?.letterSpacing ?? 0 };
      toast(`Imported ${k.font_family} · ${k.mapped.length} settings mapped; new panes use it`);
      await refresh();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }

  async function done() {
    await invoke('settings_set', { key: 'setup.done', value: Date.now() }).catch(() => {});
    await refresh();
    const id = findPane((x) => x.type === 'setup');
    if (id) closePane(id);
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="setup" tabindex="0" data-pane-focus role="region" aria-label="Set up Orchestrator">
  <div class="head">
    <h2>Set up Orchestrator</h2>
    <span class="secondary">{left ? `${left} step${left > 1 ? 's' : ''} left` : 'Ready'}</span>
    <span class="grow"></span>
    <button class="btn primary" onclick={done}>{left ? 'Finish later' : 'Done'}</button>
  </div>

  <section class="card" class:ok={steps[0].done}>
    <div class="ch"><span class="mark">{steps[0].done ? '✓' : '1'}</span><b>Claude Code</b></div>
    {#if !c}
      <p class="muted">Checking claude --version and the login…</p>
    {:else}
      <p>
        <span class="mono">{short(c.path)}</span> · {c.version ?? 'not found'}{c.untested ? ' (newer than the last tested version: run the contract tests)' : ''}
        {#if c.logged_in != null} · {c.logged_in ? `logged in (${c.auth_method}, ${c.subscription ?? 'no subscription'})` : 'not logged in'}{/if}
      </p>
      {#if c.problem}<p class="err">{c.problem} Until then agents cannot start (read-only mode).</p>{/if}
      <p class="muted">Agents run with auto-update off; Claude Code updates are yours to apply. The app never writes to ~/.claude.</p>
    {/if}
  </section>

  <section class="card" class:ok={steps[1].done}>
    <div class="ch"><span class="mark">{steps[1].done ? '✓' : '2'}</span><b>Projects</b><span class="grow"></span><button class="btn" onclick={() => (app.overlay = { kind: 'add-project' })}>Add project <Kbd keys="⌘+N" /></button></div>
    {#each projects() as p (p.id)}
      <div class="prow">
        <button class="link" onclick={() => openProject(p.id)}>{p.name}</button>
        <span class="mono muted">{short(p.path)}</span>
        <span class="grow"></span>
        {#if trust[p.id]}<span class="okt">trusted</span>
        {:else}<button class="btn" onclick={() => openTrust(p.id)}>Trust…</button>{/if}
      </div>
    {:else}
      <p class="muted">A local folder or a git URL to clone. Worktrees go to the sibling folder &lt;repo&gt;.worktrees.</p>
    {/each}
    {#if projects().length}
      <p class="muted">Trust: Claude Code asks once per repository before any hook runs. “Trust…” opens claude in the project root: pick “Yes, I trust this folder”, then /exit. The app never answers it.</p>
    {/if}
  </section>

  <section class="card" class:ok={steps[3].done}>
    <div class="ch"><span class="mark">{steps[3].done ? '✓' : '3'}</span><b>Terminal look and editor</b></div>
    <div class="row">
      <label class="grow">kitty.conf (read only) <input class="field mono" bind:value={kittyPath} spellcheck="false" /></label>
      <button class="btn" onclick={importKitty}>Import</button>
    </div>
    {#if s?.kitty?.sources?.length}
      <p class="muted">Font {s.kitty.font} · {s.kitty.mapped} settings mapped, {s.kitty.unmapped} not used · from {s.kitty.sources.map(short).join(', ')}</p>
    {/if}
    <div class="row">
      <label class="grow">Editor command <span class="muted">({'{file}'}, {'{line}'}; nvim gets a server socket per project)</span><input class="field mono" bind:value={editor} spellcheck="false" /></label>
      <button class="btn" onclick={saveEditor} disabled={!editor?.trim() || editor === s?.editor}>Save</button>
    </div>
  </section>

  <section class="card" class:ok={steps[4].done}>
    <div class="ch"><span class="mark">{steps[4].done ? '✓' : '4'}</span><b>First weekly plan</b><span class="muted">aligned to your weekly reset</span>
      <span class="grow"></span>{#if steps[4].done && !editPlan}<button class="btn ghost" onclick={() => (editPlan = true)}>Edit</button>{/if}</div>
    {#if !steps[4].done || editPlan}
      <!-- The proposed split follows the projects added above. -->
      {#key projects().map((p) => p.id).join()}<PlanEditor onSaved={() => (editPlan = false)} />{/key}
    {:else}
      <p class="muted">Saved. Edit it any time from the Week planner (e).</p>
    {/if}
    <p class="muted">The Governor runs in <b>shadow mode</b> for the first week: it logs what it would do (Week planner log) and never holds your agents. After the week you can switch it to enforce.</p>
  </section>
</div>

<style>
  .setup {
    flex: 1;
    min-height: 0;
    overflow: auto;
    outline: none;
    padding: var(--space-md);
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    font: var(--text-label);
    max-width: 880px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  .card {
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    padding: var(--space-md);
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }
  .card.ok {
    border-color: var(--color-status-done);
  }
  .ch {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    font: var(--text-title);
  }
  .mark {
    width: var(--size-icon);
    text-align: center;
    color: var(--color-text-secondary);
  }
  .ok .mark {
    color: var(--color-status-done);
  }
  .row,
  .prow {
    display: flex;
    align-items: flex-end;
    gap: var(--space-sm);
  }
  .prow {
    align-items: center;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    color: var(--color-text-secondary);
  }
  .link {
    border: 0;
    background: none;
    padding: 0;
    font-weight: var(--font-weight-semibold);
  }
  .okt {
    color: var(--color-status-done);
  }
  p {
    margin: 0;
  }
  .err {
    color: var(--color-status-error);
  }
</style>
