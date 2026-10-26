<script>
  // Status bar (§9.2): git state of the focused project, live/queued agents with
  // the pressure state in words, the 5h guard chip whenever it holds work, memory.
  import { app, agents, contextAgent, contextProject, openSingleton } from '../lib/app.svelte.js';
  import { mb, clock } from '../lib/format.js';
  import Kbd from './Kbd.svelte';
  import PressurePill from './PressurePill.svelte';

  const a = $derived(contextAgent());
  const p = $derived(contextProject());
  const git = $derived(a?.git ?? p?.git);
  const branch = $derived(a?.branch || git?.branch);
  const s = $derived(app.data?.usage?.snapshot);
  const live = $derived(agents().filter((x) => x.live).length);
  const queued = $derived((app.data?.queue ?? []).length);
  const m = $derived(app.data?.memory);
  const fiveReset = $derived(app.data?.usage?.five_hour?.resets_at);
  const watch = $derived(app.data?.memory_watch);
</script>

<footer class="status">
  {#if branch}
    <span><span class="icon" aria-hidden="true">{'\u{E725}'}</span> {branch}</span>
    {#if git}<span>↑{git.ahead} ↓{git.behind}</span><span>±{git.dirty}</span>{/if}
    <span class="muted">·</span>
  {/if}
  <span><span style:color="var(--color-status-working)" aria-hidden="true">▶</span> {live} live</span>
  <span><span style:color="var(--color-status-queued)" aria-hidden="true">⧗</span> {queued} queued</span>
  {#if s}
    <button class="plain" onclick={() => openSingleton('planner')}><PressurePill state={s.state} shadow={app.data?.usage?.shadow} /></button>
    {#if s.five_hour_guard}
      <span class="guard"><span style:color="var(--color-status-blocked-by-limit)" aria-hidden="true">⊘</span> <b>5h guard</b> cap 1{fiveReset ? ` until ${clock(fiveReset * 1000)}` : ''}</span>
    {/if}
  {/if}
  <span class="grow"></span>
  {#if watch?.alert}<span class="memwarn" title="WebKit helper above {watch.warn_mb} MB · logged every minute in {watch.log}"><span aria-hidden="true">⚠</span> {watch.alert}</span>{/if}
  {#if m}<span title="app {mb(m.app_mb)} · agents {mb(m.agents_mb)} · shells {mb(m.shells_mb)}">RAM {mb(m.total_mb)} / {mb(m.system_mb)}</span><span class="muted">·</span>{/if}
  <span class:armed={app.leader}>{app.leader ? 'LEADER' : 'leader'} <Kbd keys="⌘+;" /></span>
</footer>

<style>
  .status {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    height: var(--size-statusbar);
    padding: 0 var(--space-md);
    background: var(--color-surface-chrome);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-label);
    white-space: nowrap;
    overflow: hidden;
  }
  .plain {
    background: none;
    border: 0;
    padding: 0;
  }
  .guard {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    height: 18px;
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-full);
  }
  .memwarn {
    color: var(--color-status-error);
    font-weight: var(--font-weight-semibold);
  }
  .armed {
    color: var(--color-accent-default);
    font-weight: var(--font-weight-semibold);
  }
</style>
