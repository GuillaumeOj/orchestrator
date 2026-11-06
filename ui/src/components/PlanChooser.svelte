<script>
  // "Plan a feature for…" (P4.5 notes §8.1): the projects to choose from when more than one fits.
  // j/k or arrows move, ⏎ chooses, Esc closes.
  import { app, projectName } from '../lib/app.svelte.js';
  import { planWithOverwatch } from '../lib/intake.svelte.js';

  const ids = $derived(app.overlay?.ids ?? []);
  let i = $state(0);
  let box = $state();
  $effect(() => box?.focus());

  const close = () => (app.overlay = null);
  const choose = (id) => id && planWithOverwatch(id);
  function onKey(e) {
    if (e.key === 'Escape') return (e.preventDefault(), close());
    if (e.key === 'j' || e.key === 'ArrowDown') return (e.preventDefault(), (i = Math.min(ids.length - 1, i + 1)));
    if (e.key === 'k' || e.key === 'ArrowUp') return (e.preventDefault(), (i = Math.max(0, i - 1)));
    if (e.key === 'Enter') return (e.preventDefault(), choose(ids[i]));
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" onclick={close}></div>
<div class="dlg" role="dialog" aria-modal="true" aria-label="Plan a feature for…" tabindex="-1" bind:this={box} onkeydown={onKey}>
  <h3>Plan a feature for…</h3>
  <ul role="listbox" aria-label="Projects">
    {#each ids as id, n (id)}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <li role="option" aria-selected={n === i} class:on={n === i} onmousemove={() => (i = n)} onclick={() => choose(id)}>{projectName(id)}</li>
    {/each}
  </ul>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: var(--color-surface-chrome);
    opacity: var(--opacity-backdrop);
  }
  .dlg {
    position: fixed;
    top: 80px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 51;
    width: min(420px, 92vw);
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    padding: var(--space-lg);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
    font: var(--text-label);
    outline: none;
  }
  h3 {
    margin: 0;
    font: var(--text-title);
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  li {
    padding: var(--space-xs) var(--space-sm);
    border-radius: var(--radius-sm);
    cursor: default;
  }
  li.on {
    background: var(--color-surface-selected);
  }
</style>
