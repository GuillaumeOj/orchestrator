<script>
  // Icon rail below 1512 px (§9.2): one mark per project with a count badge when
  // agents need you (the project's `needs_you` count); hover or focus opens the project summary (Penpot 03 rail hover).
  // The context's project is marked (none in All projects); a click sets the context and opens the project.
  import { activeAgents, app, openAgent, openProjectContext, projects } from '../lib/app.svelte.js';
  import { attention, counts, needYouText, statusOf, waitLine } from '../lib/now.svelte.js';
  import AgentChip from './AgentChip.svelte';
  import StatusBadge from './StatusBadge.svelte';

  let hover = $state(null);
  const rows = $derived(
    projects().map((p) => {
      const list = activeAgents().filter((a) => a.project_id === p.id);
      return { p, list, need: counts({ project: p.id }).needs_you };
    }),
  );
</script>

<nav class="rail" aria-label="Projects">
  {#each rows as { p, list, need } (p.id)}
    {@const current = app.workspace.project === p.id}
    <div class="slot" class:on={current} role="group" onmouseenter={() => (hover = p.id)} onmouseleave={() => (hover = null)}>
      <button class="mark" onfocus={() => (hover = p.id)} onblur={() => (hover = null)} onclick={() => openProjectContext(p.id)} aria-label={p.name} aria-current={current ? 'true' : undefined}>
        {p.name[0]?.toUpperCase()}
      </button>
      {#if need}<span class="count" title={needYouText(need)}>{need}</span>{/if}
      {#if hover === p.id}
        <div class="pop">
          <b>{p.name}</b>
          <span class="muted">{p.git?.branch ?? ''}{p.git?.dirty ? ` · ±${p.git.dirty}` : ''}</span>
          {#each list as a (a.id)}
            <button class="row" onclick={() => openAgent(a.id)} title={waitLine(attention(a.id))}><StatusBadge state={statusOf(a.id)} compact /><AgentChip id={a.id} /></button>
          {:else}<span class="muted">No sessions</span>{/each}
        </div>
      {/if}
    </div>
  {/each}
</nav>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-sm);
    width: var(--size-sidebar-rail);
    flex: none;
    padding-top: var(--space-sm);
    background: var(--color-surface-chrome);
    border-right: var(--border-width-default) solid var(--color-border-subtle);
  }
  .slot {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-xxs);
  }
  .slot.on {
    background: var(--color-surface-selected);
    border-radius: var(--radius-sm);
    box-shadow: 0 0 0 var(--space-xs) var(--color-surface-selected);
  }
  .slot.on .mark {
    border-color: var(--color-accent-default);
  }
  .mark {
    width: var(--size-control);
    height: var(--size-control);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    background: var(--color-surface-pane);
  }
  .pop {
    position: absolute;
    left: calc(100% + var(--space-sm));
    top: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    min-width: 200px;
    padding: var(--space-md);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    border: 0;
    background: none;
    padding: var(--space-xxs) 0;
  }
</style>
