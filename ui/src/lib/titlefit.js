// The per-row 1280 rule at 1512 (docs/design/README improve-intake pass, ii-work-*): when a tree row's
// title would keep under 24 characters, its labels collapse to "+n" and its dependency mark to the glyph.

export const MIN_TITLE_CHARS = 24;

/**
 * Whether a row compacts. `room`: the title's width now; `natural`: its full width; `chars`: its length;
 * `extra`: what the full labels and dependency text take beyond the compact ones (0 when they show).
 */
export function compactRow({ room, natural, chars, extra = 0, min = MIN_TITLE_CHARS }) {
  if (!natural || !chars) return false;
  const full = room - extra;
  if (full >= natural) return false;
  return full / (natural / chars) < min;
}

/**
 * Action on a tree row: measures its `.title` and its `[data-fold]` slots (full labels and dependency text,
 * or their compact forms) at each resize and data change, and calls `set(compact)`. Off: always false.
 * @param {HTMLElement} node
 * @param {{ on: boolean, compact: boolean, set: (v: boolean) => void, key?: any }} opts
 */
export function titleFit(node, opts) {
  let o = opts;
  let fullW = 0;
  let frame = 0;
  const foldW = () => {
    const gap = Number.parseFloat(getComputedStyle(node).columnGap) || 0;
    let w = 0;
    for (const el of node.querySelectorAll('[data-fold]')) w += /** @type {HTMLElement} */ (el).offsetWidth + gap;
    return w;
  };
  const measure = () => {
    frame = 0;
    if (!o.on) return o.compact && o.set(false);
    const title = /** @type {HTMLElement | null} */ (node.querySelector('.title'));
    if (!title) return;
    const w = foldW();
    if (!o.compact) fullW = w;
    const next = compactRow({
      room: title.clientWidth,
      natural: title.scrollWidth,
      chars: title.textContent?.length ?? 0,
      extra: o.compact ? Math.max(0, fullW - w) : 0,
    });
    if (next !== o.compact) o.set(next);
  };
  const schedule = () => {
    if (!frame) frame = requestAnimationFrame(measure);
  };
  const ro = new ResizeObserver(schedule);
  ro.observe(node);
  schedule();
  return {
    update(next) {
      o = next;
      schedule();
    },
    destroy() {
      ro.disconnect();
      if (frame) cancelAnimationFrame(frame);
    },
  };
}
