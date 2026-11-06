// Project settings › Files (FR-112): the pure helpers of the section.

/** Material Design glyphs of the icon font (README glyph table, "Project files"). */
export const ICON = {
  file: '\u{F0214}',
  folder: '\u{F024B}',
  reveal: '\u{F0770}',
  editor: '\u{F03CC}',
  remove: '\u{F01B4}',
  dismiss: '\u{F0156}',
  drop: '\u{F0120}',
};

export const ACCESS = [
  { id: 'write', label: 'Read & write' },
  { id: 'read', label: 'Read only' },
];

export const accessLabel = (id) => ACCESS.find((a) => a.id === id)?.label ?? id;

/** One path per non-empty line. */
export const pathLines = (text) =>
  String(text ?? '')
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean);

/** The home folder written `~` wherever it appears in a text (core messages carry paths). */
export const tilde = (text, home) => {
  const t = String(text ?? '');
  return home ? t.split(`${home}/`).join('~/').split(home).join('~') : t;
};

/** "~/design/…/app.figcraft": keeps the head and the tail of a long path. */
export function midTruncate(path, max = 44) {
  const p = String(path ?? '');
  if (p.length <= max) return p;
  const tail = Math.ceil((max - 1) * 0.55);
  return `${p.slice(0, max - 1 - tail)}…${p.slice(p.length - tail)}`;
}

export const addedToast = (n, name) => `Added ${n} item${n === 1 ? '' : 's'} to ${name}`;

/** Is a drag position (physical pixels, as Tauri reports it) inside `rect`? */
export function inRect(pos, rect, ratio = 1) {
  if (!pos || !rect || rect.width === 0 || rect.height === 0) return false;
  const x = pos.x / ratio;
  const y = pos.y / ratio;
  return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
}
