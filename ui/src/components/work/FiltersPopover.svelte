<script>
  // Work › Filters popover (notes §4.2, F3): two columns, applied live; "All trackers" left, one group per tracker
  // kind in scope on the right (only kinds in scope). Footer Reset · Done (Esc). Options come from the cache.
  import { onMount } from 'svelte';
  import { dismissOn } from '../../lib/popover.js';
  import { tooltip } from '../../lib/tooltip.js';
  import { optionList } from '../../lib/tracker.js';
  import Select from '../Select.svelte';

  /**
   * @type {{ filter: any, set: (patch: any) => void, reset: () => void, close: () => void,
   *          groups: { kind: string, title: string }[], options: { labels: string[], assignees: string[], milestones: string[],
   *          issue_types: string[], linear_projects: { id: string, name: string }[] }, anchor?: HTMLElement }}
   */
  let { filter, set, reset, close, groups, options, anchor } = $props();

  /** @type {HTMLElement | undefined} */
  let el = $state();

  onMount(() => {
    /** @type {HTMLElement | null | undefined} */ (el?.querySelector('button, input'))?.focus();
    return dismissOn(() => [el, anchor], (why) => why !== 'scroll' && close());
  });

  const toggle = (key, v) => set({ [key]: filter[key].includes(v) ? filter[key].filter((x) => x !== v) : [...filter[key], v] });
  const CHECKS = [
    ['only_ready', 'Only ready', 'Items that match the Ready filter'],
    ['only_not_started', 'Only not started', 'Nothing running, queued or merged'],
    ['include_team_tasks', 'Include team tasks', 'Team tasks in search results'],
  ];
  const strings = (list) => list.map((v) => ({ value: v, label: v }));

  /** The popover owns its keys: none reaches the tree behind it; Esc closes it. */
  function onKey(e) {
    e.stopPropagation();
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
    }
  }
</script>

{#snippet chips(key, list)}
  <div class="chips">
    {#each list as o (o.value)}
      {@const on = filter[key].includes(o.value)}
      <button type="button" class="filter-chip" class:on aria-pressed={on} onclick={() => toggle(key, o.value)}>{o.label}</button>
    {:else}<span class="muted">Any</span>{/each}
  </div>
{/snippet}

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="pop" role="dialog" aria-label="Filters" tabindex="-1" bind:this={el} onkeydown={onKey}>
  <div class="cols">
    <div class="col">
      <h4>All trackers</h4>
      <span class="lbl">State</span>
      <div class="seg" role="radiogroup" aria-label="State">
        {#each [['open', 'Open'], ['closed', 'Closed'], ['all', 'All']] as [v, label] (v)}
          <button type="button" role="radio" aria-checked={filter.state === v} class:on={filter.state === v} onclick={() => set({ state: v })}>{label}</button>
        {/each}
      </div>
      {#each CHECKS as [key, label, hint] (key)}
        <label class="check cb"><input type="checkbox" checked={filter[key]} onchange={(e) => set({ [key]: e.currentTarget.checked })} /> <span>{label}<small>{hint}</small></span></label>
      {/each}
      {#if groups.length}
        <span class="lbl">Label · has any of</span>
        {@render chips('labels', strings(options.labels))}
        <span class="lbl">Assignee</span>
        {@render chips('assignees', [{ value: '@me', label: 'Me' }, ...strings(options.assignees.filter((a) => a !== '@me'))])}
      {/if}
    </div>
    {#if groups.length}
      <div class="col">
        {#each groups as g (g.kind)}
          <h4>{g.title}</h4>
          {#if g.kind === 'github'}
            <span class="lbl">Milestone</span>
            <Select aria-label="Milestone" value={filter.milestone ?? ''} options={optionList(options.milestones, 'Any')} onchange={(v) => set({ milestone: v || null })} />
            <span class="lbl">Issue type</span>
            {@render chips('issue_types', strings(options.issue_types))}
          {:else if g.kind === 'linear'}
            <span class="lbl">Cycle</span>
            <Select aria-label="Cycle" value={filter.cycle ?? ''} options={[{ value: '', label: 'Any' }, { value: 'current', label: 'Current' }, { value: 'next', label: 'Next' }]} onchange={(v) => set({ cycle: v || null })} />
            <span class="lbl">Linear project</span>
            {@render chips('linear_projects', options.linear_projects.map((p) => ({ value: p.id, label: p.name })))}
          {/if}
        {/each}
      </div>
    {/if}
  </div>
  <div class="foot">
    <button type="button" class="btn" onclick={reset}>Reset</button>
    <span class="grow"></span>
    <button type="button" class="btn primary" use:tooltip={{ label: 'Done', key: 'Esc' }} onclick={close}>Done</button>
  </div>
</div>

<style>
  .pop {
    position: absolute;
    z-index: 20;
    top: 100%;
    left: 0;
    margin-top: var(--space-xs);
    width: min(700px, calc(100vw - 64px));
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
    outline: none;
    font: var(--text-label);
  }
  .cols {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-xl);
    padding: var(--space-md) var(--space-lg);
    overflow: auto;
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    min-width: 0;
  }
  h4 {
    margin: var(--space-xs) 0;
    font: var(--text-body-strong);
  }
  .lbl {
    color: var(--color-text-muted);
    margin-top: var(--space-xs);
  }
  .seg {
    display: inline-flex;
    width: max-content;
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
  }
  .seg button {
    background: none;
    border: 0;
    padding: var(--space-xxs) var(--space-md);
    color: var(--color-text-secondary);
    font: inherit;
    cursor: pointer;
  }
  .seg button.on {
    background: var(--color-accent-subtle);
    color: var(--color-text-primary);
  }
  .cb {
    align-items: flex-start;
  }
  .cb small {
    display: block;
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
    max-height: 120px;
    overflow: auto;
  }
  .filter-chip {
    font: var(--text-caption);
    color: var(--color-text-secondary);
    cursor: pointer;
  }
  .foot {
    display: flex;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-lg);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
</style>
