<script module>
  /** Press-and-hold on ‹ ›: the first repeat after this delay, then every REPEAT ms. */
  export const HOLD_DELAY = 400;
  export const HOLD_REPEAT = 80;
</script>

<script>
  // One tab strip (design-notes §2.3 + R6, 02-tab-strip.png, 03-tab-overflow-*):
  // the workspace's and the dock's. Tabs never shrink (`flex: none`, content
  // width up to 280 px then ellipsis, the full title in a tooltip when cut).
  // On overflow the lane scrolls (wheel / trackpad), a 24 px fade marks a
  // clipped edge, and `‹ ›` at the right end scroll by the lane width minus one
  // tab (press-and-hold repeats); each is disabled at its end. The active tab
  // and a tab just opened are scrolled into view. Keyboard: a roving tabindex
  // (← → Home End move, Enter / Space activates, Delete or ⌘W closes);
  // middle-click closes.
  import { tick } from 'svelte';
  import { tooltip } from '../lib/tooltip.js';

  /**
   * @type {{ tabs?: { id: string, title: string, icon?: string, badge?: string | number | null, closable?: boolean,
   *                    tooltip?: any }[],
   *          active?: string | null, onselect?: (id: string) => void, onclose?: (id: string) => void,
   *          compact?: boolean, label?: string, idPrefix?: string, panelId?: string,
   *          leading?: import('svelte').Snippet, trailing?: import('svelte').Snippet,
   *          tab?: import('svelte').Snippet<[any]> }}
   */
  let {
    tabs = [],
    active = null,
    onselect,
    onclose,
    compact = false,
    label = 'Tabs',
    idPrefix,
    panelId,
    leading,
    trailing,
    tab: tabContent,
  } = $props();

  const uid = $props.id();
  const prefix = $derived(idPrefix ?? uid);
  /** The id of a tab's element (the pane's `aria-labelledby`). */
  const tabId = (id) => `${prefix}-tab-${id}`;

  /** @type {HTMLDivElement | undefined} */
  let lane = $state();
  let overflow = $state(false);
  let atStart = $state(true);
  let atEnd = $state(true);
  let hovered = $state(/** @type {string | null} */ (null));
  let focused = $state(/** @type {string | null} */ (null));
  /** The tab that holds tabindex 0: the last one the keyboard was on, else the active one. */
  let rove = $state(/** @type {string | null} */ (null));
  const tabbable = $derived(tabs.some((t) => t.id === rove) ? rove : tabs.some((t) => t.id === active) ? active : (tabs[0]?.id ?? null));

  function measure() {
    if (!lane) return;
    const max = lane.scrollWidth - lane.clientWidth;
    overflow = max > 1;
    atStart = lane.scrollLeft <= 0;
    atEnd = lane.scrollLeft >= max - 1;
  }

  $effect(() => {
    if (!lane) return;
    const ro = new ResizeObserver(measure);
    ro.observe(lane);
    return () => ro.disconnect();
  });

  const tabEl = (id) => /** @type {HTMLElement | null} */ (lane?.querySelector(`[data-tab="${CSS.escape(id)}"]`) ?? null);

  // The active tab, and any tab just opened, are scrolled into view.
  let known = new Set();
  $effect(() => {
    const ids = tabs.map((t) => t.id);
    const fresh = ids.filter((id) => !known.has(id));
    const first = known.size === 0;
    known = new Set(ids);
    const target = !first && fresh.length ? fresh[fresh.length - 1] : active;
    tick().then(() => {
      if (target) tabEl(target)?.scrollIntoView({ inline: 'nearest', block: 'nearest' });
      measure();
    });
  });

  /** One ‹ or › step: the lane width minus one tab. */
  function scrollStep(dir) {
    if (!lane) return;
    const one = /** @type {HTMLElement | null} */ (lane.querySelector('[role="tab"]'))?.offsetWidth ?? 0;
    const by = Math.max(lane.clientWidth - one, 1);
    const max = lane.scrollWidth - lane.clientWidth;
    lane.scrollLeft = Math.max(0, Math.min(max, lane.scrollLeft + dir * by));
    measure();
    // At the end the arrow turns disabled (or goes with the overflow) and may never
    // see its pointerup: the hold stops here instead of repeating for ever.
    if (dir < 0 ? atStart : atEnd) stopHold();
  }

  let holdTimer = null;
  function stopHold() {
    clearTimeout(holdTimer);
    clearInterval(holdTimer);
    holdTimer = null;
  }
  function startHold(e, dir) {
    if (e.button !== 0) return;
    e.preventDefault();
    stopHold();
    scrollStep(dir);
    holdTimer = setTimeout(() => {
      holdTimer = setInterval(() => scrollStep(dir), HOLD_REPEAT);
    }, HOLD_DELAY);
  }
  // A keyboard click (detail 0); pointer clicks already scrolled on pointer down.
  const keyClick = (e, dir) => e.detail === 0 && scrollStep(dir);

  $effect(() => stopHold);

  function focusTab(id) {
    rove = id;
    tabEl(id)?.focus();
  }

  function onLaneKey(e) {
    const el = /** @type {HTMLElement} */ (e.target).closest?.('[role="tab"]');
    if (!el) return;
    const id = /** @type {HTMLElement} */ (el).dataset.tab;
    const i = tabs.findIndex((t) => t.id === id);
    const t = tabs[i];
    if (!t) return;
    if (e.key === 'ArrowRight') focusTab(tabs[Math.min(tabs.length - 1, i + 1)].id);
    else if (e.key === 'ArrowLeft') focusTab(tabs[Math.max(0, i - 1)].id);
    else if (e.key === 'Home') focusTab(tabs[0].id);
    else if (e.key === 'End') focusTab(tabs[tabs.length - 1].id);
    else if (e.key === 'Enter' || e.key === ' ') onselect?.(t.id);
    else if (e.key === 'Delete' || (e.key === 'w' && (e.metaKey || e.ctrlKey))) {
      if (t.closable === false) return;
      e.stopPropagation();
      const after = tabs[i + 1] ?? tabs[i - 1];
      onclose?.(t.id);
      if (after) tick().then(() => focusTab(after.id));
    } else return;
    e.preventDefault();
  }

  function onAux(e, t) {
    if (e.button !== 1 || t.closable === false) return;
    e.preventDefault();
    onclose?.(t.id);
  }

  /** The tooltip: the tab's own, else the full title when the label is cut. */
  function tip(t) {
    return (node) => {
      if (t.tooltip) return t.tooltip;
      const name = node?.querySelector('.name');
      return name && name.scrollWidth > name.clientWidth ? t.title : null;
    };
  }
</script>

<div class="strip" class:compact>
  {#if leading}<div class="lead">{@render leading()}</div>{/if}
  <div class="lanewrap" class:fade-l={overflow && !atStart} class:fade-r={overflow && !atEnd}>
    <div class="lane" bind:this={lane} role="tablist" aria-orientation="horizontal" aria-label={label} tabindex="-1" onscroll={measure} onkeydown={onLaneKey}>
      {#each tabs as t (t.id)}
        {@const on = t.id === active}
        <!-- Keys are handled once on the tablist (onLaneKey). -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <div
          id={tabId(t.id)}
          data-tab={t.id}
          class="tab"
          class:on
          role="tab"
          aria-selected={on}
          aria-controls={panelId}
          tabindex={t.id === tabbable ? 0 : -1}
          use:tooltip={tip(t)}
          onclick={() => onselect?.(t.id)}
          onauxclick={(e) => onAux(e, t)}
          onmousedown={(e) => e.button === 1 && e.preventDefault()}
          onpointerenter={() => (hovered = t.id)}
          onpointerleave={() => hovered === t.id && (hovered = null)}
          onfocus={() => ((focused = t.id), (rove = t.id))}
          onblur={() => focused === t.id && (focused = null)}
        >
          {#if tabContent}
            {@render tabContent(t)}
          {:else}
            {#if t.icon}<span class="ic" aria-hidden="true">{t.icon}</span>{/if}
            <span class="name">{t.title}</span>
            {#if t.badge != null}<span class="badge">{t.badge}</span>{/if}
          {/if}
          {#if t.closable !== false}
            <button
              type="button"
              class="x"
              tabindex="-1"
              aria-label="Close {t.title}"
              style:visibility={on || hovered === t.id || focused === t.id ? 'visible' : 'hidden'}
              onclick={(e) => (e.stopPropagation(), onclose?.(t.id))}>{'\u{F0156}'}</button
            >
          {/if}
        </div>
      {/each}
    </div>
  </div>
  {#if overflow}
    <div class="arrows">
      <button type="button" class="arrow" aria-label="Scroll tabs left" disabled={atStart} use:tooltip={'Scroll tabs left'} onpointerdown={(e) => startHold(e, -1)} onpointerup={stopHold} onpointerleave={stopHold} onpointercancel={stopHold} onclick={(e) => keyClick(e, -1)}>{'\u{F0141}'}</button>
      <button type="button" class="arrow" aria-label="Scroll tabs right" disabled={atEnd} use:tooltip={'Scroll tabs right'} onpointerdown={(e) => startHold(e, 1)} onpointerup={stopHold} onpointerleave={stopHold} onpointercancel={stopHold} onclick={(e) => keyClick(e, 1)}>{'\u{F0142}'}</button>
    </div>
  {/if}
  {#if trailing}<div class="trail">{@render trailing()}</div>{/if}
</div>

<style>
  .strip {
    --strip-h: var(--size-pane-header);
    display: flex;
    align-items: stretch;
    height: var(--strip-h);
    min-width: 0;
    background: var(--color-surface-chrome);
    box-shadow: inset 0 -1px 0 var(--color-border-subtle);
  }
  .strip.compact {
    --strip-h: var(--size-strip);
  }
  .lead,
  .trail {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    padding: 0 var(--space-sm);
  }
  .lead {
    border-right: var(--border-width-default) solid var(--color-border-subtle);
  }
  .lanewrap {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
  }
  /* The fade on a clipped edge (R6: nothing drawn between it and ‹ ›). */
  .lanewrap::before,
  .lanewrap::after {
    content: '';
    position: absolute;
    top: 0;
    bottom: 1px;
    width: 24px;
    pointer-events: none;
    opacity: 0;
    z-index: 1;
  }
  .lanewrap::before {
    left: 0;
    background: linear-gradient(to right, var(--color-surface-chrome), transparent);
  }
  .lanewrap::after {
    right: 0;
    background: linear-gradient(to left, var(--color-surface-chrome), transparent);
  }
  .lanewrap.fade-l::before,
  .lanewrap.fade-r::after {
    opacity: 1;
  }
  .lane {
    flex: 1;
    min-width: 0;
    display: flex;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    outline: none;
  }
  .lane::-webkit-scrollbar {
    display: none;
  }
  .tab {
    position: relative;
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: min-content;
    max-width: 280px;
    padding: 0 var(--space-xs) 0 var(--space-md);
    color: var(--color-text-secondary);
    font: var(--text-label);
    white-space: nowrap;
    cursor: default;
  }
  .tab:hover {
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
  }
  .tab.on {
    background: var(--color-surface-pane);
    color: var(--color-text-primary);
  }
  .tab.on::before {
    content: '';
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    height: 2px;
    background: var(--color-accent-default);
  }
  .tab:focus-visible {
    outline-offset: -2px;
  }
  .ic {
    flex: none;
    color: var(--color-text-muted);
    font: var(--text-icon);
  }
  .tab.on .ic,
  .tab:hover .ic {
    color: var(--color-text-secondary);
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .badge {
    flex: none;
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .x {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    font: var(--text-icon);
  }
  .x:hover {
    background: var(--color-surface-selected);
    color: var(--color-text-primary);
  }
  /* Opaque, so the clipped tab ends at their left edge. */
  .arrows {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--space-xxs);
    padding: 0 var(--space-xs);
    background: var(--color-surface-chrome);
  }
  .arrow {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--size-control-sm);
    height: var(--size-control-sm);
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    font: var(--text-icon);
  }
  .arrow:hover:not(:disabled) {
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
  }
  .arrow:disabled {
    color: var(--color-text-disabled);
  }
</style>
