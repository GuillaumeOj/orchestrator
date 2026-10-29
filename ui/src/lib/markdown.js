// Markdown for feature docs, gate bodies and team questions (specs-rendering
// §3.1–3.3, D1): text → an mdast syntax tree (CommonMark + GFM + front-matter),
// never HTML. Markdown.svelte renders the tree as elements, so text an agent
// wrote can't add markup or scripts.
import { fromMarkdown } from 'mdast-util-from-markdown';
import { frontmatterFromMarkdown } from 'mdast-util-frontmatter';
import { gfmFromMarkdown } from 'mdast-util-gfm';
import { frontmatter } from 'micromark-extension-frontmatter';
import { gfm } from 'micromark-extension-gfm';

// Stateless across parses: built once.
const OPTIONS = {
  extensions: [gfm(), frontmatter(['yaml'])],
  mdastExtensions: [gfmFromMarkdown(), frontmatterFromMarkdown(['yaml'])],
};

/** The mdast tree of `md`. */
export function parse(md) {
  return fromMarkdown(unlazyTables(String(md ?? '')), OPTIONS);
}

const QUOTE_LINE = /^ {0,3}>/;
const QUOTE_MARKS = /^(?: {0,3}>[ \t]?)*/;
const FENCE = /^ {0,3}(`{3,}|~{3,})/;
const DELIMITER_ROW = /^ {0,3}\|?[ \t]*:?-+:?[ \t]*(?:\|[ \t]*:?-+:?[ \t]*)*\|?[ \t]*\r?$/;

/** The cells of a table row: split on unescaped `|`, outer pipes dropped. */
function cellCount(line) {
  const cells = line.trim().split(/(?<!\\)\|/);
  if (cells[0].trim() === '') cells.shift();
  if (cells.length && cells.at(-1).trim() === '') cells.pop();
  return cells.length;
}

/**
 * A blank line before each table whose rows lack `>` right after a quote
 * (fix-md-rendering-quoted-tables §3). micromark never starts a table on a
 * lazy continuation line, so the rows would join the quote's paragraph as text;
 * the author meant a table after the quote. Fenced code is left alone.
 */
function unlazyTables(md) {
  const lines = md.split('\n');
  const out = [];
  let fence = null; // the open fence's marker (``` / ~~~…), quoted or not
  lines.forEach((line, i) => {
    const bare = line.replace(QUOTE_MARKS, '');
    const m = FENCE.exec(bare);
    if (fence) {
      // A closing fence: the same character, at least as long, nothing after it.
      if (m && m[1][0] === fence[0] && m[1].length >= fence.length && !bare.slice(m[0].length).trim()) fence = null;
    } else if (m) {
      fence = m[1];
    } else if (
      i > 0 &&
      QUOTE_LINE.test(lines[i - 1]) &&
      !QUOTE_LINE.test(line) &&
      line.includes('|') &&
      DELIMITER_ROW.test(lines[i + 1] ?? '') &&
      lines[i + 1].includes('|') &&
      cellCount(line) === cellCount(lines[i + 1])
    ) {
      out.push('');
    }
    out.push(line);
  });
  return out.length === lines.length ? md : out.join('\n');
}

/** The plain text of a node (heading slugs). */
function toText(n) {
  if (!n) return '';
  if (typeof n.value === 'string' && n.type !== 'html') return n.value;
  if (n.type === 'image' || n.type === 'imageReference') return n.alt ?? '';
  return (n.children ?? []).map(toText).join('');
}

/** GitHub's heading anchor: lowercase, punctuation dropped, spaces → `-`. */
export function slug(text) {
  return String(text)
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{M}\p{N}\p{Pc} -]/gu, '')
    .replace(/ /g, '-');
}

function walk(n, fn) {
  fn(n);
  for (const c of n.children ?? []) walk(c, fn);
}

/** `> [!NOTE]` (GitHub alerts): the kind on `data.alert`, the marker dropped. */
function markAlert(q) {
  const p = q.children[0];
  const t = p?.type === 'paragraph' ? p.children[0] : null;
  const m = t?.type === 'text' ? /^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\][ \t]*(?:\n|$)/i.exec(t.value) : null;
  if (!m) return;
  q.data = { ...q.data, alert: m[1].toLowerCase() };
  t.value = t.value.slice(m[0].length);
  if (!t.value) p.children.shift();
  if (p.children[0]?.type === 'break') p.children.shift();
  if (!p.children.length) q.children.shift();
}

/**
 * The tree plus what rendering needs from the whole doc: link definitions,
 * footnotes in the order they are first referenced (numbered from 1), unique
 * heading slugs on `data.slug`, footnote ref ids on `data.refId`.
 */
export function prepare(md) {
  const root = parse(md);
  const defs = new Map();
  const notes = new Map();
  const nums = new Map(); // footnote id → number, in first-reference order
  const slugs = new Map();
  const used = new Set();
  const refs = new Map();
  walk(root, (n) => {
    if (n.type === 'definition') defs.has(n.identifier) || defs.set(n.identifier, n);
    else if (n.type === 'footnoteDefinition') notes.has(n.identifier) || notes.set(n.identifier, n);
    else if (n.type === 'blockquote') markAlert(n);
  });
  walk(root, (n) => {
    if (n.type === 'heading') {
      // As github-slugger: skip suffixes another heading already took (`Foo`, `Foo`, `Foo 1`).
      const base = slug(toText(n));
      let k = slugs.get(base) ?? 0;
      let s = k ? `${base}-${k}` : base;
      while (used.has(s)) s = `${base}-${++k}`;
      slugs.set(base, k + 1);
      used.add(s);
      n.data = { ...n.data, slug: s };
    } else if (n.type === 'footnoteReference' && notes.has(n.identifier)) {
      if (!nums.has(n.identifier)) nums.set(n.identifier, nums.size + 1);
      const num = nums.get(n.identifier);
      const k = (refs.get(num) ?? 0) + 1;
      refs.set(num, k);
      n.data = { ...n.data, num, refId: k === 1 ? `fnref:${num}` : `fnref:${num}-${k}` };
    }
  });
  const footnotes = [...nums].map(([id, num]) => ({ num, node: notes.get(id) }));
  return { root, defs, footnotes };
}

// biome-ignore lint/suspicious/noControlCharactersInRegex: matching them is the point.
const CONTROL = /[\x00-\x1f\x7f]/;
/** `javascript:`, `file:`, `//host`…: a scheme (or another host) of its own. */
const SCHEME = /^([a-z][a-z0-9+.-]*:|\/\/)/i;
/** `./a.md`, `././a.md` → `a.md`. */
const DOT_SLASH = /^(\.\/)+/;

/**
 * What a link target does (§3.6):
 * - `external`: http(s) / mailto, opened in the browser;
 * - `anchor`: `#heading` in the same doc;
 * - `doc`: a sibling `.md` of the feature (only when the view can switch docs);
 * - `repo`: another relative path, shown but not clickable (D4);
 * - `text`: any other scheme (`javascript:`, `file:`…), plain text.
 */
export function linkKind(url, canSwitchDoc = false) {
  const u = String(url ?? '').trim();
  // Browsers drop tabs/newlines and leading controls when reading an href, so
  // `java&#9;script:` would become `javascript:`: a link with one is text.
  if (CONTROL.test(u)) return { kind: 'text' };
  if (/^(https?:\/\/|mailto:)/i.test(u)) return { kind: 'external', url: u };
  if (u.startsWith('#')) return { kind: 'anchor', anchor: safeDecode(u.slice(1)) };
  if (SCHEME.test(u) || !u) return { kind: 'text' };
  const [path, hash = ''] = u.split('#');
  const name = safeDecode(path.replace(DOT_SLASH, '').replace(/\?.*$/, ''));
  if (canSwitchDoc && /^[^/\\]+\.md$/i.test(name)) return { kind: 'doc', name, anchor: safeDecode(hash) || null };
  return { kind: 'repo', path: u };
}

/**
 * Where an image comes from (§3.5):
 * - `remote`: http(s), never fetched (D2), a chip opening the browser;
 * - `data`: an inline raster image (`data:image/png;base64,…`);
 * - `dropped`: any other `data:` URL, not shown;
 * - `asset`: a path in the feature's docs folder, for feature_asset (`#`/`?`
 *   suffix stripped, percent-decoded, a leading `./` dropped);
 * - `other`: any other scheme, not available.
 */
export function imageTarget(url) {
  const u = String(url ?? '').trim();
  if (!u || CONTROL.test(u)) return { kind: 'other' };
  if (/^https?:\/\//i.test(u)) return { kind: 'remote', url: u };
  if (/^data:/i.test(u))
    return /^data:image\/(png|jpe?g|gif|webp);base64,[a-z0-9+/=]*$/i.test(u)
      ? { kind: 'data', url: u }
      : { kind: 'dropped' };
  if (SCHEME.test(u)) return { kind: 'other' };
  const path = safeDecode(u.replace(/[?#].*$/, '')).replace(DOT_SLASH, '');
  return path ? { kind: 'asset', path } : { kind: 'other' };
}

function safeDecode(s) {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/** Front-matter lines as key/value pairs, or null when it isn't flat `key: value`. */
export function frontPairs(yaml) {
  const lines = String(yaml ?? '')
    .split('\n')
    .filter((l) => l.trim());
  const pairs = lines.map((l) => /^([\w.-]+):[ \t]*(.*)$/.exec(l));
  return pairs.length && pairs.every(Boolean) ? pairs.map((m) => [m[1], m[2]]) : null;
}

// ---------------------------------------------------------------- lazy parts

// Highlighting (lowlight) and diagrams (mermaid) are big: imported on first use
// only, so the app's start bundle doesn't carry them.
/** @type {typeof import('./highlight.js') | null} */
let hlModule = null;
/** The highlighter once loaded (blocks mounted later highlight on their first render), else null. */
export const highlighter = () => hlModule;
export const loadHighlight = () => import('./highlight.js').then((m) => (hlModule = m));
export const loadMermaid = () => import('./mermaid.js');
