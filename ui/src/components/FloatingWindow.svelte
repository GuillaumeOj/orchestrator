<script module>
  /** Windows never get narrower than this (design-notes §2.4), dialogs pass their own. */
  export const WINDOW_MIN_W = 720;
  const MARGIN = 80;

  /** The window's box in a `vw` × `vh` app: width min(w, vw − 80), never under `minW`; height capped the same way. */
  export function windowSize(size, vw, vh, minW = WINDOW_MIN_W) {
    return {
      w: Math.max(minW, Math.min(size.w, vw - MARGIN)),
      h: size.h == null ? null : Math.min(size.h, vh - MARGIN),
      maxH: vh - MARGIN,
    };
  }

  const FOCUSABLE =
    'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';
</script>

<script>
  // The in-app window (design-notes §2.4, §3.3, 02-floating-window.png):
  // Settings, Project settings, Week planner, Keyboard shortcuts, and the
  // dialogs' shell. Backdrop over the whole app (a click closes), centred
  // window with a title bar and an optional section nav (a vertical tablist:
  // j/k ↑↓ move, Enter goes into the content, Esc in the content comes back to
  // the nav). Focus is trapped (the app root's other children are `inert`, Tab
  // wraps as a fallback) and goes back where it was on close. Esc closes, after
  // an open Select / Menu popover or an editing field had its chance
  // (`defaultPrevented`). `onclose(reason)`: 'esc' | 'close' | 'backdrop'.
  import { onMount, tick } from 'svelte';
  import { tooltip } from '../lib/tooltip.js';

  /** @type {{ title: string, subtitle?: string | null, size?: { w: number, h?: number | null }, minW?: number,
   *           onclose: (reason: 'esc' | 'close' | 'backdrop') => void,
   *           nav?: { id: string, label: string, badge?: string | number }[] | null, section?: string | null,
   *           onsection?: (id: string) => void, onrestore?: () => void,
   *           children?: import('svelte').Snippet, titleExtra?: import('svelte').Snippet }} */
  let {
    title,
    subtitle = null,
    size = { w: 960, h: 760 },
    minW = WINDOW_MIN_W,
    onclose,
    nav = null,
    section = null,
    onsection,
    onrestore,
    children,
    titleExtra,
  } = $props();

  const uid = $props.id();
  const titleId = `${uid}-title`;
  const panelId = `${uid}-panel`;
  const tabId = (id) => `${uid}-tab-${id}`;

  let vw = $state(window.innerWidth);
  let vh = $state(window.innerHeight);
  const box = $derived(windowSize(size, vw, vh, minW));
  const current = $derived(nav?.find((n) => n.id === section)?.id ?? nav?.[0]?.id ?? null);

  /** @type {HTMLDivElement | undefined} */
  let layer = $state();
  /** @type {HTMLDivElement | undefined} */
  let win = $state();
  /** @type {HTMLElement | undefined} */
  let navEl = $state();
  /** @type {HTMLDivElement | undefined} */
  let body = $state();

  const focusables = (root) => /** @type {HTMLElement[]} */ ([...(root?.querySelectorAll(FOCUSABLE) ?? [])]);
  const selectedTab = () => /** @type {HTMLElement | null} */ (navEl?.querySelector('[aria-selected="true"]') ?? null);

  onMount(() => {
    const opener = /** @type {HTMLElement | null} */ (document.activeElement);
    // Everything of the app root before this layer goes inert; what comes after
    // (the palette, the dialogs) stays live above the window.
    const root = document.getElementById('app-root') ?? layer?.parentElement;
    const made = [];
    for (const c of /** @type {HTMLElement[]} */ ([...(root?.children ?? [])])) {
      if (c === layer || c.contains(layer ?? null) || c.hasAttribute('inert')) continue;
      if (layer && c.compareDocumentPosition(layer) & Node.DOCUMENT_POSITION_PRECEDING) continue;
      c.setAttribute('inert', '');
      made.push(c);
    }
    tick().then(() => {
      const first =
        /** @type {HTMLElement | null} */ (win?.querySelector('[data-autofocus]')) ??
        selectedTab() ??
        focusables(body)[0] ??
        win;
      first?.focus();
    });
    return () => {
      for (const c of made) c.removeAttribute('inert');
      if (opener?.isConnected && opener !== document.body) opener.focus();
      else onrestore?.();
    };
  });

  function onWindowKey(e) {
    if (e.key !== 'Escape' || e.defaultPrevented) return;
    const t = /** @type {Node} */ (e.target);
    if (!win?.contains(t) && t !== document.body) return;
    e.preventDefault();
    if (nav && body?.contains(t)) selectedTab()?.focus();
    else onclose('esc');
  }

  /** Tab wraps inside the window (a fallback for `inert`). */
  function onTrapKey(e) {
    if (e.key !== 'Tab') return;
    const all = focusables(win);
    if (!all.length) return;
    const [a, z] = [all[0], all[all.length - 1]];
    if (e.shiftKey && (document.activeElement === a || document.activeElement === win)) {
      e.preventDefault();
      z.focus();
    } else if (!e.shiftKey && document.activeElement === z) {
      e.preventDefault();
      a.focus();
    }
  }

  function pick(id) {
    if (id !== current) onsection?.(id);
    tick().then(() => selectedTab()?.focus());
  }

  function onNavKey(e) {
    if (e.metaKey || e.ctrlKey || e.altKey || !nav?.length) return;
    const i = nav.findIndex((n) => n.id === current);
    const go = (j) => pick(nav[Math.max(0, Math.min(nav.length - 1, j))].id);
    if (e.key === 'j' || e.key === 'ArrowDown') go(i + 1);
    else if (e.key === 'k' || e.key === 'ArrowUp') go(i - 1);
    else if (e.key === 'Home') go(0);
    else if (e.key === 'End') go(nav.length - 1);
    else if (e.key === 'Enter') (focusables(body)[0] ?? body)?.focus();
    else return;
    e.preventDefault();
  }
</script>

<svelte:window bind:innerWidth={vw} bind:innerHeight={vh} onkeydown={onWindowKey} />

<div class="fw-layer" bind:this={layer}>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="backdrop" onclick={() => onclose('backdrop')}></div>
  <div
    class="win"
    bind:this={win}
    role="dialog"
    aria-modal="true"
    aria-labelledby={titleId}
    tabindex="-1"
    style:width="{box.w}px"
    style:height={box.h == null ? null : `${box.h}px`}
    style:max-height="{box.maxH}px"
    onkeydown={onTrapKey}
  >
    <header class="bar">
      <h2 id={titleId} class="title">{title}{#if subtitle}<span class="sub">{` · ${subtitle}`}</span>{/if}</h2>
      {#if titleExtra}<div class="extra">{@render titleExtra()}</div>{/if}
      <button type="button" class="x" aria-label="Close" use:tooltip={{ label: 'Close', key: 'esc' }} onclick={() => onclose('close')}>{'\u{F0156}'}</button>
    </header>
    <div class="main">
      {#if nav}
        <nav class="nav" bind:this={navEl} aria-label="{title} sections">
          <div role="tablist" aria-orientation="vertical" tabindex="-1" onkeydown={onNavKey}>
            {#each nav as n (n.id)}
              <button
                type="button"
                id={tabId(n.id)}
                class="tab"
                role="tab"
                aria-selected={n.id === current}
                aria-controls={panelId}
                tabindex={n.id === current ? 0 : -1}
                onclick={() => pick(n.id)}
              >
                <span class="lab">{n.label}</span>
                {#if n.badge != null}<span class="badge">{n.badge}</span>{/if}
              </button>
            {/each}
          </div>
        </nav>
        <div class="body" bind:this={body} id={panelId} role="tabpanel" aria-labelledby={current ? tabId(current) : undefined} tabindex="-1">
          {@render children?.()}
        </div>
      {:else}
        <div class="body" bind:this={body}>{@render children?.()}</div>
      {/if}
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 45;
    background: var(--color-surface-chrome);
    opacity: var(--opacity-backdrop);
  }
  .win {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 46;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border-radius: var(--radius-lg);
    border: var(--border-width-default) solid var(--color-border-strong);
    background: var(--color-surface-pane);
    box-shadow: var(--shadow-overlay);
    outline: none;
  }
  .bar {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--space-md);
    height: var(--size-titlebar);
    padding: 0 var(--space-xs) 0 var(--space-lg);
    background: var(--color-surface-chrome);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  .title {
    margin: 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: var(--text-title);
    color: var(--color-text-primary);
  }
  .sub {
    font-weight: var(--font-weight-regular);
    color: var(--color-text-secondary);
  }
  .extra {
    display: flex;
    align-items: center;
    min-width: 0;
  }
  .x {
    flex: none;
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--size-control);
    height: var(--size-control);
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    font: var(--text-icon);
  }
  .x:hover {
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
  }
  .main {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .nav {
    flex: none;
    width: 200px;
    padding: var(--space-sm);
    overflow-y: auto;
    background: var(--color-surface-chrome);
    border-right: var(--border-width-default) solid var(--color-border-subtle);
  }
  .nav [role='tablist'] {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    outline: none;
  }
  .tab {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    height: var(--size-row);
    padding: 0 var(--space-md);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    font: var(--text-body);
    text-align: left;
  }
  .tab:hover {
    background: var(--color-surface-raised);
  }
  .tab[aria-selected='true'] {
    background: var(--color-surface-selected);
    color: var(--color-text-primary);
    font: var(--text-body-strong);
  }
  .tab[aria-selected='true']::before {
    content: '';
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--color-accent-default);
  }
  .tab:focus-visible {
    outline-offset: -2px;
  }
  .lab {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .badge {
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .body {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: auto;
    padding: var(--space-xl);
    outline: none;
  }
</style>
