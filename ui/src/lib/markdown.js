// A small markdown reader for feature docs (spec.md, ux-brief.md…) in the
// Inbox spec gate and the Feature pane. It returns blocks and inline spans
// that components render as elements: no HTML is ever injected, so text an
// agent wrote can't add markup or scripts. Links show their text only.

/** @typedef {{ t: 'text' | 'b' | 'i' | 'code', s: string }} Span */
/**
 * `h` (level, spans), `p` / `quote` (spans), `ul` / `ol` (items), `code` (text), `hr`.
 * @typedef {{ type: string, level?: number, spans?: Span[], items?: { spans: Span[], checked: boolean | null }[], text?: string }} Block
 */

/** Inline spans: `code`, **bold**, *italic* / _italic_, [text](url) → text. */
export function inline(text) {
  /** @type {Span[]} */
  const out = [];
  // `_` emphasis only at word edges: snake_case names in specs stay as written.
  const re =
    /(`[^`]+`)|(\*\*[^*]+\*\*|(?<!\w)__[^_]+__(?!\w))|(\*[^*\s][^*]*\*|(?<!\w)_[^_\s][^_]*_(?!\w))|(\[[^\]]+\]\([^)]*\))/g;
  let last = 0;
  for (const m of text.matchAll(re)) {
    const s = m[0];
    if (m.index > last) out.push({ t: 'text', s: text.slice(last, m.index) });
    if (m[1]) out.push({ t: 'code', s: s.slice(1, -1) });
    else if (m[2]) out.push({ t: 'b', s: s.slice(2, -2) });
    else if (m[3]) out.push({ t: 'i', s: s.slice(1, -1) });
    else out.push({ t: 'text', s: s.slice(1, s.indexOf('](')) });
    last = m.index + s.length;
  }
  if (last < text.length) out.push({ t: 'text', s: text.slice(last) });
  return out;
}

/** @returns {Block[]} */
export function parse(md) {
  /** @type {Block[]} */
  const blocks = [];
  const lines = String(md ?? '')
    .replace(/\r\n?/g, '\n')
    .split('\n');
  let para = [];
  let quote = [];
  const flush = () => {
    if (para.length) blocks.push({ type: 'p', spans: inline(para.join(' ')) });
    if (quote.length) blocks.push({ type: 'quote', spans: inline(quote.join(' ')) });
    para = [];
    quote = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (/^\s*```/.test(line)) {
      flush();
      const code = [];
      while (++i < lines.length && !/^\s*```/.test(lines[i])) code.push(lines[i]);
      blocks.push({ type: 'code', text: code.join('\n') });
      continue;
    }
    const h = /^(#{1,6})\s+(.*?)\s*#*\s*$/.exec(line);
    if (h) {
      flush();
      blocks.push({ type: 'h', level: h[1].length, spans: inline(h[2]) });
      continue;
    }
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(line)) {
      flush();
      blocks.push({ type: 'hr' });
      continue;
    }
    const li = /^\s*(?:([-*+])|(\d+)[.)])\s+(?:\[([ xX])\]\s+)?(.*)$/.exec(line);
    if (li) {
      flush();
      const type = li[2] ? 'ol' : 'ul';
      const item = { spans: inline(li[4]), checked: li[3] === undefined ? null : li[3] !== ' ' };
      const prev = blocks[blocks.length - 1];
      if (prev && prev.type === type) prev.items.push(item);
      else blocks.push({ type, items: [item] });
      continue;
    }
    const q = /^\s*>\s?(.*)$/.exec(line);
    if (q) {
      if (para.length) flush();
      quote.push(q[1]);
      continue;
    }
    if (!line.trim()) {
      flush();
      continue;
    }
    if (quote.length) flush();
    para.push(line.trim());
  }
  flush();
  return blocks;
}
