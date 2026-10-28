// Task backends in the UI, and Project settings › Task backend: Linear
// (FR-501, FR-503; p3-design-notes §5.13, J13 · 3–5). Pure: tested in
// test/linear.test.js.
import { clock } from './format.js';
import { COLUMNS } from './teams.js';

/** `label` names the backend (the segmented control, headers); `remote` is
 *  the short name in sentences and on cards ("changed in GitHub"). */
export const BACKENDS = [
  { id: 'local', label: 'Local', remote: 'Local' },
  { id: 'github', label: 'GitHub Issues', remote: 'GitHub' },
  { id: 'linear', label: 'Linear', remote: 'Linear' },
];

const backendOf = (id) => BACKENDS.find((b) => b.id === id) ?? BACKENDS[0];
export const backendLabel = (id) => backendOf(id).label;
export const remoteName = (id) => backendOf(id).remote;

/** Board column labels by id. */
export const COLUMN_LABEL = Object.fromEntries(COLUMNS.map((c) => [c.id, c.label]));

/** The mapping rows: each Linear state with its column (saved, else its default). */
export function mappingRows(states, saved = {}) {
  return (states ?? []).map((s) => {
    const column = saved?.[s.id] ?? s.default_column;
    return {
      id: s.id,
      name: s.name,
      type: s.type,
      default_column: s.default_column,
      column,
      isDefault: column === s.default_column,
    };
  });
}

/** Only the rows moved off their default are saved. */
export function mappingToSave(rows) {
  const out = {};
  for (const r of rows) if (!r.isDefault) out[r.id] = r.column;
  return Object.keys(out).length ? out : undefined;
}

/** "7 pushed · 2 pulled · 1 conflict (Linear won)" from a board_sync answer. */
export function syncCounts(r, remote = 'Linear') {
  const c = r.conflicts ?? 0;
  return `${r.pushed ?? 0} pushed · ${r.pulled ?? 0} pulled · ${c} conflict${c === 1 ? '' : 's'}${c ? ` (${remote} won)` : ''}`;
}

/** "Last sync 15:04: 7 pushed · 2 pulled · 1 conflict (Linear won)". */
export function syncLine(r, atMs, remote = 'Linear') {
  return r ? `Last sync ${clock(atMs, atMs)}: ${syncCounts(r, remote)}` : null;
}
