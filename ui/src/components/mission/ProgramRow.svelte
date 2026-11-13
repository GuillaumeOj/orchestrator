<script>
  // A program row in Mission control's Programs lane (improve-intake notes §7.1, page 02 "Program row"):
  // 52 px, two lines (title, origin, counts in the shared word order, Open, ⋯; progress and estimate),
  // then one 28 px sub-row per feature, expanded by default with ≤ 5 features.
  import { countParts, estText, progressOf, subRowOf } from '../../lib/program.js';
  import { tooltip } from '../../lib/tooltip.js';
  import Menu from '../Menu.svelte';
  import AppChip from '../work/AppChip.svelte';

  /** @type {{ row: any, name?: string, tracker?: string, open?: boolean, liveCap?: string | null, ontoggle?: () => void, onopen?: () => void, onpause?: () => void, oncancel?: () => void, onsource?: () => void, onfeature?: (f: any) => void }} */
  let { row, name = '', tracker = 'GitHub', open = true, liveCap = null, ontoggle = () => {}, onopen = () => {}, onpause = () => {}, oncancel = () => {}, onsource = () => {}, onfeature = () => {} } = $props();

  const parts = $derived(countParts(row));
  const bars = $derived(progressOf(row));
  const paused = $derived(row.status === 'paused');
  const menuItems = $derived([
    { id: 'open', label: 'Open program', key: '⏎', run: () => onopen() },
    { id: 'pause', label: paused ? 'Resume program' : 'Pause program', key: 'p', run: () => onpause() },
    { id: 'source', label: 'Open the source item', key: 'o', run: () => onsource() },
    { id: 'cancel', label: 'Cancel program…', key: '⇧K', run: () => oncancel() },
  ]);
</script>

<div class="pr" data-program={row.id}>
  <div class="l1">
    <button type="button" class="chev" tabindex="-1" aria-expanded={open} aria-label={open ? 'Hide features' : 'Show features'} use:tooltip={{ label: open ? 'Hide features' : 'Show features', key: 'space' }} onclick={(e) => (e.stopPropagation(), ontoggle())}>{open ? '⌄' : '›'}</button>
    <span class="icon" aria-hidden="true">{'\u{F0328}'}</span>
    <b class="title" title={row.title}>{row.title}</b>
    {#if paused}<span data-paused><AppChip chip={{ tone: 'paused', text: 'paused' }} /></span>{/if}
    <span class="from muted">from {row.root_ref}{name ? ` · ${name}` : ''}</span>
    <span class="counts" data-counts>
      {#each parts as p, i (p.key)}{#if i}<span class="muted">{' · '}</span>{/if}<span class:err={p.error}>{p.text}</span>{/each}
    </span>
    <span class="grow"></span>
    <button type="button" class="btn open" tabindex="-1" use:tooltip={{ label: 'Open program', key: '⏎' }} onclick={(e) => (e.stopPropagation(), onopen())}>Open</button>
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <span class="menuwrap" onclick={(e) => e.stopPropagation()}><Menu items={menuItems} tabindex={-1} label="More for {row.title}" /></span>
  </div>
  <div class="l2">
    <span class="bars" role="img" aria-label="{row.counts?.merged ?? 0} of {bars.length} merged">
      {#each bars as on, i (i)}<span class="seg" class:on></span>{/each}
    </span>
    <span class="grow"></span>
    <span class="est muted">{estText(row.estimate)} week</span>
  </div>
  {#if open}
    <ul class="subs" aria-label="Features of {row.title}">
      {#each row.features as f (f.key)}
        {@const s = subRowOf(f, tracker, { liveCap })}
        <li class="sub {s.tone}" data-feature={f.key} data-state={f.state}>
          <span class="g" aria-hidden="true">{s.glyph}</span>
          {#if f.state === 'running' || f.state === 'pr' || f.state === 'failed'}
            <button type="button" class="slug linkish" tabindex="-1" use:tooltip={f.state === 'failed' ? 'Open the feature' : { label: 'Open the feature', key: '⏎' }} onclick={(e) => (e.stopPropagation(), onfeature(f))}>{f.slug}</button>
          {:else}
            <span class="slug">{f.slug}</span>
          {/if}
          <span class="what">{` · ${s.text}`}</span>
          {#if s.open}<span class="what">{' · '}</span><button type="button" class="linkish err" tabindex="-1" onclick={(e) => (e.stopPropagation(), onfeature(f))}>Open</button>{/if}
          {#if s.flag}<span class="flag">{` · ${s.flag}`}</span>{/if}
        </li>
      {/each}
    </ul>
  {:else if row.features.length > 0}
    <button type="button" class="more muted" tabindex="-1" onclick={(e) => (e.stopPropagation(), ontoggle())}>{row.features.length} features ▸</button>
  {/if}
</div>

<style>
  .pr {
    min-width: 0;
    flex: 1;
  }
  .l1,
  .l2 {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
    white-space: nowrap;
  }
  .l1 {
    height: 26px;
  }
  .l2 {
    height: 18px;
    padding-left: calc(var(--size-icon-sm) + var(--space-sm));
  }
  .chev {
    width: var(--size-icon-sm);
    padding: 0;
    border: 0;
    background: none;
    color: var(--color-text-muted);
    font: inherit;
    cursor: pointer;
  }
  .icon {
    color: var(--color-text-muted);
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 0 1 auto;
    min-width: 0;
  }
  .from {
    font: var(--text-mono-sm);
    flex: none;
  }
  .counts {
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    color: var(--color-text-secondary);
  }
  .bars {
    display: inline-flex;
    gap: 3px;
  }
  .seg {
    width: 18px;
    height: 5px;
    border-radius: 2px;
    background: var(--color-border-strong);
  }
  .seg.on {
    background: var(--color-status-done);
  }
  .est {
    font: var(--text-caption);
  }
  .subs {
    margin: var(--space-xs) 0 0;
    padding: 0 0 0 calc(var(--size-icon-sm) + var(--space-md));
    list-style: none;
  }
  .sub {
    display: flex;
    align-items: center;
    height: var(--size-row);
    min-width: 0;
    white-space: nowrap;
    font: var(--text-mono-sm);
    color: var(--color-text-secondary);
  }
  .sub .g {
    width: var(--size-icon-sm);
    flex: none;
  }
  .sub .what {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub.muted {
    color: var(--color-text-muted);
  }
  .sub.working .g {
    color: var(--color-status-working);
  }
  .sub.gate .g {
    color: var(--color-status-at-gate);
  }
  .sub.error,
  .sub.error .what {
    color: var(--color-status-error);
  }
  .flag {
    color: var(--color-pacing-amber);
  }
  .slug {
    color: var(--color-text-primary);
  }
  .sub.muted .slug {
    color: var(--color-text-muted);
  }
  .linkish {
    color: inherit;
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    cursor: pointer;
  }
  .linkish:hover {
    text-decoration: underline;
  }
  .more {
    margin: var(--space-xs) 0 0 calc(var(--size-icon-sm) + var(--space-md));
    padding: 0;
    border: 0;
    background: none;
    font: var(--text-mono-sm);
    cursor: pointer;
  }
</style>
