// P4.3 T-83: older days, a past day read-only, search, and the core's `focus_card`
// (spec §3.3, §3.5; api-contract §3–4; notes §6, §8). Days are calendar dates:
// the labels and the day arithmetic hold in any time zone.

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { chatOf, closedEntry, entry, kinds, morning } from './chat-fixture.js';
import { fixture, nowOf } from './fixture.js';

const calls = [];
const replies = {};
const handlers = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    const r = replies[cmd];
    return r instanceof Error ? Promise.reject(r) : Promise.resolve(typeof r === 'function' ? r(args) : r);
  },
  listen: (ev, fn) => {
    handlers[ev] = fn;
    return Promise.resolve(() => delete handlers[ev]);
  },
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
}));

const { app, run } = await import('../src/lib/app.svelte.js');
const chat = await import('../src/lib/chat.js');
const { default: ChatColumn } = await import('../src/components/chat/ChatColumn.svelte');

const MIN = 60000;
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const feed = (c) => c.querySelector('[data-chat-focus]');
const press = (c, key) => fireEvent.keyDown(feed(c), { key });
const settle = () => new Promise((r) => setTimeout(r, 0));
const lines = (c) => [...c.querySelectorAll('article.line')];
const selected = (c) => c.querySelector('article.stop.sel .card')?.getAttribute('data-decision') ?? null;

const [perm, q] = kinds();
const old = {
  ...entry('c-90', { ...perm, id: 'perm-0' }, { day: '2026-11-01', at_ms: 1000 }),
  line: {
    verb: 'allowed',
    agent: 'back-1',
    what: 'cargo add serde',
    by: 'rule',
    by_detail: 'installs',
    state: 'effect',
    at_ms: 2000,
  },
  card: perm,
  rev: 2,
};
const left = entry('c-91', { ...q, id: 'q-0' }, { day: '2026-11-01', at_ms: 3000, card: q });
const morn = { ...morning(500), id: 'c-m1', day: '2026-11-01' };
const dayReply = {
  day: '2026-11-01',
  entries: [left, old, morn],
  outcomes: [{ id: 'o-1', kind: 'pr_opened', title: 'PR #12', at_ms: 4000, agent: 'back-1' }],
};

beforeEach(() => {
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  for (const k of Object.keys(handlers)) delete handlers[k];
  replies.chat_days = [
    { day: '2026-11-01', entries: 3, decisions: 2 },
    { day: '2026-10-30', entries: 1, decisions: 0 },
  ];
  replies.chat_day = dayReply;
  app.data = fixture();
  app.now = { ...nowOf(app.data, { decisions: [] }), chat: chatOf([]) };
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

describe('day helpers (time-zone independent)', () => {
  it('label and previous day come from the date, not the clock', () => {
    expect(chat.dayLabel('2026-11-01')).toBe('Sun 1 Nov');
    expect(chat.dayLabel('2026-11-01', true)).toBe('Sunday 1 Nov');
    expect(chat.prevDay('2026-11-01')).toBe('2026-10-31');
    expect(chat.prevDay('2026-03-01')).toBe('2026-02-28');
    expect(chat.dayRowText({ day: '2026-11-01', decisions: 12 }, '2026-11-02')).toBe(
      'Yesterday · Sun 1 Nov · 12 decisions',
    );
    expect(chat.dayRowText({ day: '2026-10-30', decisions: 0 }, '2026-11-02')).toBe('Fri 30 Oct · no decisions');
    expect(chat.resultsHead('serde', 1)).toBe('1 result for “serde”');
    expect(chat.resultsHead('serde', 50)).toBe('First 50 results for “serde”. Narrow the search.');
  });
});

describe('older days', () => {
  it('the day button lists the days; a day opens read-only with its lines, morning line and outcomes', async () => {
    const { container: c } = render(ChatColumn);
    await fireEvent.click(c.querySelector('.day'));
    await settle();
    expect(calls[0]).toEqual(['chat_days', { limit: 30 }]);
    const rows = [...c.querySelectorAll('[data-day-row]')].map(text);
    expect(rows[0]).toBe('Yesterday · Sun 1 Nov · 2 decisions');
    expect(rows[1]).toBe('Fri 30 Oct · no decisions');
    await fireEvent.click(c.querySelector('[data-day-row]'));
    await settle();
    expect(calls.at(-1)).toEqual(['chat_day', { day: '2026-11-01' }]);
    expect(c.querySelector('.day').textContent).toContain('Sun 1 Nov');
    expect(text(c.querySelector('[data-past-strip]'))).toContain('Sunday 1 Nov · read-only · Back to today');
    expect(text(c.querySelector('.morning'))).toContain('Good morning');
    // Oldest first; the one with no line is "left open", muted, no tail.
    expect(lines(c).map((l) => l.dataset.verb)).toEqual(['allowed', 'left open']);
    expect(text(lines(c)[1])).toContain('left open front-1');
    expect(text(c.querySelector('.digest'))).toContain('Sun 1 Nov · 1 outcome');
    expect(text(c.querySelector('footer'))).toBe('Read-only');
    expect(c.querySelector('.filters')).toBeNull();
  });

  it('a line expands from its snapshot with no actions; keys that act do nothing; t goes back to today', async () => {
    const { container: c } = render(ChatColumn);
    await fireEvent.click(c.querySelector('.day'));
    await settle();
    await fireEvent.click(c.querySelector('[data-day-row]'));
    await settle();
    await fireEvent.click(lines(c)[0].querySelector('button'));
    const cardEl = lines(c)[0].querySelector('.card');
    expect(cardEl.classList.contains('ro')).toBe(true);
    expect(cardEl.querySelector('.actions button:not(.ghost)')).toBeNull();
    const before = calls.length;
    for (const k of ['a', 'r', 'x', 'u', 'c']) await press(c, k);
    expect(calls.length).toBe(before);
    expect(app.toasts).toEqual([]);
    await press(c, 't');
    expect(c.querySelector('[data-past-strip]')).toBeNull();
    expect(c.querySelector('.day').textContent).toContain('Today');
  });

  it('a day that fails says why and retries; an empty day says nothing was recorded', async () => {
    replies.chat_day = new Error('boom');
    const { container: c } = render(ChatColumn);
    await fireEvent.click(c.querySelector('.day'));
    await settle();
    await fireEvent.click(c.querySelector('[data-day-row]'));
    await settle();
    expect(text(feed(c))).toContain('Could not load Sun 1 Nov: boom');
    replies.chat_day = { day: '2026-11-01', entries: [], outcomes: [] };
    await fireEvent.click(feed(c).querySelector('button.link'));
    await settle();
    expect(text(feed(c))).toContain('Nothing was recorded on Sun 1 Nov.');
    expect(text(feed(c))).toContain('No outcomes that day.');
  });

  it('[ and ] step through the days; ] from the newest returns to today', async () => {
    const { container: c } = render(ChatColumn);
    await press(c, '[');
    await settle();
    expect(c.querySelector('.day').textContent).toContain('Sun 1 Nov');
    await press(c, '[');
    await settle();
    expect(c.querySelector('.day').textContent).toContain('Fri 30 Oct');
    await press(c, ']');
    await settle();
    expect(c.querySelector('.day').textContent).toContain('Sun 1 Nov');
    await press(c, ']');
    expect(c.querySelector('.day').textContent).toContain('Today');
  });

  it('no earlier days says so', async () => {
    replies.chat_days = [];
    const { container: c } = render(ChatColumn);
    await fireEvent.click(c.querySelector('.day'));
    await settle();
    expect(text(c.querySelector('.days'))).toBe('No earlier days yet.');
  });
});

describe('search', () => {
  const type = async (c, v) => {
    run('decisions.search');
    await settle();
    await fireEvent.input(c.querySelector('input.field'), { target: { value: v } });
  };

  it('results newest first with day separators; under 2 characters nothing is searched', async () => {
    replies.chat_search = [
      {
        day: '2026-11-02',
        entry: {
          ...closedEntry('c-1', perm, { verb: 'denied', agent: 'back-1', what: 'serde', by: 'you', at_ms: Date.now() }),
        },
      },
      { day: '2026-11-01', entry: old },
    ];
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { container: c } = render(ChatColumn);
    await type(c, 's');
    expect(text(c)).toContain('Type 2 or more characters.');
    await type(c, 'serde');
    expect(text(c)).toContain('Searching…');
    await vi.advanceTimersByTimeAsync(250);
    expect(calls.at(-1)).toEqual(['chat_search', { query: 'serde', limit: 50 }]);
    expect(calls.filter(([n]) => n === 'chat_search')).toHaveLength(1);
    expect(text(c.querySelector('.rhead'))).toBe('2 results for “serde”');
    expect([...c.querySelectorAll('.sep')].map(text)).toEqual(['MON 2 NOV', 'SUN 1 NOV']);
    expect(text(c.querySelectorAll('[data-result]')[1])).toContain('allowed back-1: cargo add serde');
    expect(text(c.querySelectorAll('[data-result]')[1])).toContain('Sun 1 Nov');
  });

  it('a result of an earlier day opens that day with the entry expanded; Esc returns to the results', async () => {
    replies.chat_search = [{ day: '2026-11-01', entry: old }];
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { container: c } = render(ChatColumn);
    await type(c, 'serde');
    await vi.advanceTimersByTimeAsync(250);
    await fireEvent.click(c.querySelector('[data-result]'));
    await vi.advanceTimersByTimeAsync(10);
    expect(calls.at(-1)).toEqual(['chat_day', { day: '2026-11-01' }]);
    expect(lines(c)[0].querySelector('button').getAttribute('aria-expanded')).toBe('true');
    await press(c, 'Escape');
    expect(c.querySelector('[data-result]')).not.toBeNull();
    expect(/** @type {HTMLInputElement} */ (c.querySelector('input.field')).value).toBe('serde');
  });

  it("t and Back to today from a result's day close the search; an open decision reads with its kind", async () => {
    app.now = { ...nowOf(app.data, { decisions: [perm] }), chat: chatOf([entry('c-1', perm)]) };
    replies.chat_search = [
      { day: '2026-11-02', entry: entry('c-1', perm) },
      { day: '2026-11-01', entry: old },
    ];
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { container: c } = render(ChatColumn);
    await type(c, 'serde');
    await vi.advanceTimersByTimeAsync(250);
    expect(text(c.querySelectorAll('[data-result]')[0])).toContain('● Permission · Bash · pnpm add zod');
    await fireEvent.click(c.querySelectorAll('[data-result]')[1]);
    await vi.advanceTimersByTimeAsync(10);
    await press(c, 't');
    expect(c.querySelector('[data-result]')).toBeNull();
    expect(c.querySelector('input.field')).toBeNull();
    expect(c.querySelector('.day').textContent).toContain('Today');
  });

  it('a result of today opens its line in the thread', async () => {
    const e = closedEntry('c-1', perm, {
      verb: 'allowed',
      agent: 'back-1',
      what: 'pnpm add zod',
      by: 'pane',
      at_ms: Date.now(),
    });
    app.now = { ...nowOf(app.data, { decisions: [] }), chat: chatOf([e]) };
    replies.chat_search = [{ day: '2026-11-02', entry: e }];
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { container: c } = render(ChatColumn);
    await type(c, 'zod');
    await vi.advanceTimersByTimeAsync(250);
    await fireEvent.click(c.querySelector('[data-result]'));
    await vi.advanceTimersByTimeAsync(10);
    expect(c.querySelector('[data-result]')).toBeNull();
    expect(lines(c)[0].querySelector('button').getAttribute('aria-expanded')).toBe('true');
  });

  it('no results and errors have their copy; Retry searches again', async () => {
    replies.chat_search = [];
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const { container: c } = render(ChatColumn);
    await type(c, 'zzz');
    await vi.advanceTimersByTimeAsync(250);
    expect(text(feed(c))).toBe('Nothing found for “zzz” in decisions and lines.');
    replies.chat_search = new Error('db locked');
    await type(c, 'zzzz');
    await vi.advanceTimersByTimeAsync(250);
    expect(text(feed(c))).toContain('Could not search: db locked');
    replies.chat_search = [];
    await fireEvent.click(feed(c).querySelector('button.link'));
    await vi.advanceTimersByTimeAsync(250);
    expect(text(feed(c))).toContain('Nothing found for “zzzz”');
  });
});

describe('focus_card (spec §3.5, api-contract §4)', () => {
  const closed = () =>
    closedEntry(
      'c-1',
      { ...perm, created_ms: Date.now() - MIN },
      { verb: 'allowed', agent: 'back-1', what: 'pnpm add zod', by: 'pane', at_ms: new Date().setHours(14, 2, 0, 0) },
    );

  it('focuses an open card, closes the Inspector overlay, leaves a past day', async () => {
    app.now = { ...nowOf(app.data, { decisions: [perm, q] }), chat: chatOf([entry('c-1', perm), entry('c-2', q)]) };
    app.inspector = true;
    const { container: c } = render(ChatColumn);
    await settle();
    await fireEvent.click(c.querySelector('.day'));
    await settle();
    await fireEvent.click(c.querySelector('[data-day-row]'));
    await settle();
    handlers.focus_card({ payload: { id: 'q-2' } });
    await settle();
    expect(app.inspector).toBe(false);
    expect(c.querySelector('[data-past-strip]')).toBeNull();
    expect(selected(c)).toBe('q-2');
    expect(c.querySelector('.card[data-decision="q-2"]').classList.contains('pulse')).toBe(true);
    expect(app.toasts).toEqual([]);
  });

  it('a closed decision: toasts what happened and focuses the next open card', async () => {
    app.now = { ...nowOf(app.data, { decisions: [q] }), chat: chatOf([closed(), entry('c-2', q)]) };
    const { container: c } = render(ChatColumn);
    await settle();
    handlers.focus_card({ payload: { id: 'perm-1' } });
    await settle();
    expect(app.toasts.at(-1).text).toBe('Already allowed in the pane · 14:02. Showing the next card.');
    expect(selected(c)).toBe('q-2');
  });

  it('stops listening when the column goes', async () => {
    const { unmount } = render(ChatColumn);
    await settle();
    expect(handlers.focus_card).toBeTypeOf('function');
    unmount();
    await settle();
    expect(handlers.focus_card).toBeUndefined();
  });
});

describe('still open from yesterday (F1, api-contract §1)', () => {
  const NOW = Date.now();
  const [p, qq] = kinds(NOW);
  const setup = (decisions, entries) => {
    app.now = { ...nowOf(app.data, { decisions }), chat: chatOf(entries) };
  };
  const carriedEntry = (id, d, from, at) => entry(id, d, { at_ms: at, continued_from: from });
  const groups = (c) => [...c.querySelectorAll('[data-group]')].map(text);
  const order = (c) =>
    [...feed(c).children].map((x) => (x.matches('[data-group]') ? text(x) : x.className.split(' ')[0]));

  it('one carried card: the label names the day before, right after the morning line; Today follows', () => {
    const today = { ...qq, created_ms: NOW - 5 * MIN };
    setup([p, today], [morning(NOW - 60 * MIN), entry('c-9', today), carriedEntry('c-8', p, 'c-790', NOW - 59 * MIN)]);
    const { container: c } = render(ChatColumn);
    expect(groups(c)).toEqual(['STILL OPEN FROM SUN 1 NOV', 'TODAY']);
    const o = order(c);
    expect(o.slice(0, 5)).toEqual(['morning', 'STILL OPEN FROM SUN 1 NOV', 'stop', 'TODAY', 'stop']);
    expect(c.querySelector('[data-decision="perm-1"]')).not.toBeNull();
  });

  it('two carried cards keep their order inside the group, even when a today card is older', () => {
    const a = { ...p, created_ms: NOW - 50 * MIN };
    const b = { ...qq, created_ms: NOW - 40 * MIN };
    const t = { ...qq, id: 'q-3', created_ms: NOW - 55 * MIN };
    setup(
      [a, b, t],
      [
        morning(NOW - 60 * MIN),
        entry('c-t', t),
        carriedEntry('c-1', a, 'c-70', NOW - 50 * MIN),
        carriedEntry('c-2', b, 'c-71', NOW - 40 * MIN),
      ],
    );
    const { container: c } = render(ChatColumn);
    expect(groups(c)).toEqual(['STILL OPEN FROM SUN 1 NOV', 'TODAY']);
    expect(order(c).slice(0, 6)).toEqual(['morning', 'STILL OPEN FROM SUN 1 NOV', 'stop', 'stop', 'TODAY', 'stop']);
    const ids = [...c.querySelectorAll('[data-decision]')].map((x) => x.getAttribute('data-decision'));
    expect(ids).toEqual(['perm-1', 'q-2', 'q-3']);
  });

  it('the group disappears when the carried cards close', () => {
    setup([p], [morning(NOW - 60 * MIN), carriedEntry('c-8', p, 'c-790', NOW - 59 * MIN)]);
    const { container: c, unmount } = render(ChatColumn);
    expect(groups(c)).toEqual(['STILL OPEN FROM SUN 1 NOV']);
    unmount();
    const closed = { ...p, level: 4, lifecycle: { state: 'effect', decided_by: 'user', effect_at_ms: NOW - 20 * MIN } };
    setup(
      [closed],
      [
        morning(NOW - 60 * MIN),
        {
          ...carriedEntry('c-8', p, 'c-790', NOW - 59 * MIN),
          line: {
            verb: 'allowed',
            agent: 'back-1',
            what: 'pnpm add zod',
            by: 'you',
            state: 'effect',
            at_ms: NOW - 20 * MIN,
          },
          card: closed,
        },
      ],
    );
    const r = render(ChatColumn);
    expect(groups(r.container)).toEqual([]);
    expect(r.container.querySelectorAll('article.line').length).toBe(1);
  });

  it('no Today label when nothing under it is shown (a filter hides the rest)', () => {
    const blocking = { ...p, level: 1 };
    const other = { ...qq, level: 2 };
    const t = chat.thread({
      chat: { day: '2026-11-02', entries: { a: carriedEntry('c-8', blocking, 'c-1', 1), b: entry('c-9', other) } },
      decisions: { [blocking.id]: blocking, [other.id]: other },
      filter: 'blocking',
    });
    expect(t.items.filter((i) => i.type === 'group').map((i) => i.label)).toEqual(['Still open from Sun 1 Nov']);
  });

  it('the label is a calendar date: the same in any time zone, across a month start', () => {
    expect(chat.thread({ chat: { day: '2026-11-02', entries: {} } }).items).toEqual([]);
    const label = (day) => {
      const e = carriedEntry('c-8', p, 'c-1', 1);
      const t = chat.thread({ chat: { day, entries: { 'c-8': e } }, decisions: { [p.id]: p } });
      return t.items.find((i) => i.type === 'group')?.label;
    };
    expect(label('2026-11-02')).toBe('Still open from Sun 1 Nov');
    expect(label('2026-03-01')).toBe('Still open from Sat 28 Feb');
    expect(label(null)).toBe('Still open from yesterday');
  });
});
