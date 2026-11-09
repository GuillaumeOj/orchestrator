<script>
  // One picker for every choice of a value (design-notes §2.1, 02-select.png):
  // the text-input box with a chevron; a listbox popover on the body with
  // groups, ✓ on the selected row, counts, waiting dots, disabled options with
  // a reason tooltip, and a filter field past 8 options. Keyboard: the listbox
  // pattern (↑↓ Home End, type-ahead, Enter/Space pick, Esc closes and keeps
  // the value; the Esc it consumes is preventDefault'ed so a dialog or window
  // under it does not close too). `onchange` fires only on a user pick.
  import { tick } from 'svelte';
  import { dismissOn, nextUsable, placePopover, popoverStyle, portal, rowIndex, typeahead } from '../lib/popover.js';
  import { tooltip } from '../lib/tooltip.js';

  /**
   * @type {{ value?: any, onchange?: (v: any) => void,
   *          options?: { value: any, label: string, group?: string, count?: number | string, dot?: boolean | string,
   *                      disabled?: boolean, reason?: string, icon?: string, detail?: string }[], placeholder?: string,
   *          compact?: boolean, searchable?: boolean | 'auto', error?: string | null, disabled?: boolean,
   *          id?: string, name?: string, 'aria-label'?: string, 'aria-labelledby'?: string,
   *          width?: 'fill' | 'hug', noun?: string, dot?: boolean | string, bare?: boolean }}
   */
  let {
    value = $bindable(null),
    onchange,
    options = [],
    placeholder = '',
    compact = false,
    searchable = 'auto',
    error = null,
    disabled = false,
    id,
    name,
    'aria-label': ariaLabel,
    'aria-labelledby': ariaLabelledby,
    width,
    noun = 'option',
    dot = false,
    // The closed box shows the label only: no count, no dot of the selected option (its own `dot` still shows).
    bare = false,
  } = $props();

  const uid = $props.id();
  const listId = `${uid}-list`;
  const errId = `${uid}-err`;
  const optId = (i) => `${uid}-opt-${i}`;

  let open = $state(false);
  let query = $state('');
  let hi = $state(-1);
  let place = $state(null);
  /** @type {HTMLButtonElement | undefined} */
  let trigger = $state();
  /** @type {HTMLDivElement | undefined} */
  let pop = $state();
  /** @type {HTMLInputElement | undefined} */
  let search = $state();

  const selected = $derived(options.find((o) => o.value === value) ?? null);
  const withSearch = $derived(searchable === 'auto' ? options.length > 8 : !!searchable);
  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return q ? options.filter((o) => String(o.label).toLowerCase().includes(q)) : options;
  });
  const hug = $derived((width ?? (compact ? 'hug' : 'fill')) === 'hug');
  /** The closed box's dot: its own, or the selected option's (not when `bare`). */
  const boxDot = $derived(dot || (!bare && selected?.dot));

  const enabled = (i) => i >= 0 && i < shown.length && !shown[i].disabled;
  const firstEnabled = () => nextUsable(shown.length, -1, 1, enabled);
  const lastEnabled = () => nextUsable(shown.length, shown.length, -1, enabled);
  /** A dot's accessible name (and tooltip): its own text, else "Needs you". */
  const dotLabel = (d) => (typeof d === 'string' ? d : 'Needs you');

  function startHi() {
    const i = shown.findIndex((o) => o.value === value && !o.disabled);
    return i >= 0 ? i : firstEnabled();
  }

  const step = (delta) => (hi = nextUsable(shown.length, hi, delta, enabled));

  const find = typeahead(
    () => shown.map((o) => o.label),
    (i) => shown[i].disabled,
  );

  /** Opens the list from outside (a palette command), as a click on the box does. */
  export function show() {
    return openList();
  }

  async function openList() {
    if (disabled || open) return;
    query = '';
    open = true;
    hi = startHi();
    await tick();
    measure();
    // The keyboard goes to the filter, else stays on the box (WebKit does not focus a clicked button).
    if (withSearch) search?.focus();
    else trigger?.focus();
    revealHi();
  }

  /** Closes; `refocus` puts the keyboard back on the box (a pick, Esc, Tab from the filter). */
  function close(refocus = false) {
    if (!open) return;
    open = false;
    query = '';
    place = null; // measured afresh on the next open, never shown at a stale spot
    if (refocus) trigger?.focus();
  }

  function pick(i) {
    if (!enabled(i)) return;
    const v = shown[i].value;
    close(true);
    // Re-picking the current value changes nothing, as a native select (no reset of what depends on it).
    if (v === value) return;
    value = v;
    onchange?.(v);
  }

  function measure() {
    if (!trigger) return;
    const r = trigger.getBoundingClientRect();
    const minW = Math.min(360, Math.max(200, r.width));
    const rows = shown.length + new Set(shown.map((o) => o.group).filter(Boolean)).size;
    const guess = rows * 28 + (withSearch ? 40 : 0) + 8;
    const h = pop?.offsetHeight || guess;
    const w = Math.max(pop?.offsetWidth || 0, minW);
    place = { ...placePopover(r, w, h, window.innerWidth, window.innerHeight), minW };
  }

  function revealHi() {
    tick().then(() => pop?.querySelector(`#${CSS.escape(optId(hi))}`)?.scrollIntoView?.({ block: 'nearest' }));
  }

  $effect(() => {
    if (!open) return;
    return dismissOn(
      () => [trigger, pop],
      () => close(false),
    );
  });

  /** Keys shared by the box (no filter) and the filter field. */
  function listKey(e) {
    switch (e.key) {
      case 'ArrowDown':
        step(1);
        break;
      case 'ArrowUp':
        step(-1);
        break;
      case 'Home':
        hi = firstEnabled();
        break;
      case 'End':
        hi = lastEnabled();
        break;
      case 'Enter':
        pick(hi);
        break;
      case 'Escape':
        close(true);
        break;
      default:
        return false;
    }
    e.preventDefault();
    revealHi();
    return true;
  }

  function onTriggerKey(e) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (!open) {
      if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(e.key)) {
        e.preventDefault();
        openList();
      }
      return;
    }
    if (e.key === 'Tab') return close(false);
    if (listKey(e)) return;
    if (e.key === ' ') {
      e.preventDefault();
      pick(hi);
    } else if (e.key.length === 1) {
      const i = find(e.key, hi);
      if (i >= 0) {
        hi = i;
        revealHi();
      }
      e.preventDefault();
    }
  }

  function onSearchKey(e) {
    if (e.key === 'Tab') {
      e.preventDefault();
      return close(true);
    }
    listKey(e);
  }

  function onSearchInput() {
    hi = startHi();
    tick().then(measure);
  }

  function onListMove(e) {
    const i = rowIndex(e);
    if (enabled(i)) hi = i;
  }

  // Keep the keyboard where it is (the box or the filter) when the list is clicked.
  function keepFocus(e) {
    if (e.target.tagName !== 'INPUT') e.preventDefault();
  }
</script>

<span class="sel" class:compact class:hug>
  <button
    bind:this={trigger}
    type="button"
    {id}
    class="box"
    class:open
    class:bad={!!error}
    role="combobox"
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-controls={listId}
    aria-activedescendant={open && enabled(hi) ? optId(hi) : undefined}
    aria-label={ariaLabel}
    aria-labelledby={ariaLabelledby}
    aria-invalid={error ? true : undefined}
    aria-describedby={error ? errId : undefined}
    {disabled}
    onclick={() => (open ? close(true) : openList())}
    onkeydown={onTriggerKey}
  >
    {#if selected?.icon}<span class="ic" aria-hidden="true">{selected.icon}</span>{/if}
    <span class="val" class:ph={!selected}>{selected ? selected.label : placeholder}</span>
    {#if selected?.count != null && !bare}<span class="cnt">{selected.count}</span>{/if}
    {#if boxDot}<span class="dot" role="img" aria-label={dotLabel(boxDot)} use:tooltip={typeof boxDot === 'string' ? boxDot : null}></span>{/if}
    <span class="chev" aria-hidden="true">{open ? '\u{F0143}' : '\u{F0140}'}</span>
  </button>
  <input type="hidden" {name} value={value ?? ''} />
  {#if error}<span class="msg" id={errId} role="alert">{error}</span>{/if}
</span>

{#if open}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="pop popover" class:above={place?.above} bind:this={pop} use:portal style={place ? popoverStyle(place, place.minW) : 'visibility:hidden'} onmousedown={keepFocus}>
    {#if withSearch}
      <div class="search">
        <span class="ic" aria-hidden="true">{'\u{F0349}'}</span>
        <input
          bind:this={search}
          bind:value={query}
          type="text"
          role="searchbox"
          class="q"
          placeholder="Filter {noun}s"
          aria-label="Filter {noun}s"
          aria-controls={listId}
          aria-activedescendant={enabled(hi) ? optId(hi) : undefined}
          spellcheck="false"
          oninput={onSearchInput}
          onkeydown={onSearchKey}
        />
      </div>
    {/if}
    <div class="list" id={listId} role="listbox" tabindex="-1" aria-label={ariaLabel} aria-labelledby={ariaLabelledby} onclick={(e) => pick(rowIndex(e))} onpointermove={onListMove} onkeydown={listKey}>
      {#each shown as o, i (i)}
        {#if o.group && o.group !== shown[i - 1]?.group}
          {#if i > 0}<div class="popover-sep" role="separator"></div>{/if}
          <div class="gh" role="presentation">{o.group}</div>
        {/if}
        <div
          id={optId(i)}
          data-i={i}
          class="opt popover-row"
          class:hi={i === hi}
          class:off={o.disabled}
          role="option"
          aria-selected={o.value === value}
          aria-disabled={o.disabled ? true : undefined}
          use:tooltip={o.disabled ? o.reason : (o.detail ?? null)}
        >
          <span class="ck" aria-hidden="true">{o.value === value ? '\u{F012C}' : ''}</span>
          {#if o.icon}<span class="ic" aria-hidden="true">{o.icon}</span>{/if}
          <span class="lab">{o.label}</span>
          {#if o.dot}<span class="dot" role="img" aria-label={dotLabel(o.dot)}></span>{/if}
          {#if o.count != null}<span class="cnt">{o.count}</span>{/if}
        </div>
      {/each}
    </div>
    {#if withSearch && query.trim() && !shown.length}<p class="none">No {noun} matches “{query.trim()}”</p>{/if}
  </div>
{/if}

<style>
  .sel {
    display: inline-flex;
    flex-direction: column;
    gap: var(--space-xxs);
    min-width: 0;
    vertical-align: middle;
  }
  .sel:not(.hug) {
    display: flex;
    width: 100%;
  }
  .box {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    width: 100%;
    height: var(--size-control);
    padding: 0 var(--space-xs) 0 var(--space-sm);
    border-radius: var(--radius-sm);
    border: var(--border-width-default) solid var(--color-border-strong);
    background: var(--color-surface-pane);
    color: var(--color-text-primary);
    font: var(--text-body);
    text-align: left;
  }
  .box:hover:not(:disabled) {
    background: var(--color-surface-raised);
  }
  .box.open {
    border-color: var(--color-accent-default);
  }
  .box.bad {
    border-color: var(--color-status-error);
  }
  .box:disabled {
    border-color: var(--color-border-subtle);
    color: var(--color-text-disabled);
  }
  .compact .box {
    height: var(--size-control-sm);
    max-width: 220px;
    border-color: var(--color-border-subtle);
    font: var(--text-label);
  }
  .compact .box.open {
    border-color: var(--color-accent-default);
  }
  .hug .box {
    width: auto;
    max-width: 220px;
  }
  .val {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hug .val {
    flex: 0 1 auto;
  }
  .val.ph {
    color: var(--color-text-muted);
  }
  .chev {
    flex: none;
    margin-left: auto;
    color: var(--color-text-secondary);
    font: var(--text-icon);
  }
  .box:disabled .val,
  .box:disabled .chev {
    color: var(--color-text-disabled);
  }
  .ic {
    flex: none;
    color: var(--color-text-muted);
    font: var(--text-icon);
  }
  .cnt {
    flex: none;
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .dot {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: var(--radius-full);
    background: var(--color-status-waiting-for-you);
  }
  .msg {
    color: var(--color-status-error);
    font: var(--text-caption);
  }

  /* The shared .popover box, as a column: the filter field, then the list scrolls. */
  .pop {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    margin: 0 var(--space-xs) var(--space-xs);
    padding: 0 var(--space-sm);
    height: var(--size-control);
    border-radius: var(--radius-sm);
    border: var(--border-width-default) solid var(--color-accent-default);
    background: var(--color-surface-pane);
  }
  .q {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: none;
    background: transparent;
    user-select: text;
    -webkit-user-select: text;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    outline: none;
  }
  .gh {
    padding: var(--space-xs) var(--space-md) var(--space-xxs);
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .opt {
    padding-left: var(--space-xs);
  }
  .ck {
    flex: none;
    width: 12px;
    /* Quieter than the accent (02-select, 02-menu). */
    color: var(--color-text-secondary);
    font: var(--text-icon);
  }
  .lab {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .none {
    margin: 0;
    padding: var(--space-xs) var(--space-md);
    color: var(--color-text-muted);
  }
</style>
