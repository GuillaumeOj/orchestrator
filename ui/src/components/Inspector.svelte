<script>
  // Inspector (§9.2, notes p4-2 §5): what is inspected (D10): the focused
  // agent pane's agent, else the selection (an agent, a run or a decision).
  // Docked at ≥ 1728 px; below, an overlay over the chat column's slot with a
  // banner when cards are open (P4.2 notes §4, P4.3 notes §9).
  import { app, focusCard, inspected } from '../lib/app.svelte.js';
  import { bannerText } from '../lib/chat.js';
  import { counts, openDecisions } from '../lib/now.svelte.js';
  import { tooltip } from '../lib/tooltip.js';
  import AgentInspector from './inspector/AgentInspector.svelte';
  import DecisionInspector from './inspector/DecisionInspector.svelte';
  import RunInspector from './inspector/RunInspector.svelte';

  let { overlay = false } = $props();
  const s = $derived(inspected());
  const banner = $derived(
    overlay ? bannerText(counts().decisions, openDecisions().filter((d) => d.level === 1).length) : null,
  );
</script>

<aside class="insp" class:overlay aria-label="Inspector">
  {#if banner}<button type="button" class="banner" use:tooltip={{ label: 'Decisions', key: '⌘⇧I' }} onclick={() => focusCard()}><span><span class="icon" style:color="var(--color-status-waiting-for-you)" aria-hidden="true">{'\u{F0005}'}</span> {banner}</span></button>{/if}
  <div class="head">
    <span class="muted">Inspector{s ? ` · ${s.kind}` : ''}</span>
    <button type="button" class="icon-btn" aria-label="Close inspector" use:tooltip={{ label: 'Close', key: overlay ? 'esc' : '⌘I' }} onclick={() => (app.inspector = false)}>{'\u{F0156}'}</button>
  </div>
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
    inset: 0;
    width: auto;
    z-index: 20;
    background: var(--color-surface-chrome);
    box-shadow: var(--shadow-overlay);
  }
  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-sm);
    height: 32px;
    flex: none;
    margin-top: var(--space-md);
    padding: 0 var(--space-sm);
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--color-accent-subtle);
    color: var(--color-text-primary);
    font: var(--text-label);
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    height: var(--size-pane-header);
  }
</style>
