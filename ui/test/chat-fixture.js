// Every decision kind (P4.1 fixture shapes; actions as decision.rs builds them)
// and chat entries (P4.3 api-contract §1), for the chat column's tests.

import { act, decision } from './fixture.js';

const MIN = 60000;
const agent = (id, role = 'Backend') => ({ agent: id, role, project: 'shop-api', feature: 'checkout', task: 4 });
const OPEN_PANE = act('open', 'Open pane', 'o');

/** One open decision of every kind, oldest first (created `n` minutes ago). */
export function kinds(now = Date.now()) {
  const at = (n) => now - n * MIN;
  return [
    decision({
      id: 'perm-1',
      kind: 'permission',
      level: 1,
      subject: agent('back-1'),
      title: 'Bash · pnpm add zod',
      summary: 'Add zod for the checkout schema',
      context: [{ type: 'tool_call', tool: 'Bash', input: { command: 'pnpm add zod' }, cwd: '/w/back-1' }],
      actions: [
        act('approve', 'Allow once', 'a'),
        act('reject', 'Deny', 'r'),
        act('reply', 'Deny with message', 'enter'),
        act('always', 'Always allow'),
        act('in_pane', 'Answer in the pane'),
        OPEN_PANE,
      ],
      thread: 'run-12',
      created_ms: at(30),
      payload: { always: 'Bash(pnpm add:*)' },
    }),
    decision({
      id: 'q-2',
      kind: 'question',
      level: 1,
      subject: agent('front-1', 'Frontend'),
      title: 'Which auth provider? · 2 questions',
      context: [
        {
          type: 'options',
          questions: [
            {
              question: 'Which auth provider?',
              header: 'Auth',
              multi: false,
              options: [{ label: 'Clerk' }, { label: 'Lucia' }],
            },
            {
              question: 'Which payment methods?',
              header: 'Pay',
              multi: true,
              options: [{ label: 'Card' }, { label: 'SEPA' }],
            },
          ],
        },
      ],
      actions: [act('answer', 'Answer', 'enter'), act('reject', 'Dismiss', 'r'), OPEN_PANE],
      thread: 'run-12',
      created_ms: at(29),
    }),
    decision({
      id: 'gate-spec',
      kind: 'gate',
      gate: 'spec',
      level: 1,
      subject: { agent: 'lead-1', role: 'Lead', project: 'shop-api', feature: 'checkout' },
      title: 'Spec gate · checkout',
      context: [
        { type: 'markdown', text: '**Checkout v2**', source: 'spec.md' },
        { type: 'docs', files: [{ path: '.orchestrator/features/checkout/spec.md', lines: 142 }] },
      ],
      actions: [
        act('approve', 'Approve spec', 'a'),
        act('reject', 'Send back', 'r'),
        act('reply', 'Send back with notes', 'enter'),
        OPEN_PANE,
      ],
      thread: 'run-12',
      created_ms: at(28),
    }),
    decision({
      id: 'gate-pr',
      kind: 'gate',
      gate: 'pr',
      level: 1,
      subject: { agent: 'lead-1', role: 'Lead', project: 'shop-api', feature: 'checkout' },
      title: 'PR gate · checkout',
      context: [
        { type: 'markdown', text: 'orch/checkout/integration → main' },
        { type: 'diff_summary', files: [{ path: 'src/api.rs', added: 40, removed: 3 }], added: 40, removed: 3 },
      ],
      actions: [
        act('approve', 'Approve: push + open PR', 'a'),
        act('reject', 'Reject', 'r'),
        act('reply', 'Reply', 'enter'),
      ],
      thread: 'run-12',
      created_ms: at(27),
    }),
    decision({
      id: 'gate-restart',
      kind: 'gate',
      gate: 'restart',
      level: 1,
      subject: agent('back-1'),
      title: 'Still waiting after a restart · back-1',
      context: [{ type: 'markdown', text: 'back-1 was waiting for your approval of the push plan.' }],
      actions: [act('approve', 'Approve', 'a'), act('reject', 'Reject', 'r'), act('open', 'Open', 'o')],
      thread: 'back-1',
      created_ms: at(26),
      payload: { restarted: true },
    }),
    decision({
      id: 'startup-front-1',
      kind: 'startup',
      level: 1,
      subject: agent('front-1', 'Frontend'),
      title: 'front-1 did not start',
      context: [
        { type: 'error', text: 'No session after 30 s: a dialog may be waiting in its pane', detail: 'exit status 1' },
        { type: 'pty_tail', text: 'Do you trust the files in this folder?' },
      ],
      actions: [act('restart_fresh', 'Restart fresh'), act('dismiss', 'Dismiss'), act('delete', 'Delete'), OPEN_PANE],
      thread: 'front-1',
      created_ms: at(25),
    }),
    decision({
      id: 'ask-9',
      kind: 'ask',
      level: 3,
      subject: agent('back-1'),
      title: 'Run back-1 now? ≈ +2.1 pp of the week',
      context: [
        { type: 'markdown', text: 'Queued: live cap 3/3.' },
        { type: 'cost', week_pp: 2.1, five_hour_pp: 6 },
        { type: 'countdown', default: 'reject', deadline_ms: now + 18 * MIN },
      ],
      actions: [act('approve', 'Run now', 'a'), act('reject', 'Keep queued', 'r'), act('open', 'Open', 'o')],
      thread: 'governor',
      created_ms: at(24),
    }),
    decision({
      id: 'budget-1',
      kind: 'budget',
      level: 3,
      subject: {},
      title: 'Reserve for your own work used up',
      context: [
        { type: 'markdown', text: 'Agents are squeezed harder until Mon 09:00.' },
        { type: 'cost', week_pp: 3.4 },
        { type: 'countdown', default: 'approve', deadline_ms: now + 18 * MIN },
      ],
      actions: [act('approve', 'Apply', 'a'), act('reject', 'Skip', 'r')],
      thread: 'governor',
      created_ms: at(23),
    }),
    decision({
      id: 'error-back-1',
      kind: 'error',
      level: 2,
      subject: agent('back-1'),
      title: 'back-1 · API error 529 (overloaded)',
      context: [
        { type: 'error', text: 'API error 529: overloaded · back-1 stopped after 3 retries' },
        { type: 'pty_tail', text: 'API Error: 529 overloaded' },
      ],
      actions: [act('resume', 'Resume', 'r'), OPEN_PANE],
      thread: 'back-1',
      created_ms: at(22),
    }),
    decision({
      id: 'trust-ux-1',
      kind: 'trust',
      level: 1,
      subject: agent('ux-1', 'UX'),
      title: 'Claude Code asks to trust this folder',
      summary: "Answer it in ux-1's pane: the app never does",
      context: [{ type: 'pty_tail', text: 'Do you trust the files in this folder?' }],
      actions: [OPEN_PANE],
      thread: 'ux-1',
      created_ms: at(21),
    }),
    decision({
      id: 'wait-front-1',
      kind: 'wait',
      level: 1,
      subject: agent('front-1', 'Frontend'),
      title: "A prompt in front-1's pane",
      context: [{ type: 'last_message', text: 'Should I remove the field entirely?', at_ms: now - 6 * MIN }],
      actions: [act('reply', 'Reply', 'enter'), OPEN_PANE],
      thread: 'front-1',
      created_ms: at(20),
    }),
    decision({
      id: 'alert-mcp',
      kind: 'alert',
      level: 2,
      subject: {},
      title: 'Penpot MCP needs authentication',
      context: [{ type: 'error', text: 'OAuth token expired for the Penpot MCP server' }],
      actions: [
        act('reconnect', 'Reconnect', 'a'),
        act('dismiss', 'Dismiss', 'r'),
        act('open_settings', 'Open Settings', 'o'),
      ],
      thread: 'mcp',
      created_ms: at(19),
      payload: { alert: 'mcp_needs_auth', server: 'penpot' },
    }),
    decision({
      id: 'notion-3',
      kind: 'notion',
      level: 2,
      subject: { project: 'shop-api', feature: 'checkout' },
      title: 'Comment on checkout · spec.md',
      context: [{ type: 'markdown', text: 'Sam: Can we drop the retry on 409?', source: 'Notion · checkout spec' }],
      actions: [act('reply', 'Reply', 'enter'), act('dismiss', 'Dismiss', 'r'), act('open', 'Open in Notion ↗', 'o')],
      thread: 'notion',
      created_ms: at(18),
    }),
  ];
}

const DAY = '2026-11-02';

/** A decision entry, open (no line) unless `line` is given. */
export function entry(id, d, over = {}) {
  return {
    id,
    day: DAY,
    at_ms: d.created_ms,
    author: 'core',
    kind: 'decision',
    thread: d.thread,
    decision: d.id,
    rev: 1,
    ...over,
  };
}

/** A closed decision entry: its line and snapshot (api-contract §1). */
export function closedEntry(id, d, line, over = {}) {
  return entry(id, d, { line: { state: 'effect', ...line }, card: d, rev: 2, ...over });
}

/** The morning line. */
export const morning = (at_ms, text = 'Good morning. 1 run in progress, 1 decision waiting.') => ({
  id: 'c-morning',
  day: DAY,
  at_ms,
  author: 'core',
  kind: 'morning',
  text,
  rev: 1,
});

/** `app.now.chat` with these entries. */
export const chatOf = (entries) => ({ day: DAY, entries: Object.fromEntries(entries.map((e) => [e.id, e])) });
