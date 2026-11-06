<script>
  // The Overwatch column's resize handle (allow-overwatch-menu-resizing spec §3,
  // design notes): a 4 px vertical separator on the slot's left edge, straddling
  // the column's 1 px border. Drag left = wider (live); ←/→ ±16 px (⇧ ±64),
  // Home/End min/max, ⏎ or double-click back to the default. Saved on release.
  import { app, chatBounds, chatWidthNow, setChatWidth } from '../../lib/app.svelte.js';

  const STEP = 16;
  const BIG = 64;

  let dragging = $state(false);
  const bounds = $derived(chatBounds());
  const now = $derived(chatWidthNow());

  /** Pointer capture keeps xterm and iframes from taking the drag; the window gets the col-resize cursor and no selection. */
  function drag(e) {
    if (e.button !== 0) return;
    e.preventDefault();
    const el = e.currentTarget;
    const x0 = e.clientX;
    const w0 = chatWidthNow();
    el.setPointerCapture?.(e.pointerId);
    const body = document.body;
    const [cursor, select] = [body.style.cursor, body.style.userSelect];
    body.style.cursor = 'col-resize';
    body.style.userSelect = 'none';
    dragging = true;
    const move = (ev) => {
      const b = chatBounds();
      app.chatWidth = Math.round(Math.min(b.max, Math.max(b.min, w0 + x0 - ev.clientX)));
    };
    const up = () => {
      el.removeEventListener('pointermove', move);
      el.removeEventListener('pointerup', up);
      el.removeEventListener('pointercancel', up);
      body.style.cursor = cursor;
      body.style.userSelect = select;
      dragging = false;
      setChatWidth(app.chatWidth);
    };
    el.addEventListener('pointermove', move);
    el.addEventListener('pointerup', up);
    el.addEventListener('pointercancel', up);
  }

  function key(e) {
    const step = e.shiftKey ? BIG : STEP;
    const b = chatBounds();
    const to = {
      ArrowLeft: now + step,
      ArrowRight: now - step,
      Home: b.min,
      End: b.max,
    }[e.key];
    if (e.key === 'Enter') setChatWidth(null);
    else if (to !== undefined) setChatWidth(Math.min(b.max, Math.max(b.min, to)));
    else return;
    e.preventDefault();
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_tabindex -->
<div
  class="handle resize-handle"
  class:dragging
  role="separator"
  aria-orientation="vertical"
  aria-label="Resize Overwatch"
  aria-valuenow={now}
  aria-valuemin={bounds.min}
  aria-valuemax={bounds.max}
  title="Drag to resize · double-click to reset"
  tabindex="0"
  onpointerdown={drag}
  onkeydown={key}
  ondblclick={() => setChatWidth(null)}
></div>

<style>
  /* Over the column's 1 px border: 1 px on the work column, 2 px on the Overwatch; above the Inspector overlay (z-index 20). */
  .handle {
    position: absolute;
    top: 0;
    bottom: 0;
    left: -1px;
    width: var(--space-xs);
    z-index: 21;
    cursor: col-resize;
    touch-action: none;
  }
</style>
