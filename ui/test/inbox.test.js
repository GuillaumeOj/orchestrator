import { describe, expect, it } from 'vitest';
import { baseName } from '../src/lib/format.js';
import {
  actionLabels,
  afterRemoval,
  collapse,
  due,
  effectToast,
  gateKind,
  inputText,
  keyIntent,
  kindLabel,
  move,
  outcomeToast,
  parked,
  pendingToast,
  restarted,
  tailLines,
  teamQuestion,
  UNDO_MS,
  visibleItems,
} from '../src/lib/inbox.js';

const items = [
  {
    id: 'perm-1',
    level: 1,
    kind: 'permission',
    agent_id: 'front-1',
    preview: 'pnpm add x',
    actions: ['a', 'r', 'enter', 'o', 'e'],
  },
  { id: 'wait-solo', level: 1, kind: 'question', agent_id: 'solo', actions: ['enter', 'o'] },
  { id: 'ask-3', level: 3, kind: 'budget', title: 'Defer infra', actions: ['a', 'r'] },
  { id: 'info-t', level: 4, kind: 'info', actions: ['o'] },
];

describe('inbox triage', () => {
  it('filters and hides decisions in their undo window', () => {
    expect(visibleItems(items, 'blocking').map((i) => i.id)).toEqual(['perm-1', 'wait-solo']);
    expect(visibleItems(items, 'all', [{ item: 'perm-1' }]).map((i) => i.id)).toEqual(['wait-solo', 'ask-3', 'info-t']);
  });

  it('j/k move and selection survives removal', () => {
    expect(move(items, 'perm-1', 1)).toBe('wait-solo');
    expect(move(items, 'perm-1', -1)).toBe('perm-1');
    expect(move(items, 'missing', 1)).toBe('perm-1');
    expect(afterRemoval(items, 'info-t')).toBe('ask-3');
    expect(afterRemoval(items, 'perm-1')).toBe('wait-solo');
    expect(afterRemoval([items[0]], 'perm-1')).toBe(null);
  });

  it('keys apply only where the item supports them', () => {
    expect(keyIntent('a', items[0])).toEqual({ kind: 'decide', action: 'approve' });
    expect(keyIntent('r', items[0])).toEqual({ kind: 'decide', action: 'reject' });
    expect(keyIntent('a', items[1])).toBe(null);
    expect(keyIntent('enter', items[1])).toEqual({ kind: 'reply' });
    expect(keyIntent('e', items[0])).toEqual({ kind: 'editor' });
    expect(keyIntent('j', null)).toEqual({ kind: 'move', delta: 1 });
  });

  it('copy and the undo window', () => {
    expect(pendingToast(items[0], 'approve')).toBe('Allowing once · front-1 · pnpm add x');
    expect(pendingToast(items[2], 'reject')).toBe('Skipping · Defer infra');
    expect(effectToast(items[2], 'reject', 'denied')).toBe('Skipped · Defer infra');
    const p = [
      { item: 'a', at: 0 },
      { item: 'b', at: 1000 },
    ];
    expect(due(p, UNDO_MS).map((x) => x.item)).toEqual(['a']);
    // Sent (its act in flight): never due again, whatever the clock says.
    expect(due([{ item: 'a', at: 0, sent: true }], UNDO_MS * 2)).toEqual([]);
    expect(pendingToast({ id: 'error-x', kind: 'error', agent_id: 'x' }, 'reject')).toBe('Resuming · x');
    expect(effectToast({ id: 'error-x', kind: 'error', agent_id: 'x' }, 'reject', 'resumed')).toBe('Resumed · x');
  });

  it('gates and team questions (P2)', () => {
    const spec = {
      id: 'gate-5',
      kind: 'gate',
      agent_id: 'fx-lead-1',
      payload: { action: 'spec_approval' },
      actions: ['a', 'r', 'enter', 'o'],
    };
    const pr = {
      id: 'gate-6',
      kind: 'gate',
      agent_id: 'fx-lead-1',
      payload: { action: 'push_pr' },
      actions: ['a', 'r', 'enter', 'o'],
    };
    const q = {
      id: 'q-7',
      kind: 'question',
      agent_id: 'fx-back-1',
      payload: { action: 'ask_user', question: 'Keep v1?' },
      actions: ['enter', 'r', 'o'],
    };
    expect([gateKind(spec), gateKind(pr), gateKind(q)]).toEqual(['spec', 'pr', null]);
    expect([kindLabel(spec), kindLabel(pr), kindLabel(q), kindLabel(items[1])]).toEqual([
      'Spec gate',
      'PR gate',
      'Question',
      'Waiting for you',
    ]);
    expect(actionLabels(spec)).toEqual({ a: 'Approve spec', r: 'Send back' });
    expect(actionLabels(pr).a).toBe('Approve: push + open PR');
    expect(actionLabels(q)).toEqual({ r: 'Dismiss' });
    expect(teamQuestion(q) && !teamQuestion(items[1])).toBe(true);
    expect(keyIntent('a', spec)).toEqual({ kind: 'decide', action: 'approve' });
    expect(keyIntent('a', q)).toBe(null);
    expect(pendingToast(spec, 'approve')).toBe('Approving the spec gate · fx-lead-1');
    expect(effectToast(spec, 'approve', 'allowed')).toBe('Spec gate approved · fx-lead-1');
    expect(effectToast(spec, 'reply', 'delivered')).toBe('Spec gate sent back with your notes · fx-lead-1 told');
    expect(effectToast(pr, 'reject', 'denied')).toBe('PR gate rejected · fx-lead-1');
    expect(pendingToast(q, 'reply')).toBe('Sending your reply · fx-back-1');
    expect(effectToast(q, 'reply', 'queued')).toBe('Reply queued · fx-back-1');
    expect(effectToast(q, 'reject', 'dismissed')).toBe('Dismissed · fx-back-1');
  });

  const perm = (over = {}) => ({
    id: 'inbox-4',
    level: 1,
    kind: 'permission',
    status: 'open',
    agent_id: 'back-1',
    preview: 'pnpm add zod',
    actions: ['a', 'r', 'enter', 'o', 'in_pane', 'always'],
    payload: {
      tool: 'Bash',
      tool_input: { command: 'pnpm add zod' },
      cwd: '/w/feat/back-1/',
      always: 'Bash(pnpm add:*)',
    },
    ...over,
  });
  const startup = (over = {}) => ({
    id: 'startup-2',
    level: 2,
    kind: 'startup',
    status: 'open',
    agent_id: 'back-1',
    actions: ['o', 'restart_fresh', 'dismiss'],
    payload: { waiting_on: 'startup', mode: 'new', since_ms: 0, tail: 'a\n\nb\n' },
    ...over,
  });
  const restartGate = {
    id: 'gate-3',
    kind: 'gate',
    agent_id: 'back-1',
    actions: ['a', 'r', 'o'],
    payload: { action: 'restart_gate', agent: 'back-1', detail: 'run it', restarted: true },
  };

  it('permission: labels, keys, parked', () => {
    const open = perm();
    const held = perm({ status: 'parked', actions: ['a', 'r', 'enter', 'o'] });
    expect(kindLabel(open)).toBe('Permission');
    expect(actionLabels(open)).toMatchObject({
      a: 'Allow once',
      r: 'Deny',
      enter: 'Deny with message',
      always: 'Always allow',
      in_pane: 'Answer in the pane',
    });
    expect([parked(open), parked(held)]).toEqual([false, true]);
    expect(keyIntent('enter', held)).toEqual({ kind: 'reply' });
    expect(keyIntent('a', held)).toEqual({ kind: 'decide', action: 'approve' });
    expect(baseName(open.payload.cwd)).toBe('back-1');
    expect(baseName('/')).toBe('/');
    expect(inputText({ command: 'ls\n-la', description: 'List' })).toBe('ls\n-la');
    expect(inputText({ file_path: '/a', content: 'x' })).toBe('file_path: /a\ncontent: x');
    expect(inputText({ todos: [1] })).toBe('todos: [\n  1\n]');
    expect(collapse('1\n2\n3', 6)).toEqual({ text: '1\n2\n3', more: 0 });
    expect(collapse('1\n2\n3\n4\n5\n6\n7\n8', 6)).toEqual({ text: '1\n2\n3\n4\n5\n6', more: 2 });
  });

  it('permission toasts: pending, then the effect the core answered', () => {
    const p = perm();
    expect(pendingToast(p, 'approve')).toBe('Allowing once · back-1 · pnpm add zod');
    expect(pendingToast(p, 'reject')).toBe('Denying · back-1 · pnpm add zod');
    expect(pendingToast(p, 'always')).toBe('Always allowing · back-1 · pnpm add zod');
    for (const action of ['approve', 'reject', 'reply', 'always', 'in_pane', 'restart_fresh']) {
      expect(pendingToast(p, action)).not.toMatch(/sent|Reply/);
    }
    expect(effectToast(p, 'approve', 'allowed')).toBe('Allowed · back-1 resumed');
    expect(effectToast(perm({ status: 'parked' }), 'approve', 'resumed')).toBe('Allowed · back-1 resumed');
    expect(effectToast(perm({ status: 'parked' }), 'approve', 'queued')).toBe('Allowed · back-1 queued');
    expect(effectToast(p, 'always', 'allowed')).toBe('Always allowed · Bash(pnpm add:*) · back-1 resumed');
    expect(effectToast(p, 'reject', 'denied', 'use npm')).toBe('Denied · reason sent · back-1');
    expect(effectToast(p, 'reject', 'denied')).toBe('Denied · back-1');
    expect(effectToast(p, 'in_pane', 'in_pane')).toBe('Answer in the pane · back-1');
  });

  it('startup: label, actions and toasts', () => {
    const s = startup();
    const failed = startup({
      actions: ['o', 'restart_fresh', 'dismiss', 'delete'],
      payload: { ...s.payload, failed: 'fresh' },
    });
    expect(kindLabel(s)).toBe('Startup');
    expect(actionLabels(failed)).toEqual({
      o: 'Open pane',
      restart_fresh: 'Restart fresh',
      dismiss: 'Dismiss',
      delete: 'Delete',
    });
    expect(keyIntent('o', s)).toEqual({ kind: 'open' });
    expect(keyIntent('a', s)).toBe(null);
    expect(keyIntent('r', s)).toBe(null);
    expect(tailLines(s.payload.tail)).toBe('a\nb');
    expect(tailLines(Array.from({ length: 15 }, (_, i) => i).join('\n'), 12).split('\n')).toHaveLength(12);
    expect(pendingToast(s, 'restart_fresh')).toBe('Restarting fresh · back-1');
    expect(effectToast(s, 'restart_fresh', 'restarted')).toBe('Restarted fresh · back-1');
    expect(effectToast(s, 'dismiss', 'dismissed')).toBe('Dismissed · back-1');
    expect(effectToast(failed, 'delete', 'dismissed')).toBe('Deleted · back-1');
  });

  it('restart gate and unknown gate actions are not spec gates', () => {
    const unknown = { ...restartGate, payload: { action: 'new_thing' } };
    expect([gateKind(restartGate), gateKind(unknown)]).toEqual(['other', 'other']);
    expect([kindLabel(restartGate), kindLabel(unknown)]).toEqual(['Gate', 'Gate']);
    expect(actionLabels(restartGate)).toEqual({ a: 'Approve', r: 'Reject' });
    expect(restarted(restartGate) && !restarted(unknown)).toBe(true);
    expect(keyIntent('enter', restartGate)).toBe(null);
    expect(pendingToast(restartGate, 'approve')).toBe('Approving the gate · back-1');
    expect(effectToast(restartGate, 'approve', 'delivered')).toBe('Gate approved · back-1 told');
    expect(effectToast(restartGate, 'reject', 'queued')).toBe('Gate rejected · back-1 queued');
  });

  it('outcome events decided elsewhere', () => {
    const p = perm();
    expect(outcomeToast(p, { item: p.id, decided_by: 'user', effect: 'allowed' })).toBe(null);
    expect(outcomeToast(p, { item: p.id, decided_by: 'expired', effect: 'parked' })).toBe(
      'Parked · back-1 stopped and resumes on your answer',
    );
    expect(outcomeToast(p, { item: p.id, decided_by: 'pane', effect: 'in_pane' })).toBe(
      'Answered in the pane · back-1',
    );
    expect(outcomeToast(p, { item: p.id, decided_by: 'rule:7', effect: 'allowed' })).toBe('Allowed by rule 7 · back-1');
    expect(outcomeToast(startup(), { item: 'startup-2', decided_by: 'core', effect: 'restarted' })).toBe(
      'Restarted fresh · back-1',
    );
    expect(outcomeToast(startup(), { item: 'startup-2', decided_by: 'core', effect: 'dismissed' })).toBe(null);
    expect(outcomeToast(undefined, { item: 'inbox-9', decided_by: 'expired', effect: 'parked' })).toBe(
      'Parked · an agent stopped and resumes on your answer',
    );
  });
});
