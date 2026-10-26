<script>
  // Sidebar (§9.2): Projects → Features → agents; solo sessions under their
  // project. The only navigation tree. Features come with teams (P2): agents are
  // listed as solo sessions until then.
  import { app, agents, projects, openAgent, openProject } from '../lib/app.svelte.js';
  import { displayState } from '../lib/status.js';
  import { pct } from '../lib/format.js';
  import AgentChip from './AgentChip.svelte';
  import StatusBadge from './StatusBadge.svelte';
  import Kbd from './Kbd.svelte';

  let collapsed = $state({});
  const focusedAgent = $derived(app.panes[app.focused]?.agentId ?? app.selectedAgent);
  const byProject = $derived(
    projects().map((p) => ({ p, list: agents().filter((a) => a.project_id === p.id && a.state !== 'archived') })),
  );

  function onKey(e) {
    // Arrow keys move between rows (the tree is keyboard reachable without the mouse).
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const rows = [...e.currentTarget.querySelectorAll('[data-row]')];
    const i = rows.indexOf(document.activeElement);
    rows[Math.max(0, Math.min(rows.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1)))]?.focus();
    e.preventDefault();
  }
</script>

<nav class="side" aria-label="Projects">
  <div class="head"><span class="muted">Projects</span><span class="grow"></span><button class="add" title="Add project (⌘N)" aria-label="Add project" onclick={() => (app.overlay = { kind: 'add-project' })}>+</button><Kbd keys="⌘+B" /></div>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <ul role="tree" tabindex="-1" onkeydown={onKey}>
    {#each byProject as { p, list } (p.id)}
      <li role="treeitem" aria-selected="false" aria-expanded={!collapsed[p.id]}>
        <button data-row class="row project" class:selected={app.panes[app.focused]?.type === 'project' && app.panes[app.focused]?.projectId === p.id} onclick={() => openProject(p.id)} title={p.path}>
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <span class="caret" aria-hidden="true" onclick={(e) => (e.stopPropagation(), (collapsed[p.id] = !collapsed[p.id]))}>{collapsed[p.id] ? '▸' : '▾'}</span>
          <span class="name">{p.name}</span>
          <span class="meta">
            {#if p.git?.branch}<span class="icon" aria-hidden="true">{'\u{E725}'}</span> {p.git.branch}{/if}
            {#if p.git?.ahead}↑{p.git.ahead}{/if}{#if p.git?.behind}↓{p.git.behind}{/if}{#if p.git?.dirty} ±{p.git.dirty}{/if}
          </span>
        </button>
        {#if !collapsed[p.id]}
          <ul role="group">
            {#each list as a (a.id)}
              <li role="treeitem" aria-selected={focusedAgent === a.id}>
                <button data-row class="row agent" class:selected={focusedAgent === a.id} onclick={() => openAgent(a.id)}>
                  <StatusBadge state={displayState(a)} compact />
                  <AgentChip id={a.id} />
                  <span class="meta">{a.context_pct == null && a.live ? '' : pct(a.context_pct)}</span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </li>
    {:else}
      <li class="empty muted">No projects yet. <button class="btn ghost" onclick={() => (app.overlay = { kind: 'add-project' })}>Add project <Kbd keys="⌘+N" /></button></li>
    {/each}
  </ul>
</nav>

<style>
  .side {
    display: flex;
    flex-direction: column;
    width: var(--size-sidebar);
    flex: none;
    background: var(--color-surface-chrome);
    border-right: var(--border-width-default) solid var(--color-border-subtle);
    overflow: hidden auto;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: var(--size-pane-header);
    padding: 0 var(--space-md);
    font: var(--text-label);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    width: 100%;
    height: var(--size-row);
    padding: 0 var(--space-md) 0 var(--space-sm);
    border: 0;
    background: none;
    text-align: left;
  }
  .row:hover {
    background: var(--color-surface-raised);
  }
  .row.selected {
    background: var(--color-surface-selected);
  }
  .row.agent {
    padding-left: var(--space-xl);
  }
  .add {
    border: 0;
    background: none;
    color: var(--color-text-secondary);
    padding: 0 var(--space-sm);
  }
  .caret {
    width: var(--size-icon);
    color: var(--color-text-muted);
  }
  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row.agent :global(.chip) {
    flex: 1;
  }
  .meta {
    max-width: 50%;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--color-text-muted);
    font: var(--text-label);
    white-space: nowrap;
  }
  .empty {
    padding: var(--space-md);
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }
</style>
