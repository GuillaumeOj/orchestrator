// Syntax colors for code blocks (specs-rendering §3.3): lowlight (highlight.js
// grammars returning a tree) → a flat list of runs that CodeBlock.svelte
// renders as <span> elements, never HTML. Loaded lazily (markdown.js
// loadHighlight): only when a doc shows code with a language.
import { common, createLowlight } from 'lowlight';

const lowlight = createLowlight(common);
lowlight.registerAlias({ xml: ['svelte', 'vue'] });

/** Above these, a block renders plain to keep the pane responsive (§3.3). */
export const HL_MAX_LINES = 2000;
const HL_MAX_BYTES = 100_000;

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
  'title.class': 'type',
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
  'meta.string': 'string',
  addition: 'addition',
  deletion: 'deletion',
  punctuation: 'punctuation',
  operator: 'punctuation',
};

// A few dozen distinct class lists cover a whole block: resolve each once.
const syntaxCache = new Map();

function syntaxOf(el) {
  const cls = el.properties?.className ?? [];
  const key = cls.join(' ');
  if (!syntaxCache.has(key)) syntaxCache.set(key, resolveSyntax(cls));
  return syntaxCache.get(key);
}

function resolveSyntax(cls) {
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
function codeLang(lang) {
  const l = String(lang ?? '')
    .trim()
    .toLowerCase();
  return l && lowlight.registered(l) ? l : null;
}

function tooManyLines(s) {
  let lines = 1;
  for (let i = s.indexOf('\n'); i !== -1; i = s.indexOf('\n', i + 1)) if (++lines > HL_MAX_LINES) return true;
  return false;
}

/**
 * `code` as a flat list of `{ t, v }` runs (`t` = a syntax token or null), or
 * null when it renders plain: unknown language, or a very long block.
 */
export function highlight(code, lang) {
  const l = codeLang(lang);
  const s = String(code ?? '');
  if (!l || s.length > HL_MAX_BYTES || tooManyLines(s)) return null;
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
