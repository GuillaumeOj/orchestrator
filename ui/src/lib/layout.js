// The workspace (design-notes §4.2): one tab group of panes, or null.
//   leaf  { kind: 'leaf', id, tabs: [paneId], active: paneId }
// A layout saved before the single pane may be a tree of splits
//   split { kind: 'split', id, dir, children: [node], sizes }
// that `flatten` turns into one leaf at restore; everything else takes a leaf.
// Pure functions: every operation returns a new leaf.

// Ids only need to be unique per run (a restored leaf gets a fresh one).
let seq = 0;
const nid = () => `l${++seq}`;

export const leaf = (paneId) => ({ kind: 'leaf', id: nid(), tabs: [paneId], active: paneId });

/** Adds a tab at the end, active. */
export const addTab = (tree, paneId) =>
  tree ? { ...tree, tabs: [...tree.tabs, paneId], active: paneId } : leaf(paneId);

export function activate(tree, paneId) {
  if (!tree?.tabs.includes(paneId) || tree.active === paneId) return tree;
  return { ...tree, active: paneId };
}

/** Removes a tab (the next one takes its place when it was active); null when none is left. */
export function remove(tree, paneId) {
  if (!tree?.tabs.includes(paneId)) return tree;
  const i = tree.tabs.indexOf(paneId);
  const tabs = tree.tabs.filter((t) => t !== paneId);
  if (!tabs.length) return null;
  return { ...tree, tabs, active: tree.active === paneId ? tabs[Math.min(i, tabs.length - 1)] : tree.active };
}

/** Each group's active tab, in reading order (a saved tree's focus fallback). */
function visible(tree) {
  if (!tree) return [];
  if (tree.kind === 'leaf') return [tree.active];
  return tree.children.flatMap(visible);
}

/** Every pane, in reading order (left→right, top→bottom). */
export function allPanes(tree) {
  if (!tree) return [];
  if (tree.kind === 'leaf') return [...tree.tabs];
  return tree.children.flatMap(allPanes);
}

/** Keeps only panes that still exist (a saved layout after a restart). */
export function prune(tree, exists) {
  let t = tree;
  for (const p of allPanes(tree)) if (!exists(p)) t = remove(t, p);
  return t;
}

/** A saved tree (one leaf or splits) → one fresh leaf, panes in reading order,
 *  the focused one active (else the first group's); null when it has none. */
export function flatten(tree, focused = null) {
  const ids = allPanes(tree);
  if (!ids.length) return null;
  const active = ids.includes(focused) ? focused : visible(tree)[0];
  return { kind: 'leaf', id: nid(), tabs: ids, active };
}
