// Inbox triage (FR-707, Penpot 04 Inbox and 06 J5): j/k move, a approve,
// r reject, ⏎ reply, o open, e open in nvim, x select for batch, u undo for 5 s.
// Decisions wait out the undo window on the client, then go to the core.

export const UNDO_MS = 5000;

export const FILTERS = [
  { id: 'all', label: 'All', test: () => true },
  { id: 'blocking', label: 'Blocking', test: (i) => i.level === 1 },
  { id: 'errors', label: 'Errors', test: (i) => i.level === 2 },
  { id: 'budget', label: 'Budget', test: (i) => i.level === 3 },
  { id: 'info', label: 'Info', test: (i) => i.level === 4 },
];

export const LEVELS = { 1: 'Blocking', 2: 'Errors', 3: 'Budget', 4: 'Info' };

/** Items to show: filtered, without those whose decision is in its undo window. */
export function visibleItems(items, filterId, pending = []) {
  const f = FILTERS.find((x) => x.id === filterId) ?? FILTERS[0];
  const hidden = new Set(pending.map((p) => p.item));
  return items.filter((i) => f.test(i) && !hidden.has(i.id));
}

export function move(items, selectedId, delta) {
  if (!items.length) return null;
  const i = items.findIndex((x) => x.id === selectedId);
  if (i < 0) return items[0].id;
  return items[Math.max(0, Math.min(items.length - 1, i + delta))].id;
}

/** Selection after the selected item leaves the list: the next one, else the previous. */
export function afterRemoval(items, removedId) {
  const i = items.findIndex((x) => x.id === removedId);
  const rest = items.filter((x) => x.id !== removedId);
  if (!rest.length) return null;
  return rest[Math.min(Math.max(i, 0), rest.length - 1)].id;
}

/** What a key does on the selected item (null: not handled). */
export function keyIntent(key, item) {
  switch (key) {
    case 'j':
    case 'arrowdown':
      return { kind: 'move', delta: 1 };
    case 'k':
    case 'arrowup':
      return { kind: 'move', delta: -1 };
    case 'x':
      return item ? { kind: 'mark' } : null;
    case 'u':
      return { kind: 'undo' };
    case 'a':
    case 'r':
    case 'enter':
    case 'o':
    case 'e': {
      if (!item) return null;
      if (!item.actions.includes(key)) return null;
      if (key === 'a') return { kind: 'decide', action: 'approve' };
      if (key === 'r') return { kind: 'decide', action: 'reject' };
      if (key === 'enter') return { kind: 'reply' };
      if (key === 'o') return { kind: 'open' };
      return { kind: 'editor' };
    }
    default:
      return null;
  }
}

/** A gate's flavour: `spec` (spec approval) or `pr` (push + pull request). */
export function gateKind(item) {
  if (item?.kind !== 'gate') return null;
  return item.payload?.action === 'push_pr' ? 'pr' : 'spec';
}

/** A team agent's question (ask_user): answered here, its reply becomes a turn. */
export function teamQuestion(item) {
  return item?.kind === 'question' && item.payload?.action === 'ask_user';
}

/** Kind label in the list and the detail header. */
export function kindLabel(item) {
  const g = gateKind(item);
  if (g) return g === 'pr' ? 'PR gate' : 'Spec gate';
  if (teamQuestion(item)) return 'Question';
  return (
    {
      permission: 'Permission',
      question: 'Waiting for you',
      trust: 'Trust dialog',
      error: 'Error',
      alert: 'Alert',
      budget: 'Budget',
      info: 'Info',
    }[item?.kind] ?? item?.kind
  );
}

/** Labels for an item's a/r keys (component copy, Penpot 02 Inbox item). */
export function actionLabels(item) {
  if (teamQuestion(item)) return { r: 'Dismiss' };
  switch (item?.kind) {
    case 'gate':
      return gateKind(item) === 'pr'
        ? { a: 'Approve: push + open PR', r: 'Reject' }
        : { a: 'Approve spec', r: 'Send back' };
    case 'permission':
      return { a: 'Allow once', r: 'Deny' };
    case 'budget':
      return { a: 'Apply', r: 'Skip' };
    case 'alert':
      return { a: 'Dismiss' };
    case 'error':
      return { r: 'Resume' };
    default:
      return { a: 'Approve', r: 'Reject' };
  }
}

/** Toast copy after a decision (Penpot 05: past tense plus the object). */
export function toastFor(item, action) {
  const who = item.agent_id ? ` · ${item.agent_id}` : '';
  const what = item.preview ? ` · ${item.preview.split('\n')[0].slice(0, 40)}` : '';
  if (item.kind === 'permission') return action === 'approve' ? `Allowed once${who}${what}` : `Denied${who}${what}`;
  if (item.kind === 'budget') return action === 'approve' ? `Applied · ${item.title}` : `Skipped · ${item.title}`;
  if (item.kind === 'alert' || item.kind === 'info') return `Dismissed${who}`;
  if (item.kind === 'gate') {
    const g = gateKind(item) === 'pr' ? 'PR gate' : 'Spec gate';
    if (action === 'reply') return `${g} sent back with your notes${who}`;
    return `${g} ${action === 'approve' ? 'approved' : 'rejected'}${who}`;
  }
  if (teamQuestion(item) && action === 'reject') return `Dismissed${who}`;
  if (action === 'reply') return `Reply sent${who}`;
  return `${action === 'approve' ? 'Approved' : 'Rejected'}${who}`;
}

/** Due pending decisions (their undo window passed). */
export function due(pending, now) {
  return pending.filter((p) => now - p.at >= UNDO_MS);
}
