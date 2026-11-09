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
  import { dismissOn, nextUsable, placePopover, placeSubmenu, popoverStyle, portal, rowIndex, typeahead } from '../lib/popover.js';
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

  const next = (list, from, delta) => nextUsable(list.length, from, delta, (i) => usable(list, i));
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

  /** One level of the menu (0 the menu, 1 its open submenu): its rows, highlight and type-ahead. */
  function lvl(level) {
    return level
      ? { list: subItems, cur: sub?.hi ?? -1, put: (i) => sub && (sub.hi = i), find: findSub }
      : { list: items, cur: hi, put: (i) => (hi = i), find: findTop };
  }

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
    place = null; // measured afresh on the next open, never shown at a stale spot
    subPlace = null;
    if (refocus) triggerEl()?.focus();
  }

  /** Opens row `i`'s submenu; `focus` (the keyboard) moves into it, a hover leaves the keyboard on the row. */
  async function openSub(i, focus = true) {
    if (!items[i]?.items?.length || items[i].disabled) return;
    const list = items[i].items;
    if (sub?.i !== i) subPlace = null;
    sub = { i, hi: focus ? first(list) : -1 };
    await tick();
    const row = pop?.querySelector(`[data-i="${i}"]`);
    if (row && subPop) {
      const h = subPop.offsetHeight || list.length * 28 + 8;
      subPlace = placeSubmenu(row.getBoundingClientRect(), subPop.offsetWidth || 200, h, window.innerWidth, window.innerHeight);
    }
    if (focus && sub) focusRow(1, sub.hi);
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
    const { list } = lvl(level);
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
    const { list, cur, put, find } = lvl(level);
    const set = (i) => {
      put(i);
      focusRow(level, i);
    };
    switch (e.key) {
      case 'ArrowDown':
        set(next(list, cur, 1));
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
        // Back on the trigger first, so the Tab moves on from there (the menu is on the body).
        close(true);
        return;
      default:
        if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
          const i = find(e.key, cur);
          if (i >= 0) set(i);
          break;
        }
        return;
    }
    // The window or dialog under the menu checks `defaultPrevented` (design-notes §2.1).
    e.preventDefault();
  }

  /** The pointer over row `i`: it takes the highlight (and opens its submenu without the keyboard). */
  function onRowMove(e, level) {
    const i = rowIndex(e);
    const { list, cur, put } = lvl(level);
    if (i === cur || !usable(list, i)) return;
    put(i);
    if (!level) {
      if (sub && sub.i !== i) sub = null;
      if (list[i].items?.length && sub?.i !== i) openSub(i, false);
    }
    focusRow(level, i);
  }

  const toggle = () => (open ? close(true) : openMenu());
  const triggerAttrs = $derived({
    'aria-haspopup': 'menu',
    'aria-expanded': open,
    'aria-controls': open ? menuId : undefined,
    onclick: toggle,
    onkeydown: onTriggerKey,
  });
</script>

{#snippet list(rows, level)}
  {@const checkable = rows.some((r) => r.checked != null)}
  {@const cur = lvl(level).cur}
  {#each rows as it, i (i)}
    {#if it.separator}
      <div class="popover-sep" role="separator"></div>
    {:else}
      <div
        data-i={i}
        class="item popover-row"
        class:hi={i === cur}
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
        {#if checkable}<span class="ck" aria-hidden="true">{it.checked ? '\u{F012C}' : ''}</span>{/if}
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
    <button type="button" class="icon-btn" class:pressed={open} aria-label={label} {disabled} use:tooltip={label} {...triggerAttrs}>{'\u{F01D8}'}</button>
  {/if}
</span>

{#if open}
  <div class="pop popover" id={menuId} role="menu" aria-label={label} tabindex="-1" bind:this={pop} use:portal style={place ? popoverStyle(place) : 'visibility:hidden'} onkeydown={(e) => onMenuKey(e, 0)} onclick={(e) => activate(0, rowIndex(e))} onpointermove={(e) => onRowMove(e, 0)}>
    {@render list(items, 0)}
  </div>
  {#if sub}
    <div class="pop popover" role="menu" aria-label={items[sub.i]?.label} tabindex="-1" bind:this={subPop} use:portal style={subPlace ? `left:${subPlace.left}px;top:${subPlace.top}px` : 'visibility:hidden'} onkeydown={(e) => onMenuKey(e, 1)} onclick={(e) => activate(1, rowIndex(e))} onpointermove={(e) => onRowMove(e, 1)}>
      {@render list(subItems, 1)}
    </div>
  {/if}
{/if}

<style>
  .mwrap {
    display: contents;
  }

  .ic {
    flex: none;
    width: var(--size-icon-sm);
    color: var(--color-text-secondary);
    font: var(--text-icon);
  }

  .pop {
    min-width: 200px;
  }
  .item.sub {
    background: var(--color-surface-raised);
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
</style>
