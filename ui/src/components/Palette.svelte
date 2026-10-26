<script>
  // Command palette (⌘K: every action, scoped to the focused agent first) and
  // fuzzy jump (⌘P: the sidebar tree and open panes).
  import { app, commands, jumpTargets, contextAgent } from '../lib/app.svelte.js';
  import { filter } from '../lib/fuzzy.js';
  import Kbd from './Kbd.svelte';

  let { mode } = $props();
  let query = $state('');
  let i = $state(0);
  let input;
  const scoped = $derived(contextAgent());
  const items = $derived(mode === 'jump' ? filter(query, jumpTargets()) : filter(query, commands()));

  $effect(() => {
    query;
    i = 0;
  });
  $effect(() => input?.focus());

  function close() {
    app.overlay = null;
  }
  function choose(item) {
    close();
    item?.run();
  }
  function onKey(e) {
    if (e.key === 'Escape') return (e.preventDefault(), close());
    if (e.key === 'ArrowDown' || (e.ctrlKey && e.key === 'n')) return (e.preventDefault(), (i = Math.min(items.length - 1, i + 1)));
    if (e.key === 'ArrowUp' || (e.ctrlKey && e.key === 'p')) return (e.preventDefault(), (i = Math.max(0, i - 1)));
    if (e.key === 'Enter') return (e.preventDefault(), choose(items[i]));
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" onclick={close}></div>
<div class="pal" role="dialog" aria-label={mode === 'jump' ? 'Jump' : 'Command palette'}>
  <div class="q">
    <span class="icon" aria-hidden="true">{'\u{F0349}'}</span>
    <input bind:this={input} bind:value={query} onkeydown={onKey} placeholder={mode === 'jump' ? 'Jump to a project, agent or pane' : scoped ? `Command (scoped to ${scoped.id} first)` : 'Run a command'} aria-label="Search" />
    <Kbd keys={mode === 'jump' ? '⌘+P' : '⌘+K'} />
  </div>
  <ul role="listbox">
    {#each items.slice(0, 50) as item, n (item.id ?? item.label)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <li role="option" aria-selected={n === i} class:on={n === i} onmousemove={() => (i = n)} onclick={() => choose(item)}>
        <span class="label">{item.label}</span>
        {#if item.scope === 'agent'}<span class="muted">agent</span>{/if}
        {#if item.detail}<span class="muted">{item.detail}</span>{/if}
        <Kbd keys={item.hint} />
      </li>
    {:else}
      <li class="muted">No match</li>
    {/each}
  </ul>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
  }
  .pal {
    position: fixed;
    top: 72px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 51;
    width: min(640px, 90vw);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
    overflow: hidden;
  }
  .q {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-md);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  input {
    flex: 1;
    border: 0;
    outline: none;
    background: none;
    font: var(--text-body);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: var(--space-xs);
    max-height: 50vh;
    overflow: auto;
  }
  li {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    height: var(--size-row);
    padding: 0 var(--space-sm);
    border-radius: var(--radius-sm);
  }
  li.on {
    background: var(--color-surface-selected);
  }
  .label {
    flex: 1;
  }
</style>
