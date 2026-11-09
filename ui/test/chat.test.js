// P4.3 chat column and cards (spec §3.3–3.4, §5; docs/design/p4-3-design-notes.md):
// the exit criterion (every Inbox interaction from a card), the thread (morning
// line, cards, lines, the fold, the filter), collapse timing, the snapshot after
// a decision leaves `now`, the keys, ⌘⇧I and the Inspector's overlay rule, the
// Inbox pane gone, and the 300-entry / 20-card render budget. Against a mocked core.

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { flushSync, tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { chatOf, closedEntry, entry, kinds, morning } from './chat-fixture.js';
import { act, decision, fixture, nowOf } from './fixture.js';

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

const { app, run, restoreLayout, inspectorDocked, inspectorOverlay } = await import('../src/lib/app.svelte.js');
const { applyNow } = await import('../src/lib/now.svelte.js');
const chat = await import('../src/lib/chat.js');
const L = await import('../src/lib/layout.js');
const { default: ChatColumn } = await import('../src/components/chat/ChatColumn.svelte');
const { default: TitleBar } = await import('../src/components/TitleBar.svelte');
const { default: Inspector } = await import('../src/components/Inspector.svelte');

const MIN = 60000;
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const feed = (c) => c.querySelector('[data-chat-focus]');
const press = (c, key, o = {}) => fireEvent.keyDown(feed(c), { key, ...o });
/** @returns {HTMLElement} */
const card = (c, id) => c.querySelector(`.card[data-decision="${id}"]`);
const cardIds = (c) => [...c.querySelectorAll('.feed > .stop .card')].map((x) => x.getAttribute('data-decision'));
const selected = (c) => c.querySelector('article.stop.sel .card')?.getAttribute('data-decision') ?? null;
const buttons = (el) => [...el.querySelectorAll(':scope > .actions button')].map(text);
const lines = (c) => [...c.querySelectorAll('article.line')];
const glyph = (k) => (k === 'enter' ? '⏎' : k);
const settle = () => new Promise((r) => setTimeout(r, 0));

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
  app.chatFocus = null;
  app.chatFilter = 'all';
  app.inspector = false;
  app.width = 1512;
  app.overlay = null;
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

/** `app.now` with these decisions and chat entries. */
function setNow(decisions, entries = [], more = {}) {
  app.now = { ...nowOf(app.data, { decisions, ...more }), chat: chatOf(entries) };
}

describe('exit criterion (spec §1a): every Inbox interaction is on a card', () => {
  it('every decision kind is a card with its context blocks and every action, keyed and keyless', async () => {
    const all = kinds();
    setNow(all);
    const { container: c } = render(ChatColumn);
    // 13 open: the 5 oldest show, the newer fold (notes §4, F2).
    expect(cardIds(c)).toEqual(all.slice(0, 5).map((d) => d.id));
    const fold = c.querySelector('.fold');
    expect(text(fold)).toBe('▸ 8 more open · 3 blocking · j');
    await fireEvent.click(fold);
    expect(cardIds(c)).toEqual(all.map((d) => d.id));
    for (const d of all) {
      const el = card(c, d.id);
      expect(el, d.id).toBeTruthy();
      expect([...el.querySelectorAll('[data-block]')].map((b) => b.getAttribute('data-block'))).toEqual(
        d.context.map((b) => b.type),
      );
      const keyed = d.actions.filter((x) => x.key).map((x) => `${x.label} ${glyph(x.key)}`);
      const keyless = d.actions.filter((x) => !x.key).map((x) => x.label);
      expect(buttons(el), d.id).toEqual([...keyed, ...keyless]);
      expect(el.querySelector('[data-state="open"]').textContent).toContain('Waiting for you');
    }
    // The question shows both questions; the badge counts every open card.
    expect([...card(c, 'q-2').querySelectorAll('legend')].map(text)).toEqual([
      'Question 1 of 2 · Auth',
      'Question 2 of 2 · Pick any',
    ]);
  });

  it('drives j/k a r ⏎ o x u e and c on the cards', async () => {
    const [perm, q, spec, , , startup] = kinds();
    const perm2 = { ...perm, id: 'perm-2', title: 'Bash · pnpm add zod-form', created_ms: Date.now() - 5 * MIN };
    setNow([perm, q, spec, startup, perm2], [], {
      outcomes: [{ id: 'ev-3', kind: 'task_done', at_ms: Date.now() - MIN, text: 'T-3 done', seen: false }],
    });
    replies.digest_seen = null;
    replies.editor_open = { reused: true };
    const { container: c } = render(ChatColumn);
    expect(selected(c)).toBe('perm-1'); // the oldest
    await press(c, 'j');
    expect(selected(c)).toBe('q-2');
    await press(c, 'k');
    expect(selected(c)).toBe('perm-1');
    // x marks, a applies to every marked card that has it; u undoes the last.
    await press(c, 'x');
    for (let i = 0; i < 4; i++) await press(c, 'j');
    expect(selected(c)).toBe('perm-2');
    await press(c, 'x');
    await press(c, 'a');
    expect(app.pending.map((p) => [p.item, p.action])).toEqual([
      ['perm-1', 'approve'],
      ['perm-2', 'approve'],
    ]);
    expect(card(c, 'perm-1').querySelector('[data-state="decided"]').textContent).toContain('Allowing once · Undo u');
    expect(buttons(card(c, 'perm-1'))).toEqual(['Undo u']);
    await press(c, 'u');
    expect(app.pending.map((p) => p.item)).toEqual(['perm-1']);
    // r on the question: Dismiss.
    await fireEvent.mouseDown(card(c, 'q-2'));
    await press(c, 'r');
    expect(app.pending.at(-1)).toMatchObject({ item: 'q-2', action: 'reject' });
    // ⏎ on the spec gate: the reply field in place, ⏎ sends with the notes.
    await fireEvent.mouseDown(card(c, 'gate-spec'));
    await press(c, 'Enter');
    const field = card(c, 'gate-spec').querySelector('textarea');
    expect(field).toBeTruthy();
    await fireEvent.input(field, { target: { value: 'Drop gift cards' } });
    await fireEvent.keyDown(field, { key: 'Enter' });
    expect(app.pending.at(-1)).toMatchObject({ item: 'gate-spec', action: 'reply', message: 'Drop gift cards' });
    // o opens the pane, e the editor.
    await fireEvent.mouseDown(card(c, 'startup-front-1'));
    await press(c, 'o');
    expect(Object.values(app.panes)).toMatchObject([{ type: 'agent', agentId: 'front-1' }]);
    await press(c, 'e');
    expect(calls.find(([cmd]) => cmd === 'editor_open')[1]).toMatchObject({ agentId: 'front-1' });
    // c clears the digest from anywhere in the chat.
    await press(c, 'c');
    expect(calls.find(([cmd]) => cmd === 'digest_seen')[1]).toEqual({ upTo: 'ev-3' });
  });

  it('keyless actions: Always allow decides, Answer in the pane decides nothing, Delete asks first', async () => {
    const all = kinds();
    setNow([all[0], all[5]]);
    const { container: c, getByText } = render(ChatColumn);
    await fireEvent.click(getByText('Answer in the pane'));
    expect(app.pending).toHaveLength(0);
    expect(Object.values(app.panes)).toMatchObject([{ type: 'agent', agentId: 'back-1' }]);
    await fireEvent.click(getByText('Always allow'));
    expect(app.pending.at(-1)).toMatchObject({ item: 'perm-1', action: 'always' });
    await fireEvent.click([...card(c, 'startup-front-1').querySelectorAll('button')].find((b) => text(b) === 'Delete'));
    expect(app.overlay).toMatchObject({ kind: 'confirm', text: 'Delete front-1?' });
    expect(app.pending).toHaveLength(1);
    app.overlay.onYes();
    expect(app.pending.at(-1)).toMatchObject({ item: 'startup-front-1', action: 'delete' });
  });

  it('delivery card: reason, messages, tail; Retry now primary, Open pane for no_ack and user_typing', async () => {
    const mk = (blocker) =>
      decision({
        id: 'delivery-7',
        kind: 'delivery',
        level: 2,
        subject: { agent: 'back-1' },
        title: 'back-1 did not receive 2 messages',
        payload: { blocker },
        created_ms: Date.now() - 10 * MIN,
        context: [
          { type: 'error', text: 'governor' },
          { type: 'pty_tail', text: 'a\nb' },
        ],
        actions: [
          act('open', 'Open pane', 'o'),
          act('retry', 'Retry now'),
          act('restart_fresh', 'Restart fresh'),
          act('dismiss', 'Dismiss'),
        ],
      });
    const primary = (c) => [...card(c, 'delivery-7').querySelectorAll('.actions button.primary')].map(text);
    setNow([mk('governor:off_hours')]);
    let r = render(ChatColumn);
    expect(text(card(r.container, 'delivery-7'))).toContain('Retry puts the messages back');
    expect(buttons(card(r.container, 'delivery-7'))).toEqual(['Open pane o', 'Retry now', 'Restart fresh', 'Dismiss']);
    expect(primary(r.container)).toEqual(['Retry now']);
    await fireEvent.click(r.getByText('Retry now'));
    expect(app.pending.at(-1)).toMatchObject({ item: 'delivery-7', action: 'retry' });
    expect(app.toasts.at(-1).text).toBe('Retrying · back-1');
    r.unmount();
    for (const blocker of ['no_ack', 'user_typing']) {
      app.pending = [];
      setNow([mk(blocker)]);
      r = render(ChatColumn);
      expect(primary(r.container)).toEqual(['Open pane o']);
      r.unmount();
    }
  });

  it('startup card after a crash loop says so; a plain startup card does not', () => {
    const [startup] = kinds().filter((d) => d.kind === 'startup');
    setNow([{ ...startup, payload: { failed: 'crash' } }]);
    let r = render(ChatColumn);
    expect(text(card(r.container, startup.id))).toContain('crashed again');
    r.unmount();
    setNow([startup]);
    r = render(ChatColumn);
    expect(text(card(r.container, startup.id))).not.toContain('crashed again');
  });

  it('the question answers once every question has a choice', async () => {
    const q = kinds()[1];
    setNow([q]);
    const { container: c } = render(ChatColumn);
    const answer = /** @type {HTMLButtonElement} */ (
      [...card(c, 'q-2').querySelectorAll('.actions button')].find((b) => text(b).startsWith('Answer'))
    );
    expect(answer.disabled).toBe(true);
    const pick = (label) =>
      fireEvent.click(
        [...card(c, 'q-2').querySelectorAll('label')].find((l) => l.textContent.includes(label)).querySelector('input'),
      );
    await pick('Clerk');
    await press(c, 'Enter');
    expect(app.toasts.at(-1).text).toBe('Pick an answer for every question');
    await pick('SEPA');
    flushSync();
    expect(answer.disabled).toBe(false);
    await press(c, 'Enter');
    expect(app.pending.at(-1)).toMatchObject({ item: 'q-2', action: 'answer', message: 'Clerk\nSEPA' });
  });
});

describe('the thread', () => {
  it('morning line first, then cards and lines in time order; a feed of labelled articles', () => {
    const now = Date.now();
    const [perm, q] = kinds(now);
    const ruled = { ...perm, id: 'perm-0', title: 'cargo add serde_json', created_ms: now - 40 * MIN };
    setNow(
      [perm, q],
      [
        morning(now - 60 * MIN),
        closedEntry('c-1', ruled, {
          verb: 'allowed',
          agent: 'back-1',
          what: 'cargo add serde_json',
          by: 'rule',
          by_detail: 'installs',
          at_ms: now - 40 * MIN,
        }),
        entry('c-2', perm),
        entry('c-3', q),
      ],
    );
    const { container: c } = render(ChatColumn);
    expect(feed(c).getAttribute('role')).toBe('feed');
    const order = [...feed(c).children].map((x) => x.className.split(' ')[0]);
    expect(order.slice(0, 4)).toEqual(['morning', 'line', 'stop', 'stop']);
    expect(text(c.querySelector('.morning'))).toBe('Good morning. 1 run in progress, 1 decision waiting.');
    const [l] = lines(c);
    expect(text(l)).toMatch(/^▸ allowed back-1: cargo add serde_json · rule: installs \d\d:\d\d$/);
    expect(l.getAttribute('aria-label')).toMatch(/^Allowed back-1: cargo add serde_json, by rule installs, \d\d:\d\d$/);
    expect(l.querySelector('button').getAttribute('aria-expanded')).toBe('false');
    expect(c.querySelector('article.stop').getAttribute('aria-label')).toBe('Bash · pnpm add zod');
  });

  it('a decision with no entry yet is a card at its creation time; an entry rev only goes up', () => {
    const now = Date.now();
    const [perm] = kinds(now);
    setNow([perm], [morning(now - 60 * MIN)]);
    const { container: c } = render(ChatColumn);
    expect(cardIds(c)).toEqual(['perm-1']);
    const ch = chat.upsertEntries(chatOf([entry('c-2', perm, { rev: 3 })]), [entry('c-2', perm, { rev: 2, at_ms: 1 })]);
    expect(ch.entries['c-2'].rev).toBe(3);
    chat.upsertEntries(ch, [{ ...entry('c-9', perm), day: '2026-11-03' }]);
    expect(ch).toMatchObject({ day: '2026-11-03' });
    expect(Object.keys(ch.entries)).toEqual(['c-9']);
    chat.upsertEntries(ch, [entry('c-2', perm, { rev: 9 })]); // an earlier day's: not today's
    expect(Object.keys(ch.entries)).toEqual(['c-9']);
  });

  it('the now event upserts chat entries (highest rev) and a new day starts the thread over', () => {
    const [perm] = kinds();
    setNow([], []);
    app.now.rev = 1;
    applyNow({ rev: 2, chat: [entry('c-2', perm, { rev: 2 })], topics: [] });
    applyNow({ rev: 3, chat: [entry('c-2', perm, { rev: 1, thread: 'stale' })], topics: [] });
    expect(app.now.chat.entries['c-2']).toMatchObject({ rev: 2, thread: 'run-12' });
    applyNow({ rev: 4, chat: [{ ...morning(Date.now()), id: 'c-m2', day: '2026-11-03' }], topics: [] });
    expect(app.now.chat.day).toBe('2026-11-03');
    expect(Object.keys(app.now.chat.entries)).toEqual(['c-m2']);
  });

  it('"↳ after" names the latest earlier decision of the same run that closed today, and jumps to it', async () => {
    const now = Date.now();
    const [perm, , spec] = kinds(now);
    const earlier = { ...spec, created_ms: now - 50 * MIN };
    setNow(
      [perm],
      [
        closedEntry('c-1', earlier, {
          verb: 'approved',
          agent: 'lead-1',
          what: 'Spec gate · checkout',
          by: 'you',
          at_ms: now - 45 * MIN,
        }),
        entry('c-2', perm),
      ],
    );
    const { container: c } = render(ChatColumn);
    const ref = card(c, 'perm-1').querySelector('.ref');
    expect(text(ref)).toMatch(/^↳ after: approved Spec gate · checkout · \d\d:\d\d$/);
    expect(ref.getAttribute('aria-label')).toMatch(/^Follows: approved Spec gate · checkout, \d\d:\d\d\. Jump to it$/);
    await fireEvent.click(ref);
    await tick();
    expect(lines(c)[0].querySelector('button').getAttribute('aria-expanded')).toBe('true');
    expect(document.activeElement).toBe(lines(c)[0].querySelector('button'));
  });
});

describe('collapse (notes §2.1)', () => {
  const effect = (d, over = {}) => ({
    ...d,
    lifecycle: {
      state: 'effect',
      decided_by: 'user',
      effect: 'allowed',
      effect_at_ms: Date.now(),
      version: 3,
      ...over,
    },
  });

  it('a decision you settled is a card until 10 s after its effect, then its line', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    const [perm] = kinds();
    setNow([effect(perm)], [entry('c-1', perm)]);
    const { container: c } = render(ChatColumn);
    expect(card(c, 'perm-1').querySelector('[data-state="effect"]').textContent).toContain('Allowed · back-1 resumed');
    vi.advanceTimersByTime(9000);
    flushSync();
    expect(card(c, 'perm-1')).toBeTruthy();
    // Its line arrives with the close; the card holds until its 10 s.
    app.now.chat.entries['c-1'] = closedEntry('c-1', perm, {
      verb: 'allowed',
      agent: 'back-1',
      what: 'pnpm add zod',
      by: 'you',
      at_ms: Date.now(),
    });
    flushSync();
    expect(card(c, 'perm-1')).toBeTruthy();
    vi.advanceTimersByTime(1100);
    flushSync();
    expect(card(c, 'perm-1')).toBeNull();
    expect(text(lines(c)[0])).toMatch(/^▸ allowed back-1: pnpm add zod \d\d:\d\d$/);
  });

  it('the pointer over the card holds its collapse', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    const [perm] = kinds();
    setNow([effect(perm)]);
    const { container: c } = render(ChatColumn);
    await fireEvent.mouseEnter(card(c, 'perm-1').parentElement);
    vi.advanceTimersByTime(20000);
    flushSync();
    expect(card(c, 'perm-1')).toBeTruthy();
    await fireEvent.mouseLeave(card(c, 'perm-1').parentElement);
    flushSync();
    expect(card(c, 'perm-1')).toBeNull();
  });

  it('Dismiss collapses at once; rule, pane, expiry and the app are lines from the start', () => {
    const now = Date.now();
    const [perm, q, , , , startup, ask] = kinds(now);
    setNow(
      [{ ...startup, lifecycle: { state: 'dismissed', decided_by: 'user', decided_at_ms: now } }],
      [
        closedEntry('c-1', perm, {
          verb: 'allowed',
          agent: 'back-1',
          what: 'cargo add serde_json',
          by: 'rule',
          by_detail: 'installs',
          at_ms: now - 3 * MIN,
        }),
        closedEntry('c-2', q, {
          verb: 'answered',
          agent: 'front-1',
          what: 'Which auth provider?',
          by: 'pane',
          at_ms: now - 2 * MIN,
        }),
        closedEntry('c-3', ask, {
          verb: 'kept queued',
          agent: 'back-1',
          what: 'Run back-1 now?',
          by: 'expired',
          at_ms: now - MIN,
        }),
        entry('c-4', startup),
      ],
    );
    const { container: c } = render(ChatColumn);
    expect(cardIds(c)).toEqual([]);
    expect(lines(c).map((l) => text(l.querySelector('button')).replace(/ \d\d:\d\d$/, ''))).toEqual([
      '▸ allowed back-1: cargo add serde_json · rule: installs',
      '▸ answered front-1: Which auth provider? · in the pane',
      '▸ dismissed front-1: front-1 did not start', // keeps its place in the thread
      '▸ kept queued back-1: Run back-1 now? · expired',
    ]);
    expect(text(c)).toContain('Nothing needs you · last settled');
  });

  it('parked, failed and stale stay cards: failed with its actions again, stale with Dismiss', () => {
    const [perm, , , pr, , , , , err] = kinds();
    setNow([
      {
        ...perm,
        lifecycle: { state: 'parked', decided_by: 'expired' },
        actions: perm.actions.filter((x) => x.id !== 'in_pane'),
      },
      {
        ...pr,
        lifecycle: { state: 'failed', error: 'push rejected: non-fast-forward' },
        actions: [...pr.actions, act('dismiss', 'Dismiss')],
      },
      { ...err, lifecycle: { state: 'stale', stale_why: 'the agent moved on' }, actions: [act('dismiss', 'Dismiss')] },
    ]);
    const { container: c } = render(ChatColumn);
    expect(cardIds(c)).toEqual(['perm-1', 'gate-pr', 'error-back-1']);
    expect(text(card(c, 'perm-1').querySelector('.kind'))).toContain('Permission · parked');
    expect(text(card(c, 'perm-1').querySelector('[data-state="parked"]'))).toContain(
      'Parked · back-1 stopped; resumes on your answer',
    );
    expect(text(card(c, 'gate-pr').querySelector('[data-state="failed"]'))).toContain(
      'Failed · push rejected: non-fast-forward',
    );
    expect(buttons(card(c, 'gate-pr'))).toEqual(['Approve: push + open PR a', 'Reject r', 'Reply ⏎', 'Dismiss']);
    expect(card(c, 'error-back-1').classList.contains('muted')).toBe(true);
    expect(buttons(card(c, 'error-back-1'))).toEqual(['Dismiss']);
  });

  it('after the decision leaves now, its line expands to the read-only card from the snapshot', async () => {
    const now = Date.now();
    const [perm] = kinds(now);
    const snap = { ...perm, lifecycle: { state: 'effect', decided_by: 'rule:installs', effect: 'allowed' } };
    setNow(
      [],
      [
        closedEntry(
          'c-1',
          perm,
          { verb: 'allowed', agent: 'back-1', what: 'pnpm add zod', by: 'rule', by_detail: 'installs', at_ms: now },
          { card: snap },
        ),
      ],
    );
    const { container: c } = render(ChatColumn);
    expect(card(c, 'perm-1')).toBeNull();
    await fireEvent.click(lines(c)[0].querySelector('button'));
    const ro = card(c, 'perm-1');
    expect(ro.classList.contains('ro')).toBe(true);
    expect(text(ro.querySelector('.kind'))).toContain('Permission · by rule');
    expect(ro.querySelector('[data-block="tool_call"]')).toBeTruthy();
    expect(text(ro.querySelector('[data-state]'))).toMatch(/Allowed by rule installs · \d\d:\d\d$/);
    expect(buttons(ro)).toEqual(['Open pane o']);
    // ⏎ on the focused line collapses it again (a button: its own ⏎).
    await fireEvent.click(lines(c)[0].querySelector('button'));
    expect(card(c, 'perm-1')).toBeNull();
  });
});

describe('filter, fold and keys', () => {
  it('the filter shows its cards only; lines and the badge ignore it', async () => {
    const now = Date.now();
    const all = kinds(now);
    const decisions = [all[0], all[6], all[8]]; // level 1, 3, 2
    setNow(decisions, [
      closedEntry('c-1', all[1], {
        verb: 'answered',
        agent: 'front-1',
        what: 'Which auth provider?',
        by: 'you',
        at_ms: now - 50 * MIN,
      }),
    ]);
    const { container: c } = render(ChatColumn);
    const title = render(TitleBar).container;
    expect([...c.querySelectorAll('.filter-chip')].map(text)).toEqual(['All 3', 'Blocking 1', 'Errors 1', 'Budget 1']);
    await fireEvent.click([...c.querySelectorAll('.filter-chip')].find((b) => text(b) === 'Errors 1'));
    expect(cardIds(c)).toEqual(['error-back-1']);
    expect(lines(c)).toHaveLength(1);
    expect(text(c.querySelector('[data-strip]'))).toBe('Showing Errors · 1 of 3 open cards. 2 hidden · Show all');
    expect(text(title.querySelector('.decisions .count'))).toBe('3');
    expect(title.querySelector('.decisions').getAttribute('aria-label')).toBe('3 decisions need you · ⌘⇧I');
    await press(c, 'f');
    expect(app.chatFilter).toBe('budget');
    await fireEvent.click(c.querySelector('[data-strip] .link'));
    expect(cardIds(c)).toHaveLength(3);
  });

  it('the filter row hides with fewer than 2 open cards on All', () => {
    setNow([kinds()[0]]);
    const { container: c } = render(ChatColumn);
    expect(c.querySelector('.filters')).toBeNull();
  });

  it('past 5 open the newer fold; j from the 5th lands on the fold, ⏎ shows them, j/k skip lines', async () => {
    const now = Date.now();
    const all = kinds(now).slice(0, 7);
    setNow(all, [
      ...all.map((d, i) => entry(`c-${i}`, d)),
      closedEntry(
        'c-x',
        { ...all[0], id: 'old', created_ms: all[1].created_ms + 1 },
        { verb: 'allowed', agent: 'back-1', what: 'ls', by: 'rule', at_ms: now },
      ),
    ]);
    const { container: c } = render(ChatColumn);
    expect(cardIds(c)).toEqual(all.slice(0, 5).map((d) => d.id));
    expect(lines(c)).toHaveLength(1);
    await press(c, 'j'); // perm-1 → q-2: the line between is skipped
    expect(selected(c)).toBe('q-2');
    for (let i = 0; i < 3; i++) await press(c, 'j');
    expect(selected(c)).toBe('gate-restart');
    await press(c, 'j');
    expect(c.querySelector('.fold').classList.contains('sel')).toBe(true);
    expect(text(c.querySelector('.fold'))).toBe('▸ 2 more open · 1 blocking · j');
    await press(c, 'Enter');
    expect(cardIds(c)).toEqual(all.map((d) => d.id));
    expect(text(c.querySelector('.fold'))).toBe('▾ Fold 2 cards · ⏎');
    await press(c, 'j');
    expect(selected(c)).toBe('startup-front-1');
  });

  it('Esc returns the focus to the centre pane; Mission control when none', async () => {
    setNow([kinds()[0]]);
    const { container: c } = render(ChatColumn);
    await press(c, 'Escape');
    expect(Object.values(app.panes)).toMatchObject([{ type: 'mission' }]);
  });

  it('⌘⇧I closes the overlay and focuses the oldest open card, the filter back on All', async () => {
    const all = kinds();
    setNow(all.slice(0, 3));
    app.inspector = true;
    app.chatFilter = 'errors';
    const { container: c } = render(ChatColumn);
    run('decisions');
    await settle();
    expect(app.inspector).toBe(false);
    expect(app.chatFilter).toBe('all');
    expect(selected(c)).toBe('perm-1');
    expect(document.activeElement).toBe(feed(c));
    // Docked at 1728: ⌘⇧I leaves the Inspector open.
    app.width = 1728;
    app.inspector = true;
    run('decisions');
    expect(app.inspector).toBe(true);
  });

  it('the palette\'s "Filter decisions: …" keeps its filter (not reset to All by the focus)', async () => {
    const all = kinds();
    setNow([all[0], all[6], all[8]]);
    const { container: c } = render(ChatColumn);
    run('decisions.filter.errors');
    await settle();
    expect(app.chatFilter).toBe('errors');
    expect(cardIds(c)).toEqual(['error-back-1']);
  });

  it("a card's Undo button undoes that card, not the last decision; a sent one shows no actions", async () => {
    const [perm, q] = kinds();
    setNow([perm, q]);
    const { container: c } = render(ChatColumn);
    await press(c, 'a'); // perm-1
    await fireEvent.mouseDown(card(c, 'q-2'));
    await press(c, 'r');
    expect(app.pending.map((p) => p.item)).toEqual(['perm-1', 'q-2']);
    await fireEvent.click([...card(c, 'perm-1').querySelectorAll('.actions button')].find((b) => text(b) === 'Undo u'));
    expect(app.pending.map((p) => p.item)).toEqual(['q-2']);
    // Sent, the core not answered yet: no actions again, no Undo.
    app.pending[0].sent = true;
    flushSync();
    expect(buttons(card(c, 'q-2'))).toEqual([]);
    await fireEvent.mouseDown(card(c, 'q-2'));
    await press(c, 'r');
    expect(app.pending).toHaveLength(1);
  });

  it('keys on a focused line never act on another card', async () => {
    const now = Date.now();
    const all = kinds(now);
    setNow(
      [all[0]],
      [
        closedEntry('c-1', all[1], {
          verb: 'answered',
          agent: 'front-1',
          what: 'Which auth provider?',
          by: 'you',
          at_ms: now - 50 * MIN,
        }),
      ],
    );
    const { container: c } = render(ChatColumn);
    await fireEvent.keyDown(lines(c)[0].querySelector('button'), { key: 'a' });
    expect(app.pending).toEqual([]);
  });

  it('Mission control d / a chip: focusCard opens a folded card', async () => {
    const all = kinds();
    setNow(all);
    const { container: c } = render(ChatColumn);
    app.chatFocus = { id: 'notion-3' };
    await settle();
    expect(selected(c)).toBe('notion-3');
    expect(card(c, 'notion-3').classList.contains('pulse')).toBe(true);
  });

  it('focusCard on a closed decision says what happened and shows the next card, else its line', async () => {
    const now = new Date().setHours(14, 2, 0, 0);
    const [perm, q] = kinds();
    const done = closedEntry(
      'c-1',
      { ...perm, created_ms: now - MIN },
      { verb: 'allowed', agent: 'back-1', what: 'pnpm add zod', by: 'pane', at_ms: now },
    );
    setNow([q], [done, entry('c-2', q)]);
    const { container: c } = render(ChatColumn);
    app.chatFocus = { id: 'perm-1' };
    await settle();
    expect(app.toasts.at(-1).text).toBe('Already allowed in the pane · 14:02. Showing the next card.');
    expect(selected(c)).toBe('q-2');
    cleanup();
    setNow([], [done]);
    const r = render(ChatColumn).container;
    app.chatFocus = { id: 'perm-1' };
    await settle();
    expect(app.toasts.at(-1).text).toBe('Already allowed in the pane · 14:02');
    expect(lines(r)[0].querySelector('button').getAttribute('aria-expanded')).toBe('true');
    app.chatFocus = { id: 'gone-9' };
    await settle();
    expect(app.toasts.at(-1).text).toBe('That decision is no longer listed.');
  });

  it('⌘L opens the search field with its placeholder', async () => {
    setNow([]);
    const { container: c } = render(ChatColumn);
    run('decisions.search');
    await settle();
    const f = c.querySelector('.search input');
    expect(f.getAttribute('placeholder')).toBe('Search decisions and lines');
    expect(document.activeElement).toBe(f);
  });
});

describe('the shell (notes §9)', () => {
  it('the Inspector docks at ≥ 1728 and overlays the chat below, at 1280 / 1512 / 1728', () => {
    app.inspector = true;
    for (const { w, docked } of [
      { w: 1280, docked: false },
      { w: 1512, docked: false },
      { w: 1728, docked: true },
    ]) {
      app.width = w;
      expect(inspectorDocked()).toBe(docked);
      expect(inspectorOverlay()).toBe(!docked);
    }
  });

  it('the overlay says what is behind it: "3 cards need you, 1 blocking · ⌘⇧I"', async () => {
    const all = kinds();
    setNow([all[0], all[6], all[8]]);
    app.now.counts.global.decisions = 3;
    app.inspector = true;
    const { container: c } = render(Inspector, { overlay: true });
    const banner = c.querySelector('.banner');
    expect(text(banner.querySelector('span'))).toMatch(/^\S+ 3 cards need you, 1 blocking$/);
    expect(text(banner.querySelector('.kbd'))).toBe('⌘+⇧+I');
    await fireEvent.click(banner);
    expect(app.inspector).toBe(false);
    expect(app.chatFocus).toEqual({ id: null });
    cleanup();
    setNow([]);
    app.now.counts.global.decisions = 0;
    expect(render(Inspector, { overlay: true }).container.querySelector('.banner')).toBeNull();
  });

  it('a saved Inbox pane (and a Week planner pane, a window now) is dropped at restore; Mission control stays home', async () => {
    // A layout saved with a split (before the single pane, design-notes §4.2).
    const tree = { kind: 'split', id: 's1', dir: 'row', sizes: [0.5, 0.5], children: [L.leaf('p1'), L.leaf('p2')] };
    replies.settings_get = ({ key }) =>
      key === 'layout'
        ? {
            layout: tree,
            panes: { p1: { type: 'inbox', title: 'Inbox' }, p2: { type: 'planner', title: 'Week planner' } },
            focused: 'p1',
            inspector: true,
          }
        : null;
    app.data = { ...fixture(), setup: { done: true } };
    await restoreLayout();
    expect(Object.values(app.panes).map((p) => p.type)).toEqual(['mission']);
    expect(app.window).toBeNull();
    // Below 1728 the saved Inspector does not open over the chat by itself.
    expect(app.inspector).toBe(false);
    replies.settings_get = ({ key }) =>
      key === 'layout'
        ? { layout: L.leaf('p1'), panes: { p1: { type: 'inbox', title: 'Inbox' } }, focused: 'p1' }
        : null;
    app.panes = {};
    app.layout = null;
    await restoreLayout();
    expect(Object.values(app.panes).map((p) => p.type)).toEqual(['mission']);
  });

  it('no code path opens an Inbox pane (spec §1b)', async () => {
    const { readdirSync, readFileSync, existsSync } = await import('node:fs');
    const { join } = await import('node:path');
    const src = join(import.meta.dirname, '../src');
    expect(existsSync(join(src, 'panes/InboxPane.svelte'))).toBe(false);
    const walk = (d) =>
      readdirSync(d, { withFileTypes: true }).flatMap((f) =>
        f.isDirectory() ? walk(join(d, f.name)) : [join(d, f.name)],
      );
    const hits = walk(src)
      .filter((f) => /\.(js|svelte)$/.test(f))
      .filter((f) => /openSingleton\('inbox'\)|type: 'inbox'|InboxPane|'Inbox|Inbox /.test(readFileSync(f, 'utf8')));
    expect(hits).toEqual([]);
  });
});

describe('lines copy (notes §2.2–2.3)', () => {
  const L1 = (verb, by, more = {}) =>
    chat.lineText({ verb, agent: 'back-1', what: 'cargo add serde_json', by, ...more });
  it('verb × who', () => {
    expect(L1('allowed', 'you')).toBe('allowed back-1: cargo add serde_json');
    expect(L1('allowed', 'rule', { by_detail: 'installs' })).toBe(
      'allowed back-1: cargo add serde_json · rule: installs',
    );
    expect(L1('denied', 'rule')).toBe('denied back-1: cargo add serde_json · rule');
    // P4.4: policy answers, and the core's denials with their reason.
    expect(L1('allowed', 'policy', { by_detail: 'installs' })).toBe(
      'allowed back-1: cargo add serde_json · policy: installs',
    );
    expect(chat.byWords({ by: 'policy', by_detail: 'installs' })).toBe('by policy installs');
    expect(L1('denied', 'core', { by_detail: 'git push: the core pushes' })).toBe(
      'denied back-1: cargo add serde_json · core: git push: the core pushes',
    );
    expect(L1('allowed', 'pane')).toBe('allowed back-1: cargo add serde_json · in the pane');
    expect(L1('kept queued', 'expired')).toBe('kept queued back-1: cargo add serde_json · expired');
    expect(L1('expired', 'expired')).toBe('expired back-1: cargo add serde_json');
    expect(L1('dismissed', 'core')).toBe('dismissed back-1: cargo add serde_json · by the app');
    expect(L1('run now', 'you')).toBe('started back-1: cargo add serde_json');
    expect(chat.lineText({ verb: 'applied', what: 'Reserve used up', by: 'expired' })).toBe(
      'applied: Reserve used up · expired',
    );
    expect(chat.warnVerb('denied') && chat.warnVerb('expired') && !chat.warnVerb('allowed')).toBe(true);
    expect(chat.alreadyText({ verb: 'allowed', by: 'pane', at_ms: new Date().setHours(14, 2) })).toBe(
      'Already allowed in the pane · 14:02',
    );
  });

  it('digest states (notes §7) and the badge label', () => {
    expect(chat.digestText([{}, {}], 2)).toMatchObject({ head: 'Today · 2 outcomes', clear: true, more: null });
    expect(chat.digestText([], 5)).toEqual({ head: 'Today · 5 done', clear: false, body: '5 done today, all seen.' });
    expect(chat.digestText([], 0)).toEqual({
      head: 'Today · no outcomes yet',
      clear: false,
      body: 'Nothing done yet today.',
    });
    expect(chat.badgeLabel(0)).toBe('No decisions waiting · ⌘⇧I');
    expect(chat.bannerText(1, 0)).toBe('1 card needs you');
  });
});

describe('performance (spec §5)', () => {
  // Wall-clock under a parallel test run: the median of 20 events, retried when the machine is busy.
  it('a day of 300 entries and 20 open cards renders a now event in < 16 ms', { retry: 2 }, async () => {
    const now = Date.now();
    const base = kinds(now);
    const open = Array.from({ length: 20 }, (_, i) => ({
      ...base[i % base.length],
      id: `open-${i}`,
      created_ms: now - ((300 - i) * MIN) / 2,
    }));
    const closed = Array.from({ length: 279 }, (_, i) =>
      closedEntry(
        `c-${i}`,
        { ...base[0], id: `done-${i}`, created_ms: now - ((600 - i) * MIN) / 2 },
        {
          verb: 'allowed',
          agent: 'back-1',
          what: `cargo add crate-${i}`,
          by: i % 2 ? 'rule' : 'pane',
          at_ms: now - ((600 - i) * MIN) / 2,
        },
      ),
    );
    setNow(open, [morning(now - (700 * MIN) / 2), ...closed, ...open.map((d, i) => entry(`o-${i}`, d))]);
    app.now.rev = 1;
    const { container: c } = render(ChatColumn);
    expect(lines(c)).toHaveLength(279);
    let rev = 1;
    const push = (i) => {
      const d = app.now.decisions[`open-${i % 20}`];
      applyNow({
        rev: ++rev,
        decisions: { upsert: [{ ...d, title: `${d.title} ${rev}` }], closed: [] },
        chat: [{ ...app.now.chat.entries[`o-${i % 20}`], rev: rev }],
        topics: [],
      });
      flushSync();
    };
    for (let i = 0; i < 5; i++) push(i); // warm up
    const times = [];
    for (let i = 0; i < 20; i++) {
      const t0 = performance.now();
      push(i);
      times.push(performance.now() - t0);
    }
    times.sort((a, b) => a - b);
    expect(times[Math.floor(times.length / 2)]).toBeLessThan(16);
  });
});
