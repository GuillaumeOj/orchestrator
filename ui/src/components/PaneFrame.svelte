<script>
  // Pane + pane header (Penpot 02): number (⌘1…9), type and title, zoom/close
  // hints. Focused = raised surface + accent border.
  import { app, focusPane, closePane, toggleZoom } from '../lib/app.svelte.js';
  import { visible } from '../lib/layout.js';
  import Kbd from './Kbd.svelte';
  import AgentPane from '../panes/AgentPane.svelte';
  import TermPaneView from '../panes/TermPaneView.svelte';
  import OverviewPane from '../panes/OverviewPane.svelte';
  import InboxPane from '../panes/InboxPane.svelte';
  import PlannerPane from '../panes/PlannerPane.svelte';

  let { paneId, zoomed = false } = $props();
  const spec = $derived(app.panes[paneId]);
  const focused = $derived(app.focused === paneId);
  const n = $derived(visible(app.layout).indexOf(paneId) + 1);
  const kind = $derived({ agent: 'claude', shell: 'zsh', editor: 'nvim', overview: 'overview', inbox: 'inbox', planner: 'planner' }[spec?.type]);
  const total = $derived(visible(app.layout).length);
</script>

{#if spec}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_no_noninteractive_element_interactions -->
  <section class="pane" class:focused data-pane={paneId} onmousedown={() => !focused && focusPane(paneId)} aria-label="{kind} {spec.title}">
    <header class="ph">
      <span class="num">{n || '·'}</span>
      <span class="muted">{kind}</span>
      <b class="t">{spec.title}{#if spec.type === 'agent' && spec.projectId}<span class="secondary">&nbsp;· {spec.projectId}</span>{/if}</b>
      {#if zoomed}<span class="zoomtag">ZOOMED · {n || 1} of {total}</span>{/if}
      <span class="grow"></span>
      <button class="hint" onclick={toggleZoom}><Kbd keys="⌘+; z" /> {zoomed ? 'restore' : 'zoom'}</button>
      <button class="hint" onclick={() => closePane(paneId)}><Kbd keys="⌘+W" /> close</button>
    </header>
    <div class="body">
      {#if spec.type === 'agent'}<AgentPane {paneId} />
      {:else if spec.type === 'shell' || spec.type === 'editor'}<TermPaneView {paneId} />
      {:else if spec.type === 'overview'}<OverviewPane {paneId} />
      {:else if spec.type === 'inbox'}<InboxPane {paneId} />
      {:else if spec.type === 'planner'}<PlannerPane {paneId} />{/if}
    </div>
  </section>
{/if}

<style>
  .pane {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }
  .pane.focused {
    border-color: var(--color-accent-default);
  }
  .ph {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    height: var(--size-pane-header);
    flex: none;
    padding: 0 var(--space-sm);
    background: var(--color-surface-chrome);
    font: var(--text-label);
    white-space: nowrap;
  }
  .focused .ph {
    background: var(--color-surface-raised);
  }
  .num {
    display: inline-flex;
    justify-content: center;
    width: 16px;
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    font: var(--text-caption);
  }
  .focused .num {
    border-color: var(--color-accent-default);
  }
  .t {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .grow {
    flex: 1;
  }
  .hint {
    display: inline-flex;
    gap: var(--space-xs);
    align-items: center;
    background: none;
    border: 0;
    padding: 0;
    color: var(--color-text-secondary);
  }
  .zoomtag {
    color: var(--color-accent-default);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
