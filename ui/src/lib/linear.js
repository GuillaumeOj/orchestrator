// Project settings › Task backend: Linear (FR-501, FR-503; p3-design-notes
// §5.13, J13 · 3–5). Pure: tested in test/linear.test.js.
import { clock } from './format.js';

export const BACKENDS = [
  { id: 'local', label: 'Local' },
  { id: 'github', label: 'GitHub Issues' },
  { id: 'linear', label: 'Linear' },
];

export const COLUMN_LABEL = {
  backlog: 'Backlog',
  ready: 'Ready',
  in_progress: 'In progress',
  review: 'Review',
  done: 'Done',
};

/** The mapping rows: each Linear state with its column (saved, else its default). */
export function mappingRows(states, saved = {}) {
  return (states ?? []).map((s) => {
    const column = saved?.[s.id] ?? s.default_column;
    return { id: s.id, name: s.name, type: s.type, column, isDefault: column === s.default_column };
  });
}

/** Only the rows moved off their default are saved. */
export function mappingToSave(rows) {
  const out = {};
  for (const r of rows) if (!r.isDefault) out[r.id] = r.column;
  return Object.keys(out).length ? out : undefined;
}

/** "Last sync 15:04: 7 pushed · 2 pulled · 1 conflict (Linear won)". */
export function syncLine(r, atMs, remote = 'Linear') {
  if (!r) return null;
  const pulled = (r.imported ?? 0) + (r.updated ?? 0);
  const c = r.conflicts ?? 0;
  return `Last sync ${clock(atMs, atMs)}: ${r.pushed ?? 0} pushed · ${pulled} pulled · ${c} conflict${c === 1 ? '' : 's'}${c ? ` (${remote} won)` : ''}`;
}

/**
 * D8: the core refuses a switch to Linear without `migrate` while the
 * project has tasks, with the counts in its error. Null when it is another error.
 */
export function migrateAsk(err) {
  const e = String(err ?? '');
  if (!/migrate/i.test(e)) return null;
  const open = Number(e.match(/(\d+)\s+open/i)?.[1] ?? 0);
  const done = Number(e.match(/(\d+)\s+done/i)?.[1] ?? 0);
  return { open, done };
}

/** The saved backend for the form. */
export function backendForm(kind, linear) {
  if (kind !== 'linear') return { type: kind };
  return {
    type: 'linear',
    team: linear.team,
    ...(linear.project ? { project: linear.project } : {}),
    ...(linear.states ? { states: linear.states } : {}),
  };
}
