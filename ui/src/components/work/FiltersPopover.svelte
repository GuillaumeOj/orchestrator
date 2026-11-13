<script>
  // Work › Filters popover (notes §4.2, F3): two columns, applied live; "All trackers" left, one group per tracker
  // kind in scope on the right (only kinds in scope). Footer Reset · Done (Esc). Options come from the cache.
  import { onMount } from 'svelte';
  import { dismissOn } from '../../lib/popover.js';
  import { tooltip } from '../../lib/tooltip.js';
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
  const has = (kind) => groups.some((g) => g.kind === kind);

  function onKey(e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  }
</script>

{#snippet chips(key, list, words = (x) => x)}
  <div class="chips">
    {#each list as v (v)}
      <button type="button" class="tchip" aria-pressed={filter[key].includes(v)} onclick={() => toggle(key, v)}>{words(v)}</button>
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
      <label class="cb"><input type="checkbox" checked={filter.only_ready} onchange={(e) => set({ only_ready: e.currentTarget.checked })} /> <span>Only ready<small>Items that match the Ready filter</small></span></label>
      <label class="cb"><input type="checkbox" checked={filter.only_not_started} onchange={(e) => set({ only_not_started: e.currentTarget.checked })} /> <span>Only not started<small>Nothing running, queued or merged</small></span></label>
      <label class="cb"><input type="checkbox" checked={filter.include_team_tasks} onchange={(e) => set({ include_team_tasks: e.currentTarget.checked })} /> <span>Include team tasks<small>Team tasks in search results</small></span></label>
      {#if has('github') || has('linear')}
        <span class="lbl">Label · has any of</span>
        {@render chips('labels', options.labels)}
        <span class="lbl">Assignee</span>
        {@render chips('assignees', ['@me', ...options.assignees.filter((a) => a !== '@me')], (a) => (a === '@me' ? 'Me' : a))}
      {/if}
    </div>
    {#if groups.length}
      <div class="col">
        {#each groups as g (g.kind)}
          <h4>{g.title}</h4>
          {#if g.kind === 'github'}
            <span class="lbl">Milestone</span>
            <Select aria-label="Milestone" value={filter.milestone ?? ''} options={[{ value: '', label: 'Any' }, ...options.milestones.map((m) => ({ value: m, label: m }))]} onchange={(v) => set({ milestone: v || null })} />
            <span class="lbl">Issue type</span>
            {@render chips('issue_types', options.issue_types)}
          {:else if g.kind === 'linear'}
            <span class="lbl">Cycle</span>
            <Select aria-label="Cycle" value={filter.cycle ?? ''} options={[{ value: '', label: 'Any' }, { value: 'current', label: 'Current' }, { value: 'next', label: 'Next' }]} onchange={(v) => set({ cycle: v || null })} />
            <span class="lbl">Linear project</span>
            {@render chips('linear_projects', options.linear_projects.map((p) => p.id), (id) => options.linear_projects.find((p) => p.id === id)?.name ?? id)}
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
    display: flex;
    gap: var(--space-sm);
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
  .tchip {
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-full);
    background: none;
    color: var(--color-text-secondary);
    font: var(--text-caption);
    cursor: pointer;
  }
  .tchip[aria-pressed='true'] {
    border-color: var(--color-focus);
    background: var(--color-accent-subtle);
    color: var(--color-text-primary);
  }
  .foot {
    display: flex;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-lg);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
  .grow {
    flex: 1;
  }
</style>
