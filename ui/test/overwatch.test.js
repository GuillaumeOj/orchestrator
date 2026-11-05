// P4.4 F1: the chat with the Overwatch (spec §3.9 and §5 UI; docs/design/p4-4-design-notes.md
// §1–§4, §9, §11): the status in the header and its strip, the input in every status, the
// user's and the Overwatch's text entries with their source links, the deciding marker
// and its hand-over, the annotation, `/` and ⌘L, the Overwatch settings. Against a mocked core.

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { flushSync, tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { chatOf, closedEntry, entry, kinds } from './chat-fixture.js';
import { fixture, nowOf } from './fixture.js';

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

const { app, run } = await import('../src/lib/app.svelte.js');
const { applyNow } = await import('../src/lib/now.svelte.js');
const ow = await import('../src/lib/overwatch.js');
const { classify } = await import('../src/lib/keyboard.js');
const { thread } = await import('../src/lib/chat.js');
const { default: ChatColumn } = await import('../src/components/chat/ChatColumn.svelte');
const { default: SettingsPane } = await import('../src/panes/SettingsPane.svelte');

const MIN = 60000;
const DAY = '2026-11-02';
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const settle = () => new Promise((r) => setTimeout(r, 0));
const feed = (c) => c.querySelector('[data-chat-focus]');
const field = (c) => c.querySelector('textarea[aria-label="Message the Overwatch"]');
const status = (c) => c.querySelector('[data-ow-status]');
const card = (c, id) => c.querySelector(`.card[data-decision="${id}"]`);
const live = (c) => text(c.querySelector('.sr'));

const OW = (status, over = {}) => ({
  status,
  events: 0,
  since_ms: Date.now() - 5 * MIN,
  agent: 'overwatch',
  has_session: true,
  ...over,
});
/** A text entry of the thread. */
const msg = (id, author, at_ms, over = {}) => ({
  id,
  day: DAY,
  at_ms,
  author,
  kind: 'text',
  text: id,
  rev: 1,
  ...over,
});

// jsdom has no layout: a clamped box "overflows" (its text is longer than the clamp), an open one fits.
Object.defineProperty(HTMLElement.prototype, 'scrollHeight', {
  configurable: true,
  get() {
    return this.classList?.contains('clamped') || this.classList?.contains('clamp') ? 500 : 0;
  },
});
Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
  configurable: true,
  get() {
    return this.classList?.contains('clamped') || this.classList?.contains('clamp') ? 100 : 0;
  },
});

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
  app.chatDraft = '';
  app.inspector = false;
  app.width = 1512;
  app.overlay = null;
  app.dock = { open: false, height: 300, tabs: [], active: null, v: 2 };
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

/** `app.now` with these decisions, entries and Overwatch status. */
function setNow(decisions = [], entries = [], overwatch = OW('asleep'), more = {}) {
  app.now = { ...nowOf(app.data, { decisions, ...more }), chat: chatOf(entries), overwatch };
}

describe('the status in the header (notes §1)', () => {
  /** @type {[string, any, string][]} */
  const CASES = [
    ['off', OW('off', { has_session: false }), 'off'],
    ['asleep', OW('asleep'), 'asleep'],
    ['waking', OW('waking', { events: 2 }), 'waking…'],
    ['working', OW('working', { events: 2 }), 'working · 2 events'],
    ['working (one)', OW('working', { events: 1 }), 'working · 1 event'],
    ['paused', OW('paused', { why: 'Pause all' }), 'paused'],
    ['down', OW('down', { why: 'login expired' }), 'down'],
  ];

  it('the title is "Overwatch" and the status is text after it, every state', () => {
    for (const [, o, short] of CASES) {
      setNow([], [], o);
      const { container: c } = render(ChatColumn);
      expect(text(c.querySelector('h2.title'))).toBe('Overwatch');
      expect(text(status(c))).toMatch(new RegExp(`${short.replace('…', '…')}$`));
      cleanup();
    }
  });

  it('tooltips and accessible names are the long form', () => {
    const tip = (o) => {
      setNow([], [], o);
      const r = render(ChatColumn);
      const el = status(r.container);
      const t = el.getAttribute('title');
      cleanup();
      return t;
    };
    expect(tip(OW('off', { has_session: false }))).toBe('Overwatch is off. Turn it on in Settings.');
    // A status with a session today is a control: the tooltip adds the key.
    const open = ' · ⏎ open its session';
    expect(tip(OW('asleep'))).toBe(`Overwatch is asleep. It wakes on the next event.${open}`);
    expect(tip(OW('asleep', { has_session: false }))).toBe('Overwatch is asleep. It wakes on the next event.');
    expect(tip(OW('waking', { events: 2 }))).toBe(`Overwatch is waking · 2 events waiting${open}`);
    expect(tip(OW('working', { events: 2 }))).toBe(`Overwatch is working on 2 events${open}`);
    expect(tip(OW('paused', { why: 'limit reached' }))).toBe(`Overwatch is paused: limit reached${open}`);
    expect(tip(OW('down', { why: 'login expired' }))).toBe(
      `Overwatch is down: login expired. Cards still work.${open}`,
    );
  });

  it('a click on the status opens its session over the board; no session or off: not a control', async () => {
    setNow([], [], OW('working', { events: 1 }));
    const { container: c, unmount } = render(ChatColumn);
    expect(status(c).tagName).toBe('BUTTON');
    await fireEvent.click(status(c));
    expect(Object.values(app.panes).some((p) => p.type === 'agent' && p.agentId === 'overwatch')).toBe(true);
    unmount();
    for (const o of [OW('asleep', { has_session: false }), OW('off', { has_session: true })]) {
      setNow([], [], o);
      const r = render(ChatColumn);
      expect(status(r.container).tagName).toBe('SPAN');
      r.unmount();
    }
  });

  it('down: a strip with the reason and Restart; Restart reads "Restarting…" until it leaves down', async () => {
    replies.overwatch_restart = { ok: true };
    setNow([], [], OW('down', { why: 'login expired' }));
    const { container: c } = render(ChatColumn);
    const strip = c.querySelector('[data-ow-strip]');
    expect(text(strip.querySelector('.l1'))).toBe('Overwatch is down');
    expect(text(strip.querySelector('.l2'))).toBe('login expired · cards still work');
    await fireEvent.click(strip.querySelector('button'));
    await settle();
    expect(calls).toContainEqual(['overwatch_restart', undefined]);
    expect(text(strip.querySelector('button'))).toBe('Restarting…');
    expect(/** @type {HTMLButtonElement} */ (strip.querySelector('button')).disabled).toBe(true);
    app.now.overwatch = OW('waking', { events: 1 });
    await settle();
    expect(c.querySelector('[data-ow-strip]')).toBeNull();
  });

  it('a failed Restart gives the button back and says why', async () => {
    replies.overwatch_restart = new Error('no login');
    setNow([], [], OW('down', { why: 'could not start' }));
    const { container: c } = render(ChatColumn);
    await fireEvent.click(c.querySelector('[data-ow-strip] button'));
    await settle();
    expect(text(c.querySelector('[data-ow-strip] button'))).toBe('Restart');
    expect(app.toasts.at(-1).text).toBe('Could not restart: no login');
    // The strip's second line says it once, in place of the reason.
    expect(text(c.querySelector('[data-ow-strip] .l2'))).toBe('Could not restart: no login');
  });

  it('no strip while off or paused; Restart is dim while the core is unreachable', () => {
    for (const o of [OW('off'), OW('paused', { why: 'Pause all' })]) {
      setNow([], [], o);
      const r = render(ChatColumn);
      expect(r.container.querySelector('[data-ow-strip]')).toBeNull();
      r.unmount();
    }
    app.data.core.ok = false;
    setNow([], [], OW('down', { why: 'stopped unexpectedly' }));
    const { container: c } = render(ChatColumn);
    expect(/** @type {HTMLButtonElement} */ (c.querySelector('[data-ow-strip] button')).disabled).toBe(true);
  });

  it('down and back are announced once; "Overwatch is back." is a toast', async () => {
    setNow([], [], OW('working', { events: 1 }));
    const { container: c } = render(ChatColumn);
    app.now.overwatch = OW('down', { why: 'login expired' });
    await settle();
    expect(live(c)).toBe('Overwatch is down: login expired. Cards still work.');
    expect(app.toasts).toEqual([]);
    app.now.overwatch = OW('asleep');
    await settle();
    expect(live(c)).toBe('Overwatch is back.');
    expect(app.toasts.at(-1).text).toBe('Overwatch is back.');
    app.now.overwatch = OW('paused', { why: 'Pause all' });
    await settle();
    expect(app.toasts).toHaveLength(1);
  });

  it('the snapshot and the delta carry now.overwatch; an older core means off', () => {
    applyNow({ rev: app.now.rev + 1, overwatch: OW('working', { events: 3 }), topics: ['overwatch'] });
    expect(app.now.overwatch.status).toBe('working');
    expect(ow.overwatchOf({}).status).toBe('off');
  });
});

describe('the input (notes §2)', () => {
  it('is there in every status with its placeholder and label', () => {
    for (const s of ['off', 'asleep', 'waking', 'working', 'paused', 'down']) {
      setNow([], [], OW(s));
      const r = render(ChatColumn);
      const f = field(r.container);
      expect(f, s).toBeTruthy();
      expect(f.getAttribute('placeholder')).toBe('Ask the Overwatch…');
      expect(f.disabled).toBe(false);
      r.unmount();
    }
  });

  it('the hint under it says what the status means', () => {
    const hint = (o) => {
      setNow([], [], o);
      const r = render(ChatColumn);
      const t = text(r.container.querySelector('.hints .state'));
      r.unmount();
      return t;
    };
    expect(hint(OW('asleep'))).toBe('');
    expect(hint(OW('working'))).toBe('');
    expect(hint(OW('paused'))).toBe('Paused · it answers when it resumes');
    expect(hint(OW('down'))).toBe('Down · messages are kept until it is back');
    expect(hint(OW('off'))).toBe('Off · turn it on in Settings');
  });

  it('"turn it on in Settings" opens the Overwatch section', async () => {
    setNow([], [], OW('off'));
    const { container: c } = render(ChatColumn);
    await fireEvent.click(c.querySelector('.hints .link'));
    expect(Object.values(app.panes).find((p) => p.type === 'settings')?.section).toBe('overwatch');
  });

  it('⏎ sends and clears; ⇧⏎ adds a line; empty or blank sends nothing', async () => {
    replies.chat_send = (a) => ({ entry: msg('c-1', 'user', Date.now(), { text: a.text, delivery: 'delivered' }) });
    setNow();
    const { container: c } = render(ChatColumn);
    const f = field(c);
    await fireEvent.input(f, { target: { value: 'why is checkout slow today?' } });
    await fireEvent.keyDown(f, { key: 'Enter', shiftKey: true });
    expect(calls.filter(([k]) => k === 'chat_send')).toEqual([]);
    await fireEvent.keyDown(f, { key: 'Enter' });
    await settle();
    expect(calls.filter(([k]) => k === 'chat_send')).toEqual([['chat_send', { text: 'why is checkout slow today?' }]]);
    expect(app.chatDraft).toBe('');
    expect(c.querySelector('article.text.user')).toBeTruthy();
    await fireEvent.input(f, { target: { value: '   ' } });
    await fireEvent.keyDown(f, { key: 'Enter' });
    expect(calls.filter(([k]) => k === 'chat_send')).toHaveLength(1);
  });

  it('⏎ does not send while an IME composes', async () => {
    setNow();
    const { container: c } = render(ChatColumn);
    await fireEvent.input(field(c), { target: { value: 'にほん' } });
    await fireEvent.keyDown(field(c), { key: 'Enter', isComposing: true });
    expect(calls.filter(([k]) => k === 'chat_send')).toEqual([]);
  });

  it('Esc leaves to the thread and keeps the draft; a second Esc is the centre pane', async () => {
    setNow();
    const { container: c } = render(ChatColumn);
    field(c).focus();
    await fireEvent.input(field(c), { target: { value: 'half a thought' } });
    await fireEvent.keyDown(field(c), { key: 'Escape' });
    expect(document.activeElement).toBe(feed(c));
    expect(app.chatDraft).toBe('half a thought');
    expect(field(c).value).toBe('half a thought');
  });

  it('the card keys never fire while the input has the focus', async () => {
    const [perm] = kinds();
    setNow([perm], [entry('c-1', perm)]);
    const { container: c } = render(ChatColumn);
    field(c).focus();
    for (const key of ['a', 'r', 'u', 'j', 'f', '/']) await fireEvent.keyDown(field(c), { key });
    expect(app.pending).toEqual([]);
    expect(c.querySelector('.search')).toBeNull();
  });

  it('a draft past 1 800 characters shows a counter; past 2 000 send is refused', async () => {
    setNow();
    const { container: c } = render(ChatColumn);
    await fireEvent.input(field(c), { target: { value: 'x'.repeat(1799) } });
    expect(c.querySelector('[data-count]')).toBeNull();
    await fireEvent.input(field(c), { target: { value: 'x'.repeat(1950) } });
    expect(text(c.querySelector('[data-count]'))).toBe('1 950 / 2 000');
    await fireEvent.input(field(c), { target: { value: 'x'.repeat(2001) } });
    await fireEvent.keyDown(field(c), { key: 'Enter' });
    expect(calls.filter(([k]) => k === 'chat_send')).toEqual([]);
    expect(text(c.querySelector('[role=alert]'))).toBe(
      'Too long for a message. Put it in a file and ask the Overwatch to read it.',
    );
  });

  it('the draft survives the column being closed and opened again', () => {
    setNow();
    const first = render(ChatColumn);
    app.chatDraft = 'later';
    first.unmount();
    const { container: c } = render(ChatColumn);
    expect(field(c).value).toBe('later');
  });

  it('a refused send keeps the message at the foot with "Not sent: … · Retry"', async () => {
    replies.chat_send = new Error('core is busy');
    setNow();
    const { container: c } = render(ChatColumn);
    await fireEvent.input(field(c), { target: { value: 'hello' } });
    await fireEvent.keyDown(field(c), { key: 'Enter' });
    await settle();
    const art = c.querySelector('article.text.user');
    expect(text(art)).toContain('hello');
    expect(text(art)).toContain('Not sent: core is busy · Retry');
    replies.chat_send = (a) => ({ entry: msg('c-2', 'user', Date.now(), { text: a.text, delivery: 'delivered' }) });
    await fireEvent.click(art.querySelector('button.link'));
    await settle();
    expect(c.querySelector('[role=alert]')).toBeNull();
    expect(c.querySelectorAll('article.text.user')).toHaveLength(1);
  });

  it('a past day has no input, only the read-only footer', async () => {
    replies.chat_days = [{ day: '2026-11-01', decisions: 2 }];
    replies.chat_day = { entries: [], outcomes: [] };
    setNow();
    const { container: c } = render(ChatColumn);
    await fireEvent.click(c.querySelector('.day'));
    await settle();
    await fireEvent.click(c.querySelector('[data-day-row]'));
    await settle();
    expect(field(c)).toBeNull();
    expect(text(c.querySelector('footer.foot'))).toContain('Read-only · Back to today');
  });
});

describe('/ and ⌘L (notes §2.3, §11)', () => {
  it('⌘L is "Ask the Overwatch"; search has no chord', () => {
    expect(classify({ key: 'l', code: 'KeyL', metaKey: true }, {}).command).toBe('overwatch.ask');
  });

  it('⌘L focuses the input from anywhere and closes an open search', async () => {
    setNow();
    const { container: c } = render(ChatColumn);
    run('decisions.search');
    await settle();
    expect(c.querySelector('.search input')).toBeTruthy();
    run('overwatch.ask');
    await settle();
    expect(c.querySelector('.search')).toBeNull();
    expect(document.activeElement).toBe(field(c));
  });

  it('⌘L on a past day goes back to today and focuses the input', async () => {
    replies.chat_days = [{ day: '2026-11-01', decisions: 2 }];
    replies.chat_day = { entries: [], outcomes: [] };
    setNow();
    const { container: c } = render(ChatColumn);
    await fireEvent.click(c.querySelector('.day'));
    await settle();
    await fireEvent.click(c.querySelector('[data-day-row]'));
    await settle();
    run('overwatch.ask');
    await settle();
    expect(document.activeElement).toBe(field(c));
  });

  it('⌘L with the input already focused changes nothing; the draft stays', async () => {
    setNow();
    const { container: c } = render(ChatColumn);
    field(c).focus();
    await fireEvent.input(field(c), { target: { value: 'typing' } });
    run('overwatch.ask');
    await settle();
    expect(document.activeElement).toBe(field(c));
    expect(field(c).value).toBe('typing');
  });

  it('/ opens the search from the thread and the header button does too; in the input it types', async () => {
    setNow();
    const { container: c } = render(ChatColumn);
    expect(c.querySelector('.iconbtn').getAttribute('aria-label')).toBe('Search decisions · /');
    await fireEvent.keyDown(feed(c), { key: '/' });
    await settle();
    expect(document.activeElement).toBe(c.querySelector('.search input'));
    await fireEvent.keyDown(c.querySelector('.search input'), { key: 'Escape' });
    await fireEvent.click(c.querySelector('.iconbtn'));
    await settle();
    expect(c.querySelector('.search input')).toBeTruthy();
  });

  it('the palette lists Ask the Overwatch (⌘L) and Search decisions; Restart only while down', async () => {
    const { commands } = await import('../src/lib/app.svelte.js');
    setNow([], [], OW('asleep'));
    let ids = commands().map((x) => x.id);
    expect(ids).toContain('overwatch.ask');
    expect(ids).toContain('decisions.search');
    expect(ids).toContain('overwatch.session');
    expect(ids).not.toContain('overwatch.restart');
    setNow([], [], OW('down', { has_session: false }));
    ids = commands().map((x) => x.id);
    expect(ids).toContain('overwatch.restart');
    expect(ids).not.toContain('overwatch.session');
  });
});

describe('text entries (notes §2.1)', () => {
  it('your message is right-aligned plain text; the Overwatch answers in markdown, in time order with the cards', () => {
    const now = Date.now();
    const [perm] = kinds();
    setNow(
      [perm],
      [
        msg('c-1', 'user', now - 3 * MIN, { text: 'why is **checkout** slow?', delivery: 'delivered' }),
        entry('c-2', perm, { at_ms: now - 2 * MIN }),
        msg('c-3', 'overwatch', now - MIN, {
          text: 'It waited on `back-1`:\n\n- the live cap\n- a permission',
          reply_to: 'c-1',
        }),
      ],
      OW('asleep'),
    );
    const { container: c } = render(ChatColumn);
    const order = [...c.querySelectorAll('.feed > article, .feed > .stop')].map(
      (x) => x.getAttribute('data-entry') ?? 'card',
    );
    expect(order.slice(0, 3)).toEqual(['c-1', 'card', 'c-3']);
    const user = c.querySelector('[data-entry="c-1"]');
    expect(user.getAttribute('aria-label')).toMatch(/^You, \d\d:\d\d$/);
    expect(user.querySelector('strong')).toBeNull(); // plain text: no markdown for your own message
    expect(text(user)).toContain('why is **checkout** slow?');
    const reply = c.querySelector('[data-entry="c-3"]');
    expect(reply.getAttribute('aria-label')).toMatch(/^Overwatch, \d\d:\d\d$/);
    expect(reply.querySelector('code').textContent).toBe('back-1');
    expect([...reply.querySelectorAll('li')].map(text)).toEqual(['the live cap', 'a permission']);
  });

  it('raw HTML in its text is shown as text, never as markup', () => {
    setNow([], [msg('c-1', 'overwatch', Date.now(), { text: 'hi <img src=x onerror=alert(1)> <b>x</b>' })]);
    const { container: c } = render(ChatColumn);
    expect(c.querySelector('[data-entry="c-1"] img')).toBeNull();
    expect(c.querySelector('[data-entry="c-1"] b')).toBeNull();
  });

  it('a "↳ re:" line shows when the question is not just above, and jumps to it', async () => {
    const now = Date.now();
    const [perm] = kinds();
    setNow(
      [perm],
      [
        msg('c-1', 'user', now - 5 * MIN, { text: 'why is checkout slow today?', delivery: 'delivered' }),
        entry('c-2', perm, { at_ms: now - 4 * MIN }),
        msg('c-3', 'overwatch', now - MIN, { text: 'answer', reply_to: 'c-1' }),
        msg('c-4', 'user', now - 10000, { text: 'thanks', delivery: 'delivered' }),
        msg('c-5', 'overwatch', now - 5000, { text: 'adjacent answer', reply_to: 'c-4' }),
      ],
    );
    const { container: c } = render(ChatColumn);
    const ref = c.querySelector('[data-entry="c-3"] .ref');
    expect(text(ref)).toBe('↳ re: why is checkout slow today?');
    expect(c.querySelector('[data-entry="c-5"] .ref')).toBeNull();
    await fireEvent.click(ref);
    await settle();
    expect(c.querySelector('[data-entry="c-1"]').classList.contains('pulse')).toBe(true);
  });

  it('a long answer clamps at 12 lines with Show more / Show less; ⏎ on it toggles', async () => {
    const long = Array.from({ length: 20 }, (_, i) => `line ${i + 1}`).join('\n\n');
    setNow([], [msg('c-1', 'overwatch', Date.now(), { text: long })]);
    const { container: c } = render(ChatColumn);
    const art = c.querySelector('[data-entry="c-1"]');
    expect(art.querySelector('.body').classList.contains('clamped')).toBe(true);
    const more = art.querySelector('button.more');
    expect(text(more)).toBe('Show more');
    await fireEvent.click(more);
    expect(art.querySelector('.body').classList.contains('clamped')).toBe(false);
    expect(text(more)).toBe('Show less');
    await fireEvent.keyDown(art, { key: 'Enter' });
    expect(art.querySelector('.body').classList.contains('clamped')).toBe(true);
    // The card keys do not run on ⏎ here.
    expect(app.pending).toEqual([]);
  });

  describe('source links', () => {
    const NOW = Date.now();
    const answer = (body, extra = []) => [
      msg('c-1', 'user', NOW - 5 * MIN, { text: 'q', delivery: 'delivered' }),
      closedEntry(
        'c-2',
        { ...kinds()[1], created_ms: NOW - 4 * MIN },
        { verb: 'answered', agent: 'front-1', what: 'API pagination', by: 'you', at_ms: NOW - 3 * MIN },
      ),
      msg('c-3', 'overwatch', NOW - MIN, { text: body }),
      ...extra,
    ];

    it('a feature doc opens in the feature pane at that doc', async () => {
      app.data.features = [{ id: 'f1', slug: 'checkout', project_id: 'shop-api' }];
      setNow([], answer('See [the contract](.orchestrator/features/checkout/api-contract.md#3).'));
      const { container: c } = render(ChatColumn);
      const link = c.querySelector('[data-entry="c-3"] button.ref');
      expect(/** @type {HTMLElement} */ (link).dataset.ref).toBe('doc');
      expect(link.textContent).toBe('api-contract.md §3↗'); // the label form, not the model's words
      await fireEvent.click(link);
      const pane = Object.values(app.panes).find((p) => p.type === 'feature');
      expect(pane).toMatchObject({ featureId: 'f1', doc: 'api-contract.md' });
    });

    it('chat:<id> jumps to that entry and highlights it', async () => {
      setNow([], answer('Per [your answer](chat:c-1).'));
      const { container: c } = render(ChatColumn);
      await fireEvent.click(c.querySelector('[data-entry="c-3"] button.ref'));
      await settle();
      expect(c.querySelector('[data-entry="c-1"]').classList.contains('pulse')).toBe(true);
    });

    it('chat:<id> of a line expands that line', async () => {
      setNow([], answer('Per [the card](chat:c-2).'));
      const { container: c } = render(ChatColumn);
      await fireEvent.click(c.querySelector('[data-entry="c-3"] button.ref'));
      await settle();
      expect(c.querySelector('article.line button').getAttribute('aria-expanded')).toBe('true');
    });

    it('agent:<id> opens its session; run:<id> selects the run in the inspector', async () => {
      app.data.agents.push({ id: 'back-1', state: 'working', live: true, project_id: 'shop-api' });
      setNow([], answer('[back-1](agent:back-1) and [run checkout](run:run-12)'), OW('asleep'), {});
      app.now.runs['run-12'] = { run: 'run-12' };
      const { container: c } = render(ChatColumn);
      const links = c.querySelectorAll('[data-entry="c-3"] button.ref');
      await fireEvent.click(links[0]);
      expect(Object.values(app.panes).some((p) => p.type === 'agent' && p.agentId === 'back-1')).toBe(true);
      await fireEvent.click(links[1]);
      expect(app.selection).toEqual({ kind: 'run', id: 'run-12' });
      expect(app.inspector).toBe(true);
    });

    it('a link that no longer resolves is plain text with a tooltip; http opens the browser', async () => {
      setNow([], answer('[gone](chat:c-99) · [ghost](agent:nope) · [site](https://example.com)'));
      const { container: c } = render(ChatColumn);
      const art = c.querySelector('[data-entry="c-3"]');
      expect(art.querySelectorAll('button.ref')).toHaveLength(0);
      expect(art.querySelectorAll('.gone')).toHaveLength(2);
      expect(art.querySelector('.gone').getAttribute('title')).toBe('No longer available');
      replies.open_url = null;
      await fireEvent.click(art.querySelector('a.ext'));
      expect(calls).toContainEqual(['open_url', { url: 'https://example.com' }]);
    });
  });

  it('a new Overwatch reply is announced as "The Overwatch replied", not read out', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    setNow([], [msg('c-1', 'user', Date.now() - 1000, { delivery: 'delivered' })], OW('working', { events: 1 }));
    const { container: c } = render(ChatColumn);
    app.now.chat.entries['c-2'] = msg('c-2', 'overwatch', Date.now(), { text: 'a secret answer', reply_to: 'c-1' });
    flushSync();
    await tick();
    expect(live(c)).toBe('The Overwatch replied');
    expect(live(c)).not.toContain('secret');
  });
});

describe('delivery and what became of the message (notes §2, §2.2, §8)', () => {
  const sent = (over) => msg('c-1', 'user', Date.now() - 20000, { text: 'ping', ...over });

  it('pending: "Sent when the Overwatch is back / resumes / is turned on", with Remove', async () => {
    const say = (o) => {
      setNow([], [sent({ delivery: 'pending' })], o);
      const r = render(ChatColumn);
      const t = text(r.container.querySelector('[data-entry="c-1"] .mark'));
      r.unmount();
      return t;
    };
    expect(say(OW('down'))).toBe('Sent when the Overwatch is back · Remove');
    expect(say(OW('paused'))).toBe('Sent when the Overwatch resumes · Remove');
    expect(say(OW('off'))).toBe('Sent when the Overwatch is turned on · Remove');
  });

  it('Remove asks the core and drops the entry', async () => {
    replies.chat_discard = { ok: true };
    setNow([], [sent({ delivery: 'pending' })], OW('down'));
    const { container: c } = render(ChatColumn);
    await fireEvent.click(c.querySelector('[data-entry="c-1"] .mark button'));
    await settle();
    expect(calls).toContainEqual(['chat_discard', { entry: 'c-1' }]);
    expect(c.querySelector('[data-entry="c-1"]')).toBeNull();
  });

  it('waking…: until the status leaves waking, then nothing', async () => {
    setNow([], [sent({ delivery: 'waking' })], OW('waking', { events: 1 }));
    const { container: c } = render(ChatColumn);
    expect(text(c.querySelector('[data-entry="c-1"] .mark'))).toBe('waking…');
    app.now.overwatch = OW('working', { events: 1 });
    await settle();
    expect(c.querySelector('[data-entry="c-1"] .mark')).toBeNull();
  });

  it('delivered while working: no marker, and a muted "working on it" row that counts in 5 s steps', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'] });
    setNow([], [sent({ delivery: 'delivered', at_ms: Date.now() - 12000 })], OW('working', { events: 1 }));
    const { container: c } = render(ChatColumn);
    expect(c.querySelector('[data-entry="c-1"] .mark')).toBeNull();
    expect(text(c.querySelector('[data-working]'))).toBe('Overwatch is working on it · 10 s');
    await vi.advanceTimersByTimeAsync(5000);
    expect(text(c.querySelector('[data-working]'))).toBe('Overwatch is working on it · 15 s');
    // Its reply removes the row.
    app.now.chat.entries['c-2'] = msg('c-2', 'overwatch', Date.now(), { reply_to: 'c-1' });
    await vi.advanceTimersByTimeAsync(0);
    expect(c.querySelector('[data-working]')).toBeNull();
  });

  it('no reply: the turn ended and nothing came back, with a link to its session', async () => {
    const at = Date.now() - 4 * MIN;
    setNow([], [sent({ delivery: 'delivered', at_ms: at })], OW('asleep', { since_ms: at + MIN }));
    const { container: c } = render(ChatColumn);
    const note = c.querySelector('[data-entry="c-1"] [data-noreply]');
    expect(text(note)).toBe('No reply. Ask again, or look at its session.');
    await fireEvent.click(note.querySelector('button'));
    expect(Object.values(app.panes).some((p) => p.type === 'agent' && p.agentId === 'overwatch')).toBe(true);
    // Without a session today the link is plain.
    cleanup();
    setNow([], [sent({ delivery: 'delivered', at_ms: at })], OW('asleep', { since_ms: at + MIN, has_session: false }));
    const r = render(ChatColumn).container;
    expect(text(r.querySelector('[data-noreply]'))).toBe('No reply. Ask again, or look at its session.');
    expect(r.querySelector('[data-noreply] button')).toBeNull();
  });

  it('no "No reply" while it is simply asleep from before the message', () => {
    const at = Date.now() - MIN;
    setNow([], [sent({ delivery: 'delivered', at_ms: at })], OW('asleep', { since_ms: at - 10 * MIN }));
    const { container: c } = render(ChatColumn);
    expect(c.querySelector('[data-noreply]')).toBeNull();
  });
});

describe('deciding (notes §3, F1, F13)', () => {
  const T0 = Date.now();
  /** @type {any} */
  const FAKE = { toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'], now: T0 };
  const deciding = (d, secs = 42) => ({ ...d, waits_on: 'overwatch', deciding_until_ms: T0 + secs * 1000 });

  it('shows "Overwatch is deciding · 42 s left", ticking each second; the actions stay live', async () => {
    vi.useFakeTimers(FAKE);
    const [perm] = kinds(T0);
    setNow([deciding(perm)], [entry('c-1', perm)], OW('working', { events: 1 }));
    const { container: c } = render(ChatColumn);
    const mark = c.querySelector('[data-deciding]');
    expect(text(mark)).toBe('Overwatch is deciding · 42 s left');
    expect(mark.getAttribute('aria-label')).toBe('Overwatch is deciding, up to 1 minute');
    expect(c.querySelector('.card .lc.open')).toBeNull();
    await vi.advanceTimersByTimeAsync(3000);
    expect(text(c.querySelector('[data-deciding]'))).toBe('Overwatch is deciding · 39 s left');
    const allow = [...card(c, 'perm-1').querySelectorAll('.actions button')].find((b) =>
      b.textContent.startsWith('Allow once'),
    );
    await fireEvent.click(allow);
    expect(app.pending.map((p) => p.item)).toEqual(['perm-1']);
  });

  it('hands over at the deadline: "Waiting for you · just now", announced, in the same slot', async () => {
    vi.useFakeTimers(FAKE);
    const [perm] = kinds(T0);
    setNow([deciding(perm, 2)], [entry('c-1', perm)], OW('working', { events: 1 }));
    const { container: c } = render(ChatColumn);
    expect(c.querySelector('[data-deciding]')).toBeTruthy();
    await vi.advanceTimersByTimeAsync(2100);
    expect(c.querySelector('[data-deciding]')).toBeNull();
    expect(text(card(c, 'perm-1').querySelector('.lc'))).toMatch(/Waiting for you · just now$/);
    expect(live(c)).toBe('Waiting for you: Bash · pnpm add zod');
  });

  it('the core handing over early (waits_on user) does the same, with its annotation shown', async () => {
    vi.useFakeTimers(FAKE);
    const [perm] = kinds(T0);
    setNow([deciding(perm)], [entry('c-1', perm)], OW('working', { events: 1 }));
    const { container: c } = render(ChatColumn);
    app.now.decisions['perm-1'] = {
      ...perm,
      waits_on: 'user',
      annotation: { note: 'Not in the allow list; I would deny.', recommend: 'reject', at_ms: T0 },
    };
    await vi.advanceTimersByTimeAsync(0);
    expect(c.querySelector('[data-deciding]')).toBeNull();
    expect(text(card(c, 'perm-1').querySelector('.lc'))).toMatch(/Waiting for you · just now$/);
    expect(card(c, 'perm-1').querySelector('[data-annotation]')).toBeTruthy();
  });

  it('is never folded and does not count toward the 5; the fold counts only the rest', () => {
    const all = kinds(T0);
    const open = all.slice(0, 6).map((d, i) => (i === 5 ? deciding(d) : d));
    setNow(
      open,
      open.map((d, i) => entry(`c-${i}`, d, { at_ms: T0 - (20 - i) * MIN })),
    );
    const { container: c } = render(ChatColumn);
    // Five counted cards + the deciding one shown; nothing folded.
    expect(c.querySelectorAll('.feed > .stop .card')).toHaveLength(6);
    expect(c.querySelector('.fold')).toBeNull();
    cleanup();
    const seven = all.slice(0, 7).map((d, i) => (i === 6 ? deciding(d) : d));
    setNow(
      seven,
      seven.map((d, i) => entry(`c-${i}`, d, { at_ms: T0 - (20 - i) * MIN })),
    );
    const r = render(ChatColumn).container;
    expect(text(r.querySelector('.fold'))).toContain('1 more open');
    expect(card(r, seven[6].id)).toBeTruthy();
  });

  it('thread(): deciding is flagged and the hand-over is the next due time', () => {
    const [perm, q] = kinds(T0);
    const t = thread({
      chat: chatOf([entry('c-1', perm), entry('c-2', q)]),
      decisions: { [perm.id]: deciding(perm), [q.id]: q },
      now: T0,
    });
    expect(t.cards.map((c) => [c.id, !!c.deciding])).toEqual([
      ['perm-1', true],
      ['q-2', false],
    ]);
    expect(t.nextDue).toBe(T0 + 42000);
  });

  it('⌘⇧I and the first stop skip a deciding card unless nothing else is open', async () => {
    vi.useFakeTimers(FAKE);
    const [perm, q] = kinds(T0);
    setNow([deciding(perm), q], [entry('c-1', perm), entry('c-2', q)], OW('working'));
    const { container: c } = render(ChatColumn);
    app.chatFocus = { id: null };
    await vi.advanceTimersByTimeAsync(0);
    expect(c.querySelector('article.stop.sel .card').getAttribute('data-decision')).toBe('q-2');
    cleanup();
    setNow([deciding(perm)], [entry('c-1', perm)], OW('working'));
    const r = render(ChatColumn).container;
    app.chatFocus = { id: null };
    await vi.advanceTimersByTimeAsync(0);
    expect(r.querySelector('article.stop.sel .card').getAttribute('data-decision')).toBe('perm-1');
  });

  it("is not announced as a new card while it deciding; the badge-side count is the core's", async () => {
    vi.useFakeTimers(FAKE);
    const [perm] = kinds(T0);
    setNow([], [], OW('working'));
    const { container: c } = render(ChatColumn);
    app.now.decisions['perm-1'] = deciding(perm);
    app.now.chat.entries['c-1'] = entry('c-1', perm);
    await vi.advanceTimersByTimeAsync(0);
    expect(live(c)).toBe('');
  });

  it('isDeciding / decidingText are pure on the clock', () => {
    const d = { waits_on: 'overwatch', deciding_until_ms: 100000 };
    expect(ow.isDeciding(d, 99999)).toBe(true);
    expect(ow.isDeciding(d, 100000)).toBe(false);
    expect(ow.isDeciding({ ...d, waits_on: 'user' }, 0)).toBe(false);
    expect(ow.isDeciding({ waits_on: 'overwatch' }, 0)).toBe(false);
    expect(ow.decidingText(d, 58000)).toBe('Overwatch is deciding · 42 s left');
    expect(ow.decidingText(d, 99001)).toBe('Overwatch is deciding · 1 s left');
  });
});

describe('the annotation (notes §4)', () => {
  const NOW = Date.now();
  const noted = (over = {}, d = kinds(NOW)[0]) => ({
    ...d,
    annotation: {
      note: 'Not in the allow list; nothing in the docs says registry.internal is used.',
      recommend: 'reject',
      sources: [],
      at_ms: NOW,
      ...over,
    },
  });
  const btn = (c, label) =>
    [...card(c, 'perm-1').querySelectorAll('.actions button')].find((b) => b.textContent.startsWith(label));

  it('a block above the actions: "Overwatch · recommends deny", the note, the recommended action highlighted', () => {
    const d = noted();
    setNow([d], [entry('c-1', d)]);
    const { container: c } = render(ChatColumn);
    const block = card(c, 'perm-1').querySelector('[data-annotation]');
    expect(text(block.querySelector('.nh'))).toBe('Overwatch · recommends deny');
    expect(text(block.querySelector('.nt'))).toBe(
      'Not in the allow list; nothing in the docs says registry.internal is used.',
    );
    // Above the actions.
    expect(
      block.compareDocumentPosition(card(c, 'perm-1').querySelector('.actions')) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(btn(c, 'Deny').classList.contains('rec')).toBe(true);
    expect(btn(c, 'Deny').textContent).toContain('recommended');
    expect(btn(c, 'Deny').getAttribute('aria-label')).toBe('Deny, recommended by the Overwatch');
    expect(card(c, 'perm-1').querySelectorAll('.rec')).toHaveLength(1);
  });

  it("never pre-selected: the primary key, the focus and ⏎ stay the card's own", async () => {
    const d = noted();
    setNow([d], [entry('c-1', d)]);
    const { container: c } = render(ChatColumn);
    // `a` is still Allow once even though the Overwatch recommends deny.
    expect(btn(c, 'Allow once').classList.contains('primary')).toBe(true);
    expect(btn(c, 'Deny').classList.contains('primary')).toBe(false);
    expect(card(c, 'perm-1').contains(document.activeElement)).toBe(false);
    await fireEvent.keyDown(feed(c), { key: 'a' });
    expect(app.pending.map((p) => [p.item, p.action])).toEqual([['perm-1', 'approve']]);
  });

  it('a keyless action can be the recommendation (Always allow)', () => {
    const d = noted({ recommend: 'always', note: 'Routine for this project.' });
    setNow([d], [entry('c-1', d)]);
    const { container: c } = render(ChatColumn);
    expect(text(c.querySelector('[data-annotation] .nh'))).toBe('Overwatch · recommends always allow');
    expect(btn(c, 'Always allow').classList.contains('rec')).toBe(true);
  });

  it('a recommendation the card does not have, or none: the note shows and nothing is highlighted', () => {
    for (const rec of ['approve_everything', undefined]) {
      const d = noted({ recommend: rec });
      setNow([d], [entry('c-1', d)]);
      const r = render(ChatColumn);
      expect(text(r.container.querySelector('[data-annotation] .nh'))).toBe('Overwatch');
      expect(r.container.querySelector('.rec')).toBeNull();
      r.unmount();
    }
  });

  it('sources are links (up to 3, "+ 2 more" expands): a doc as file § and a chat entry as "your answer"', async () => {
    app.data.features = [{ id: 'f1', slug: 'checkout', project_id: 'shop-api' }];
    const d = noted({
      sources: [
        '.orchestrator/features/checkout/allow-list.md#2',
        'chat:c-9',
        'agent:back-1',
        'run:run-12',
        'docs/design/p4-4-design-notes.md#3',
      ],
    });
    setNow([d], [entry('c-1', d), msg('c-9', 'user', NOW - MIN, { delivery: 'delivered' })]);
    app.now.agents['back-1'] = { agent: 'back-1' };
    const { container: c } = render(ChatColumn);
    const srcs = () => [...card(c, 'perm-1').querySelectorAll('.srcs > *:not([aria-hidden])')].map(text);
    expect(srcs()).toEqual([
      'allow-list.md §2 ↗',
      expect.stringMatching(/^your message \d\d:\d\d ↗$/),
      'back-1 ↗',
      '+ 2 more',
    ]);
    await fireEvent.click(card(c, 'perm-1').querySelector('[data-source]'));
    expect(Object.values(app.panes).find((p) => p.type === 'feature')).toMatchObject({ doc: 'allow-list.md' });
    await fireEvent.click(
      [...card(c, 'perm-1').querySelectorAll('.srcs .link')].find((b) => b.textContent === '+ 2 more'),
    );
    expect(srcs()).toHaveLength(5);
  });

  it('a long note clamps to 3 lines with Show more', async () => {
    const d = noted({ note: 'x '.repeat(130).trim() });
    setNow([d], [entry('c-1', d)]);
    const { container: c } = render(ChatColumn);
    const nt = c.querySelector('[data-annotation] .nt');
    expect(nt.classList.contains('clamp')).toBe(true);
    await fireEvent.click(c.querySelector('[data-annotation] button.link'));
    expect(nt.classList.contains('clamp')).toBe(false);
  });

  it('a replaced note is announced; an expanded line keeps the note read-only, with no highlight', async () => {
    const d = noted();
    setNow([d], [entry('c-1', d)]);
    const { container: c } = render(ChatColumn);
    app.now.decisions['perm-1'] = {
      ...d,
      annotation: { ...d.annotation, note: 'Changed my mind.', at_ms: NOW + 1000 },
    };
    await settle();
    expect(live(c)).toBe('The Overwatch updated its note');
    cleanup();
    const closed = noted();
    setNow(
      [],
      [closedEntry('c-1', closed, { verb: 'denied', agent: 'back-1', what: 'pnpm add zod', by: 'you', at_ms: NOW })],
    );
    const r = render(ChatColumn).container;
    await fireEvent.click(r.querySelector('article.line button'));
    const ro = r.querySelector('[data-annotation]');
    expect(ro.classList.contains('ro')).toBe(true);
    expect(r.querySelector('.rec')).toBeNull();
  });

  it('refKind and sourceLabel are pure', () => {
    expect(ow.refKind('api-contract.md#3')).toEqual({
      kind: 'doc',
      path: 'api-contract.md',
      name: 'api-contract.md',
      section: '3',
    });
    expect(ow.refKind('chat:c-790')).toEqual({ kind: 'chat', id: 'c-790' });
    expect(ow.refKind('https://example.com')).toBeNull();
    expect(ow.refKind('javascript:alert(1)')).toBeNull();
    expect(ow.sourceLabel('docs/x/spec.md#scope')).toBe('spec.md#scope');
    expect(ow.sourceLabel('chat:nope', {})).toBeNull();
    expect(ow.featureDoc('.orchestrator/features/checkout/spec.md')).toEqual({ slug: 'checkout', name: 'spec.md' });
    expect(ow.featureDoc('docs/spec.md')).toBeNull();
  });
});

describe('Settings → Overwatch (notes §9.1)', () => {
  const SETTINGS = {
    enabled: true,
    model: 'sonnet',
    batch_secs: 30,
    permission_secs: 60,
    idle_secs: 120,
    rule_threshold: 3,
  };
  const open = async () => {
    replies.overwatch_settings = { ...SETTINGS };
    replies.set_overwatch_settings = ({ settings }) => ({ ...SETTINGS, ...settings });
    app.panes = { p1: { type: 'settings', section: 'overwatch' } };
    const r = render(SettingsPane, { paneId: 'p1' });
    await settle();
    return r.container;
  };
  const row = (c, key) => c.querySelector(`[data-row="${key}"]`);

  it('is a section after Integrations, with the six rows and their defaults', async () => {
    const c = await open();
    const nav = [...c.querySelectorAll('.sec')].map(text);
    expect(nav.indexOf('Overwatch')).toBe(nav.indexOf('Integrations') + 1);
    expect(row(c, 'enabled').querySelector('input').checked).toBe(true);
    expect(row(c, 'model').querySelector('select').value).toBe('sonnet');
    expect([...row(c, 'model').querySelectorAll('option')].map(text)).toEqual(['Haiku', 'Sonnet', 'Opus']);
    for (const [k, v] of [
      ['batch_secs', 30],
      ['permission_secs', 60],
      ['idle_secs', 120],
      ['rule_threshold', 3],
    ])
      expect(row(c, k).querySelector('input').value).toBe(String(v));
    expect(text(row(c, 'rule_threshold'))).toContain('Proposes a rule after you give the same answer this many times.');
  });

  it('saves a changed value as a patch', async () => {
    const c = await open();
    const input = row(c, 'idle_secs').querySelector('input');
    await fireEvent.change(input, { target: { value: '300' } });
    await settle();
    expect(calls).toContainEqual(['set_overwatch_settings', { settings: { idle_secs: 300 } }]);
    await fireEvent.change(row(c, 'model').querySelector('select'), { target: { value: 'opus' } });
    await settle();
    expect(calls).toContainEqual(['set_overwatch_settings', { settings: { model: 'opus' } }]);
  });

  it('an out-of-range value shows its message and is not sent; the old value stays', async () => {
    const c = await open();
    await fireEvent.change(row(c, 'batch_secs').querySelector('input'), { target: { value: '2' } });
    await settle();
    expect(text(row(c, 'batch_secs').querySelector('.err'))).toBe('5–300 s. Still using 30 s.');
    expect(row(c, 'batch_secs').querySelector('input').getAttribute('aria-invalid')).toBe('true');
    await fireEvent.change(row(c, 'rule_threshold').querySelector('input'), { target: { value: '11' } });
    await fireEvent.change(row(c, 'permission_secs').querySelector('input'), { target: { value: '300' } });
    await settle();
    expect(text(row(c, 'permission_secs').querySelector('.err'))).toBe(
      'Must be below the hold length (300 s). Still using 60 s.',
    );
    expect(calls.filter(([k]) => k === 'set_overwatch_settings')).toEqual([]);
  });

  it('the core refusing a value shows its reason under the field', async () => {
    const c = await open();
    replies.set_overwatch_settings = new Error('permission_secs must be between 1 and the hold (200 s), exclusive');
    await fireEvent.change(row(c, 'permission_secs').querySelector('input'), { target: { value: '250' } });
    await settle();
    expect(text(row(c, 'permission_secs').querySelector('.err'))).toBe(
      'permission_secs must be between 1 and the hold (200 s), exclusive Still using 60 s.',
    );
  });

  it('the switch says what turning it off does', async () => {
    const c = await open();
    await fireEvent.click(row(c, 'enabled').querySelector('input'));
    await settle();
    expect(calls).toContainEqual(['set_overwatch_settings', { settings: { enabled: false } }]);
    expect(text(row(c, 'enabled').querySelector('[data-note]'))).toBe(
      'Off. Open permissions come to you. Its session ends now.',
    );
    await fireEvent.click(row(c, 'enabled').querySelector('input'));
    await settle();
    expect(text(row(c, 'enabled').querySelector('[data-note]'))).toBe('On. It wakes on the next event.');
  });

  it('down or paused shows in the section header, with Restart while down', async () => {
    app.now.overwatch = OW('down', { why: 'login expired' });
    const c = await open();
    expect(text(c.querySelector('[data-ow-state]'))).toBe('Overwatch · down: login expired');
    replies.overwatch_restart = { ok: true };
    await fireEvent.click(c.querySelector('.h .btn'));
    expect(calls).toContainEqual(['overwatch_restart', undefined]);
    app.now.overwatch = OW('paused', { why: 'Pause all' });
    await settle();
    expect(text(c.querySelector('[data-ow-state]'))).toBe('Overwatch · paused: Pause all');
    expect(c.querySelector('.h .btn')).toBeNull();
  });
});

describe('pure helpers', () => {
  it('statusText, stripText and deliveryText', () => {
    expect(ow.statusText(OW('working', { events: 0 }))).toBe('working');
    expect(ow.stripText(OW('asleep'))).toBeNull();
    expect(ow.stripText(OW('down'))).toBe('Overwatch is down · cards still work');
    expect(ow.deliveryText({ delivery: 'delivered' }, OW('down'))).toBe('');
    expect(ow.deliveryText({ delivery: 'pending' }, OW('asleep'))).toBe('waking…');
  });

  it('awaiting / noReply / workingText', () => {
    const u = { id: 'u', author: 'user', at_ms: 1000, delivery: 'delivered' };
    expect(ow.awaiting([u])).toBe(u);
    expect(ow.awaiting([u, { id: 'o', author: 'overwatch', at_ms: 2000, reply_to: 'u' }])).toBeNull();
    expect(ow.awaiting([u, { id: 'o', author: 'overwatch', at_ms: 2000, reply_to: 'other' }])).toBe(u);
    expect(ow.awaiting([{ ...u, delivery: 'pending' }])).toBeNull();
    expect(ow.noReply(u, { status: 'asleep', since_ms: 3000 })).toBe(true);
    expect(ow.noReply(u, { status: 'asleep', since_ms: 500 })).toBe(false);
    expect(ow.noReply(u, { status: 'working', since_ms: 3000 })).toBe(false);
    expect(ow.workingText(u, 1000 + 12000)).toBe('Overwatch is working on it · 10 s');
    expect(ow.workingText(u, 1000 + 130000)).toBe('Overwatch is working on it · 2 min');
  });

  it('statusChange: down, and back after down; nothing for the rest', () => {
    expect(ow.statusChange(OW('working'), OW('down', { why: 'login expired' }))).toBe(
      'Overwatch is down: login expired. Cards still work.',
    );
    expect(ow.statusChange(OW('down'), OW('waking'))).toBe('Overwatch is back.');
    expect(ow.statusChange(OW('asleep'), OW('working'))).toBeNull();
    expect(ow.statusChange(OW('paused'), OW('asleep'))).toBeNull();
    expect(ow.statusChange(OW('asleep'), OW('asleep'))).toBeNull();
  });
});

describe('review fixes (ui review of T-226)', () => {
  it('an unknown chat_send reads as "the core doesn\'t support messages yet", not the raw error', async () => {
    replies.chat_send = new Error('Command chat_send not found');
    setNow();
    const { container: c } = render(ChatColumn);
    await fireEvent.input(field(c), { target: { value: 'hi' } });
    await fireEvent.keyDown(field(c), { key: 'Enter' });
    await settle();
    expect(text(c.querySelector('article.text.user'))).toContain("Not sent: the core doesn't support messages yet");
  });

  it('the Overwatch session always opens in watch mode, even without Mission control', async () => {
    setNow([], [], OW('working', { events: 1 }));
    const { container: c } = render(ChatColumn);
    await fireEvent.click(status(c));
    const pane = Object.values(app.panes).find((p) => p.type === 'agent' && p.agentId === 'overwatch');
    expect(pane?.watch).toBe(true);
  });

  it('"back" only from down to asleep, waking or working', () => {
    expect(ow.statusChange(OW('down'), OW('paused'))).toBeNull();
    expect(ow.statusChange(OW('down'), OW('off'))).toBeNull();
    expect(ow.statusChange(OW('down'), OW('working'))).toBe('Overwatch is back.');
  });

  it('the ring and its name follow the real window (permission_secs)', () => {
    const d = { created_ms: 1000, deciding_until_ms: 1000 + 120000 };
    expect(ow.decidingShare(d, 1000 + 60000)).toBe(0.5);
    expect(ow.decidingName(d)).toBe('Overwatch is deciding, up to 2 minutes');
    expect(ow.decidingName({ created_ms: 0, deciding_until_ms: 90000 })).toBe('Overwatch is deciding, up to 90 s');
  });

  it('the why-asked line: core and policy reasons, or the Overwatch passing it on', () => {
    const [perm] = kinds();
    const with_ = (a) => {
      setNow([{ ...perm, asks_because: a }], [entry('c-1', perm)]);
      const r = render(ChatColumn);
      const t = r.container.querySelector('[data-asks]');
      const out = t ? text(t) : null;
      r.unmount();
      return out;
    };
    expect(with_({ kind: 'core', detail: 'network host outside the allow-list' })).toBe(
      'Only you can allow this: network host outside the allow-list.',
    );
    expect(with_({ kind: 'policy', detail: 'policy installs-ask' })).toBe(
      'Only you can allow this: policy installs-ask.',
    );
    expect(with_({ kind: 'overwatch_unsure', detail: '' })).toBe('The Overwatch did not decide this.');
    expect(with_({ kind: 'default', detail: '' })).toBeNull();
  });

  it('a card the Overwatch settles under the pointer stays, saying who settled it; it collapses on leave', async () => {
    const [perm] = kinds();
    setNow([perm], [entry('c-1', perm)]);
    const { container: c } = render(ChatColumn);
    await fireEvent.mouseEnter(card(c, 'perm-1').parentElement);
    const at = Date.now();
    app.now.decisions['perm-1'] = {
      ...perm,
      lifecycle: { state: 'effect', effect: 'allowed', decided_by: 'overwatch', effect_at_ms: at },
    };
    app.now.chat.entries['c-1'] = closedEntry(
      'c-1',
      perm,
      { verb: 'allowed', agent: 'back-1', what: 'pnpm add zod', by: 'overwatch', at_ms: at },
      { rev: 3 },
    );
    await settle();
    expect(text(card(c, 'perm-1').querySelector('.lc'))).toMatch(/Allowed by the Overwatch · \d\d:\d\d$/);
    await fireEvent.mouseLeave(card(c, 'perm-1').parentElement);
    await settle();
    expect(card(c, 'perm-1')).toBeNull();
    expect(c.querySelector('article.line')).toBeTruthy();
  });

  it('a reply while scrolled up shows "New from the Overwatch ↓" and does not scroll; at the foot it stays', async () => {
    setNow([], [msg('c-1', 'user', Date.now() - 1000, { delivery: 'delivered' })], OW('working', { events: 1 }));
    const { container: c } = render(ChatColumn);
    const f = feed(c);
    // Scrolled up: not at the foot.
    Object.defineProperty(f, 'scrollHeight', { configurable: true, value: 1000 });
    Object.defineProperty(f, 'clientHeight', { configurable: true, value: 300 });
    f.scrollTop = 100;
    await fireEvent.scroll(f);
    app.now.chat.entries['c-2'] = msg('c-2', 'overwatch', Date.now(), { reply_to: 'c-1' });
    await settle();
    expect(text(c.querySelector('[data-new-reply]'))).toBe('New from the Overwatch ↓');
    expect(f.scrollTop).toBe(100);
    await fireEvent.click(c.querySelector('[data-new-reply]'));
    await settle();
    expect(c.querySelector('[data-new-reply]')).toBeNull();
  });

  it('a search hit on a text entry scrolls to it', async () => {
    replies.chat_search = [
      { day: app.now.chat?.day ?? DAY, entry: msg('c-1', 'overwatch', Date.now(), { text: 'checkout waited' }) },
    ];
    setNow([], [msg('c-1', 'overwatch', Date.now(), { text: 'checkout waited' })]);
    app.now.chat.day = DAY;
    replies.chat_search[0].day = DAY;
    const { container: c } = render(ChatColumn);
    run('decisions.search');
    await settle();
    await fireEvent.input(c.querySelector('.search input'), { target: { value: 'checkout' } });
    await new Promise((r) => setTimeout(r, 260));
    await fireEvent.click(c.querySelector('[data-result]'));
    await settle();
    expect(c.querySelector('[data-entry="c-1"]').classList.contains('pulse')).toBe(true);
  });

  it('the Permission time follows the real hold: strictly below it, with "Still using 60 s."', async () => {
    replies.overwatch_settings = {
      enabled: true,
      model: 'sonnet',
      batch_secs: 30,
      permission_secs: 60,
      idle_secs: 120,
      rule_threshold: 3,
    };
    replies.settings_get = ({ key }) => (key === 'permissions.hold_secs' ? 120 : null);
    app.panes = { p1: { type: 'settings', section: 'overwatch' } };
    const { container: c } = render(SettingsPane, { paneId: 'p1' });
    await settle();
    const input = /** @type {HTMLInputElement} */ (c.querySelector('[data-row="permission_secs"] input'));
    await fireEvent.change(input, { target: { value: '120' } });
    await settle();
    expect(text(c.querySelector('[data-row="permission_secs"] .err'))).toBe(
      'Must be below the hold length (120 s). Still using 60 s.',
    );
    expect(input.value).toBe('60');
  });
});
