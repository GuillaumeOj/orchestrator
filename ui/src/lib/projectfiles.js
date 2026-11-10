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
  error: '\u{F0159}',
};

/** A project file's access (`write` / `read` are the stored ids): the Select's options and the toast's words. */
export const ACCESS = [
  { value: 'write', label: 'Read & write' },
  { value: 'read', label: 'Read only' },
];

export const accessLabel = (id) => ACCESS.find((a) => a.value === id)?.label ?? id;

/** The access options of one row, each with what it lets agents do (the option's tooltip). */
export const accessOptions = (kind) => [
  { ...ACCESS[0], detail: `Agents can change this ${kind}` },
  { ...ACCESS[1], detail: `Agents can only read this ${kind === 'folder' ? "folder's files" : 'file'}` },
];

/** One path per non-empty line. */
export const pathLines = (text) =>
  String(text ?? '')
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean);

/** The home folder written `~` at the start of a path (or of a core message, which leads with its path). */
export const tilde = (text, home) => {
  const t = String(text ?? '');
  if (!home) return t;
  // Only a whole path component: `/Users/me` must not eat the head of `/Users/media`.
  const h = home.replace(/\/+$/, '').replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return h ? t.replace(new RegExp(`^${h}(?![^/\\s:,;'")\\]])`), '~') : t;
};

/** "~/design/…/app.figcraft": keeps the head and the tail of a long path. */
export function midTruncate(path, max = 44) {
  const p = String(path ?? '');
  if (p.length <= max) return p;
  const tail = Math.ceil((max - 1) * 0.55);
  return `${p.slice(0, max - 1 - tail)}…${p.slice(p.length - tail)}`;
}

export const items = (n) => `${n} item${n === 1 ? '' : 's'}`;

export const addedToast = (n, name) => `Added ${items(n)} to ${name}`;

/** Is a drag position (physical pixels, as Tauri reports it) inside `rect`? */
export function inRect(pos, rect, ratio = 1) {
  if (!pos || !rect || rect.width === 0 || rect.height === 0) return false;
  const x = pos.x / ratio;
  const y = pos.y / ratio;
  return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
}
