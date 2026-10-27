// Teams in the UI (P2): run states, the team timeline's filters and copy,
// and the Board's columns and moves. Pure: tested in test/teams.test.js.

/** Team-run states (§6.2) → status colors (Penpot 02 Feature card). */
export function runStateColor(s) {
  return (
    {
      draft: 'var(--color-text-muted)',
      spec: 'var(--color-status-working)',
      spec_gate: 'var(--color-status-at-gate)',
      planning: 'var(--color-status-working)',
      executing: 'var(--color-status-working)',
      integrating: 'var(--color-status-working)',
      pr_gate: 'var(--color-status-at-gate)',
      done: 'var(--color-status-done)',
      paused: 'var(--color-status-paused)',
      deferred: 'var(--color-status-queued)',
      failed: 'var(--color-status-error)',
    }[s] ?? 'var(--color-text-muted)'
  );
}

export function runStateLabel(s) {
  return (
    {
      draft: 'Draft',
      spec: 'Spec',
      spec_gate: 'Spec gate',
      planning: 'Planning',
      executing: 'Executing',
      integrating: 'Integrating',
      pr_gate: 'PR gate',
      done: 'Done',
      paused: 'Paused',
      deferred: 'Deferred',
      failed: 'Stopped',
    }[s] ?? s
  );
}

/** Timeline filters (FR-408, Penpot 04 Feature): event kinds by family. */
export const TL_FILTERS = [
  { id: 'all', label: 'All' },
  { id: 'messages', label: 'Messages' },
  { id: 'tasks', label: 'Tasks' },
  { id: 'git', label: 'Git' },
  { id: 'gates', label: 'Gates' },
  { id: 'run', label: 'Run' },
];

const FAMILY = (k) =>
  k === 'message' || k === 'asked_user'
    ? 'messages'
    : k.startsWith('task.')
      ? 'tasks'
      : k.startsWith('git.')
        ? 'git'
        : k === 'run.state' || k === 'run.launched' || k === 'agent.added' || k === 'run.team_changed'
          ? 'run'
          : 'other';

export function timelineFilter(id) {
  if (id === 'all') return () => true;
  if (id === 'gates')
    return (e) => e.kind === 'run.state' && /gate/.test(`${e.detail?.to} ${e.detail?.from} ${e.detail?.why}`);
  return (e) => FAMILY(e.kind) === id;
}

/** One line per timeline event. */
export function timelineText(e) {
  const d = e.detail ?? {};
  switch (e.kind) {
    case 'message':
      return `${d.kind && d.kind !== 'message' ? `${String(d.kind).replace('_', ' ')} ` : ''}→ ${d.to}: ${d.body ?? ''}`;
    case 'run.state':
      return `${runStateLabel(d.from)} → ${runStateLabel(d.to)}${d.why ? ` (${d.why})` : ''}`;
    case 'run.launched':
      return `launched ${d.team} v${d.version} on ${d.feature}`;
    case 'agent.added':
      return `${d.node} joined on ${d.branch}`;
    case 'task.created':
      return `created T-${d.task}: ${d.title}${d.node ? ` → ${d.node}` : ''}${d.by === 'user' ? ' (you)' : ''}`;
    case 'task.assigned':
      return `T-${d.task} assigned: ${d.title}`;
    case 'task.reported':
      return `reported T-${d.task} done`;
    case 'task.done':
      return `T-${d.task} merged and done`;
    case 'task.updated':
      return `T-${d.task} → ${String(d.status).replace('_', ' ')}${d.by === 'user' ? ' (you)' : ''}`;
    case 'task.reassigned':
      return `T-${d.task} reassigned to ${d.node}`;
    case 'git.merge': {
      const r = d.result ?? {};
      if (r.outcome === 'conflict') return `merge of ${d.branch}: conflicts in ${(r.files ?? []).join(', ')}`;
      if (r.error) return `merge of ${d.branch} failed: ${r.error}`;
      return `merged ${d.branch}`;
    }
    case 'git.sync':
      return d.outcome === 'conflict'
        ? `synced integration: conflicts in ${(d.files ?? []).join(', ')}`
        : 'synced integration';
    case 'asked_user':
      return `asked you: ${d.question}`;
    case 'run.team_changed':
      return `team changed (v${d.version})`;
    default:
      return e.kind;
  }
}

/** Board columns (FR-502). */
export const COLUMNS = [
  { id: 'backlog', label: 'Backlog', state: 'queued' },
  { id: 'ready', label: 'Ready', state: 'idle' },
  { id: 'in_progress', label: 'In progress', state: 'working' },
  { id: 'review', label: 'Review', state: 'waiting_gate' },
  { id: 'done', label: 'Done', state: 'done' },
];

/** ⇧h / ⇧l on the board: the previous / next column's status (null at the edges). */
export function shiftStatus(status, delta) {
  const i = COLUMNS.findIndex((c) => c.id === status);
  const j = i + delta;
  return i < 0 || j < 0 || j >= COLUMNS.length ? null : COLUMNS[j].id;
}

/** A task's handle: `#12` for a GitHub issue, `T-3` locally. */
export const handle = (t) => (t.backend === 'github' && t.external_id ? `#${t.external_id}` : `T-${t.id}`);

/** Dependencies of a task not done yet (FR-404). */
export function blockedBy(task, tasks) {
  return task.depends_on.filter((d) => tasks.find((t) => t.id === d)?.status !== 'done');
}

/** Board keyboard cursor: { col, row } moved by h/j/k/l over the visible columns. */
export function moveCursor(cols, cur, key) {
  let { col, row } = cur;
  if (key === 'h') col = Math.max(0, col - 1);
  if (key === 'l') col = Math.min(cols.length - 1, col + 1);
  if (key === 'j') row += 1;
  if (key === 'k') row -= 1;
  const n = cols[col]?.length ?? 0;
  row = n ? Math.max(0, Math.min(n - 1, row)) : 0;
  return { col, row };
}

/** A feature slug from a title (the Launch dialog's default). */
export function slugify(s) {
  return (
    String(s)
      .toLowerCase()
      .normalize('NFKD')
      .replace(/[̀-ͯ]/g, '')
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 30) || ''
  );
}
