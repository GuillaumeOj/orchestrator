<script>
  // ADR-007 states: glyph shape differs per state and the word is always shown.
  // Model windows add `exhausted` (FR-815).
  import { WINDOW_STATE } from '../lib/pacing.js';

  let { state, shadow = false } = $props();
  const p = $derived(WINDOW_STATE[state] ?? WINDOW_STATE.green);
</script>

<span class="pill" style:border-color={p.color} title={shadow ? 'Shadow mode: decisions are logged, not enforced' : ''}>
  <span style:color={p.color} aria-hidden="true">{p.icon}</span>
  {p.label}{#if shadow}<span class="muted"> · shadow</span>{/if}
</span>

<style>
  .pill {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    height: 18px;
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid;
    border-radius: var(--radius-full);
    font: var(--text-label);
    white-space: nowrap;
  }
</style>
