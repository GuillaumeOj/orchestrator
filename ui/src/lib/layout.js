// Tiling workspace (§9.2): a tree of splits whose leaves are tab groups of
// panes. Pure functions: every operation returns a new tree.
//   leaf  { kind: 'leaf', id, tabs: [paneId], active: paneId }
//   split { kind: 'split', id, dir: 'row' | 'col', children: [node], sizes: [fraction] }
// 'row' = side by side (leader v), 'col' = stacked (leader s), as in vim.

// A restored layout gets fresh ids (`reid`), so ids only need to be unique per run.
let seq = 0;
const nid = (p) => `${p}${++seq}`;

export const leaf = (paneId) => ({ kind: 'leaf', id: nid('l'), tabs: [paneId], active: paneId });

export function leafOf(node, paneId) {
  if (!node) return null;
  if (node.kind === 'leaf') return node.tabs.includes(paneId) ? node : null;
  for (const c of node.children) {
    const f = leafOf(c, paneId);
    if (f) return f;
  }
  return null;
}

function map(node, fn) {
  const r = fn(node);
  if (r !== node) return r;
  if (node.kind === 'split') {
    const children = node.children.map((c) => map(c, fn));
    if (children.some((c, i) => c !== node.children[i])) return { ...node, children };
  }
  return node;
}

/** Splits the pane's group: the new pane goes after it, in `dir`. */
export function split(tree, paneId, dir, newPaneId) {
  if (!tree) return leaf(newPaneId);
  const target = leafOf(tree, paneId) ?? firstLeaf(tree);
  const fresh = leaf(newPaneId);
  // Same direction as the parent: add a sibling instead of nesting.
  const parent = parentOf(tree, target);
  if (parent && parent.dir === dir) {
    return map(tree, (n) => {
      if (n !== parent) return n;
      const i = n.children.indexOf(target);
      const children = [...n.children.slice(0, i + 1), fresh, ...n.children.slice(i + 1)];
      return { ...n, children, sizes: children.map(() => 1 / children.length) };
    });
  }
  return map(tree, (n) =>
    n === target ? { kind: 'split', id: nid('s'), dir, children: [target, fresh], sizes: [0.5, 0.5] } : n,
  );
}

export function addTab(tree, paneId, newPaneId) {
  if (!tree) return leaf(newPaneId);
  const target = leafOf(tree, paneId) ?? firstLeaf(tree);
  return map(tree, (n) => (n === target ? { ...n, tabs: [...n.tabs, newPaneId], active: newPaneId } : n));
}

export function activate(tree, paneId) {
  const target = leafOf(tree, paneId);
  if (!target || target.active === paneId) return tree;
  return map(tree, (n) => (n === target ? { ...n, active: paneId } : n));
}

function parentOf(node, child) {
  if (node.kind !== 'split') return null;
  if (node.children.includes(child)) return node;
  for (const c of node.children) {
    const p = parentOf(c, child);
    if (p) return p;
  }
  return null;
}

export function firstLeaf(node) {
  return node.kind === 'leaf' ? node : firstLeaf(node.children[0]);
}

/** Removes a pane; empty groups disappear and one-child splits collapse. */
export function remove(tree, paneId) {
  function walk(n) {
    if (n.kind === 'leaf') {
      if (!n.tabs.includes(paneId)) return n;
      const i = n.tabs.indexOf(paneId);
      const tabs = n.tabs.filter((t) => t !== paneId);
      if (!tabs.length) return null;
      return { ...n, tabs, active: n.active === paneId ? tabs[Math.min(i, tabs.length - 1)] : n.active };
    }
    const kept = [];
    const sizes = [];
    n.children.forEach((c, i) => {
      const w = walk(c);
      if (w) {
        kept.push(w);
        sizes.push(n.sizes[i]);
      }
    });
    if (!kept.length) return null;
    if (kept.length === 1) return kept[0];
    const total = sizes.reduce((a, b) => a + b, 0);
    return { ...n, children: kept, sizes: sizes.map((s) => s / total) };
  }
  return tree ? walk(tree) : null;
}

/** Visible panes (each group's active tab), in reading order: ⌘1…⌘9. */
export function visible(tree) {
  if (!tree) return [];
  if (tree.kind === 'leaf') return [tree.active];
  return tree.children.flatMap(visible);
}

export function allPanes(tree) {
  if (!tree) return [];
  if (tree.kind === 'leaf') return [...tree.tabs];
  return tree.children.flatMap(allPanes);
}

/** Normalized rectangles of the visible panes. */
export function rects(tree, box = { x: 0, y: 0, w: 1, h: 1 }, out = {}) {
  if (!tree) return out;
  if (tree.kind === 'leaf') {
    out[tree.active] = box;
    return out;
  }
  let off = 0;
  tree.children.forEach((c, i) => {
    const s = tree.sizes[i];
    const b =
      tree.dir === 'row'
        ? { x: box.x + off * box.w, y: box.y, w: s * box.w, h: box.h }
        : { x: box.x, y: box.y + off * box.h, w: box.w, h: s * box.h };
    rects(c, b, out);
    off += s;
  });
  return out;
}

/** The visible pane next to `paneId` in a direction (leader h j k l). */
export function neighbor(tree, paneId, direction) {
  const r = rects(tree);
  const a = r[paneId];
  if (!a) return null;
  const eps = 1e-6;
  const overlap = (a0, a1, b0, b1) => Math.min(a1, b1) - Math.max(a0, b0);
  let best = null;
  for (const [id, b] of Object.entries(r)) {
    if (id === paneId) continue;
    let dist;
    let ov;
    if (direction === 'right' && b.x >= a.x + a.w - eps)
      [dist, ov] = [b.x - (a.x + a.w), overlap(a.y, a.y + a.h, b.y, b.y + b.h)];
    else if (direction === 'left' && b.x + b.w <= a.x + eps)
      [dist, ov] = [a.x - (b.x + b.w), overlap(a.y, a.y + a.h, b.y, b.y + b.h)];
    else if (direction === 'down' && b.y >= a.y + a.h - eps)
      [dist, ov] = [b.y - (a.y + a.h), overlap(a.x, a.x + a.w, b.x, b.x + b.w)];
    else if (direction === 'up' && b.y + b.h <= a.y + eps)
      [dist, ov] = [a.y - (b.y + b.h), overlap(a.x, a.x + a.w, b.x, b.x + b.w)];
    else continue;
    if (ov <= eps) continue;
    if (!best || dist < best.dist - eps || (Math.abs(dist - best.dist) <= eps && ov > best.ov)) best = { id, dist, ov };
  }
  return best?.id ?? null;
}

/** Moves the divider after child `index` of split `splitId` by `delta` (fraction). */
export function resize(tree, splitId, index, delta, min = 0.1) {
  return map(tree, (n) => {
    if (n.kind !== 'split' || n.id !== splitId) return n;
    const sizes = [...n.sizes];
    const d = Math.max(min - sizes[index], Math.min(delta, sizes[index + 1] - min));
    sizes[index] += d;
    sizes[index + 1] -= d;
    return { ...n, sizes };
  });
}

/** Keeps only panes that still exist (a saved layout after a restart). */
export function prune(tree, exists) {
  let t = tree;
  for (const p of allPanes(tree)) if (!exists(p)) t = remove(t, p);
  return t;
}

/** Fresh ids for every node (a saved layout may carry ids that clash). */
export function reid(tree) {
  if (!tree) return tree;
  if (tree.kind === 'leaf') return { ...tree, id: nid('l') };
  return { ...tree, id: nid('s'), children: tree.children.map(reid) };
}
