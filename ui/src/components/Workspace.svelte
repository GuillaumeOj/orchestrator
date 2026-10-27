<script>
  // Tiling workspace (§9.2): splits, tabs, zoom. Hidden panes don't render; their
  // terminals keep their buffers (§9.1 principle 6, FR-1005).
  import { app } from '../lib/app.svelte.js';
  import Kbd from './Kbd.svelte';
  import Node from './Node.svelte';
  import PaneFrame from './PaneFrame.svelte';
</script>

<main class="ws">
  {#if app.zoomed && app.panes[app.zoomed]}
    <PaneFrame paneId={app.zoomed} zoomed />
  {:else if app.layout}
    <Node node={app.layout} />
  {:else}
    <div class="empty">
      <p class="muted">No pane open.</p>
      <p><button class="btn" onclick={() => (app.overlay = { kind: 'new-session' })}>New session <Kbd keys="⌘+T" /></button></p>
      <p class="muted">Overview <Kbd keys="⌘+; o" /> · Week planner <Kbd keys="⌘+; g" /> · Shell <Kbd keys="⌘+; t" /></p>
    </div>
  {/if}
</main>

<style>
  .ws {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    padding: var(--space-xs);
    background: var(--color-surface-chrome);
  }
  .empty {
    margin: auto;
    text-align: center;
  }
</style>
