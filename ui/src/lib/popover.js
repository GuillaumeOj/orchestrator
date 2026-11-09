// The popover under Select and Menu (design-notes §2.1, §2.2): rendered on
// document.body (a clipped container, the dock or a dialog, never cuts it),
// placed under its trigger or flipped above when there is no room below (the
// dock is at the bottom), and closed on an outside pointer down, an ancestor's
// scroll or the window losing focus.

const GAP = 4;
const EDGE = 8;
/** A popover never gets taller than this (it scrolls past it). */
export const POPOVER_MAX_H = 320;

/** Svelte action: moves the node to document.body while it lives. */
export function portal(node) {
  document.body.appendChild(node);
  return { destroy: () => node.remove() };
}

/**
 * Where a popover of `w` × `h` goes for a trigger rect `r` in a `vw` × `vh` window.
 * `align` 'start' lines up left edges, 'end' right edges. Above when it does not
 * fit below and there is more room above.
 */
export function placePopover(r, w, h, vw, vh, align = 'start') {
  const below = vh - r.bottom - GAP - EDGE;
  const aboveRoom = r.top - GAP - EDGE;
  // Measured before its max-height applies: what shows is at most POPOVER_MAX_H.
  const above = Math.min(h, POPOVER_MAX_H) > below && aboveRoom > below;
  const left = align === 'end' ? r.right - w : r.left;
  return {
    above,
    left: Math.max(EDGE, Math.min(left, vw - EDGE - w)),
    top: above ? null : r.bottom + GAP,
    bottom: above ? vh - r.top + GAP : null,
    maxHeight: Math.max(0, above ? aboveRoom : below),
  };
}

/** Inline style for `placePopover`'s result. */
export function popoverStyle(p, minWidth) {
  const parts = [`left:${p.left}px`, `max-height:${Math.min(POPOVER_MAX_H, p.maxHeight)}px`];
  if (minWidth) parts.push(`min-width:${minWidth}px`);
  parts.push(p.above ? `bottom:${p.bottom}px` : `top:${p.top}px`);
  return parts.join(';');
}

/**
 * Closes on a pointer down outside `inside()` nodes, a scroll that is not inside
 * them, and window blur. Returns the cleanup.
 */
export function dismissOn(inside, close) {
  const within = (t) => inside().some((n) => n?.contains(t));
  const down = (e) => !within(e.target) && close('outside');
  const scroll = (e) => !within(e.target) && close('scroll');
  const blur = () => close('blur');
  document.addEventListener('pointerdown', down, true);
  document.addEventListener('scroll', scroll, true);
  window.addEventListener('blur', blur);
  return () => {
    document.removeEventListener('pointerdown', down, true);
    document.removeEventListener('scroll', scroll, true);
    window.removeEventListener('blur', blur);
  };
}

/** Type-ahead (500 ms buffer): returns `find(key)` → the index of the next label starting with the typed text. */
export function typeahead(labels, isDisabled = (_i) => false, ms = 500) {
  let buf = '';
  let at = 0;
  return (key, from) => {
    const now = Date.now();
    buf = now - at > ms ? key : buf + key;
    at = now;
    const list = labels();
    // The same letter typed again and again cycles through the labels starting with it.
    const same = [...buf].every((c) => c === buf[0]);
    const q = (same ? buf[0] : buf).toLowerCase();
    const n = list.length;
    // A single letter moves on; a longer prefix stays on the current match first.
    const start = same ? from + 1 : from;
    for (let k = 0; k < n; k++) {
      const i = (((start + k) % n) + n) % n;
      if (!isDisabled(i) && String(list[i]).toLowerCase().startsWith(q)) return i;
    }
    return -1;
  };
}
