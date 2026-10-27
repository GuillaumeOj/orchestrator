<script>
  // The shell dock (§9.2): every shell in one panel under the panes, one tab
  // each, so shells never crowd the agents. ⌘J shows or hides it; hidden
  // shells keep running and keep their buffers (§9.1 principle 6).
  import { app, closePane, focusPane, openShell, saveLayout } from '../lib/app.svelte.js';
  import TermPaneView from '../panes/TermPaneView.svelte';
  import Kbd from './Kbd.svelte';

  let el = $state();
  const clamp = (h) => Math.min(0.8, Math.max(0.12, h));

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

<section class="dock" style:flex-basis="{app.dock.height * 100}%" bind:this={el} aria-label="Shells">
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
  <div class="tabs" role="tablist">
    {#each app.dock.tabs as t (t)}
      <div class="tab" class:on={t === app.dock.active} class:focused={t === app.focused} role="tab" aria-selected={t === app.dock.active}>
        <button class="label" onclick={() => focusPane(t)}>{app.panes[t]?.title ?? t}</button>
        <button class="x" aria-label="Close shell" onclick={() => closePane(t)}>×</button>
      </div>
    {/each}
    <button class="add" aria-label="New shell" onclick={() => openShell()}>+</button>
    <span class="grow"></span>
    <span class="muted hint"><Kbd keys="⌘+J" /> hide</span>
  </div>
  <!-- One terminal on screen; the others keep running (their xterm is kept). -->
  {#key app.dock.active}
    {#if app.dock.active && app.panes[app.dock.active]}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="body" data-pane={app.dock.active} onmousedown={() => app.focused !== app.dock.active && focusPane(app.dock.active)}>
        <TermPaneView paneId={app.dock.active} />
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
  .tab {
    display: flex;
    align-items: center;
    height: 100%;
    border-radius: var(--radius-sm) var(--radius-sm) 0 0;
    background: var(--color-surface-chrome);
    font: var(--text-label);
  }
  .tab.on {
    background: var(--color-surface-raised);
  }
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
    padding: 0 var(--space-xs);
    color: var(--color-text-muted);
  }
  .grow {
    flex: 1;
  }
  .hint {
    font: var(--text-caption);
    padding-right: var(--space-sm);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    border-radius: 0 var(--radius-sm) var(--radius-sm) var(--radius-sm);
    overflow: hidden;
  }
</style>
