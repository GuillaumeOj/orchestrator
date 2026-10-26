<script>
  // Toasts (Penpot 05): bottom-right, 6 s, past tense plus the object; undo when it can be undone.
  import { app, dismissToast, undoItem } from '../lib/app.svelte.js';
  import Kbd from './Kbd.svelte';
</script>

<div class="toasts" aria-live="polite">
  {#each app.toasts as t (t.id)}
    <div class="toast" class:error={t.kind === 'error'}>
      <span class="icon" style:color={t.kind === 'error' ? 'var(--color-status-error)' : 'var(--color-status-done)'} aria-hidden="true">{t.kind === 'error' ? '\u{F0159}' : '\u{F012C}'}</span>
      <span class="text">{t.text}</span>
      {#if t.undo}<button class="btn ghost" onclick={() => undoItem(t.item)}>Undo <Kbd keys="u" /></button>{/if}
      {#if t.action}<button class="btn ghost" onclick={() => (dismissToast(t.id), t.action.run())}>{t.action.label}</button>{/if}
      <button class="x" aria-label="Dismiss" onclick={() => dismissToast(t.id)}>×</button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: var(--space-md);
    bottom: calc(var(--size-statusbar) + var(--space-md));
    z-index: 60;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    align-items: flex-end;
  }
  .toast {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    max-width: 520px;
    padding: var(--space-sm) var(--space-sm) var(--space-sm) var(--space-md);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
    font: var(--text-label);
  }
  .toast.error {
    border-color: var(--color-status-error);
  }
  .text {
    flex: 1;
  }
  .x {
    background: none;
    border: 0;
    color: var(--color-text-muted);
  }
</style>
