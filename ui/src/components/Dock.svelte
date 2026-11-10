<script>
  // The dock (§9.2, design-notes §5.2): agents and shells in one panel under the
  // panes, two views (Agents / Shells) on one compact tab strip. ⌘J shows or
  // hides it; hidden tabs keep running and keep their buffers (§9.1 principle 6).
  // With no workspace tab at all it fills the whole work column.
  import {
    agentShort,
    app,
    closePaneByUser,
    dockFills,
    dockFilter,
    dockProjectOptions,
    dockProjectWaiting,
    dockShells,
    dockTabs,
    dockView,
    dockViewOptions,
    dockViewWaiting,
    focusPane,
    openShell,
    projectName,
    saveLayout,
    setDockProject,
    setDockView,
    toggleDock,
  } from '../lib/app.svelte.js';
  import { attention, statusOf, waitLine } from '../lib/now.svelte.js';
  import { shellTip } from '../lib/shellcwd.js';
  import { tooltip } from '../lib/tooltip.js';
  import AgentPane from '../panes/AgentPane.svelte';
  import TermPaneView from '../panes/TermPaneView.svelte';
  import AgentChip from './AgentChip.svelte';
  import Select from './Select.svelte';
  import StatusBadge from './StatusBadge.svelte';
  import TabStrip from './TabStrip.svelte';

  const PREFIX = 'dock';
  const PANEL = 'dock-panel';

  let el = $state();
  const clamp = (h) => Math.min(0.8, Math.max(0.12, h));

  const fill = $derived(dockFills());
  const view = $derived(dockView());
  const filter = $derived(dockFilter());
  const active = $derived(app.dock.active);
  const activeSpec = $derived(app.panes[active]);

  /** An agent tab's tooltip: its id and what it waits for; a shell's: its title and the full path. */
  const tip = (spec) =>
    spec.type === 'agent'
      ? [spec.agentId, waitLine(attention(spec.agentId))].filter(Boolean).join(' · ')
      : shellTip(spec.title, spec.live ? spec.cwd : null);

  const tabs = $derived(
    dockTabs().map((id) => {
      const spec = app.panes[id];
      return { id, title: spec?.type === 'agent' ? agentShort(spec.agentId) : (spec?.title ?? id), tooltip: tip(spec ?? {}) };
    }),
  );

  /** What an empty view says. */
  const emptyText = $derived(
    view === 'shells' ? 'No shell open' : filter === 'all' ? 'No agent running' : `No agent running in ${projectName(filter)}`,
  );

  /** Drag the top edge: the dock's share of the column. */
  function drag(e) {
    e.preventDefault();
    const box = el.parentElement.getBoundingClientRect();
    const move = (ev) => {
      app.dock.height = clamp((box.bottom - ev.clientY) / box.height);
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      saveLayout();
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  function nudge(e) {
    const d = { ArrowUp: 0.05, ArrowDown: -0.05 }[e.key];
    if (d) {
      app.dock.height = clamp(app.dock.height + d);
      saveLayout();
      e.preventDefault();
    }
  }
</script>

<section class="dock" class:fill style:flex-basis={fill ? null : `${app.dock.height * 100}%`} bind:this={el} aria-label="Dock">
  {#if !fill}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_tabindex -->
    <div
      class="handle resize-handle"
      role="separator"
      aria-orientation="horizontal"
      aria-valuenow={Math.round(app.dock.height * 100)}
      tabindex="0"
      onpointerdown={drag}
      onkeydown={nudge}
    ></div>
  {/if}
  <TabStrip compact {tabs} {active} label="Dock tabs" idPrefix={PREFIX} panelId={PANEL} onselect={focusPane} onclose={closePaneByUser}>
    {#snippet leading()}
      <Select compact width="hug" aria-label="Dock view" noun="view" value={view} options={dockViewOptions()} dot={dockViewWaiting() ?? false} onchange={setDockView} />
      {#if view === 'agents'}
        <Select compact bare width="hug" aria-label="Project filter" noun="project" value={filter} options={dockProjectOptions()} dot={dockProjectWaiting() ?? false} onchange={setDockProject} />
      {/if}
      {#if !tabs.length}
        <span class="muted empty-note">{emptyText}</span>
        {#if view === 'shells'}<button class="btn sm" onclick={() => openShell()}>New shell</button>{/if}
      {/if}
    {/snippet}
    {#snippet tab(t)}
      {@const spec = app.panes[t.id]}
      {#if spec?.type === 'agent'}
        <AgentChip id={spec.agentId} compact />
        <span class="name">{t.title}</span>
        <StatusBadge state={statusOf(spec.agentId)} compact />
      {:else}
        <span class="name">{t.title}</span>
      {/if}
    {/snippet}
    {#snippet trailing()}
      {#if view === 'shells' && dockShells().length}
        <button type="button" class="icon-btn" aria-label="New shell" use:tooltip={'New shell'} onclick={() => openShell()}>+</button>
      {/if}
      <button type="button" class="icon-btn" aria-label="Hide dock" use:tooltip={{ label: 'Hide dock', key: '⌘J' }} onclick={toggleDock}>{'\u{F0140}'}</button>
    {/snippet}
  </TabStrip>
  <!-- One terminal on screen; the others keep running (their xterm is kept). -->
  {#key active}
    {#if activeSpec}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="body" id={PANEL} role="tabpanel" tabindex="-1" aria-labelledby="{PREFIX}-tab-{active}" data-pane={active} onmousedown={() => app.focused !== active && focusPane(active)}>
        {#if activeSpec.type === 'agent'}<AgentPane paneId={active} />{:else}<TermPaneView paneId={active} />{/if}
      </div>
    {:else}
      <div class="body" id={PANEL}></div>
    {/if}
  {/key}
</section>

<style>
  .dock {
    flex: none;
    min-height: 120px;
    display: flex;
    flex-direction: column;
    background: var(--color-surface-chrome);
    padding: 0 var(--space-xs) var(--space-xs);
  }
  /* Filling the column (no workspace tab): nothing to resize against. */
  .dock.fill {
    flex: 1;
    min-width: 0;
    padding-top: var(--space-xs);
  }
  .handle {
    height: var(--space-xs);
    flex: none;
    cursor: row-resize;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The badge's own title is the status label: the tab's tooltip (id · wait line) wins. */
  .dock :global(.tab .badge) {
    pointer-events: none;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--color-surface-pane);
    border-radius: 0 0 var(--radius-sm) var(--radius-sm);
    overflow: hidden;
  }
  .empty-note {
    white-space: nowrap;
  }
  .dock :global(.trail .icon-btn),
  .dock :global(.lead .btn) {
    height: var(--size-control-sm);
    min-height: 0;
  }
  .dock :global(.trail .icon-btn) {
    width: var(--size-control-sm);
  }
</style>
