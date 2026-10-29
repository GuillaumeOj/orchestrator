// P4.1 F2: the Inbox renders Decisions (api-contract §2): each context block,
// each lifecycle state, toasts only once the core pushed the effect, the
// outcome digest with Clear, the waiting-on chip's links. Against a mocked core.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, decision, fixture, nowOf, permDecision, rec } from './fixture.js';

const calls = [];
const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    const r = replies[cmd];
    return r instanceof Error ? Promise.reject(r) : Promise.resolve(typeof r === 'function' ? r(args) : r);
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
}));

const { app, decide, flushDecisions } = await import('../src/lib/app.svelte.js');
const { applyNow } = await import('../src/lib/now.svelte.js');
const { LIFECYCLE } = await import('../src/lib/status.js');
const { UNDO_MS } = await import('../src/lib/inbox.js');
const { default: InboxPane } = await import('../src/panes/InboxPane.svelte');
const { default: ContextBlocks } = await import('../src/components/ContextBlocks.svelte');
const { default: LifecycleMarker } = await import('../src/components/LifecycleMarker.svelte');
const { default: WaitingChip } = await import('../src/components/WaitingChip.svelte');
const { default: DiffPane } = await import('../src/panes/DiffPane.svelte');

const MIN = 60000;
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
/** @returns {HTMLElement} */
const block = (c, type) => c.querySelector(`[data-block="${type}"]`);

beforeEach(() => {
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.now = nowOf(app.data);
  app.pending = [];
  app.toasts = [];
  app.panes = {};
  app.focused = null;
  app.layout = null;
});
afterEach(cleanup);

const question = (over = {}) =>
  decision({
    id: 'q-3',
    kind: 'question',
    subject: { agent: 'front-1', project: 'shop-api' },
    title: 'Which auth provider? · 2 questions',
    created_ms: Date.now() - 10 * MIN,
    context: [
      { type: 'markdown', text: 'Which auth provider?\n\nWhich payment methods?' },
      {
        type: 'options',
        questions: [
          {
            question: 'Which auth provider?',
            header: 'Auth',
            multi: false,
            options: [
              { label: 'Clerk', description: 'hosted' },
              { label: 'Lucia', description: null },
            ],
          },
          {
            question: 'Which payment methods?',
            header: 'Pay',
            multi: true,
            options: [
              { label: 'Card' },
              { label: 'Apple Pay', description: 'needs domain verification' },
              { label: 'SEPA' },
            ],
          },
        ],
      },
      {
        type: 'last_message',
        text: Array.from({ length: 9 }, (_, i) => `line ${i + 1}`).join('\n'),
        at_ms: new Date(2026, 9, 29, 14, 2).getTime(),
      },
    ],
    actions: [act('answer', 'Answer', 'enter'), act('reject', 'Dismiss', 'r'), act('open', 'Open pane', 'o')],
    ...over,
  });

describe('context blocks: one component per type, in order', () => {
  const every = decision({
    id: 'gate-8',
    kind: 'gate',
    gate: 'pr',
    subject: { agent: 'lead-1', project: 'shop-api', feature: 'checkout' },
    context: [
      { type: 'tool_call', tool: 'Bash', input: { command: 'pnpm test' }, cwd: '/w/checkout/back-1' },
      { type: 'markdown', text: '**Checkout v2**', source: 'spec.md' },
      { type: 'docs', files: [{ path: '.orchestrator/features/checkout/spec.md', lines: 142 }] },
      {
        type: 'diff_summary',
        files: [
          { path: 'src/api.rs', added: 40, removed: 3 },
          { path: 'ui/Pay.svelte', added: 102, removed: 0 },
        ],
        added: 142,
        removed: 3,
      },
      { type: 'last_message', text: 'Done.', at_ms: 0 },
      { type: 'pty_tail', text: Array.from({ length: 15 }, (_, i) => `l${i}`).join('\n') },
      { type: 'cost', week_pp: 2.14, five_hour_pp: 6 },
      { type: 'countdown', default: 'reject', deadline_ms: Date.now() + 18 * MIN },
      { type: 'error', text: 'push rejected', detail: 'non-fast-forward' },
    ],
  });

  it('renders every block type with its notes §3.3 copy', () => {
    const { container: c } = render(ContextBlocks, { decision: every });
    expect([...c.querySelectorAll('[data-block]')].map((b) => b.getAttribute('data-block'))).toEqual(
      every.context.map((b) => b.type),
    );
    expect(text(block(c, 'tool_call'))).toBe('Bash in back-1 pnpm test');
    expect(text(block(c, 'markdown'))).toBe('From spec.md Checkout v2');
    expect(text(block(c, 'docs'))).toBe('Docs · 1 file spec.md 142 lines');
    expect(text(block(c, 'diff_summary'))).toMatch(
      /^2 files changed · \+142 −3 lines src\/api\.rs \+40 −3\s*ui\/Pay\.svelte \+102 Show patch$/,
    );
    expect(block(c, 'diff_summary').querySelectorAll('.bar i.add').length).toBeGreaterThan(0);
    expect(text(block(c, 'last_message'))).toBe('Last message Done.');
    expect(block(c, 'pty_tail').querySelector('pre').textContent.split('\n')).toHaveLength(12);
    expect(text(block(c, 'cost'))).toBe('Cost ≈ +2.1 pp of the week · +6 % of the 5h window');
    expect(text(block(c, 'countdown'))).toMatch(/Skips in 1[78]m$/);
    expect(text(block(c, 'error'))).toBe('Error push rejected Show detail');
  });

  it('diff summary: the patch on demand, its error, and "Show n more files" past 12', async () => {
    replies.decision_patch = '@@ -1 +1 @@\n-old\n+new';
    const files = Array.from({ length: 14 }, (_, i) => ({ path: `f${i}.ts`, added: i, removed: 0 }));
    const { container: c, getByText } = render(ContextBlocks, {
      decision: decision({ id: 'gate-6', context: [{ type: 'diff_summary', files, added: 91, removed: 0 }] }),
    });
    expect(block(c, 'diff_summary').querySelectorAll('.row')).toHaveLength(12);
    await fireEvent.click(getByText('Show 2 more files'));
    expect(block(c, 'diff_summary').querySelectorAll('.row')).toHaveLength(14);
    await fireEvent.click(getByText('Show patch'));
    await waitFor(() => expect(block(c, 'diff_summary').querySelector('.patch')).toBeTruthy());
    expect(calls.find(([cmd]) => cmd === 'decision_patch')[1]).toEqual({ id: 'gate-6', path: null });
    expect(block(c, 'diff_summary').querySelector('.patch .del').textContent).toBe('-old');
    expect(block(c, 'diff_summary').querySelector('.patch .add').textContent).toBe('+new');
    await fireEvent.click(getByText('Hide patch'));
    replies.decision_patch = new Error('git diff exited (status 128)');
    await fireEvent.click(getByText('Show patch'));
    await waitFor(() =>
      expect(text(block(c, 'diff_summary'))).toContain('Could not load the patch: git diff exited (status 128)'),
    );
  });

  it('countdown: an ask keeps the agent queued', () => {
    const { container: c } = render(ContextBlocks, {
      decision: decision({
        kind: 'ask',
        context: [{ type: 'countdown', default: 'reject', deadline_ms: Date.now() + 18 * MIN + 1000 }],
      }),
    });
    expect(text(block(c, 'countdown'))).toMatch(/Keeps queued in 18m$/);
  });

  it('countdown past its deadline applies the default', () => {
    const { container: c } = render(ContextBlocks, {
      decision: decision({ context: [{ type: 'countdown', default: 'approve', deadline_ms: Date.now() - 1000 }] }),
    });
    expect(text(block(c, 'countdown'))).toMatch(/ Applying the default…$/);
  });

  it('options: every question, multi-select included; Answer sends the choices once each has one', async () => {
    app.now = nowOf(app.data, { decisions: [question()] });
    const { container: c } = render(InboxPane);
    const opts = block(c, 'options');
    expect([...opts.querySelectorAll('legend')].map(text)).toEqual([
      'Question 1 of 2 · Auth',
      'Question 2 of 2 · Pick any',
    ]);
    expect(opts.querySelectorAll('input[type=radio]')).toHaveLength(2);
    expect(opts.querySelectorAll('input[type=checkbox]')).toHaveLength(3);
    expect(text(block(c, 'last_message'))).toMatch(/^Last message · 14:02 line 1 .* line 6 Show more$/);
    const answer = /** @type {HTMLButtonElement} */ (
      [...c.querySelectorAll('.actions button')].find((b) => b.textContent.startsWith('Answer'))
    );
    expect(answer.disabled).toBe(true);
    const pick = (label) =>
      fireEvent.click(
        [...opts.querySelectorAll('label')].find((l) => l.textContent.includes(label)).querySelector('input'),
      );
    await pick('Clerk');
    await pick('Card');
    await pick('Apple Pay');
    flushSync();
    expect(answer.disabled).toBe(false);
    await fireEvent.click(answer);
    expect(app.pending[0]).toMatchObject({ item: 'q-3', action: 'answer', message: 'Clerk\nCard, Apple Pay' });
  });
});

describe('lifecycle marker: every state, its icon and color', () => {
  const states = {
    open: {},
    parked: {},
    decided: { decided_by: 'rule:git-safe' },
    sent: { sent_at_ms: Date.now() },
    effect: { effect: 'allowed', decided_by: 'user' },
    failed: { error: 'push rejected' },
    stale: { stale_why: 'the agent moved on' },
    dismissed: { decided_by: 'user' },
    interrupted: {},
  };
  for (const [state, more] of Object.entries(states)) {
    it(state, () => {
      const { container: c } = render(LifecycleMarker, { decision: permDecision({ lifecycle: { state, ...more } }) });
      const m = c.querySelector('[data-state]');
      expect(m.getAttribute('data-state')).toBe(state);
      expect(m.querySelector('.icon').textContent).toBe(LIFECYCLE[state].icon);
      expect(m.querySelector('.icon').getAttribute('style')).toContain(`--color-lifecycle-${state}`);
      expect(m.classList.contains('muted')).toBe(['stale', 'dismissed', 'interrupted'].includes(state));
    });
  }

  it('a PR opened links its URL', async () => {
    const d = decision({
      kind: 'gate',
      gate: 'pr',
      subject: { feature: 'checkout' },
      lifecycle: { state: 'effect', effect: 'pr_opened', effect_detail: 'https://github.com/o/shop/pull/12' },
    });
    const { getByText } = render(LifecycleMarker, { decision: d });
    await fireEvent.click(getByText(/PR opened · checkout #12 ↗/));
    expect(calls.find(([cmd]) => cmd === 'open_url')[1]).toEqual({ url: 'https://github.com/o/shop/pull/12' });
  });
});

describe('toasts: never before sent, the effect when the core pushes it', () => {
  const push = (rev, d) => applyNow({ rev, decisions: { upsert: [d], closed: [] }, topics: [] });

  it('Allow once: undo toast, then nothing at decided or sent, then "Allowed · front-1 resumed"', async () => {
    const perm = permDecision();
    app.now = nowOf(app.data, { decisions: [perm] });
    replies.inbox_act = { ok: true, lifecycle: { state: 'sent' }, effect: 'allowed' };
    decide(perm, 'approve');
    expect(app.toasts.map((t) => t.text)).toEqual(['Allowing once · front-1 · pnpm add @stripe/stripe-js']);
    await flushDecisions(Date.now() + UNDO_MS);
    expect(calls.find(([cmd]) => cmd === 'inbox_act')[1]).toEqual({ item: 'perm-1', action: 'approve', message: null });
    // inbox_act said "sent" and even carried the old effect: no toast yet.
    expect(app.toasts).toHaveLength(1);
    expect(app.now.decisions['perm-1'].lifecycle.state).toBe('sent');
    push(2, { ...perm, lifecycle: { state: 'decided', decided_by: 'user' } });
    push(3, { ...perm, lifecycle: { state: 'sent', decided_by: 'user' } });
    expect(app.toasts).toHaveLength(1);
    push(4, {
      ...perm,
      lifecycle: { state: 'effect', decided_by: 'user', effect: 'allowed', effect_at_ms: Date.now() },
    });
    expect(app.toasts.at(-1).text).toBe('Allowed · front-1 resumed');
    // The same state pushed again: no second toast.
    push(5, { ...perm, lifecycle: { state: 'effect', decided_by: 'user', effect: 'allowed' } });
    expect(app.toasts).toHaveLength(2);
  });

  it('a synthesized answer: its version-0 lifecycle waits for the final upsert, which toasts once', async () => {
    const wait = decision({
      id: 'wait-front-1',
      kind: 'wait',
      subject: { agent: 'front-1' },
      lifecycle: { state: 'open', version: 5 },
      actions: [act('reply', 'Reply', 'enter')],
    });
    app.now = nowOf(app.data, { decisions: [wait] });
    replies.inbox_act = { ok: true, lifecycle: { state: 'sent', version: 0 } };
    decide(wait, 'reply', 'yes, go on');
    await flushDecisions(Date.now() + UNDO_MS);
    // Version 0: applied with the version held (the card leaves the open list), the push still wins.
    expect(app.now.decisions['wait-front-1'].lifecycle).toEqual({ state: 'sent', version: 5 });
    expect(app.toasts).toHaveLength(1);
    const final = { ...wait, lifecycle: { state: 'effect', effect: 'delivered', decided_by: 'user', version: 9 } };
    applyNow({ rev: 2, decisions: { upsert: [final], closed: ['wait-front-1'] }, topics: [] });
    expect(app.toasts.at(-1).text).toBe('Answer delivered · front-1 reads it next');
    // The same push again (older or equal version): no second toast.
    applyNow({
      rev: 3,
      decisions: { upsert: [{ ...final, lifecycle: { ...final.lifecycle, state: 'sent', version: 8 } }], closed: [] },
      topics: [],
    });
    applyNow({ rev: 4, decisions: { upsert: [final], closed: [] }, topics: [] });
    expect(app.toasts).toHaveLength(2);
    expect(app.now.decisions['wait-front-1'].lifecycle.state).toBe('effect');
  });

  it('a synthesized answer (version 0) after its final push came is ignored: the effect stays', async () => {
    const wait = decision({
      id: 'wait-front-1',
      kind: 'wait',
      subject: { agent: 'front-1' },
      lifecycle: { state: 'open', version: 5 },
      actions: [act('reply', 'Reply', 'enter')],
    });
    app.now = nowOf(app.data, { decisions: [wait] });
    const final = { ...wait, lifecycle: { state: 'effect', effect: 'delivered', decided_by: 'user', version: 9 } };
    replies.inbox_act = () => {
      applyNow({ rev: 2, decisions: { upsert: [final], closed: ['wait-front-1'] }, topics: [] });
      return { ok: true, lifecycle: { state: 'sent', version: 0 } };
    };
    decide(wait, 'reply', 'yes, go on');
    await flushDecisions(Date.now() + UNDO_MS);
    expect(app.now.decisions['wait-front-1'].lifecycle).toMatchObject({ state: 'effect', version: 9 });
    expect(app.toasts.at(-1).text).toBe('Answer delivered · front-1 reads it next');
  });

  it('an inbox_act answer older than the push already held is ignored', async () => {
    const perm = permDecision({ lifecycle: { state: 'open', version: 2 } });
    app.now = nowOf(app.data, { decisions: [perm] });
    replies.inbox_act = () => {
      // The effect is pushed before the answer comes back.
      push(2, { ...perm, lifecycle: { state: 'effect', decided_by: 'user', effect: 'allowed', version: 4 } });
      return { ok: true, lifecycle: { state: 'sent', version: 3 } };
    };
    decide(perm, 'approve');
    await flushDecisions(Date.now() + UNDO_MS);
    expect(app.now.decisions['perm-1'].lifecycle).toMatchObject({ state: 'effect', version: 4 });
    expect(app.toasts.map((t) => t.text).filter((t) => t.startsWith('Allowed'))).toEqual(['Allowed · front-1 resumed']);
  });

  it('a push at the inbox_act answer’s version brings the rest of the card (a failed one: Dismiss)', async () => {
    const perm = permDecision({ lifecycle: { state: 'open', version: 2 } });
    app.now = nowOf(app.data, { decisions: [perm] });
    const failed = { state: 'failed', decided_by: 'user', error: 'hook gone', version: 3 };
    replies.inbox_act = { ok: true, lifecycle: failed };
    decide(perm, 'approve');
    await flushDecisions(Date.now() + UNDO_MS);
    const errors = app.toasts.filter((t) => t.kind === 'error').length;
    push(2, { ...perm, lifecycle: failed, actions: [act('dismiss', 'Dismiss')] });
    expect(app.now.decisions['perm-1'].actions.map((a) => a.id)).toEqual(['dismiss']);
    expect(app.toasts.filter((t) => t.kind === 'error')).toHaveLength(errors); // no second toast
  });

  it('a failed PR gate: Dismiss from its actions, then the dismissed toast', async () => {
    const gate = decision({
      id: 'gate-6',
      kind: 'gate',
      gate: 'pr',
      subject: { agent: 'lead-1', feature: 'checkout' },
      title: 'PR gate · checkout',
      lifecycle: { state: 'failed', error: 'push rejected: non-fast-forward', version: 4 },
      actions: [act('dismiss', 'Dismiss'), act('open', 'Open pane', 'o')],
    });
    app.now = nowOf(app.data, { decisions: [gate] });
    const { container: c } = render(InboxPane);
    expect(c.querySelector('.detail [data-state="failed"]').textContent).toContain(
      'Failed · push rejected: non-fast-forward',
    );
    const dismiss = [...c.querySelectorAll('.detail .actions button')].find((b) => b.textContent.trim() === 'Dismiss');
    await fireEvent.click(dismiss);
    expect(app.pending[0]).toMatchObject({ item: 'gate-6', action: 'dismiss' });
    expect(app.toasts.at(-1).text).toBe('Dismissing · PR gate · checkout');
    replies.inbox_act = {
      ok: true,
      lifecycle: { state: 'dismissed', effect: 'dismissed', decided_by: 'user', version: 5 },
    };
    await flushDecisions(Date.now() + UNDO_MS);
    expect(calls.find(([cmd]) => cmd === 'inbox_act')[1]).toEqual({ item: 'gate-6', action: 'dismiss', message: null });
    expect(app.toasts.at(-1).text).toBe('Dismissed · PR gate · checkout');
  });

  it('Always allow names the rule; a failure stays until dismissed and opens the decision', async () => {
    const perm = permDecision();
    app.now = nowOf(app.data, { decisions: [perm] });
    replies.inbox_act = { ok: true, lifecycle: { state: 'sent' } };
    decide(perm, 'always');
    await flushDecisions(Date.now() + UNDO_MS);
    push(2, { ...perm, lifecycle: { state: 'effect', decided_by: 'user', effect: 'allowed' } });
    expect(app.toasts.at(-1).text).toBe('Always allowed · Bash(pnpm add:*) · front-1 resumed');
    const pr = decision({ id: 'gate-6', kind: 'gate', gate: 'pr', subject: { agent: 'lead-1' } });
    push(3, { ...pr, lifecycle: { state: 'failed', decided_by: 'user', error: 'push rejected: non-fast-forward' } });
    expect(app.toasts.at(-1)).toMatchObject({
      text: 'Failed: push rejected: non-fast-forward',
      kind: 'error',
      sticky: true,
    });
    app.toasts.at(-1).action.run();
    expect(app.inboxFocus).toBe('gate-6');
  });

  it('an effect in the inbox_act answer itself toasts once, and its later push does not again', async () => {
    const perm = permDecision();
    app.now = nowOf(app.data, { decisions: [perm] });
    const done = { state: 'effect', decided_by: 'user', effect: 'allowed' };
    replies.inbox_act = { ok: true, lifecycle: done, effect: 'allowed' };
    decide(perm, 'approve');
    await flushDecisions(Date.now() + UNDO_MS);
    expect(app.toasts.at(-1).text).toBe('Allowed · front-1 resumed');
    const n = app.toasts.length;
    push(app.now.rev + 1, { ...perm, lifecycle: done });
    expect(app.toasts).toHaveLength(n);
  });

  it('a PR opened toasts with its link', () => {
    const pr = decision({ id: 'gate-6', kind: 'gate', gate: 'pr', subject: { agent: 'lead-1', feature: 'checkout' } });
    push(2, {
      ...pr,
      lifecycle: {
        state: 'effect',
        decided_by: 'user',
        effect: 'pr_opened',
        effect_detail: 'https://github.com/o/shop/pull/12',
      },
    });
    expect(app.toasts.at(-1).text).toBe('PR opened · checkout #12 ↗');
    expect(app.toasts.at(-1).action.label).toBe('Open PR');
  });
});

describe('outcome digest (level 4)', () => {
  const at = (h, m) => new Date(new Date().setHours(h, m, 0, 0)).getTime();
  const outcomes = () => [
    {
      id: 'ev-11',
      kind: 'task_done',
      at_ms: at(0, 2),
      agent: 'back-1',
      task: 4,
      project: 'shop-api',
      text: 'T-4 done · checkout API',
      seen: false,
    },
    { id: 'ev-12', kind: 'merge_done', at_ms: at(0, 10), text: 'Merged back-1 into integration', seen: false },
    { id: 'ev-13', kind: 'run_phase', at_ms: at(0, 1), text: 'checkout: planning → executing', seen: false },
    {
      id: 'ev-15',
      kind: 'pr_opened',
      at_ms: at(0, 30),
      url: 'https://github.com/o/shop/pull/12',
      text: 'PR opened · checkout #12 ↗',
      seen: false,
    },
    {
      id: 'ev-14',
      kind: 'turn_done',
      at_ms: at(0, 20),
      text: 'blog › fix-feed: Fixed the feed pagination',
      seen: false,
    },
    { id: 'ev-9', kind: 'run_done', at_ms: at(0, 0) - 3600000, text: 'yesterday', seen: false },
    { id: 'ev-10', kind: 'run_done', at_ms: at(0, 0) + 1000, text: 'seen already', seen: true },
  ];

  it("today's outcomes newest first; Clear marks them seen and keeps done today", async () => {
    app.now = nowOf(app.data, { decisions: [], outcomes: outcomes() });
    app.now.counts.global.done_today = 6;
    replies.digest_seen = null;
    const { container: c, getByText } = render(InboxPane);
    const rows = [...c.querySelectorAll('.list [role=option][title]')];
    expect(rows.map((r) => r.getAttribute('title'))).toEqual([
      'PR opened · checkout #12 ↗',
      'blog › fix-feed: Fixed the feed pagination',
      'Merged back-1 into integration',
      'T-4 done · checkout API',
      'checkout: planning → executing',
    ]);
    expect(text(rows[3])).toMatch(/^\S+ T-4 done · checkout API \(back-1\) 00:02$/);
    expect(rows[0].querySelector('.link')).toBeTruthy();
    expect(text(c.querySelector('.lh.digest'))).toBe('4 · Today · 5 outcomes Clear c');
    await fireEvent.keyDown(c.querySelector('[data-pane-focus]'), { key: 'c' });
    await waitFor(() => expect(text(c.querySelector('.list'))).toContain('All caught up · 6 done today'));
    expect(calls.find(([cmd]) => cmd === 'digest_seen')[1]).toEqual({ upTo: 'ev-15' });
    expect(getByText('All caught up · 6 done today')).toBeTruthy();
  });

  it('a failed run is the one marked outcome', () => {
    const now = Date.now();
    app.now = nowOf(app.data, {
      decisions: [],
      outcomes: [
        {
          id: 'ev-1',
          kind: 'run_phase',
          from: 'integrating',
          to: 'failed',
          at_ms: now,
          text: 'checkout: integrating → failed',
          seen: false,
        },
        {
          id: 'ev-2',
          kind: 'run_phase',
          from: 'planning',
          to: 'executing',
          at_ms: now - 1,
          text: 'checkout: planning → executing',
          seen: false,
        },
      ],
    });
    const { container: c } = render(InboxPane);
    const icons = [...c.querySelectorAll('.list [role=option][title] .icon')];
    expect(icons[0].getAttribute('style')).toContain('--color-phase-failed');
    expect(icons[1].classList.contains('muted')).toBe(true);
  });

  it('with a digest row selected, a/r/⏎ do not act on the decision above it', async () => {
    app.now = nowOf(app.data, { outcomes: outcomes().slice(0, 1) });
    const { container: c } = render(InboxPane);
    await fireEvent.click(c.querySelector('.list [role=option][title]'));
    const root = c.querySelector('[data-pane-focus]');
    for (const key of ['a', 'r', 'Enter']) await fireEvent.keyDown(root, { key });
    expect(app.pending).toEqual([]);
    // j/k go back to the decisions: a acts again.
    await fireEvent.keyDown(root, { key: 'k' });
    await fireEvent.keyDown(root, { key: 'a' });
    expect(app.pending).toHaveLength(1);
  });

  it('a selected digest row that left the digest no longer blocks a/r/⏎', async () => {
    app.now = nowOf(app.data, { outcomes: outcomes().slice(0, 1) });
    const { container: c } = render(InboxPane);
    await fireEvent.click(c.querySelector('.list [role=option][title]'));
    app.now.outcomes = [];
    flushSync();
    await fireEvent.keyDown(c.querySelector('[data-pane-focus]'), { key: 'a' });
    expect(app.pending).toHaveLength(1);
  });

  it('empty: "Nothing done yet today."', () => {
    app.now = nowOf(app.data, { decisions: [] });
    const { container: c } = render(InboxPane);
    expect(text(c.querySelector('.list'))).toContain('Nothing done yet today.');
    expect(c.querySelector('.lh.digest .link')).toBeNull();
  });
});

describe('waiting-on chip', () => {
  it('a decision ref opens the Inbox on it; an agent ref focuses the agent', async () => {
    const user = rec('front-1', 'waiting_for_you', {
      reason: 'permission: Bash pnpm add zod',
      waiting_on: { kind: 'user', ref: 'perm-1', since_ms: Date.now() - 2 * MIN },
    });
    const { container: c } = render(WaitingChip, { rec: user });
    expect(text(c)).toMatch(/permission: Bash pnpm add zod · 2 min$/);
    await fireEvent.click(c.querySelector('.link'));
    expect(app.inboxFocus).toBe('perm-1');
    expect(Object.values(app.panes).some((p) => p.type === 'inbox')).toBe(true);
    cleanup();
    const agent = rec('front-1', 'idle', {
      level: 'waiting',
      reason: '→ back-1 · review of T-4',
      waiting_on: { kind: 'agent', ref: 'back-1', since_ms: Date.now() },
    });
    const { container: c2 } = render(WaitingChip, { rec: agent, compact: true });
    expect(text(c2)).toMatch(/→ back-1 · review of T-4 just now$/);
    await fireEvent.click(c2.querySelector('.link'));
    expect(Object.values(app.panes).some((p) => p.type === 'agent' && p.agentId === 'back-1')).toBe(true);
  });

  it('only the ref is the link when the reason names it', () => {
    const r = rec('front-1', 'idle', {
      level: 'waiting',
      reason: '→ lead-1 · review of T-4',
      waiting_on: { kind: 'agent', ref: 'lead-1', since_ms: Date.now() },
    });
    const { container: c } = render(WaitingChip, { rec: r });
    expect(c.querySelector('.link').textContent).toBe('lead-1');
    expect(text(c.querySelector('.reason'))).toBe('→ lead-1 · review of T-4');
  });

  it('running shows its activity and no since', () => {
    const { container: c } = render(WaitingChip, {
      rec: rec('a', 'working', { activity: 'Edit src/feed.ts', since_ms: 0 }),
    });
    expect(text(c)).toMatch(/Edit src\/feed\.ts$/);
    expect(c.querySelector('.since')).toBeNull();
  });
});

describe('diff pane', () => {
  it('watches its worktree while open (T-33) and stops when it closes', async () => {
    replies.worktree_diff = { files: [], patch: '' };
    app.panes = { p1: { type: 'diff', projectId: 'shop-api', path: '/w/back-1', title: 'diff' } };
    const { unmount } = render(DiffPane, { paneId: 'p1' });
    expect(calls.find(([cmd]) => cmd === 'watch_worktree')[1]).toEqual({ path: '/w/back-1' });
    unmount();
    expect(calls.find(([cmd]) => cmd === 'unwatch_worktree')[1]).toEqual({ path: '/w/back-1' });
  });
});
