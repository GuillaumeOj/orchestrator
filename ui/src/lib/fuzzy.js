// Fuzzy matching for the command palette (⌘K) and jump (⌘P): characters in
// order, scored by contiguity and word starts.
export function score(query, text) {
  const q = query.toLowerCase().replace(/\s+/g, '');
  const t = text.toLowerCase();
  if (!q) return 1;
  let ti = 0;
  let s = 0;
  let run = 0;
  for (const c of q) {
    const at = t.indexOf(c, ti);
    if (at < 0) return 0;
    run = at === ti ? run + 1 : 1;
    const wordStart = at === 0 || /[\s/·›\-_.]/.test(t[at - 1]);
    s += 1 + run * 2 + (wordStart ? 3 : 0) - Math.min(at - ti, 5) * 0.2;
    ti = at + 1;
  }
  return s / (1 + t.length * 0.01);
}

export function filter(query, items, key = (i) => i.label) {
  return items
    .map((item) => ({ item, s: score(query, key(item)) }))
    .filter((x) => x.s > 0)
    .sort((a, b) => b.s - a.s)
    .map((x) => x.item);
}
