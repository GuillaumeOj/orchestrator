<script>
  // Inspector (§9.2, notes p4-2 §5): what is inspected (D10): the focused
  // agent pane's agent, else the selection (an agent, a run or a decision).
  // Docked at ≥ 1512 px, overlay below.
  import { app, inspected } from '../lib/app.svelte.js';
  import AgentInspector from './inspector/AgentInspector.svelte';
  import DecisionInspector from './inspector/DecisionInspector.svelte';
  import RunInspector from './inspector/RunInspector.svelte';
  import Kbd from './Kbd.svelte';

  let { overlay = false } = $props();
  const s = $derived(inspected());
</script>

<aside class="insp" class:overlay aria-label="Inspector">
  <div class="head"><span class="muted">Inspector{s ? ` · ${s.kind}` : ''}</span>{#if overlay}<button class="x" onclick={() => (app.inspector = false)}><Kbd keys="esc" /></button>{:else}<Kbd keys="⌘+I" />{/if}</div>
  {#if s?.kind === 'agent'}{#key s.id}<AgentInspector id={s.id} />{/key}
  {:else if s?.kind === 'run'}{#key s.id}<RunInspector id={s.id} />{/key}
  {:else if s?.kind === 'decision'}{#key s.id}<DecisionInspector id={s.id} />{/key}
  {:else}
    <p class="muted">Select an agent in the sidebar or a row in Mission control.</p>
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
</style>
