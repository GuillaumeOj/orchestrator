<script>
  import { app, focusPane, closePane, saveLayout } from '../lib/app.svelte.js';
  import * as L from '../lib/layout.js';
  import PaneFrame from './PaneFrame.svelte';
  import Node from './Node.svelte';

  let { node } = $props();
  let el = $state();

  function drag(i, e) {
    e.preventDefault();
    const box = el.getBoundingClientRect();
    const total = node.dir === 'row' ? box.width : box.height;
    let last = node.dir === 'row' ? e.clientX : e.clientY;
    const move = (ev) => {
      const cur = node.dir === 'row' ? ev.clientX : ev.clientY;
      app.layout = L.resize(app.layout, node.id, i, (cur - last) / total);
      last = cur;
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      saveLayout();
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  function nudge(i, e) {
    const d = { ArrowLeft: -0.05, ArrowUp: -0.05, ArrowRight: 0.05, ArrowDown: 0.05 }[e.key];
    if (d) {
      app.layout = L.resize(app.layout, node.id, i, d);
      saveLayout();
      e.preventDefault();
    }
  }
</script>

{#if node.kind === 'split'}
  <div class="split {node.dir}" bind:this={el}>
    {#each node.children as child, i (child.id)}
      <div class="cell" style:flex-grow={node.sizes[i]}><Node node={child} /></div>
      {#if i < node.children.length - 1}
        <!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_tabindex -->
        <div
          class="handle"
          role="separator"
          aria-orientation={node.dir === 'row' ? 'vertical' : 'horizontal'}
          aria-valuenow={Math.round(node.sizes[i] * 100)}
          tabindex="0"
          onpointerdown={(e) => drag(i, e)}
          onkeydown={(e) => nudge(i, e)}
        ></div>
      {/if}
    {/each}
  </div>
{:else}
  <div class="group">
    {#if node.tabs.length > 1}
      <div class="tabs" role="tablist">
        {#each node.tabs as t (t)}
          <div class="tab" class:on={t === node.active} role="tab" aria-selected={t === node.active}>
            <button class="label" onclick={() => focusPane(t)}>{app.panes[t]?.title ?? t}</button>
            <button class="x" aria-label="Close tab" onclick={() => closePane(t)}>×</button>
          </div>
        {/each}
      </div>
    {/if}
    {#key node.active}<PaneFrame paneId={node.active} />{/key}
  </div>
{/if}

<style>
  .split {
    display: flex;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }
  .split.col {
    flex-direction: column;
  }
  .cell {
    display: flex;
    flex-basis: 0;
    min-width: 0;
    min-height: 0;
  }
  .handle {
    flex: none;
    background: transparent;
  }
  .row > .handle {
    width: var(--space-xs);
    cursor: col-resize;
  }
  .col > .handle {
    height: var(--space-xs);
    cursor: row-resize;
  }
  .handle:hover,
  .handle:focus-visible {
    background: var(--color-border-strong);
  }
  .group {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }
  .tabs {
    display: flex;
    gap: var(--space-xxs);
    height: 24px;
    flex: none;
  }
  .tab {
    display: flex;
    align-items: center;
    border-radius: var(--radius-sm) var(--radius-sm) 0 0;
    background: var(--color-surface-chrome);
    font: var(--text-label);
  }
  .tab.on {
    background: var(--color-surface-raised);
  }
  .tab button {
    border: 0;
    background: none;
    padding: 0 var(--space-sm);
    height: 100%;
  }
  .tab .x {
    padding: 0 var(--space-xs);
    color: var(--color-text-muted);
  }
</style>
