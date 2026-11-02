<script>
  // Inspector · decision (notes §5.3): kind and title, subject, lifecycle
  // marker, context blocks (read-only), Show card. It never decides: its card
  // in the chat column owns the undo window.
  import { app, focusCard } from '../../lib/app.svelte.js';
  import { subjectLine } from '../../lib/inbox.js';
  import { decisionKind } from '../../lib/status.js';
  import ContextBlocks from '../ContextBlocks.svelte';
  import Kbd from '../Kbd.svelte';
  import LifecycleMarker from '../LifecycleMarker.svelte';

  let { id } = $props();
  const d = $derived(app.now.decisions[id] ?? null);
  const k = $derived(decisionKind(d));
</script>

{#if d}
  <div class="kind" style:color={k.color}><span aria-hidden="true">{k.icon}</span> {k.label}</div>
  <h3>{d.title}</h3>
  <div class="secondary">{subjectLine(d)}</div>
  <LifecycleMarker decision={d} />
  <ContextBlocks decision={d} />
  <div class="acts">
    <button class="btn" onclick={() => focusCard(d.id)}>Show card <Kbd keys="d" /></button>
  </div>
  <p class="muted">Decide on its card: it owns the undo window.</p>
{:else}
  <p class="muted">This decision is closed.</p>
{/if}

<style>
  .kind {
    display: flex;
    gap: var(--space-xs);
    font: var(--text-label);
  }
  h3 {
    margin: 0;
    font: var(--text-body-strong);
  }
  .acts {
    padding-top: var(--space-md);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
</style>
