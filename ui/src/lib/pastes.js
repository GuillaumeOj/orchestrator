// Pasted long texts in the Overwatch input and thread (allow-long-text-in-overwatch spec §3.2,
// §3.3, api-contract §0, design-notes). Pure: tested in test/pastes.test.js.
//
// A long plain-text paste becomes a placeholder token `[Pasted text #<n> · <chars> chars]` in the
// draft; the text itself is kept in `app.chatPastes = [{ n, text, chars, lines }]` and sent as
// `context.pastes = [{ n, text }]` for the placeholders still whole at send.

export const PASTE_MIN_CHARS = 800; // a paste of more than this, or ...
export const PASTE_MIN_LINES = 10; // ... of more lines than this, becomes a placeholder
export const MAX_PASTE_CHARS = 200000;
export const MAX_PASTES = 10;
export const MAX_N = 99;

/** `3 214`: a plain space groups the thousands, like the input's counter. */
export const spaced = (x) => x.toLocaleString('en-US').replaceAll(',', ' ');

/** CRLF and a lone CR become LF (api-contract §0). */
export const normalize = (text) => text.replace(/\r\n?/g, '\n');

/** Code points. */
export const countChars = (text) => {
  let n = 0;
  for (const _ of text) n++;
  return n;
};

/** Rust `str::lines().count()` on normalised text: `""` 0, `"a"` 1, `"a\n"` 1, `"a\nb"` 2. */
export function countLines(text) {
  if (!text) return 0;
  const parts = text.split('\n');
  if (parts[parts.length - 1] === '') parts.pop();
  return parts.length;
}

/** A normalised paste long enough to be a placeholder: over 800 characters or over 10 lines. */
export const isLong = (text) =>
  (text.length > PASTE_MIN_CHARS && countChars(text) > PASTE_MIN_CHARS) || countLines(text) > PASTE_MIN_LINES;

/** The token for paste `n` of `chars` code points. */
export const placeholder = (n, chars) => `[Pasted text #${n} · ${spaced(chars)} chars]`;

const SPACES = '[\\d \\u00a0\\u202f]+';
const tokenRe = () => new RegExp(`\\[Pasted text #([1-9]\\d?) · ${SPACES} chars\\]`, 'g');

/** Every placeholder-shaped token of a text: `{ start, end, n, token }`. */
export function placeholdersIn(text) {
  const out = [];
  for (const m of text.matchAll(tokenRe()))
    out.push({ start: m.index, end: m.index + m[0].length, n: Number(m[1]), token: m[0] });
  return out;
}

/** The placeholders that are whole and unchanged: the token is exactly the one of a kept paste. */
export function wholeIn(text, pastes) {
  const byN = new Map((pastes ?? []).map((p) => [p.n, p]));
  return placeholdersIn(text).filter((t) => {
    const p = byN.get(t.n);
    return p && t.token === placeholder(p.n, p.chars);
  });
}

/** The numbers of the kept pastes whose placeholder is still whole in `text`. */
export const referenced = (text, pastes) => new Set(wholeIn(text, pastes).map((t) => t.n));

/** The next number: above every kept paste (referenced or not, for ⌘Z), 1 after a send. */
export const nextN = (pastes) => Math.max(0, ...(pastes ?? []).map((p) => p.n)) + 1;

/** The error for a long paste of `chars` code points that cannot be inserted, or ''. */
export function pasteError(text, chars, pastes) {
  if (chars > MAX_PASTE_CHARS)
    return `Not pasted: ${spaced(chars)} characters, at most ${spaced(MAX_PASTE_CHARS)} in a pasted text.`;
  if (referenced(text, pastes).size >= MAX_PASTES || nextN(pastes) > MAX_N)
    return `Not pasted: at most ${MAX_PASTES} pasted texts in one message.`;
  return '';
}

/** The kept pastes after adding one: the referenced ones and the 10 most recent others (⌘Z can bring them back). */
export function keep(text, pastes, added) {
  const refs = referenced(text, pastes);
  const loose = pastes.filter((p) => !refs.has(p.n)).sort((a, b) => b.n - a.n);
  const dropped = new Set(loose.slice(MAX_PASTES).map((p) => p.n));
  return [...pastes.filter((p) => !dropped.has(p.n)), added];
}

/** The kept pastes a send carries: those still referenced, by number, at most 10 (⌘Z can bring an 11th back). */
export function pastesToSend(text, pastes) {
  const refs = referenced(text, pastes);
  return (pastes ?? [])
    .filter((p) => refs.has(p.n))
    .sort((a, b) => a.n - b.n)
    .slice(0, MAX_PASTES);
}

/** The `context.pastes` of `chat_send` for those pastes. */
export const pastesContext = (pastes) => pastes.map((p) => ({ n: p.n, text: p.text }));

/** The whole placeholder a Backspace (`back`) just after it or a Delete just before it removes: `{ start, end, n }` or null. */
export function wholeAt(text, caret, dir, pastes) {
  return wholeIn(text, pastes).find((t) => (dir === 'back' ? t.end === caret : t.start === caret)) ?? null;
}

/** Is the `@` at `at` typed inside a whole placeholder (then it opens no mention popup)? */
export function atInPlaceholder(text, at, pastes) {
  if (text[at] !== '@') return false;
  const without = text.slice(0, at) + text.slice(at + 1);
  return wholeIn(without, pastes).some((t) => t.start < at && at < t.end);
}

/** `3 214 chars` style piece of speech: `1 line`, `120 lines`. */
const count = (n, one, many) => `${spaced(n)} ${n === 1 ? one : many}`;

/** The chip's three tiers (design-notes §2): `full`, `mid` (no lines), `short`. Pieces for the CSS tiers. */
export const chipParts = (p) => ({
  n: `#${p.n}`,
  chars: `${spaced(p.chars)} chars`,
  lines: count(p.lines, 'line', 'lines'),
});

/** The chip's accessible name. */
export const chipName = (p) =>
  `Pasted text ${p.n}, ${count(p.chars, 'character', 'characters')}, ${count(p.lines, 'line', 'lines')}`;

/** The panel header's label. */
export const panelHead = (p) => `#${p.n} · ${spaced(p.chars)} chars · ${count(p.lines, 'line', 'lines')}`;

/**
 * Cut the plain runs (`{ t }`) of a user text at the placeholders whose `n` is in `byN`:
 * `{ t, p }` is a chip (`p` = the paste's `{ n, chars, lines, text? }`).
 */
export function pasteRuns(runs, byN) {
  if (!byN?.size) return runs;
  const out = [];
  for (const r of runs) {
    if (r.m || r.p) {
      out.push(r);
      continue;
    }
    let at = 0;
    for (const { start, end, n, token } of placeholdersIn(r.t)) {
      const p = byN.get(n);
      if (!p) continue;
      if (start > at) out.push({ t: r.t.slice(at, start) });
      out.push({ t: token, p });
      at = end;
    }
    if (at < r.t.length) out.push({ t: r.t.slice(at) });
  }
  return out;
}

/** The pastes an entry carries, `Map<n, { n, chars, lines, text? }>`: its `meta.pastes` (a sent entry,
 *  an unsent one with the text in memory), else the `context.pastes` it was sent with (a waking one). */
export function pastesOf(entry) {
  const out = new Map();
  for (const p of entry?.meta?.pastes ?? []) out.set(p.n, p);
  if (!out.size)
    for (const p of entry?.context?.pastes ?? []) {
      const text = normalize(String(p.text ?? ''));
      out.set(p.n, { n: p.n, chars: countChars(text), lines: countLines(text), text });
    }
  return out;
}

/** The draft with `from..to` replaced by `s` as the smallest edit: `{ from, to, s }` (the span to select, what to insert). */
export function diffEdit(oldText, newText) {
  let p = 0;
  const max = Math.min(oldText.length, newText.length);
  while (p < max && oldText[p] === newText[p]) p++;
  let q = 0;
  while (q < max - p && oldText[oldText.length - 1 - q] === newText[newText.length - 1 - q]) q++;
  return { from: p, to: oldText.length - q, s: newText.slice(p, newText.length - q) };
}
