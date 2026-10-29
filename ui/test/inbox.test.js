// Inbox helpers over Decisions (P4.1, api-contract §2; copy: p4-design-notes §3–§4).
import { describe, expect, it } from 'vitest';
import { baseName, waited } from '../src/lib/format.js';
import {
  afterRemoval,
  answerText,
  collapse,
  decisionToast,
  due,
  effectText,
  inputText,
  keyIntent,
  kindLabel,
  marker,
  move,
  pendingToast,
  subjectLine,
  tailLines,
  UNDO_MS,
  visibleItems,
} from '../src/lib/inbox.js';
import { act, budgetDecision, decision, permDecision } from './fixture.js';

const T = new Date(2026, 9, 29, 14, 0).getTime();
const at = (h, m) => new Date(2026, 9, 29, h, m).getTime();
const lc = (d, lifecycle) => ({ ...d, lifecycle });
const spec = decision({
  id: 'gate-5',
  kind: 'gate',
  gate: 'spec',
  subject: { agent: 'lead-1', role: 'Lead', project: 'shop-api', feature: 'checkout' },
  title: 'Spec gate · checkout',
  actions: [
    act('approve', 'Approve spec', 'a'),
    act('reject', 'Send back', 'r'),
    act('reply', 'Send back with notes', 'enter'),
    act('open', 'Open pane', 'o'),
  ],
});
const pr = {
  ...spec,
  id: 'gate-6',
  gate: 'pr',
  title: 'PR gate · checkout',
  payload: { branch: 'orch/checkout/integration' },
};
const question = decision({
  id: 'q-2',
  kind: 'question',
  subject: { agent: 'front-1' },
  actions: [act('answer', 'Answer', 'enter'), act('reject', 'Dismiss', 'r'), act('open', 'Open pane', 'o')],
});
const notion = decision({
  id: 'alert-9',
  kind: 'notion',
  level: 2,
  actions: [act('dismiss', 'Dismiss', 'r'), act('open', 'Open in Notion ↗', 'o')],
});
const mcp = decision({
  id: 'alert-3',
  kind: 'alert',
  level: 2,
  payload: { alert: 'mcp_needs_auth', name: 'Penpot' },
  actions: [
    act('reconnect', 'Reconnect', 'a'),
    act('dismiss', 'Dismiss', 'r'),
    act('open_settings', 'Open Settings', 'o'),
  ],
});

describe('inbox triage', () => {
  const list = [permDecision(), question, budgetDecision()];

  it('filters by level and hides decisions in their undo window', () => {
    expect(visibleItems(list, 'blocking').map((d) => d.id)).toEqual(['perm-1', 'q-2']);
    expect(visibleItems(list, 'budget').map((d) => d.id)).toEqual(['ask-7']);
    expect(visibleItems(list, 'all', [{ item: 'perm-1' }]).map((d) => d.id)).toEqual(['q-2', 'ask-7']);
  });

  it('j/k move and selection survives removal', () => {
    expect(move(list, 'perm-1', 1)).toBe('q-2');
    expect(afterRemoval(list, 'perm-1')).toBe('q-2');
    expect(afterRemoval(list, 'ask-7')).toBe('q-2');
    expect(afterRemoval([list[0]], 'perm-1')).toBe(null);
  });

  it('a/r/⏎/o run the action that has the key; core-answered ones go at once', () => {
    const p = permDecision();
    expect(keyIntent('a', p)).toEqual({ kind: 'decide', action: 'approve' });
    expect(keyIntent('r', p)).toEqual({ kind: 'decide', action: 'reject' });
    expect(keyIntent('enter', p)).toEqual({ kind: 'reply' });
    expect(keyIntent('o', p)).toEqual({ kind: 'pane' });
    expect(keyIntent('e', p)).toEqual({ kind: 'editor' });
    expect(keyIntent('a', question)).toBe(null);
    expect(keyIntent('enter', question)).toEqual({ kind: 'answer' });
    expect(keyIntent('o', notion)).toEqual({ kind: 'core', action: 'open' });
    expect(keyIntent('r', notion)).toEqual({ kind: 'decide', action: 'dismiss' });
    expect(keyIntent('a', mcp)).toEqual({ kind: 'core', action: 'reconnect' });
    expect(keyIntent('o', mcp)).toEqual({ kind: 'core', action: 'open_settings' });
    expect(keyIntent('e', budgetDecision())).toBe(null);
    expect(keyIntent('j', null)).toEqual({ kind: 'move', delta: 1 });
  });

  it('subject line and kind labels (notes §3.1–3.2)', () => {
    expect(
      subjectLine(
        decision({ subject: { agent: 'back-1', role: 'Backend', project: 'shop-api', feature: 'checkout', task: 4 } }),
      ),
    ).toBe('back-1 · Backend · shop-api › checkout · T-4');
    expect(subjectLine(decision({ subject: { agent: 'fix-feed', project: 'blog' } }))).toBe('fix-feed · blog');
    expect(subjectLine(budgetDecision())).toBe('');
    expect([spec, pr, question, budgetDecision(), notion].map(kindLabel)).toEqual([
      'Spec gate',
      'PR gate',
      'Question',
      'Budget',
      'Notion comment',
    ]);
    expect(kindLabel(decision({ kind: 'gate', gate: 'restart' }))).toBe('Restart gate');
    expect(kindLabel(decision({ kind: 'ask' }))).toBe('Run now?');
  });

  it('an answer: the chosen labels per question, one line each', () => {
    expect(answerText([['Clerk'], ['Card', 'Apple Pay']])).toBe('Clerk\nCard, Apple Pay');
  });

  it('text helpers and the undo window', () => {
    expect(baseName('/w/feat/back-1/')).toBe('back-1');
    expect(inputText({ command: 'ls\n-la', description: 'List' })).toBe('ls\n-la');
    expect(inputText({ file_path: '/a', content: 'x' })).toBe('file_path: /a\ncontent: x');
    expect(collapse('1\n2\n3\n4\n5\n6\n7\n8', 6)).toEqual({ text: '1\n2\n3\n4\n5\n6', more: 2 });
    expect(tailLines('a\n\nb\n')).toBe('a\nb');
    expect(
      due(
        [
          { item: 'a', at: 0 },
          { item: 'b', at: 1000 },
        ],
        UNDO_MS,
      ).map((x) => x.item),
    ).toEqual(['a']);
    expect(due([{ item: 'a', at: 0, sent: true }], UNDO_MS * 2)).toEqual([]);
    expect([
      waited(T - 20000, T),
      waited(T - 3 * 60000, T),
      waited(T - 65 * 60000, T),
      waited(T - 50 * 3600000, T),
    ]).toEqual(['just now', '3 min', '1 h 5 min', '2 d']);
  });
});

describe('toasts: what will be sent, then the pushed effect (notes §4)', () => {
  const p = permDecision();
  const effect = (d, effect, more = {}) => lc(d, { state: 'effect', decided_by: 'user', effect, ...more });

  it('pending (the undo window) names the action, never the effect', () => {
    expect(pendingToast(p, 'approve')).toBe('Allowing once · front-1 · pnpm add @stripe/stripe-js');
    expect(pendingToast(p, 'always')).toBe('Always allowing · front-1 · pnpm add @stripe/stripe-js');
    expect(pendingToast(spec, 'approve')).toBe('Approving the spec gate · lead-1');
    expect(pendingToast(spec, 'reply')).toBe('Sending the spec gate back with your notes · lead-1');
    expect(pendingToast(budgetDecision(), 'reject')).toBe('Skipping · Defer "infra" until after the weekly reset');
    for (const a of ['approve', 'reject', 'reply', 'always'])
      expect(pendingToast(p, a)).not.toMatch(/Allowed|Denied|resumed/);
  });

  it('the effect, by kind and action', () => {
    expect(effectText(effect(p, 'allowed'), { action: 'approve' })).toBe('Allowed · front-1 resumed');
    expect(effectText(effect(p, 'allowed'), { action: 'always' })).toBe(
      'Always allowed · Bash(pnpm add:*) · front-1 resumed',
    );
    expect(effectText(effect(p, 'denied'), { action: 'reject' })).toBe('Denied · front-1');
    expect(effectText(effect(p, 'denied'), { action: 'reply', message: 'use npm' })).toBe(
      'Denied · reason sent to front-1',
    );
    expect(effectText(effect(question, 'resumed'))).toBe('Answer sent · front-1 resumed');
    expect(effectText(effect(question, 'delivered'))).toBe('Answer delivered · front-1 reads it next');
    expect(effectText(effect(question, 'queued', { effect_detail: '≈14:20' }))).toBe(
      'Answer queued · front-1 starts ≈14:20',
    );
    expect(effectText(effect(spec, 'resumed'), { action: 'approve' })).toBe('Spec approved · lead-1 resumed');
    expect(effectText(effect(spec, 'resumed'), { action: 'reply' })).toBe(
      'Spec sent back with your notes · lead-1 resumed',
    );
    expect(effectText(effect(pr, 'pr_opened', { effect_detail: 'https://github.com/o/shop/pull/12' }))).toBe(
      'PR opened · checkout #12 ↗',
    );
    expect(effectText(effect(budgetDecision(), 'applied'))).toBe(
      'Applied · Defer "infra" until after the weekly reset',
    );
    expect(effectText(effect(mcp, 'dismissed'))).toBe('Dismissed until it fails again · Penpot');
    expect(effectText(effect(decision({ kind: 'startup', subject: { agent: 'ui-1' } }), 'deleted'))).toBe(
      'Deleted · ui-1',
    );
    expect(effectText(lc(pr, { state: 'failed', error: 'push rejected: non-fast-forward' }))).toBe(
      'Failed: push rejected: non-fast-forward',
    );
  });

  it('only effect, failed and parked toast; someone else decided only as P4.0 did', () => {
    expect(decisionToast(lc(p, { state: 'decided', decided_by: 'user' }), { action: 'approve' })).toBe(null);
    expect(decisionToast(lc(p, { state: 'sent', decided_by: 'user' }), { action: 'approve' })).toBe(null);
    expect(decisionToast(effect(p, 'allowed'), { action: 'approve' })).toBe('Allowed · front-1 resumed');
    expect(decisionToast(lc(p, { state: 'parked', decided_by: 'expired' }))).toBe(
      'Parked · front-1 stopped and resumes on your answer',
    );
    expect(decisionToast(lc(p, { state: 'effect', decided_by: 'pane', effect: 'allowed' }))).toBe(
      'Answered in the pane · front-1',
    );
    expect(decisionToast(lc(p, { state: 'effect', decided_by: 'rule:git-safe', effect: 'denied' }))).toBe(
      'Denied by rule git-safe · front-1',
    );
    expect(decisionToast(lc(p, { state: 'effect', decided_by: 'core', effect: 'dismissed' }))).toBe(null);
  });
});

describe('lifecycle marker: every state (notes §3.4)', () => {
  const p = permDecision({ created_ms: T - 4 * 60000 });
  const m = (d, pending = null) => marker(d, pending, T);

  it('reads each state from the lifecycle only', () => {
    expect(m(p)).toEqual({ state: 'open', text: 'Waiting for you · 4 min' });
    expect(m(p, { action: 'approve' }).text).toBe('Allowing once · Undo u');
    expect(m(lc(p, { state: 'parked' })).text).toBe('Parked · front-1 stopped; resumes on your answer');
    expect(m(lc(p, { state: 'decided', decided_by: 'rule:git-safe' })).text).toBe('Decided by rule git-safe · sending');
    expect(m(lc(p, { state: 'sent', sent_at_ms: at(14, 2) })).text).toBe('Sent to front-1 · 14:02');
    expect(m(lc(pr, { state: 'sent' })).text).toBe('Pushing orch/checkout/integration…');
    expect(m(lc(p, { state: 'effect', decided_by: 'user', effect: 'allowed', effect_at_ms: at(14, 2) })).text).toBe(
      'Allowed · front-1 resumed · 14:02',
    );
    const opened = m(
      lc(pr, { state: 'effect', effect: 'pr_opened', effect_detail: 'https://x/pull/12', effect_at_ms: at(15, 30) }),
    );
    expect(opened).toEqual({ state: 'effect', text: 'PR opened · checkout #12 ↗ · 15:30', url: 'https://x/pull/12' });
    expect(m(lc(p, { state: 'failed', error: 'push rejected' })).text).toBe('Failed · push rejected');
    expect(m(lc(p, { state: 'stale', stale_why: 'the agent moved on' })).text).toBe('Stale · the agent moved on');
    expect(m(lc(p, { state: 'dismissed', decided_by: 'user', decided_at_ms: at(14, 5) })).text).toBe(
      'Dismissed by you · 14:05',
    );
    expect(m(lc(p, { state: 'interrupted' })).text).toBe('Interrupted · front-1 stopped before it was answered');
  });
});
