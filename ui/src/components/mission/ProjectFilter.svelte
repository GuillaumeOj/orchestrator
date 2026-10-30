<script>
  // Project filter popover (`f`, notes §1.9, Penpot P4.2 filter): All
  // projects first, each project with the core's counts; j/k move, space
  // toggles, a selects all, ⏎ or Esc applies and closes. The last project
  // cannot be unchecked.
  import { onMount } from 'svelte';
  import { countParts, toggleProject } from '../../lib/mission.js';
  import Kbd from '../Kbd.svelte';

  let { filter, projects, counts, onapply } = $props();
  // svelte-ignore state_referenced_locally
  let draft = $state(filter);
  let cur = $state(0);
  let el = $state();
  const ids = $derived(projects.map((p) => p.id));
  const on = (id) => !draft || draft.includes(id);

  onMount(() => el?.focus());

  /** Applies the choice and closes (the button, ⏎ and Esc refocus the pane; a click away does not). */
  export function apply(opts = {}) {
    onapply(draft, opts);
  }

  function toggle(i) {
    if (i === 0) draft = null;
    else draft = toggleProject(draft, ids[i - 1], ids);
  }

  function onKey(e) {
    const k = e.key;
    const act = (fn) => {
      e.preventDefault();
      e.stopPropagation();
      fn();
    };
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (k === 'j' || k === 'ArrowDown') return act(() => (cur = Math.min(projects.length, cur + 1)));
    if (k === 'k' || k === 'ArrowUp') return act(() => (cur = Math.max(0, cur - 1)));
    if (k === ' ') return act(() => toggle(cur));
    if (k === 'a') return act(() => (draft = null));
    if (k === 'Enter' || k === 'Escape') return act(() => apply({ refocus: true }));
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="pop" role="dialog" aria-label="Filter by project" tabindex="-1" bind:this={el} onkeydown={onKey} onfocusout={(e) => !el.contains(/** @type {Node} */ (e.relatedTarget)) && apply()}>
  <ul role="listbox" aria-multiselectable="true">
    <li role="option" aria-selected={!draft} class:cur={cur === 0}>
      <label><input type="checkbox" tabindex="-1" checked={!draft} onchange={() => toggle(0)} /> <b>All projects</b></label>
    </li>
    {#each projects as p, i (p.id)}
      <li role="option" aria-selected={on(p.id)} class:cur={cur === i + 1}>
        <label><input type="checkbox" tabindex="-1" checked={on(p.id)} onchange={() => toggle(i + 1)} /> {p.name}</label>
        <span class="grow"></span>
        <span class="muted">{countParts({ ...counts?.projects?.[p.id] }).join(' · ')}</span>
      </li>
    {/each}
  </ul>
  <div class="foot muted">Remembered for next time <span class="grow"></span><Kbd keys="j k" /> move <Kbd keys="space" /> toggle <Kbd keys="a" /> all <Kbd keys="⏎" /> apply</div>
</div>

<style>
  .pop {
    position: absolute;
    z-index: 10;
    top: calc(100% + var(--space-xs));
    left: 0;
    min-width: 460px;
    padding: var(--space-xs);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    background: var(--color-surface-raised);
    box-shadow: var(--shadow-overlay);
    font: var(--text-label);
    outline: none;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  li {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-xs) var(--space-sm);
    border-radius: var(--radius-sm);
    white-space: nowrap;
  }
  li.cur {
    background: var(--color-surface-selected);
  }
  label {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    margin-top: var(--space-xs);
    padding: var(--space-xs) var(--space-sm) 0;
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
</style>
