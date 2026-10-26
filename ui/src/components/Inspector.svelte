<script>
  // Inspector (§9.2): the selection's details. Docked at ≥ 1512 px, overlay below.
  import { app, contextAgent, projects, agentAction, openShell, openInKitty, confirm } from '../lib/app.svelte.js';
  import { displayState, queueReason, sessionPp, cacheState } from '../lib/status.js';
  import { clock, mb, pct } from '../lib/format.js';
  import AgentChip from './AgentChip.svelte';
  import StatusBadge from './StatusBadge.svelte';
  import Kbd from './Kbd.svelte';

  let { overlay = false } = $props();
  const a = $derived(contextAgent());
  const p = $derived(projects().find((x) => x.id === a?.project_id));
  const pp = $derived(sessionPp(a, app.data?.usage?.snapshot));
  const cache = $derived(cacheState(a, clock));
  const home = $derived(app.data?.core?.user_home ?? '');
  const short = (path) => (home && String(path).startsWith(home) ? `~${String(path).slice(home.length)}` : path);
</script>

<aside class="insp" class:overlay aria-label="Inspector">
  <div class="head"><span class="muted">Inspector</span>{#if overlay}<button class="x" onclick={() => (app.inspector = false)}><Kbd keys="esc" /></button>{:else}<Kbd keys="⌘+I" />{/if}</div>
  {#if a}
    <div class="who"><AgentChip id={a.id} /><StatusBadge state={displayState(a)} /></div>
    <dl>
      <dt>Project</dt><dd>{p?.name ?? '—'}</dd>
      <dt>Branch</dt><dd>{a.branch || '—'}</dd>
      <dt>Worktree</dt><dd class="wrap">{short(a.worktree)}</dd>
      <dt>Model</dt><dd>{a.model ?? 'default'}{a.effort ? ` · effort ${a.effort}` : ''}</dd>
      <dt>Context</dt><dd>{pct(a.context_pct)}{cache ? ` · cache ${cache}` : ''}</dd>
      <dt>Usage</dt><dd>{pp == null ? '—' : `${pp.toFixed(1)} pp this session`} · {a.turns ?? 0} turns</dd>
      <dt>Governor</dt><dd>{a.queue_entry ? `Queued · ${queueReason(a.queue_entry)}` : a.live ? 'Admitted' : '—'}</dd>
      <dt>Memory</dt><dd>{a.memory_mb ? mb(a.memory_mb) : '—'}</dd>
      <dt>Session</dt><dd class="wrap mono">{a.session_id}</dd>
      {#if a.git}<dt>Changes</dt><dd>±{a.git.dirty} · ↑{a.git.ahead} ↓{a.git.behind}</dd>{/if}
    </dl>
    <div class="acts">
      {#if a.live}
        <button class="btn" onclick={() => agentAction(a.id, 'interrupt')}>Interrupt <Kbd keys="esc" /></button>
        <button class="btn" onclick={() => agentAction(a.id, 'suspend')}>Suspend <Kbd keys="s" /></button>
      {:else}
        <button class="btn" onclick={() => agentAction(a.id, 'resume')}>Resume <Kbd keys="r" /></button>
      {/if}
      <button class="btn danger" onclick={() => confirm(`Kill ${a.id}?`, 'SIGTERM to its group and to leftover processes in its worktree, SIGKILL after 5 s.', () => agentAction(a.id, 'kill'), 'Kill')}>Kill <Kbd keys="⇧+K" /></button>
      <button class="btn ghost" onclick={() => openShell({ agentId: a.id })}>Shell here <Kbd keys="⌘+; t" /></button>
      <button class="btn ghost" onclick={() => openInKitty(a.worktree)}>Open in kitty</button>
    </div>
  {:else}
    <p class="muted">Select an agent in the sidebar or the Overview.</p>
  {/if}
</aside>

<style>
  .insp {
    width: var(--size-inspector);
    flex: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    padding: 0 var(--space-md) var(--space-md);
    background: var(--color-surface-chrome);
    border-left: var(--border-width-default) solid var(--color-border-subtle);
    overflow: auto;
    font: var(--text-label);
  }
  .insp.overlay {
    position: absolute;
    right: var(--space-sm);
    top: calc(var(--size-titlebar) + var(--space-sm));
    bottom: calc(var(--size-statusbar) + var(--space-sm));
    z-index: 20;
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-surface-pane);
    box-shadow: var(--shadow-overlay);
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    height: var(--size-pane-header);
  }
  .x {
    background: none;
    border: 0;
    padding: 0;
  }
  .who {
    display: flex;
    gap: var(--space-sm);
    align-items: center;
  }
  dl {
    display: grid;
    grid-template-columns: 84px 1fr;
    gap: var(--space-sm) var(--space-md);
    margin: 0;
    padding-bottom: var(--space-md);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  dt {
    color: var(--color-text-muted);
  }
  dd {
    margin: 0;
  }
  .wrap {
    overflow-wrap: anywhere;
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
  }
</style>
