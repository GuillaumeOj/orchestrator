// Markdown for feature docs, gate bodies and team questions (specs-rendering
// §3.1–3.3, D1): text → an mdast syntax tree (CommonMark + GFM + front-matter),
// never HTML. Markdown.svelte renders the tree as elements, so text an agent
// wrote can't add markup or scripts. Code gets a token list from lowlight, also
// rendered as elements.
import { common, createLowlight } from 'lowlight';
import { fromMarkdown } from 'mdast-util-from-markdown';
import { frontmatterFromMarkdown } from 'mdast-util-frontmatter';
import { gfmFromMarkdown } from 'mdast-util-gfm';
import { frontmatter } from 'micromark-extension-frontmatter';
import { gfm } from 'micromark-extension-gfm';

/** The mdast tree of `md`. */
export function parse(md) {
  return fromMarkdown(String(md ?? ''), {
    extensions: [gfm(), frontmatter(['yaml'])],
    mdastExtensions: [gfmFromMarkdown(), frontmatterFromMarkdown(['yaml'])],
  });
}

/** The plain text of a node (heading slugs, image alts). */
export function toText(n) {
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

const ALERTS = { NOTE: 'note', TIP: 'tip', IMPORTANT: 'important', WARNING: 'warning', CAUTION: 'caution' };

/** `> [!NOTE]` (GitHub alerts): the kind on `data.alert`, the marker dropped. */
function markAlert(q) {
  const p = q.children[0];
  const t = p?.type === 'paragraph' ? p.children[0] : null;
  const m = t?.type === 'text' ? /^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\][ \t]*(?:\n|$)/i.exec(t.value) : null;
  if (!m) return;
  q.data = { ...q.data, alert: ALERTS[m[1].toUpperCase()] };
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
  const order = [];
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
      if (!order.includes(n.identifier)) order.push(n.identifier);
      const num = order.indexOf(n.identifier) + 1;
      const k = (refs.get(num) ?? 0) + 1;
      refs.set(num, k);
      n.data = { ...n.data, num, refId: k === 1 ? `fnref:${num}` : `fnref:${num}-${k}` };
    }
  });
  const footnotes = order.map((id, i) => ({ num: i + 1, node: notes.get(id) }));
  return { root, defs, footnotes };
}

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
  if ([...u].some((c) => c.charCodeAt(0) < 0x20 || c.charCodeAt(0) === 0x7f)) return { kind: 'text' };
  if (/^(https?:\/\/|mailto:)/i.test(u)) return { kind: 'external', url: u };
  if (u.startsWith('#')) return { kind: 'anchor', anchor: safeDecode(u.slice(1)) };
  if (/^[a-z][a-z0-9+.-]*:/i.test(u) || u.startsWith('//') || !u) return { kind: 'text' };
  const [path, hash = ''] = u.split('#');
  const name = safeDecode(path.replace(/^\.\//, '').replace(/\?.*$/, ''));
  if (canSwitchDoc && /^[^/\\]+\.md$/i.test(name)) return { kind: 'doc', name, anchor: safeDecode(hash) || null };
  return { kind: 'repo', path: u };
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

// ---------------------------------------------------------------- code

const lowlight = createLowlight(common);
lowlight.registerAlias({ xml: ['svelte', 'vue'] });

/** Above these, a block renders plain to keep the pane responsive (§3.3). */
export const HL_MAX_LINES = 2000;
export const HL_MAX_BYTES = 100_000;

/**
 * highlight.js scopes → `--color-syntax-*` tokens (ux-brief §2), the one
 * place they map. A sub-scope (`title.function`) is tried before its scope.
 */
const SYNTAX = {
  keyword: 'keyword',
  doctag: 'keyword',
  bullet: 'keyword',
  'selector-tag': 'keyword',
  string: 'string',
  regexp: 'string',
  link: 'string',
  char: 'constant',
  number: 'number',
  comment: 'comment',
  quote: 'comment',
  title: 'function',
  'title.function': 'function',
  'title.class': 'type',
  'title.class.inherited': 'type',
  section: 'function',
  type: 'type',
  built_in: 'type',
  class: 'type',
  variable: 'variable',
  'variable.language': 'keyword',
  'template-variable': 'variable',
  subst: 'variable',
  params: 'variable',
  property: 'attr',
  literal: 'constant',
  symbol: 'constant',
  attr: 'attr',
  attribute: 'attr',
  'selector-attr': 'attr',
  'selector-class': 'attr',
  'selector-id': 'attr',
  'selector-pseudo': 'attr',
  tag: 'tag',
  name: 'tag',
  meta: 'meta',
  'meta.keyword': 'meta',
  'meta.string': 'string',
  addition: 'addition',
  deletion: 'deletion',
  punctuation: 'punctuation',
  operator: 'punctuation',
};

function syntaxOf(el) {
  const cls = el.properties?.className ?? [];
  const scope = cls.find((c) => c.startsWith('hljs-'))?.slice(5);
  if (!scope) return null;
  const subs = cls.filter((c) => !c.startsWith('hljs-')).map((c) => c.replace(/_+$/, ''));
  for (let i = subs.length; i >= 0; i--) {
    const t = SYNTAX[[scope, ...subs.slice(0, i)].join('.')];
    if (t) return t;
  }
  return null;
}

/** The fence's language as lowlight knows it, or null (no guessing: highlightAuto is off). */
export function codeLang(lang) {
  const l = String(lang ?? '')
    .trim()
    .toLowerCase();
  return l && lowlight.registered(l) ? l : null;
}

/**
 * `code` as a flat list of `{ t, v }` runs (`t` = a syntax token or null), or
 * null when it renders plain: unknown language, or a very long block.
 */
export function highlight(code, lang) {
  const l = codeLang(lang);
  const s = String(code ?? '');
  if (!l || s.length > HL_MAX_BYTES || s.split('\n').length > HL_MAX_LINES) return null;
  const runs = [];
  const push = (t, v) => {
    const last = runs[runs.length - 1];
    if (last && last.t === t) last.v += v;
    else runs.push({ t, v });
  };
  const visit = (n, t) => {
    if (n.type === 'text') push(t, n.value);
    else for (const c of n.children ?? []) visit(c, n.type === 'element' ? (syntaxOf(n) ?? t) : t);
  };
  visit(lowlight.highlight(l, s), null);
  return runs;
}
