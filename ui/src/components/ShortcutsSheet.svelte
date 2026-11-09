<script>
  // Keyboard shortcuts (⌘/, Help › Keyboard Shortcuts, the palette; design-notes
  // §7, 04-keyboard-shortcuts-*.png): every real binding in two columns of
  // groups, a search field in the title bar that filters rows by label or key.
  import { closeWindow } from '../lib/app.svelte.js';
  import { filterGroups, GROUPS, LEADER_KEY } from '../lib/shortcuts.js';
  import FloatingWindow from './FloatingWindow.svelte';
  import Kbd from './Kbd.svelte';

  let query = $state('');
  const groups = $derived(filterGroups(GROUPS, query));
</script>

<FloatingWindow title="Keyboard shortcuts" size={{ w: 720, h: 760 }} onclose={() => closeWindow()} restoreFocus={false}>
  {#snippet titleExtra()}
    <label class="search">
      <span class="ic" aria-hidden="true">{'\u{F0349}'}</span>
      <!-- Esc in the field closes the window (it is not an editing field: nothing to lose). -->
      <input class="field" type="search" placeholder="Search keys" aria-label="Search keys" bind:value={query} data-autofocus spellcheck="false" autocomplete="off" />
    </label>
  {/snippet}
  <p class="lead muted">Leader = {LEADER_KEY} · real bindings only · also in Help › Keyboard shortcuts</p>
  {#if groups.length}
    <div class="cols">
      {#each groups as g (g.id)}
        <section class="group" aria-labelledby="sk-{g.id}">
          <h3 id="sk-{g.id}">{g.title}</h3>
          <dl>
            {#each g.rows as r, i (i)}
              <div class="row"><dt>{r.label}</dt><dd><Kbd keys={r.keys} /></dd></div>
            {/each}
          </dl>
        </section>
      {/each}
    </div>
  {:else}
    <p class="muted empty">No shortcut matches “{query.trim()}”</p>
  {/if}
</FloatingWindow>

<style>
  .search {
    position: relative;
    display: flex;
    align-items: center;
    width: 240px;
  }
  .search .ic {
    position: absolute;
    left: var(--space-sm);
    color: var(--color-text-muted);
    font: var(--text-icon);
    pointer-events: none;
  }
  .search input {
    padding-left: calc(var(--space-sm) + var(--space-lg));
  }
  .lead {
    margin: 0 0 var(--space-md);
    font: var(--text-caption);
  }
  .cols {
    column-count: 2;
    column-gap: var(--space-xl);
  }
  .group {
    break-inside: avoid;
    margin-bottom: var(--space-lg);
  }
  h3 {
    margin: 0 0 var(--space-sm);
    font: var(--text-body-strong);
    color: var(--color-text-primary);
  }
  dl {
    margin: 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: var(--size-control-sm);
  }
  dt {
    flex: 1;
    min-width: 0;
    color: var(--color-text-secondary);
    font: var(--text-body);
  }
  dd {
    margin: 0;
  }
  .empty {
    margin: 0;
  }
</style>
