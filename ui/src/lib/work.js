// Entry to the Work browser (improve-intake notes §1.2): every caller (sidebar, Project pane, palette,
// leader ⌘; b, Feature "Show in Work", run inspector) goes through `openWork`. One tab per scope:
// the same scope focuses its tab (and lands the reveal / search there), another opens a new tab.
import { addPane, app, findPane, showPane, toast } from './app.svelte.js';
import { scopeKey } from './workmodel.js';

/** The pane spec's scope: sorted, unique ids, or null for every project. */
export const scopeOf = (ids) => (ids?.length ? [...new Set(ids.map(String))].sort() : null);

const sameScope = (s, list) =>
  s.type === 'work' && (s.projects == null) === (list == null) && scopeKey(s.projects) === scopeKey(list);

/**
 * Opens Work on `projects` (ids; null = all), revealing `reveal` (a WorkNode id) when given, or with
 * the search focused (`find`, the palette's "Find work item…").
 * @param {{ projects?: string[] | null, reveal?: string | null, find?: boolean }} [scope]
 */
export function openWork({ projects = null, reveal = null, find = false } = {}) {
  const list = scopeOf(projects);
  // A one-shot request the pane clears; `land` is never saved with the layout.
  const land = reveal || find ? { reveal, find, seq: Date.now() } : null;
  const existing = findPane((s) => sameScope(s, list));
  if (existing) {
    if (land) app.panes[existing] = { ...app.panes[existing], land };
    return showPane(existing);
  }
  addPane({ type: 'work', projects: list, ...(land ? { land } : {}) });
}

/**
 * Run as program (`p`): the plan dialog of `roots` (one node, or ≥ 2 siblings). F5 implements the
 * dialog; until then a toast says so.
 * @param {string[]} roots
 * @param {{ title?: string }} [opts]
 */
export function openProgramPlan(roots, { title = '' } = {}) {
  toast(`Run as program${title ? ` · ${title}` : ''}: programs are not available yet (${roots.length} selected).`);
}

/** Open program (`p` on a program root, the program chip): the Program view (F5); a toast until then. */
export function openProgram(id, title = '') {
  toast(`Program ${title || id}: the Program view is not available yet.`);
}
