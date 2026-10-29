// Inbox triage (FR-707, Penpot 04 Inbox and 06 J5) over the core's Decisions
// (P4.1, api-contract §2): j/k move, a/r/⏎/o run the action with that key,
// e open in nvim, x select for batch, u undo for 5 s, c clear the digest.
// Decisions wait out the undo window on the client, then go to the core.
// Copy: docs/design/p4-design-notes.md §3–§5.

import { clock, waited } from './format.js';
import { decisionKind } from './status.js';

export const UNDO_MS = 5000;

export const FILTERS = [
  { id: 'all', label: 'All', test: () => true },
  { id: 'blocking', label: 'Blocking', test: (d) => d.level === 1 },
  { id: 'errors', label: 'Errors', test: (d) => d.level === 2 },
  { id: 'budget', label: 'Budget', test: (d) => d.level === 3 },
];

export const LEVELS = { 1: 'Blocking', 2: 'Errors', 3: 'Budget' };

/** Decisions to show: filtered, without those whose decision is in its undo window. */
export function visibleItems(list, filterId, pending = []) {
  const f = FILTERS.find((x) => x.id === filterId) ?? FILTERS[0];
  const hidden = new Set(pending.map((p) => p.item));
  return list.filter((d) => f.test(d) && !hidden.has(d.id));
}

export { move } from './select.js';

/** Selection after the selected item leaves the list: the next one, else the previous. */
export function afterRemoval(items, removedId) {
  const i = items.findIndex((x) => x.id === removedId);
  const rest = items.filter((x) => x.id !== removedId);
  if (!rest.length) return null;
  return rest[Math.min(Math.max(i, 0), rest.length - 1)].id;
}

// ---------------------------------------------------------------- facts

export const agentOf = (d) => d?.subject?.agent ?? null;
export const parked = (d) => d?.lifecycle?.state === 'parked';
/** The lifecycle's version (api-contract §2.1): +1 per write; a lower one is old. */
export const version = (d) => d?.lifecycle?.version ?? 0;
/** A lifecycle older than the decision held (a late push or answer): ignored. One
 *  at the same version is taken (the core re-pushes it with fresh actions); one
 *  with no version (an older core) too. */
export const older = (lc, held) => lc?.version != null && lc.version < version(held);
export const stale = (d) => d?.lifecycle?.state === 'stale';
/** Still waiting on the user: open or parked. */
export const isOpen = (d) => d?.lifecycle?.state === 'open' || parked(d);
/** Came back after a core restart: the card says so, unless it is parked. */
export const restarted = (d) => d?.payload?.restarted === true && !parked(d);
/** An MCP server that needs sign-in (FR-604): Reconnect, Open Settings, Dismiss. */
const mcpAlert = (d) => d?.kind === 'alert' && d.payload?.alert === 'mcp_needs_auth';
export const actionByKey = (d, key) => d?.actions?.find((a) => a.key === key) ?? null;
/** A decision's context block of that type, if any. */
export const blockOf = (d, type) => d?.context?.find((b) => b.type === type) ?? null;
/** Actions with no key: buttons in the detail only. */
export const keyless = (d) => (d?.actions ?? []).filter((a) => !a.key);
/** Kind label (notes §3.2), from the decision-kind palette. */
export const kindLabel = (d) => decisionKind(d).label;

/** "back-1 · Backend · shop-api › checkout · T-4" (notes §3.1), parts left out when absent. */
export function subjectLine(d) {
  const s = d?.subject ?? {};
  const where = [s.project, s.feature].filter(Boolean).join(' › ');
  return [s.agent, s.role, where, s.task != null ? `T-${s.task}` : null].filter(Boolean).join(' · ');
}

// The core answers these at once (no undo window): where to go, or a reconnect.
const AT_ONCE = new Set(['open', 'reconnect', 'open_settings']);

/** What an action id does: decide (undo window), reply or answer (a text first),
 *  open the agent's pane, or ask the core at once. */
export function intentFor(d, id) {
  if (id === 'in_pane') return { kind: 'pane' };
  if (id === 'reply' || id === 'answer') return { kind: id };
  // A Notion comment, a battery or budget note: the core says where to go.
  if (id === 'open' && agentOf(d) && d.kind !== 'notion') return { kind: 'pane' };
  if (AT_ONCE.has(id)) return { kind: 'core', action: id };
  return { kind: 'decide', action: id };
}

/** What a key does on the selected decision (null: not handled). */
export function keyIntent(key, d) {
  switch (key) {
    case 'j':
    case 'arrowdown':
      return { kind: 'move', delta: 1 };
    case 'k':
    case 'arrowup':
      return { kind: 'move', delta: -1 };
    case 'x':
      return d ? { kind: 'mark' } : null;
    case 'u':
      return { kind: 'undo' };
    case 'e':
      return agentOf(d) ? { kind: 'editor' } : null;
    case 'a':
    case 'r':
    case 'enter':
    case 'o': {
      const act = actionByKey(d, key);
      return act ? intentFor(d, act.id) : null;
    }
    default:
      return null;
  }
}

// ---------------------------------------------------------------- text helpers

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

/** The last `n` non-blank lines of a PTY tail. */
export function tailLines(text, n = 12) {
  const lines = String(text ?? '')
    .split('\n')
    .map((l) => l.trimEnd())
    .filter((l) => l.trim());
  return lines.slice(-n).join('\n');
}

const cut = (text, n) => {
  const s = String(text ?? '');
  return s.length > n ? `${s.slice(0, n - 1)}…` : s;
};

/** "Answer: Clerk\nCard, Apple Pay": the chosen labels per question, one line each. */
export const answerText = (choices) => choices.map((c) => c.join(', ')).join('\n');

// ---------------------------------------------------------------- copy

const who = (d) => (agentOf(d) ? ` · ${agentOf(d)}` : '');
const toolCall = (d) => blockOf(d, 'tool_call');
const what = (d) => {
  const t = inputText(toolCall(d)?.input);
  return t ? ` · ${t.split('\n')[0].slice(0, 40)}` : '';
};
const GATE_WORD = { spec: 'spec gate', pr: 'PR gate', restart: 'gate' };

/** What will be sent while a decision waits out its undo window ("Allowing
 *  once"): the marker's text, and the start of the undo toast. */
export function pendingVerb(d, action) {
  // Dismiss (a stale decision's only action) is never a deny, a reject or a skip.
  if (action === 'dismiss') return 'Dismissing';
  switch (d.kind) {
    case 'permission':
      return { approve: 'Allowing once', always: 'Always allowing' }[action] ?? 'Denying';
    case 'startup':
      return { restart_fresh: 'Restarting fresh', delete: 'Deleting' }[action] ?? 'Dismissing';
    case 'budget':
      return action === 'approve' ? 'Applying' : 'Skipping';
    case 'ask':
      return action === 'approve' ? 'Running now' : 'Keeping queued';
    case 'error':
      if (action === 'resume') return 'Resuming';
      if (action === 'restart_fresh') return 'Restarting fresh';
      break; // Dismiss: as any other kind's
    case 'gate': {
      const obj = ` the ${GATE_WORD[d.gate] ?? 'gate'}`;
      if (action === 'reply') return `Sending${obj} back with your notes`;
      return `${action === 'approve' ? 'Approving' : 'Rejecting'}${obj}`;
    }
  }
  return action === 'dismiss' || action === 'reject' ? 'Dismissing' : 'Sending your answer';
}

/** The undo toast: what will be sent, not what happened (the core has not answered yet). */
export function pendingToast(d, action) {
  const verb = pendingVerb(d, action);
  if (d.kind === 'permission') return `${verb}${who(d)}${what(d)}`;
  if (d.kind === 'budget') return `${verb} · ${d.title}`;
  if (verb === 'Dismissing' && d.kind !== 'startup') return `${verb} · ${cut(d.title, 40)}`;
  return `${verb}${who(d)}`;
}

const BY = {
  user: 'by you',
  pane: 'in the pane',
  expired: 'when the hold expired',
  core: 'by the app',
  policy: 'by policy',
  overwatch: 'by the Overwatch',
};
/** "by you", "by rule git-safe" (notes §3.4). */
const byText = (by) => (by?.startsWith('rule:') ? `by rule ${by.slice(5)}` : (BY[by] ?? ''));
/** "Decided by you", or "Decided" alone when who is unknown. */
const byLine = (verb, by) => [verb, byText(by)].filter(Boolean).join(' ');

/** "PR opened · checkout #12 ↗". */
function prOpened(d, url) {
  const n = String(url ?? '').match(/\/pull\/(\d+)/)?.[1];
  const name = d.subject?.feature ?? d.subject?.project ?? '';
  return `PR opened${name ? ` · ${name}` : ''}${n ? ` #${n}` : ''} ↗`;
}

// A spec gate's answer: decided in another window, the effect alone does not say which.
const specAnswer = (action) =>
  ({ approve: 'Spec approved', reject: 'Spec sent back', reply: 'Spec sent back with your notes' })[action] ??
  'Spec gate answered';

/**
 * The effect, said plainly (notes §4), from the decision's lifecycle. `mine`:
 * the action and message this window sent, when it did (else read from the
 * effect alone).
 */
export function effectText(d, mine = {}) {
  const lc = d.lifecycle ?? {};
  const agent = agentOf(d) ?? 'the agent';
  const eta = lc.effect_detail ? ` starts ${lc.effect_detail}` : '';
  const { action, message } = mine;
  if (lc.state === 'failed') return `Failed: ${cut(lc.error ?? 'unknown error', 80)}`;
  switch (lc.effect) {
    case 'allowed':
      if (action === 'always') {
        const rule = d.payload?.always;
        return `Always allowed${rule ? ` · ${rule}` : ''} · ${agent} resumed`;
      }
      return `Allowed · ${agent} resumed`;
    case 'denied':
      return message?.trim() || action === 'reply' ? `Denied · reason sent to ${agent}` : `Denied · ${agent}`;
    case 'resumed':
      if (d.kind === 'permission') return `Allowed · ${agent} resumed`;
      if (d.kind === 'ask') return `Running now · ${agent}`;
      if (d.kind === 'error') return `Resumed · ${agent}`;
      if (d.kind === 'gate') {
        if (d.gate === 'spec') return `${specAnswer(action)} · ${agent} resumed`;
        if (d.gate === 'pr') return `PR gate sent back · ${agent} resumed`;
        return `${action === 'reject' ? 'Rejected' : 'Approved'} · ${agent} resumed`;
      }
      return `Answer sent · ${agent} resumed`;
    case 'delivered':
      return `Answer delivered · ${agent} reads it next`;
    case 'queued':
      if (d.kind === 'ask') return `Kept queued · ${agent}${eta}`;
      if (d.kind === 'gate' && d.gate === 'spec') return `${specAnswer(action)} · ${agent}${eta}`;
      return `Answer queued · ${agent}${eta}`;
    case 'pr_opened':
      return prOpened(d, lc.effect_detail);
    case 'restarted':
      return `Restarted fresh · ${agent}`;
    case 'dismissed':
      if (mcpAlert(d)) return `Dismissed until it fails again · ${d.payload?.name ?? d.payload?.server ?? ''}`;
      return `Dismissed · ${cut(d.title, 40)}`;
    // The Governor's ask is answered at once with applied / skipped.
    case 'applied':
      return d.kind === 'ask' ? `Running now · ${agent}` : `Applied · ${d.title}`;
    case 'skipped':
      return d.kind === 'ask' ? `Kept queued · ${agent}${eta}` : `Skipped · ${d.title}`;
    case 'deleted':
      return `Deleted · ${agent}`;
    default:
      return null;
  }
}

/**
 * The toast for a lifecycle push (notes §4): only for effect, dismissed,
 * failed and parked, never before the core sent. `mine` = what this window decided (its
 * own toast says the effect); someone else's decision toasts only as P4.0 did:
 * the pane, a rule, the hold's expiry, a fresh restart by the app.
 */
export function decisionToast(d, mine = null) {
  const lc = d?.lifecycle ?? {};
  const agent = agentOf(d) ?? 'an agent';
  if (lc.state === 'parked') return `Parked · ${agent} stopped and resumes on your answer`;
  // A dismissal (Dismiss, Delete, an alert's Restart fresh, answered in the
  // pane) keeps the `dismissed` state; its effect says what happened.
  if (lc.state !== 'effect' && lc.state !== 'failed' && lc.state !== 'dismissed') return null;
  if (mine || lc.decided_by === 'user' || lc.state === 'failed') return effectText(d, mine ?? {});
  const by = lc.decided_by ?? '';
  if (lc.effect === 'restarted') return `Restarted fresh · ${agent}`;
  if (by === 'pane') return `Answered in the pane · ${agent}`;
  if (by.startsWith('rule:'))
    return `${lc.effect === 'denied' ? 'Denied' : 'Allowed'} by rule ${by.slice(5)} · ${agent}`;
  return null;
}

/**
 * The lifecycle marker (notes §3.4): one line, never a spinner. `pending` = this
 * window's decision still in its undo window. Returns { state, text, url? }.
 */
export function marker(d, pending = null, now = Date.now()) {
  const lc = d.lifecycle ?? {};
  const agent = agentOf(d) ?? 'the agent';
  const at = (ms) => (ms ? ` · ${clock(ms, now)}` : '');
  if (pending && !pending.sent) return { state: 'decided', text: `${pendingVerb(d, pending.action)} · Undo u` };
  switch (lc.state) {
    case 'parked':
      return { state: 'parked', text: `Parked · ${agent} stopped; resumes on your answer` };
    case 'decided':
      return { state: 'decided', text: `${byLine('Decided', lc.decided_by)} · sending` };
    case 'sent':
      if (d.kind === 'gate' && d.gate === 'pr')
        return { state: 'sent', text: `Pushing ${d.payload?.branch ?? 'the branch'}…` };
      return { state: 'sent', text: `Sent to ${agent}${at(lc.sent_at_ms)}` };
    case 'effect': {
      const url = lc.effect === 'pr_opened' ? lc.effect_detail : null;
      return { state: 'effect', text: `${effectText(d) ?? 'Done'}${at(lc.effect_at_ms)}`, url };
    }
    case 'failed':
      return { state: 'failed', text: `Failed · ${lc.error ?? 'unknown error'}` };
    case 'stale':
      return { state: 'stale', text: `Stale · ${lc.stale_why ?? 'the context changed'}` };
    case 'dismissed':
      return {
        state: 'dismissed',
        text: `${byLine('Dismissed', lc.decided_by)}${at(lc.decided_at_ms)}`,
      };
    case 'interrupted':
      return { state: 'interrupted', text: `Interrupted · ${agent} stopped before it was answered` };
    default:
      return { state: 'open', text: `Waiting for you · ${waited(d.created_ms, now)}` };
  }
}

/** Due pending decisions (their undo window passed). */
export function due(pending, now) {
  return pending.filter((p) => !p.sent && now - p.at >= UNDO_MS);
}
