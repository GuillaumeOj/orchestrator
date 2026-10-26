<script>
  // Title bar (§9.2): palette entry, the two-window pacing indicator, the Inbox
  // badge. The window's traffic lights sit over its left edge (titleBarStyle Overlay).
  import { app, openSingleton } from '../lib/app.svelte.js';
  import Kbd from './Kbd.svelte';
  import PacingIndicator from './PacingIndicator.svelte';
  const badge = $derived(app.data?.inbox?.badge ?? 0);
</script>

<header class="title" data-tauri-drag-region>
  <div class="lights" data-tauri-drag-region></div>
  <button class="search" onclick={() => (app.overlay = { kind: 'palette' })}>
    <span class="icon" aria-hidden="true">{'\u{F0349}'}</span>
    <span class="muted">Jump or run a command</span>
    <Kbd keys="⌘+K" />
  </button>
  <div class="center" data-tauri-drag-region><PacingIndicator /></div>
  <button class="inbox" class:on={badge > 0} onclick={() => openSingleton('inbox')}>
    {#if badge > 0}<span class="icon" style:color="var(--color-status-waiting-for-you)" aria-hidden="true">{'\u{F0005}'}</span>{/if}
    Inbox <span class="count" class:zero={badge === 0}>{badge > 99 ? '99+' : badge}</span>
    <Kbd keys="⌘+⇧+I" />
  </button>
</header>

<style>
  .title {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-md);
    height: var(--size-titlebar);
    padding: 0 var(--space-md) 0 0;
    background: var(--color-surface-chrome);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-label);
  }
  .lights {
    width: 76px;
    height: 100%;
    flex: none;
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    width: 300px;
    height: 24px;
    padding: 0 var(--space-xs) 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    background: var(--color-surface-pane);
  }
  .search .muted {
    flex: 1;
    text-align: left;
  }
  .center {
    flex: 1;
    display: flex;
    justify-content: center;
    min-width: 0;
  }
  .inbox {
    display: inline-flex;
    align-items: center;
    gap: var(--space-sm);
    height: 24px;
    padding: 0 var(--space-xs) 0 var(--space-sm);
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    font: var(--text-body-strong);
  }
  .inbox:hover,
  .inbox.on {
    background: var(--color-surface-raised);
  }
  .count.zero {
    border-color: var(--color-border-strong);
    color: var(--color-text-muted);
  }
  @media (max-width: 1280px) {
    .search {
      width: 250px;
    }
  }
</style>
