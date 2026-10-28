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

export { move } from './select.js';

/** Selection after the selected item leaves the list: the next one, else the previous. */
export function afterRemoval(items, removedId) {
  const i = items.findIndex((x) => x.id === removedId);
  const rest = items.filter((x) => x.id !== removedId);
  if (!rest.length) return null;
  return rest[Math.min(Math.max(i, 0), rest.length - 1)].id;
}

/** An MCP server that needs sign-in (FR-604): a Reconnect, Open Settings, Dismiss item (D3). */
export const mcpAlert = (item) => item?.kind === 'alert' && item.payload?.alert === 'mcp_needs_auth';

/** Weekly report digest, Notion comment, Linear sync note: `o` goes through the core. */
export const coreOpens = (item) =>
  item?.payload?.digest === 'weekly_report' ||
  item?.payload?.source === 'notion' ||
  item?.payload?.action === 'task_conflict';

/** What a key does on the selected item (null: not handled). */
export function keyIntent(key, item) {
  // a Reconnect and o Open Settings go to the core at once (no undo: nothing is decided).
  if (mcpAlert(item) && (key === 'a' || key === 'o'))
    return { kind: 'alert', action: key === 'a' ? 'reconnect' : 'open' };
  // P3 items whose `o` the core answers with where to go (a report, a page, a task).
  if (key === 'o' && item?.actions?.includes('o') && coreOpens(item)) return { kind: 'alert', action: 'open' };
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

/** A gate's flavour: `spec` (spec approval), `pr` (push + pull request), or
 *  `other` (a legacy hold back after a restart, or an action this UI doesn't know). */
export function gateKind(item) {
  if (item?.kind !== 'gate') return null;
  return { spec_approval: 'spec', push_pr: 'pr' }[item.payload?.action] ?? 'other';
}

const GATE_LABEL = { spec: 'Spec gate', pr: 'PR gate', other: 'Gate' };

/** A team agent's question (ask_user): answered here, its reply becomes a turn. */
export function teamQuestion(item) {
  return item?.kind === 'question' && item.payload?.action === 'ask_user';
}

/** Came back after a core restart (api-contract §1): the card says so. */
export function restarted(item) {
  return item?.payload?.restarted === true;
}

/** A permission whose hold expired: the agent stopped and resumes on the answer. */
export function parked(item) {
  return item?.status === 'parked';
}

/** Kind label in the list and the detail header. */
export function kindLabel(item) {
  const g = gateKind(item);
  if (g) return GATE_LABEL[g];
  if (teamQuestion(item)) return 'Question';
  if (item?.payload?.source === 'notion') return 'Question · Notion';
  return (
    {
      permission: 'Permission',
      startup: 'Did not start',
      question: 'Waiting for you',
      trust: 'Trust dialog',
      error: 'Error',
      alert: 'Alert',
      budget: 'Budget',
      info: 'Info',
    }[item?.kind] ?? item?.kind
  );
}

/** Labels for an item's actions (component copy, Penpot 02 Inbox item), keyed by action. */
export function actionLabels(item) {
  if (teamQuestion(item)) return { r: 'Dismiss' };
  if (item?.payload?.source === 'notion') return { r: 'Dismiss', o: 'Open in Notion ↗' };
  switch (item?.kind) {
    case 'gate': {
      const g = gateKind(item);
      if (g === 'pr') return { a: 'Approve: push + open PR', r: 'Reject' };
      if (g === 'spec') return { a: 'Approve spec', r: 'Send back' };
      return { a: 'Approve', r: 'Reject' };
    }
    case 'permission':
      return {
        a: 'Allow once',
        r: 'Deny',
        enter: 'Deny with message',
        always: 'Always allow',
        in_pane: 'Answer in the pane',
      };
    case 'startup':
      return { o: 'Open pane', restart_fresh: 'Restart fresh', dismiss: 'Dismiss', delete: 'Delete' };
    case 'budget':
      return { a: 'Apply', r: 'Skip' };
    case 'alert':
      return mcpAlert(item) ? { a: 'Reconnect', r: 'Dismiss', o: 'Open Settings' } : { a: 'Dismiss' };
    case 'error':
      return { r: 'Resume' };
    default:
      return { a: 'Approve', r: 'Reject' };
  }
}

/** A tool call's input as text: a lone string field (a Bash command, a path) as is,
 *  else one `key: value` per field. The description is the card's title already. */
export function inputText(input) {
  if (input == null) return '';
  if (typeof input !== 'object') return String(input);
  const fields = Object.entries(input).filter(([k]) => k !== 'description');
  if (fields.length === 1 && typeof fields[0][1] === 'string') return fields[0][1];
  return fields.map(([k, v]) => `${k}: ${typeof v === 'string' ? v : JSON.stringify(v, null, 2)}`).join('\n');
}

/** Text cut to its first `n` lines; `more` = the lines left out. */
export function collapse(text, n = 6) {
  const lines = String(text ?? '').split('\n');
  return lines.length <= n ? { text, more: 0 } : { text: lines.slice(0, n).join('\n'), more: lines.length - n };
}

/** The last `n` non-blank lines of a PTY tail (a startup card's preview). */
export function tailLines(text, n = 12) {
  const lines = String(text ?? '')
    .split('\n')
    .map((l) => l.trimEnd())
    .filter((l) => l.trim());
  return lines.slice(-n).join('\n');
}

/** A worktree path shortened to its folder name (the full path goes on hover). */
export function cwdName(cwd) {
  return (
    String(cwd ?? '')
      .replace(/\/+$/, '')
      .split('/')
      .pop() ?? ''
  );
}

const who = (item) => (item.agent_id ? ` · ${item.agent_id}` : '');
const what = (item) => (item.preview ? ` · ${item.preview.split('\n')[0].slice(0, 40)}` : '');

/** Toast while a decision waits out its undo window: what will be sent, not
 *  what happened (the core has not answered yet). */
export function pendingToast(item, action) {
  if (item.kind === 'permission') {
    const verb = { approve: 'Allowing once', always: 'Always allowing', in_pane: 'Handing over to the pane' }[action];
    return `${verb ?? 'Denying'}${who(item)}${what(item)}`;
  }
  if (item.kind === 'startup') {
    return `${{ restart_fresh: 'Restarting fresh', delete: 'Deleting' }[action] ?? 'Dismissing'}${who(item)}`;
  }
  if (mcpAlert(item)) return `Dismissing · ${item.payload.name ?? item.payload.server}`;
  if (item.payload?.source === 'notion' && action === 'reject') return `Dismissing · ${item.title}`;
  if (item.kind === 'budget') return `${action === 'approve' ? 'Applying' : 'Skipping'} · ${item.title}`;
  if (item.kind === 'alert' || item.kind === 'info' || (teamQuestion(item) && action === 'reject')) {
    return `Dismissing${who(item)}`;
  }
  const g = gateKind(item);
  const obj = g ? ` the ${GATE_LABEL[g].toLowerCase()}` : '';
  if (action === 'reply')
    return g ? `Sending${obj} back with your notes${who(item)}` : `Sending your reply${who(item)}`;
  return `${action === 'approve' ? 'Approving' : 'Rejecting'}${obj}${who(item)}`;
}

// What the agent got, by `effect` (api-contract §2).
const AGENT_EFFECT = { resumed: 'resumed', allowed: 'resumed', delivered: 'told', queued: 'queued' };

/** Toast once the core answered `inbox_act` with `{ effect }`: the effect, said plainly. */
export function effectToast(item, action, effect, message = null) {
  const agent = item.agent_id ?? 'the agent';
  const after = AGENT_EFFECT[effect] ? ` · ${agent} ${AGENT_EFFECT[effect]}` : who(item);
  if (item.kind === 'permission') {
    if (effect === 'in_pane') return `Answer in the pane${who(item)}`;
    if (action === 'always') return `Always allowed${item.payload?.always ? ` · ${item.payload.always}` : ''}${after}`;
    if (action === 'approve') return `Allowed${after}`;
    return message?.trim() ? `Denied · reason sent${who(item)}` : `Denied${who(item)}`;
  }
  if (item.kind === 'startup') {
    if (effect === 'restarted') return `Restarted fresh${who(item)}`;
    return `${action === 'delete' ? 'Deleted' : 'Dismissed'}${who(item)}`;
  }
  if (item.kind === 'budget') return action === 'approve' ? `Applied · ${item.title}` : `Skipped · ${item.title}`;
  if (mcpAlert(item)) return `Dismissed until it fails again · ${item.payload.name ?? item.payload.server}`;
  if (item.payload?.source === 'notion' && action === 'reject') return `Dismissed · ${item.title}`;
  if (effect === 'dismissed' || item.kind === 'alert' || item.kind === 'info') return `Dismissed${who(item)}`;
  const g = gateKind(item);
  if (g) {
    if (action === 'reply') return `${GATE_LABEL[g]} sent back with your notes${after}`;
    return `${GATE_LABEL[g]} ${action === 'approve' ? 'approved' : 'rejected'}${after}`;
  }
  if (action === 'reply') {
    return effect === 'queued'
      ? `Reply queued · ${agent}`
      : effect === 'resumed'
        ? `Reply sent · ${agent} resumed`
        : `Reply sent${who(item)}`;
  }
  return `${action === 'approve' ? 'Approved' : 'Rejected'}${after}`;
}

/** Toast for an `inbox_outcome` event decided elsewhere (the pane, a rule, the
 *  hold's expiry, the core); null for the user's own decisions (their
 *  `inbox_act` answer has its toast) and outcomes worth no words. */
export function outcomeToast(item, outcome) {
  const by = outcome?.decided_by ?? '';
  if (by === 'user') return null;
  const agent = item?.agent_id ?? 'an agent';
  if (outcome.effect === 'parked') return `Parked · ${agent} stopped and resumes on your answer`;
  if (outcome.effect === 'restarted') return `Restarted fresh · ${agent}`;
  if (by === 'pane') return `Answered in the pane · ${agent}`;
  if (by.startsWith('rule:'))
    return `${outcome.effect === 'denied' ? 'Denied' : 'Allowed'} by rule ${by.slice(5)} · ${agent}`;
  return null;
}

/** Due pending decisions (their undo window passed). */
export function due(pending, now) {
  return pending.filter((p) => now - p.at >= UNDO_MS);
}
