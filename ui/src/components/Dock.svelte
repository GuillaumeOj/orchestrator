<script>
  // The dock (§9.2): agents and shells in one panel under the panes, one tab
  // each, so a terminal never crowds the workspace. ⌘J shows or hides it;
  // hidden tabs keep running and keep their buffers (§9.1 principle 6). With no
  // workspace pane, or zoomed (⌘; z), it fills the whole work column.
  import { tick } from 'svelte';
  import { agent, app, closePaneByUser, dockFills, focusPane, inDock, openShell, saveLayout, toggleZoom } from '../lib/app.svelte.js';
  import { attention, statusOf, waitLine } from '../lib/now.svelte.js';
  import AgentPane from '../panes/AgentPane.svelte';
  import TermPaneView from '../panes/TermPaneView.svelte';
  import AgentChip from './AgentChip.svelte';
  import Kbd from './Kbd.svelte';
  import StatusBadge from './StatusBadge.svelte';

  let el = $state();
  let lane = $state();
  /** The tab lane is wider than its box: a divider shows the clipped tab as cut. */
  let overflow = $state(false);
  let scrolled = $state(false);
  const clamp = (h) => Math.min(0.8, Math.max(0.12, h));

  const fill = $derived(dockFills());
  const zoomed = $derived(!!app.zoomed && inDock(app.zoomed));
  const activeSpec = $derived(app.panes[app.dock.active]);
  /** The first tab is active and nothing is scrolled off: it sits on the body's top-left corner. */
  const squareTL = $derived(app.dock.tabs[0] === app.dock.active && !scrolled);

  /** An agent tab's name: its team node, else its id. */
  const short = (spec) => agent(spec.agentId)?.team?.node ?? spec.agentId;
  const tip = (spec) =>
    spec.type === 'agent'
      ? [spec.agentId, waitLine(attention(spec.agentId))].filter(Boolean).join(' · ')
      : (spec.title ?? '');

  function measure() {
    if (!lane) return;
    overflow = lane.scrollWidth > lane.clientWidth + 1;
    scrolled = lane.scrollLeft > 0;
  }

  $effect(() => {
    if (!lane) return;
    const ro = new ResizeObserver(measure);
    ro.observe(lane);
    return () => ro.disconnect();
  });

  // The active tab is always in view.
  $effect(() => {
    const id = app.dock.active;
    void app.dock.tabs.length;
    tick().then(() => {
      lane?.querySelector(`[data-tab="${id}"]`)?.scrollIntoView({ inline: 'nearest', block: 'nearest' });
      measure();
    });
  });

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
      class="handle"
      role="separator"
      aria-orientation="horizontal"
      aria-valuenow={Math.round(app.dock.height * 100)}
      tabindex="0"
      onpointerdown={drag}
      onkeydown={nudge}
    ></div>
  {/if}
  <div class="tabs" role="tablist">
    <div class="lane" bind:this={lane} onscroll={measure}>
      {#each app.dock.tabs as t (t)}
        {@const spec = app.panes[t]}
        {#if spec}
          <div class="tab" class:on={t === app.dock.active} class:focused={t === app.focused} data-tab={t} role="tab" aria-selected={t === app.dock.active} title={tip(spec)}>
            <button class="label" onclick={() => focusPane(t)}>
              {#if spec.type === 'agent'}
                <AgentChip id={spec.agentId} compact />
                <span class="name">{short(spec)}</span>
                <StatusBadge state={statusOf(spec.agentId)} compact />
              {:else}
                <span class="name">{spec.title ?? t}</span>
              {/if}
            </button>
            <button
              class="x"
              aria-label={spec.type === 'agent' ? 'Close tab' : 'Close shell'}
              title={spec.type === 'agent' ? 'Close tab · the agent keeps running' : 'Close shell'}
              onclick={() => closePaneByUser(t)}>×</button>
          </div>
        {/if}
      {/each}
    </div>
    <span class="sep" class:on={overflow}></span>
    <button class="add" aria-label="New shell" title="New shell" onclick={() => openShell()}>+</button>
    <span class="grow"></span>
    {#if zoomed}
      <span class="zoomtag">ZOOMED</span>
      <button class="hint" onclick={toggleZoom}><Kbd keys="⌘+; z" /> restore</button>
    {/if}
    <span class="muted hint"><Kbd keys="⌘+J" /> hide</span>
  </div>
  <!-- One terminal on screen; the others keep running (their xterm is kept). -->
  {#key app.dock.active}
    {#if activeSpec}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="body" class:square={squareTL} data-pane={app.dock.active} onmousedown={() => app.focused !== app.dock.active && focusPane(app.dock.active)}>
        {#if activeSpec.type === 'agent'}<AgentPane paneId={app.dock.active} />{:else}<TermPaneView paneId={app.dock.active} />{/if}
      </div>
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
  /* Filling the column (no workspace pane, or zoomed): nothing to resize against. */
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
  .handle:hover,
  .handle:focus-visible {
    background: var(--color-border-strong);
  }
  .tabs {
    display: flex;
    align-items: center;
    gap: var(--space-xxs);
    height: 24px;
    flex: none;
  }
  /* The tabs scroll inside their lane; `+` and the hint stay pinned. */
  .lane {
    display: flex;
    gap: var(--space-xxs);
    height: 100%;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .lane::-webkit-scrollbar {
    display: none;
  }
  .sep {
    width: 1px;
    height: 12px;
    flex: none;
    background: transparent;
  }
  .sep.on {
    background: var(--color-border-subtle);
  }
  .tab {
    display: flex;
    align-items: center;
    flex: none;
    max-width: 200px;
    height: 100%;
    border-radius: var(--radius-sm) var(--radius-sm) 0 0;
    background: var(--color-surface-chrome);
    font: var(--text-label);
  }
  .tab:hover {
    background: var(--color-surface-raised);
  }
  .tab.on {
    background: var(--color-surface-pane);
  }
  .tab .label {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    min-width: 0;
    flex: 1;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tab:hover .label,
  .tab.focused .label {
    color: var(--color-text-primary);
  }
  .tab button,
  .add {
    border: 0;
    background: none;
    padding: 0 var(--space-sm);
    height: 100%;
    color: var(--color-text-secondary);
  }
  .tab .x {
    flex: none;
    padding: 0 var(--space-xs);
    color: var(--color-text-muted);
  }
  .tab:hover .x {
    color: var(--color-text-secondary);
  }
  .add {
    flex: none;
  }
  .grow {
    flex: 1;
  }
  .zoomtag {
    font: var(--text-caption);
    color: var(--color-accent-default);
  }
  .hint {
    flex: none;
    font: var(--text-caption);
    padding-right: var(--space-sm);
  }
  button.hint {
    border: 0;
    background: none;
    color: var(--color-text-muted);
    padding-left: var(--space-sm);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--color-surface-pane);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }
  .body.square {
    border-top-left-radius: 0;
  }
</style>
