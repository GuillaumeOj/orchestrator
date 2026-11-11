<script>
  // The @mention list above the Overwatch input (design-notes §1–3, §7; ui section for the
  // highlighted-row colours and the list height). Focus never leaves the textarea: the
  // input owns the keys and the combobox attributes, this only draws the rows.
  import { runStateColor, runStateLabel } from '../../lib/teams.js';

  /** @type {{ current: any[], other: any[], active: number, query: string, none: boolean, id?: string, optionId: (i: number) => string, onpick: (row: any) => void, onhover: (i: number) => void }} */
  let { current, other, active, query, none, id = 'ow-mention-list', optionId, onpick, onhover } = $props();
  const rows = $derived([...current, ...other]);
  const headers = $derived(current.length > 0 && other.length > 0);
  const groups = $derived(
    [
      { key: 'current', label: 'Current', rows: current, from: 0 },
      { key: 'other', label: 'Other features', rows: other, from: current.length },
    ].filter((g) => g.rows.length),
  );
  const echo = $derived(query.length > 24 ? `${query.slice(0, 24)}…` : query);

  $effect(() => {
    if (rows.length) document.getElementById(optionId(active))?.scrollIntoView?.({ block: 'nearest' });
  });
</script>

{#snippet runs(list)}{#each list as s}{#if s.on}<b>{s.t}</b>{:else}{s.t}{/if}{/each}{/snippet}

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="popup" data-mention-popup onmousedown={(e) => e.preventDefault()}>
  {#if rows.length}
    <div class="list" role="listbox" {id} aria-label="Features" class:headed={headers}>
      {#each groups as g (g.key)}
        <div class="group" role="group" aria-labelledby={headers ? `ow-mention-h-${g.key}` : undefined}>
          {#if headers}<div class="head" data-mention-head id="ow-mention-h-{g.key}" aria-hidden="true">{g.label}</div>{/if}
          {#each g.rows as r, i (r.id)}
            {@const n = g.from + i}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div
              class="row"
              class:on={n === active}
              id={optionId(n)}
              role="option"
              tabindex="-1"
              aria-selected={n === active}
              aria-label="{r.f.slug}, {r.f.title ?? ''}, {r.project}, {r.f.run ? runStateLabel(r.f.run.status) : 'No run'}"
              data-mention-row={r.id}
              onmousemove={() => n !== active && onhover(n)}
              onclick={() => onpick(r)}
            >
              <span class="slug" aria-hidden="true">@{@render runs(r.slug)}</span>
              <span class="title" aria-hidden="true">{@render runs(r.title)}</span>
              <span class="proj" aria-hidden="true">{@render runs(r.proj)}</span>
              {#if r.f.run}
                <span class="state" aria-hidden="true"><i style:background={runStateColor(r.f.run.status)}></i>{runStateLabel(r.f.run.status)}</span>
              {:else}
                <span class="state norun" aria-hidden="true">No run</span>
              {/if}
            </div>
          {/each}
        </div>
      {/each}
    </div>
    <div class="foot muted" data-mention-foot aria-hidden="true">↑↓ to choose · ⏎ to insert · esc to close</div>
  {:else}
    <div class="none muted" data-mention-none>{none ? 'No features yet' : `No feature matches “${echo}”`}</div>
  {/if}
</div>

<style>
  .popup {
    position: absolute;
    left: 0;
    right: 0;
    bottom: calc(100% + var(--border-width-default));
    z-index: 5;
    container-type: inline-size;
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
  }
  .list {
    /* 8 rows, the group headers, the padding; the ⌘P list's 50vh cap. */
    max-height: min(calc(8 * 32px + 2 * var(--space-xs)), 50vh);
    overflow-y: auto;
    padding: var(--space-xs);
  }
  .list.headed {
    max-height: min(calc(8 * 32px + 2 * 20px + 2 * var(--space-xs)), 50vh);
  }
  .head {
    height: 20px;
    padding-left: var(--space-sm);
    color: var(--color-text-muted);
    font: var(--text-caption);
    line-height: 20px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: 32px;
    padding: 0 var(--space-sm);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .row.on {
    background: var(--color-surface-selected);
  }
  .slug {
    flex: 0 1 auto;
    min-width: 8ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-secondary);
    font: var(--text-mono);
  }
  .slug b,
  .title b,
  .proj b {
    color: var(--color-text-primary);
    font-weight: var(--font-weight-semibold);
    text-decoration: underline;
    text-decoration-thickness: 1px;
    text-underline-offset: 2px;
  }
  .title {
    flex: 1 1 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-muted);
    font: var(--text-body);
  }
  .proj {
    flex: none;
    max-width: 12ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    flex: none;
    color: var(--color-text-secondary);
    font: var(--text-caption);
    white-space: nowrap;
  }
  .state i {
    width: var(--size-status-dot);
    height: var(--size-status-dot);
    border-radius: 50%;
  }
  .state.norun {
    color: var(--color-text-muted);
  }
  /* The highlighted row: the muted parts go secondary (contrast on surface-selected). */
  .row.on .title,
  .row.on .proj,
  .row.on .state.norun {
    color: var(--color-text-secondary);
  }
  .foot {
    height: 24px;
    padding-left: var(--space-sm);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-caption);
    line-height: 24px;
  }
  .none {
    display: flex;
    align-items: center;
    min-height: 32px;
    margin: var(--space-xs);
    padding: 0 var(--space-sm);
    font: var(--text-body);
  }
  /* A 300 px column: the title and the footer go; the slug, project and state stay. */
  @container (max-width: 337px) {
    .title,
    .foot {
      display: none;
    }
  }
</style>
