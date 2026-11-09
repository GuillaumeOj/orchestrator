<script>
  // A list of actions behind a trigger (design-notes §2.2, 02-menu.png): the
  // Select popover holding actions. Items: icon, label, right-aligned key (only
  // where the key works in that place), separator, destructive, disabled with a
  // reason tooltip, checkable (✓), submenu (›). Keyboard: Enter / Space / ↓
  // open on the first item; ↑↓ Home End type-ahead; Enter runs and closes; →
  // opens a submenu, ← closes it; Esc closes and gives focus back to the
  // trigger. The default trigger is the `⋯` icon button, its tooltip the label
  // only (the menu has no key of its own).
  import { tick } from 'svelte';
  import { dismissOn, placePopover, popoverStyle, portal, typeahead } from '../lib/popover.js';
  import { tooltip } from '../lib/tooltip.js';
  import Kbd from './Kbd.svelte';

  /**
   * @typedef {{ id?: string, label?: string, icon?: string, key?: string, destructive?: boolean, disabled?: boolean,
   *             reason?: string, checked?: boolean, items?: any[], separator?: boolean, run?: () => void }} MenuItem
   */
  /** @type {{ items?: MenuItem[], onselect?: (item: MenuItem) => void, label?: string, disabled?: boolean,
   *           trigger?: import('svelte').Snippet<[Record<string, any>]> }} */
  let { items = [], onselect, label = 'More actions', disabled = false, trigger } = $props();

  const uid = $props.id();
  const menuId = `${uid}-menu`;

  let open = $state(false);
  let hi = $state(-1);
  /** The open submenu: the parent row's index and its own highlight. */
  let sub = $state(/** @type {{ i: number, hi: number } | null} */ (null));
  let place = $state(null);
  let subPlace = $state(null);
  /** @type {HTMLElement | undefined} */
  let wrap = $state();
  /** @type {HTMLDivElement | undefined} */
  let pop = $state();
  /** @type {HTMLDivElement | undefined} */
  let subPop = $state();

  const triggerEl = () => /** @type {HTMLElement | null} */ (wrap?.querySelector('button, [role="button"]') ?? null);
  const usable = (list, i) => i >= 0 && i < list.length && !list[i].separator && !list[i].disabled;
  const subItems = $derived(sub ? (items[sub.i]?.items ?? []) : []);

  function next(list, from, delta) {
    for (let i = from + delta; i >= 0 && i < list.length; i += delta) if (usable(list, i)) return i;
    return from;
  }
  const first = (list) => next(list, -1, 1);
  const last = (list) => next(list, list.length, -1);

  const findTop = typeahead(
    () => items.map((x) => x.label ?? ''),
    (i) => !usable(items, i),
  );
  const findSub = typeahead(
    () => subItems.map((x) => x.label ?? ''),
    (i) => !usable(subItems, i),
  );

  function focusRow(level, i) {
    tick().then(() => /** @type {HTMLElement | null | undefined} */ ((level ? subPop : pop)?.querySelector(`[data-i="${i}"]`))?.focus());
  }

  async function openMenu() {
    if (disabled || open) return;
    open = true;
    sub = null;
    hi = first(items);
    await tick();
    const t = triggerEl();
    if (t && pop) {
      const r = t.getBoundingClientRect();
      const h = pop.offsetHeight || items.length * 28 + 8;
      place = placePopover(r, pop.offsetWidth || 200, h, window.innerWidth, window.innerHeight, 'end');
    }
    focusRow(0, hi);
  }

  /** Closes everything; `refocus` gives the keyboard back to the trigger. */
  function close(refocus = true) {
    if (!open) return;
    open = false;
    sub = null;
    if (refocus) triggerEl()?.focus();
  }

  async function openSub(i) {
    if (!items[i]?.items?.length || items[i].disabled) return;
    const list = items[i].items;
    sub = { i, hi: first(list) };
    await tick();
    const row = pop?.querySelector(`[data-i="${i}"]`);
    if (row && subPop) {
      const r = row.getBoundingClientRect();
      const w = subPop.offsetWidth || 200;
      const h = subPop.offsetHeight || list.length * 28 + 8;
      const room = window.innerWidth - r.right;
      const left = room >= w ? r.right + 2 : Math.max(8, r.left - w - 2);
      const top = Math.max(8, Math.min(r.top - 4, window.innerHeight - 8 - h));
      subPlace = { left, top };
    }
    focusRow(1, sub.hi);
  }

  function closeSub() {
    if (!sub) return;
    const i = sub.i;
    sub = null;
    focusRow(0, i);
  }

  function run(item) {
    if (!item || item.disabled || item.separator) return;
    close(true);
    item.run?.();
    onselect?.(item);
  }

  function activate(level, i) {
    const list = level ? subItems : items;
    if (!usable(list, i)) return;
    if (!level && list[i].items?.length) openSub(i);
    else run(list[i]);
  }

  $effect(() => {
    if (!open) return;
    return dismissOn(
      () => [wrap, pop, subPop],
      () => close(false),
    );
  });

  function onTriggerKey(e) {
    if (open || e.metaKey || e.ctrlKey || e.altKey) return;
    if (['ArrowDown', 'Enter', ' '].includes(e.key)) {
      e.preventDefault();
      openMenu();
    }
  }

  function onMenuKey(e, level) {
    const list = level ? subItems : items;
    const cur = level ? (sub?.hi ?? -1) : hi;
    const set = (i) => {
      if (level && sub) sub.hi = i;
      else hi = i;
      focusRow(level, i);
    };
    switch (e.key) {
      case 'ArrowDown':
        set(cur < 0 ? first(list) : next(list, cur, 1));
        break;
      case 'ArrowUp':
        set(cur < 0 ? last(list) : next(list, cur, -1));
        break;
      case 'Home':
        set(first(list));
        break;
      case 'End':
        set(last(list));
        break;
      case 'ArrowRight':
        if (!level) openSub(cur);
        break;
      case 'ArrowLeft':
        if (level) closeSub();
        break;
      case 'Enter':
      case ' ':
        activate(level, cur);
        break;
      case 'Escape':
        if (level) closeSub();
        else close(true);
        break;
      case 'Tab':
        close(false);
        return;
      default:
        if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
          const i = (level ? findSub : findTop)(e.key, cur);
          if (i >= 0) set(i);
          break;
        }
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  }

  function onRowEnter(level, i) {
    const list = level ? subItems : items;
    if (!usable(list, i)) return;
    if (level && sub) sub.hi = i;
    else {
      hi = i;
      if (sub && sub.i !== i) sub = null;
      if (list[i].items?.length) openSub(i);
    }
    focusRow(level, i);
  }

  function onRowClick(e, level) {
    const row = e.target.closest?.('[data-i]');
    if (row) activate(level, Number(row.dataset.i));
  }

  function onRowMove(e, level) {
    const row = e.target.closest?.('[data-i]');
    if (!row) return;
    const i = Number(row.dataset.i);
    if (i !== (level ? sub?.hi : hi)) onRowEnter(level, i);
  }

  const triggerAttrs = $derived({
    'aria-haspopup': 'menu',
    'aria-expanded': open,
    'aria-controls': open ? menuId : undefined,
    onclick: () => (open ? close(true) : openMenu()),
    onkeydown: onTriggerKey,
  });
</script>

{#snippet list(rows, level)}
  {#each rows as it, i (i)}
    {#if it.separator}
      <div class="sep" role="separator"></div>
    {:else}
      <div
        data-i={i}
        class="item"
        class:hi={i === (level ? sub?.hi : hi)}
        class:danger={it.destructive}
        class:off={it.disabled}
        class:sub={!level && sub?.i === i}
        role={it.checked != null ? 'menuitemcheckbox' : 'menuitem'}
        aria-checked={it.checked != null ? !!it.checked : undefined}
        aria-disabled={it.disabled ? true : undefined}
        aria-haspopup={it.items?.length ? 'menu' : undefined}
        aria-expanded={it.items?.length ? !level && sub?.i === i : undefined}
        tabindex="-1"
        use:tooltip={it.disabled ? it.reason : null}
      >
        {#if rows.some((r) => r.checked != null)}<span class="ck" aria-hidden="true">{it.checked ? '\u{F012C}' : ''}</span>{/if}
        {#if it.icon}<span class="ic" aria-hidden="true">{it.icon}</span>{/if}
        <span class="lab">{it.label}</span>
        {#if it.key}<span class="key"><Kbd keys={it.key} /></span>{/if}
        {#if it.items?.length}<span class="more" aria-hidden="true">{'\u{F0142}'}</span>{/if}
      </div>
    {/if}
  {/each}
{/snippet}

<span class="mwrap" bind:this={wrap}>
  {#if trigger}
    {@render trigger(triggerAttrs)}
  {:else}
    <button type="button" class="mtrig" class:pressed={open} aria-label={label} {disabled} use:tooltip={label} {...triggerAttrs}>{'\u{F01D8}'}</button>
  {/if}
</span>

{#if open}
  <div class="pop" id={menuId} role="menu" aria-label={label} tabindex="-1" bind:this={pop} use:portal style={place ? popoverStyle(place) : 'visibility:hidden'} onkeydown={(e) => onMenuKey(e, 0)} onclick={(e) => onRowClick(e, 0)} onpointermove={(e) => onRowMove(e, 0)}>
    {@render list(items, 0)}
  </div>
  {#if sub}
    <div class="pop" role="menu" aria-label={items[sub.i]?.label} tabindex="-1" bind:this={subPop} use:portal style={subPlace ? `left:${subPlace.left}px;top:${subPlace.top}px` : 'visibility:hidden'} onkeydown={(e) => onMenuKey(e, 1)} onclick={(e) => onRowClick(e, 1)} onpointermove={(e) => onRowMove(e, 1)}>
      {@render list(subItems, 1)}
    </div>
  {/if}
{/if}

<style>
  .mwrap {
    display: contents;
  }
  .mtrig {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: var(--size-control-sm);
    height: var(--size-control-sm);
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    font: var(--text-icon);
  }
  .mtrig:hover:not(:disabled) {
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
  }
  .mtrig.pressed {
    background: var(--color-surface-selected);
    color: var(--color-text-primary);
  }
  .mtrig:disabled {
    color: var(--color-text-disabled);
  }

  .ic {
    flex: none;
    width: var(--size-icon-sm);
    color: var(--color-text-secondary);
    font: var(--text-icon);
  }

  .pop {
    position: fixed;
    z-index: 70;
    min-width: 200px;
    max-width: 360px;
    overflow-y: auto;
    padding: var(--space-xs) 0;
    border-radius: var(--radius-md);
    border: var(--border-width-default) solid var(--color-border-strong);
    background: var(--color-surface-pane);
    box-shadow: var(--shadow-overlay);
    color: var(--color-text-primary);
    font: var(--text-body);
    outline: none;
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    height: var(--size-row);
    margin: 0 var(--space-xs);
    padding: 0 var(--space-sm);
    border-radius: var(--radius-sm);
    white-space: nowrap;
    outline: none;
  }
  .item.hi,
  .item.sub {
    background: var(--color-surface-raised);
  }
  .item:focus-visible {
    outline: none;
  }
  .item.danger {
    color: var(--color-status-error);
  }
  .item.danger .ic {
    color: var(--color-status-error);
  }
  .item.danger.hi {
    background: color-mix(in srgb, var(--color-status-error) 20%, transparent);
  }
  .item.off,
  .item.off .ic {
    color: var(--color-text-disabled);
  }
  .ck {
    flex: none;
    width: var(--size-icon-sm);
    color: var(--color-accent-default);
    font: var(--text-icon);
  }
  .lab {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .key {
    margin-left: auto;
    padding-left: var(--space-md);
  }
  .more {
    margin-left: auto;
    color: var(--color-text-secondary);
    font: var(--text-icon);
  }
  .sep {
    height: 1px;
    margin: var(--space-xs) 0;
    background: var(--color-border-subtle);
  }
</style>
