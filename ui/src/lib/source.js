// Feature pane, source header and Tasks heading (notes §9.1, §9.2; states S14, S15). Pure: tested in
// test/feature-source.test.js. The input is `run.source` (api-contract §2).
import { remoteName } from './linear.js';

const TRACKERS = { github: 'GitHub', linear: 'Linear', local: 'Local' };
export const trackerKindLabel = (t) => TRACKERS[t] ?? remoteName(t);

/**
 * What the header shows. `kind`: 'source' (a tracker item), 'brief' (the Overwatch's brief) or 'none' (started by hand).
 * `run.source.writeback` ({ status, message }, the WorkNode's shape) is read for a failed write-back.
 */
export function sourceModel(run, brief = null) {
  const s = run?.source ?? null;
  if (!s) {
    return brief?.title
      ? {
          kind: 'brief',
          title: `From the Overwatch's brief: ${brief.title}`,
          url: brief.url ?? null,
          canShowInWork: false,
        }
      : { kind: 'none', title: 'No tracker item · started by you', canShowInWork: false };
  }
  const tracker = trackerKindLabel(s.tracker);
  const crumbs = (s.breadcrumb ?? []).map((c) => (c.title && c.title !== c.ref ? `${c.ref} ${c.title}` : c.ref));
  const trail = [
    crumbs.length ? `in ${crumbs.join(' › ')}` : null,
    s.program ? `program ${s.program.title} (${s.program.index} of ${s.program.total})` : null,
  ]
    .filter(Boolean)
    .join(' · ');
  const wb = s.writeback ?? null;
  let writeBack = null;
  if (wb?.status === 'failed')
    writeBack = { state: 'failed', text: `Write-back failed: ${wb.message ?? `could not update ${s.ref}`}` };
  else if (s.tracker !== 'local')
    writeBack = {
      state: s.write_back === 'off' ? 'off' : 'on',
      text: s.write_back === 'off' ? 'Write-back off' : 'Write-back on',
    };
  return {
    kind: 'source',
    glyph: s.tracker === 'linear' ? '◐' : '⊙',
    ref: s.ref,
    title: s.title,
    tracker,
    url: s.url ?? null,
    trail,
    writeBack,
    closed: s.closed_upstream && !s.deleted ? `closed in ${tracker}` : null,
    deleted: s.deleted ? `${s.ref} was deleted in ${tracker}. The run continues.` : null,
    canShowInWork: !s.deleted,
    node: s.node,
  };
}

/** The right side of the Tasks heading: "GitHub sub-issues of #13 · 1 of 4 done" / "Local · 1 of 4 done". */
export function tasksSummary(source, tasks) {
  const n = `${tasks.filter((t) => t.status === 'done').length} of ${tasks.length} done`;
  if (source && source.tracker !== 'local' && source.write_back !== 'off')
    return `${trackerKindLabel(source.tracker)} sub-issues of ${source.ref} · ${n}`;
  return `Local · ${n}`;
}

/** Marks on a task card: `sync_state` 'pending' = "not synced", 'conflict' = conflict; Linear's remote-won stays a conflict. */
export function taskMarks(t) {
  return { notSynced: t.sync_state === 'pending', conflict: t.sync_state === 'conflict' || !!t.remote_won };
}
