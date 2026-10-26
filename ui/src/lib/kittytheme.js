// Theme derived from kitty.conf (§9.1): only *surface, foreground and accent*
// come from the kitty palette; every other token is computed from those three,
// and the status colors stay fixed. A contrast check guards the result: if any
// pair is unreadable the derivation is refused and the caller falls back.

/** `[r, g, b]` (0–255) from `#rgb` / `#rrggbb`, else null. A trailing alpha (kitty's background_opacity) is dropped. */
export function parseHex(s) {
  const m = /^#?(?:([0-9a-f]{3})[0-9a-f]?|([0-9a-f]{6})(?:[0-9a-f]{2})?)$/i.exec(String(s ?? '').trim());
  if (!m) return null;
  const h = m[2] ?? [...m[1]].map((c) => c + c).join('');
  return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16));
}

const toHex = (c) => `#${c.map((v) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, '0')).join('')}`;

/** WCAG relative luminance. */
export function luminance([r, g, b]) {
  const f = (v) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

/** WCAG contrast ratio, 1–21. */
export function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** `a` moved `t` (0–1) of the way toward `b`. */
export const mix = (a, b, t) => a.map((v, i) => v + (b[i] - v) * t);

const MIN_TEXT = 4.5; // primary text on surfaces
const MIN_MUTED = 3; // secondary text, accent
const MIN_SECONDARY = 4.5;

/**
 * CSS variables (and the color scheme their status colors pair with) for a
 * kitty theme (xterm.js option names: background, foreground, cursor, blue…),
 * or null when the palette is missing or fails the contrast check.
 */
export function deriveKittyTheme(t) {
  const bg = parseHex(t?.background);
  const fg = parseHex(t?.foreground);
  if (!bg || !fg) return null;
  const dark = luminance(bg) < 0.179; // where white and black give the surface equal contrast
  const toward = dark ? [255, 255, 255] : [0, 0, 0];
  // Accent: the cursor color if it is its own color (often it is just the foreground),
  // else kitty's blue; it must stand out from the background.
  const cursor = parseHex(t.cursor);
  const own = cursor && contrast(cursor, fg) > 1.5 ? cursor : null;
  const accent = [own, parseHex(t.blue), parseHex(t.brightBlue)].find((c) => c && contrast(c, bg) >= MIN_MUTED);
  if (!accent) return null;

  const surface = {
    '--color-surface-pane': bg,
    '--color-surface-chrome': mix(bg, dark ? [0, 0, 0] : [255, 255, 255], 0.25),
    '--color-surface-raised': mix(bg, toward, 0.06),
    '--color-surface-selected': mix(bg, accent, 0.28),
  };
  const raised = surface['--color-surface-raised'];
  const text = {
    '--color-text-primary': fg,
    '--color-text-secondary': mix(fg, bg, 0.15),
    '--color-text-muted': mix(fg, bg, 0.3),
    '--color-text-disabled': mix(fg, bg, 0.55),
    '--color-text-inverse': bg,
  };
  const rest = {
    '--color-border-subtle': mix(bg, fg, 0.16),
    '--color-border-strong': mix(bg, fg, 0.32),
    '--color-accent-default': accent,
    '--color-accent-subtle': mix(bg, accent, 0.28),
    '--color-focus': accent,
  };

  // Contrast check on the pairs the UI actually draws.
  const ok =
    contrast(fg, bg) >= MIN_TEXT &&
    contrast(fg, raised) >= MIN_TEXT &&
    contrast(text['--color-text-secondary'], bg) >= MIN_SECONDARY &&
    contrast(text['--color-text-muted'], raised) >= MIN_MUTED &&
    contrast(accent, bg) >= MIN_MUTED;
  if (!ok) return null;

  const vars = {};
  for (const group of [surface, text, rest]) for (const [k, v] of Object.entries(group)) vars[k] = toHex(v);
  return { scheme: dark ? 'dark' : 'light', vars };
}
