// Tooltip (design-notes §2.5, 02-tooltip.png): `use:tooltip={{ label, key?, detail? }}`.
// One shared element on document.body: 500 ms after hover, at once on keyboard
// focus; hides on leave / blur / Esc / pointer down. Below the control with a 6 px
// gap, flipped above at the bottom of the window, kept inside the window. The
// params may be a function of the node, read when the tooltip shows (a title
// known to be cut only then); a falsy label shows nothing.

export const HOVER_DELAY = 500;
const GAP = 6;
const EDGE = 4;
const TIP_ID = 'orch-tooltip';

let el = null;
let owner = null;

function tipEl() {
  if (el?.isConnected) return el;
  el = document.createElement('div');
  el.className = 'tooltip';
  el.id = TIP_ID;
  el.setAttribute('role', 'tooltip');
  document.body.appendChild(el);
  return el;
}

/** Normalised params: `{ label, key, detail }` or null when there is nothing to show. */
export function tipParams(p, node = null) {
  const v = typeof p === 'function' ? p(node) : p;
  if (!v) return null;
  if (typeof v === 'string') return { label: v, key: null, detail: null };
  return v.label ? { label: v.label, key: v.key ?? null, detail: v.detail ?? null } : null;
}

function fill(t, { label, key, detail }) {
  t.replaceChildren();
  const line = document.createElement('div');
  line.className = 'tip-line';
  line.append(label);
  if (key) {
    const dot = document.createElement('span');
    dot.className = 'tip-dot';
    dot.textContent = '·';
    const k = document.createElement('span');
    k.className = 'kbd';
    k.textContent = key;
    line.append(dot, k);
  }
  t.append(line);
  if (detail) {
    const d = document.createElement('div');
    d.className = 'tip-detail';
    d.textContent = detail;
    t.append(d);
  }
}

/** Where the tooltip goes for a control `r` and a tooltip of `w` × `h` in a `vw` × `vh` window. */
export function placeTip(r, w, h, vw, vh) {
  let top = r.bottom + GAP;
  let above = false;
  if (top + h > vh - EDGE && r.top - GAP - h >= EDGE) {
    top = r.top - GAP - h;
    above = true;
  }
  const left = Math.max(EDGE, Math.min(r.left + r.width / 2 - w / 2, vw - EDGE - w));
  return { top, left, above };
}

/** Adds or drops the tooltip's id in `node`'s aria-describedby, keeping the ids it has of its own. */
function describe(node, on) {
  const ids = (node.getAttribute('aria-describedby') ?? '').split(/\s+/).filter((x) => x && x !== TIP_ID);
  if (on) ids.push(TIP_ID);
  if (ids.length) node.setAttribute('aria-describedby', ids.join(' '));
  else node.removeAttribute('aria-describedby');
}

/** Esc hides the tooltip wherever the keyboard is (a hovered control need not have focus). */
function onDocKey(e) {
  if (e.key === 'Escape' && owner) hide(owner);
}

function show(node, p) {
  if (owner && owner !== node) hide(owner);
  const t = tipEl();
  fill(t, p);
  t.hidden = false;
  const r = node.getBoundingClientRect();
  const { top, left, above } = placeTip(r, t.offsetWidth, t.offsetHeight, window.innerWidth, window.innerHeight);
  t.style.top = `${top}px`;
  t.style.left = `${left}px`;
  t.classList.toggle('above', above);
  owner = node;
  describe(node, true);
  document.addEventListener('keydown', onDocKey, true);
}

function hide(node) {
  if (owner !== node) return;
  owner = null;
  if (el) el.hidden = true;
  describe(node, false);
  document.removeEventListener('keydown', onDocKey, true);
}

/** Svelte action. */
export function tooltip(node, params) {
  let current = params;
  let timer = null;

  const cancel = () => {
    clearTimeout(timer);
    timer = null;
  };
  const open = () => {
    cancel();
    const p = tipParams(current, node);
    if (p) show(node, p);
  };
  const close = () => {
    cancel();
    hide(node);
  };
  const onEnter = () => {
    cancel();
    timer = setTimeout(open, HOVER_DELAY);
  };
  const onFocus = () => {
    // Keyboard focus only: a click focuses too, and a tooltip must never hold a control.
    let keyboard = true;
    try {
      keyboard = node.matches(':focus-visible');
    } catch {}
    if (keyboard) open();
  };

  node.addEventListener('pointerenter', onEnter);
  node.addEventListener('pointerleave', close);
  node.addEventListener('pointerdown', close);
  node.addEventListener('focus', onFocus);
  node.addEventListener('blur', close);

  return {
    update(p) {
      current = p;
      if (owner === node) {
        const n = tipParams(p, node);
        if (n) show(node, n);
        else close();
      }
    },
    destroy() {
      close();
      node.removeEventListener('pointerenter', onEnter);
      node.removeEventListener('pointerleave', close);
      node.removeEventListener('pointerdown', close);
      node.removeEventListener('focus', onFocus);
      node.removeEventListener('blur', close);
    },
  };
}
