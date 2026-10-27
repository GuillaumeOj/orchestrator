import { describe, expect, it } from 'vitest';
import {
  actionLabels,
  afterRemoval,
  due,
  gateKind,
  keyIntent,
  kindLabel,
  move,
  teamQuestion,
  toastFor,
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
    expect(toastFor(items[0], 'approve')).toBe('Allowed once · front-1 · pnpm add x');
    expect(toastFor(items[2], 'reject')).toBe('Skipped · Defer infra');
    const p = [
      { item: 'a', at: 0 },
      { item: 'b', at: 1000 },
    ];
    expect(due(p, UNDO_MS).map((x) => x.item)).toEqual(['a']);
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
    expect(toastFor(spec, 'approve')).toBe('Spec gate approved · fx-lead-1');
    expect(toastFor(spec, 'reply')).toBe('Spec gate sent back with your notes · fx-lead-1');
    expect(toastFor(pr, 'reject')).toBe('PR gate rejected · fx-lead-1');
  });
});
