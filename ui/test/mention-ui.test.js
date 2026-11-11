// @mention in the Overwatch input and thread (add-mention-in-overwatch spec §3.2, §3.3, §5):
// the popup (trigger, rows, keys, insertion, ARIA, live region), what a send carries, the
// mention link in a sent entry, the removed feature, an unsent retry. Against a mocked core.

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { chatOf } from './chat-fixture.js';
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

const { app } = await import('../src/lib/app.svelte.js');
const IS = await import('../src/lib/intake.svelte.js');
const { default: ChatColumn } = await import('../src/components/chat/ChatColumn.svelte');
const { default: ChatText } = await import('../src/components/chat/ChatText.svelte');

const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const settle = () => new Promise((r) => setTimeout(r, 0));
const F = (id, slug, over = {}) => ({
  id,
  project_id: 'shop-api',
  slug,
  title: `Title of ${slug}`,
  branch: `orch/${slug}/integration`,
  archived: false,
  run: null,
  tasks: { done: 0, total: 0 },
  ...over,
});
const run = (status) => ({ id: `r-${status}`, status });
const FEATURES = [
  F('f1', 'add-mention-in-overwatch', { title: 'Add Mention In Overwatch', run: run('executing'), created_at_ms: 3 }),
  F('f2', 'refunds', { run: run('spec_gate'), created_at_ms: 2 }),
  F('f3', 'old-thing', { run: run('done'), created_at_ms: 9 }),
  F('f4', 'rss-feed', { project_id: 'blog', created_at_ms: 1 }),
];

let c;
let ta;
const sends = () => calls.filter(([k]) => k === 'chat_send').map(([, a]) => a);
const popup = () => c.querySelector('[data-mention-popup]');
const rowSlugs = () => [...c.querySelectorAll('[role="option"]')].map((r) => r.querySelector('.slug').textContent);
const live = () => c.querySelector('[aria-live="polite"]').textContent;
async function type(value, caret = value.length) {
  ta.value = value;
  ta.setSelectionRange(caret, caret);
  await fireEvent.input(ta);
}
const key = (k, o = {}) => fireEvent.keyDown(ta, { key: k, ...o });
async function mount(
  features = FEATURES,
  entries = [],
  overwatch = { status: 'asleep', events: 0, since_ms: Date.now(), agent: 'overwatch', has_session: true },
) {
  app.data = { ...fixture(), features };
  app.now = { ...nowOf(app.data), chat: chatOf(entries), overwatch };
  const r = render(ChatColumn);
  await settle();
  c = r.container;
  ta = c.querySelector('textarea[aria-label="Message the Overwatch"]');
}

beforeEach(() => {
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  replies.chat_send = (a) => ({
    entry: { id: 'c-u1', day: '2026-11-02', at_ms: Date.now(), author: 'user', kind: 'text', text: a.text, rev: 1 },
  });
  app.pending = [];
  app.toasts = [];
  app.panes = {};
  app.focused = null;
  app.layout = null;
  app.chatFocus = null;
  app.chatFilter = 'all';
  app.chatDraft = '';
  app.chatMentions = [];
  app.width = 1512;
  app.overlay = null;
  app.workspace = { project: 'all' };
  IS.intakeUi.plan = null;
});
afterEach(() => cleanup());

describe('the popup', () => {
  it('opens on @ with the current group, then the others; headers when both; combobox attributes', async () => {
    await mount();
    expect(popup()).toBeNull();
    expect(ta.getAttribute('aria-expanded')).toBe('false');
    expect(ta.getAttribute('aria-controls')).toBeNull();
    await type('@');
    expect(popup()).not.toBeNull();
    expect(rowSlugs()).toEqual(['@add-mention-in-overwatch', '@refunds', '@old-thing', '@rss-feed']);
    expect([...c.querySelectorAll('[data-mention-head]')].map(text)).toEqual(['Current', 'Other features']);
    expect(ta.getAttribute('role')).toBe('combobox');
    expect(ta.getAttribute('aria-autocomplete')).toBe('list');
    expect(ta.getAttribute('aria-expanded')).toBe('true');
    expect(ta.getAttribute('aria-controls')).toBe('ow-mention-list');
    const list = c.querySelector('#ow-mention-list');
    expect(list.getAttribute('role')).toBe('listbox');
    const first = c.querySelector('[role="option"]');
    expect(first.getAttribute('aria-selected')).toBe('true');
    expect(ta.getAttribute('aria-activedescendant')).toBe(first.id);
    expect(first.getAttribute('aria-label')).toBe(
      'add-mention-in-overwatch, Add Mention In Overwatch, shop-api, Executing',
    );
    expect(c.querySelectorAll('[role="option"]')[3].getAttribute('aria-label')).toContain('No run');
    expect(text(popup())).toContain('↑↓ to choose · ⏎ to insert · esc to close');
  });

  it('no headers when a single group has rows', async () => {
    await mount([F('a', 'alpha', { run: run('executing') }), F('b', 'beta', { run: run('paused') })]);
    await type('@');
    expect(c.querySelectorAll('[data-mention-head]').length).toBe(0);
  });

  it('filters by the query and emphasises the matched chars', async () => {
    await mount();
    await type('@ment');
    expect(rowSlugs()).toEqual(['@add-mention-in-overwatch']);
    expect([...c.querySelectorAll('.slug b')].map((b) => b.textContent)).toEqual(['ment']);
    expect(c.querySelectorAll('[data-mention-head]').length).toBe(0);
  });

  it('does not open in a word, in code, or from a caret move; closes on a space, on blur', async () => {
    await mount();
    await type('me@x');
    expect(popup()).toBeNull();
    await type('`a @ad');
    expect(popup()).toBeNull();
    await type('hi @ad');
    expect(popup()).not.toBeNull();
    await type('hi @ad ');
    expect(popup()).toBeNull();
    await type('hi @ad');
    expect(popup()).not.toBeNull();
    await fireEvent.blur(ta);
    expect(popup()).toBeNull();
    await type('hello @rss');
    ta.setSelectionRange(0, 0);
    await fireEvent.keyUp(ta, { key: 'ArrowLeft' });
    expect(popup()).toBeNull(); // the caret left the token
    ta.setSelectionRange(10, 10);
    await fireEvent.click(ta);
    expect(popup()).toBeNull(); // a click never opens it
  });

  it('the live region counts, once with the help, on each query change', async () => {
    await mount();
    await type('@');
    await new Promise((r) => setTimeout(r, 300));
    expect(live()).toBe('4 features. Use up and down arrows to choose, enter to insert, escape to close');
    await type('@ment');
    await new Promise((r) => setTimeout(r, 300));
    expect(live()).toBe('1 feature');
    await type('@zzzz');
    await new Promise((r) => setTimeout(r, 300));
    expect(live()).toBe('No feature matches');
  });
});

describe('keys', () => {
  it('↓ ↑ ⌃N ⌃P move without wrapping; ⏎ inserts @slug + space and does not send', async () => {
    await mount();
    await type('@');
    const on = () => c.querySelector('[role="option"][aria-selected="true"] .slug').textContent;
    await key('ArrowDown');
    expect(on()).toBe('@refunds');
    await key('n', { ctrlKey: true });
    await key('n', { ctrlKey: true });
    await key('ArrowDown');
    await key('ArrowDown');
    expect(on()).toBe('@rss-feed'); // stops at the last
    await key('p', { ctrlKey: true });
    expect(on()).toBe('@old-thing');
    await key('ArrowUp');
    await key('ArrowUp');
    await key('ArrowUp');
    expect(on()).toBe('@add-mention-in-overwatch');
    await key('ArrowDown');
    await key('Enter');
    await settle();
    expect(app.chatDraft).toBe('@refunds ');
    expect(ta.selectionStart).toBe(9);
    expect(popup()).toBeNull();
    expect(sends()).toEqual([]);
    expect(app.chatMentions).toEqual([{ token: '@refunds', kind: 'feature', id: 'f2' }]);
  });

  it('⇥ picks too, mid-text, keeping the text after and not doubling the space', async () => {
    await mount();
    await type('see @ad please', 7);
    await key('Tab');
    await settle();
    expect(app.chatDraft).toBe('see @add-mention-in-overwatch please');
    expect(ta.selectionStart).toBe(30);
  });

  it('a click picks that row, the field keeps its focus (mousedown is prevented)', async () => {
    await mount();
    await type('@');
    expect(await fireEvent.mouseDown(popup())).toBe(false); // preventDefault called
    await fireEvent.mouseMove(c.querySelectorAll('[role="option"]')[1]);
    expect(c.querySelectorAll('[role="option"]')[1].getAttribute('aria-selected')).toBe('true');
    await fireEvent.click(c.querySelectorAll('[role="option"]')[1]);
    await settle();
    expect(app.chatDraft).toBe('@refunds ');
  });

  it('Esc closes the popup only and keeps the text; the next edit of the token reopens it; a second Esc leaves', async () => {
    await mount();
    await type('@re');
    await key('Escape');
    expect(popup()).toBeNull();
    expect(app.chatDraft).toBe('@re');
    expect(document.activeElement).not.toBe(c.querySelector('[data-chat-focus]'));
    await type('@ref');
    expect(popup()).not.toBeNull();
    await key('Escape');
    await key('Escape'); // popup already closed: leaves to the thread as today
    expect(app.chatDraft).toBe('@ref');
  });

  it('no match: ⏎ and ⇥ close, keep the text, send nothing; an edit reopens; the next ⏎ sends', async () => {
    await mount();
    await type('@xyz');
    expect(text(popup())).toBe('No feature matches “xyz”');
    expect(c.querySelectorAll('[role="option"]').length).toBe(0);
    expect(c.querySelector('[data-mention-foot]')).toBeNull();
    await key('Enter');
    expect(popup()).toBeNull();
    expect(sends()).toEqual([]);
    expect(app.chatDraft).toBe('@xyz');
    await type('@xyzq');
    expect(popup()).not.toBeNull();
    await key('Tab');
    expect(popup()).toBeNull();
    expect(sends()).toEqual([]);
    await key('Enter');
    await settle();
    expect(sends()).toEqual([{ text: '@xyzq' }]);
  });

  it('a long query is cut at 24 characters in the no-match line', async () => {
    await mount();
    await type(`@${'z'.repeat(30)}`);
    expect(text(popup())).toBe(`No feature matches “${'z'.repeat(24)}…”`);
  });

  it('no features at all: the popup opens with "No features yet"; ⏎ closes it', async () => {
    await mount([F('a', 'gone', { archived: true })]);
    await type('@');
    expect(text(popup())).toBe('No features yet');
    await key('Enter');
    expect(popup()).toBeNull();
    expect(sends()).toEqual([]);
  });

  it('⏎ while composing does not pick; ⇧⏎ is a newline and closes the popup', async () => {
    await mount();
    await type('@');
    await key('Enter', { isComposing: true });
    expect(app.chatDraft).toBe('@');
    await type('@\n');
    expect(popup()).toBeNull();
  });
});

describe('what a send carries', () => {
  it('context.mentions beside context.plan; the picks and the drafts are cleared', async () => {
    await mount();
    IS.planWithOverwatch('shop-api');
    await settle();
    await type('what about @re');
    await key('Enter');
    await settle();
    await type(`${app.chatDraft}now?`);
    await key('Enter');
    await settle();
    expect(sends()).toEqual([
      { text: 'what about @refunds now?', context: { plan: 'shop-api', mentions: [{ kind: 'feature', id: 'f2' }] } },
    ]);
    expect(app.chatMentions).toEqual([]);
    expect(app.chatDraft).toBe('');
  });

  it('without a plan: context holds the mentions only; with none, no context', async () => {
    await mount();
    await type('@add');
    await key('Enter');
    await settle();
    await key('Enter');
    await settle();
    expect(sends()[0]).toEqual({
      text: '@add-mention-in-overwatch',
      context: { mentions: [{ kind: 'feature', id: 'f1' }] },
    });
    await type('plain');
    await key('Enter');
    await settle();
    expect(sends()[1]).toEqual({ text: 'plain' });
  });

  it('a deleted pick is dropped; a hand-typed exact unique slug is sent; an ambiguous one is not', async () => {
    await mount([...FEATURES, F('f5', 'twin', { project_id: 'shop-api' }), F('f6', 'twin', { project_id: 'blog' })]);
    await type('@ref');
    await key('Enter');
    await settle();
    await type('then deleted');
    await key('Enter');
    await settle();
    expect(sends()[0]).toEqual({ text: 'then deleted' });
    await type('about @rss-feed and @twin and @add ok');
    await key('Enter');
    await settle();
    expect(sends()[1]).toEqual({
      text: 'about @rss-feed and @twin and @add ok',
      context: { mentions: [{ kind: 'feature', id: 'f4' }] },
    });
  });

  it('a slug shared by two projects inserts @<project key>/<slug>, and the right id is remembered', async () => {
    await mount([F('f5', 'checkout'), F('f6', 'checkout', { project_id: 'blog' })]);
    await type('@che');
    expect(rowSlugs().length).toBe(2);
    await key('Enter');
    await settle();
    expect(app.chatDraft).toBe('@blog/checkout ');
    await key('Enter');
    await settle();
    expect(sends()[0].context).toEqual({ mentions: [{ kind: 'feature', id: 'f6' }] });
  });

  it('a refused send keeps its mentions: the unsent entry renders them, a retry sends them again', async () => {
    await mount();
    replies.chat_send = new Error('busy');
    await type('check @ref');
    await key('Enter');
    await settle();
    await type(`${app.chatDraft}`);
    expect(c.querySelector('[data-entry^="unsent"]')).toBeNull(); // nothing yet: the pick was not made
    await type('@ref');
    await key('Enter');
    await settle();
    await key('Enter');
    await settle();
    const un = c.querySelector('[data-entry^="unsent"]');
    expect(un.querySelector('.mention')?.textContent).toBe('@refunds');
    replies.chat_send = (a) => ({
      entry: { id: 'c-r', day: '2026-11-02', at_ms: Date.now(), author: 'user', kind: 'text', text: a.text, rev: 1 },
    });
    await fireEvent.click(un.querySelector('button.link'));
    await settle();
    expect(sends().at(-1)).toEqual({ text: '@refunds', context: { mentions: [{ kind: 'feature', id: 'f2' }] } });
  });
});

describe('the mention in the thread', () => {
  const m1 = {
    kind: 'feature',
    id: 'f1',
    slug: 'add-mention-in-overwatch',
    title: 'Add Mention In Overwatch',
    project_id: 'shop-api',
    project: 'shop-api',
  };
  const user = (id, t, meta) => ({
    id,
    day: '2026-11-02',
    at_ms: Date.now() - 60000,
    author: 'user',
    kind: 'text',
    text: t,
    rev: 1,
    delivery: 'delivered',
    ...(meta ? { meta } : {}),
  });
  const body = (id) => c.querySelector(`[data-entry="${id}"] .body`);

  it('renders the token as a link, the text around it untouched, opens the feature on click and ⏎', async () => {
    await mount(FEATURES, [user('u1', 'what is\n  @add-mention-in-overwatch. waiting on?', { mentions: [m1] })]);
    const link = body('u1').querySelector('[role="link"]');
    expect(link.textContent).toBe('@add-mention-in-overwatch');
    expect(link.getAttribute('tabindex')).toBe('0');
    expect(link.getAttribute('aria-label')).toBe('Open feature @add-mention-in-overwatch');
    expect(body('u1').textContent).toBe('what is\n  @add-mention-in-overwatch. waiting on?'); // no template whitespace
    await fireEvent.click(link);
    expect(Object.values(app.panes).some((p) => p.type === 'feature' && p.featureId === 'f1')).toBe(true);
    app.panes = {};
    await fireEvent.keyDown(link, { key: 'Enter' });
    expect(Object.values(app.panes).some((p) => p.type === 'feature' && p.featureId === 'f1')).toBe(true);
  });

  it('both token forms link; a token without a mention stays plain', async () => {
    await mount(FEATURES, [
      user('u1', '@shop-api/add-mention-in-overwatch @refunds @add-mention-in-overwatch', { mentions: [m1] }),
    ]);
    expect([...body('u1').querySelectorAll('.mention')].map((x) => x.textContent)).toEqual([
      '@shop-api/add-mention-in-overwatch',
      '@add-mention-in-overwatch',
    ]);
  });

  it('@add never links inside the longer @add-mention-in-overwatch', async () => {
    await mount(FEATURES, [user('u1', '@add-mention-in-overwatch', { mentions: [{ ...m1, id: 'f2', slug: 'add' }] })]);
    expect(body('u1').querySelector('.mention')).toBeNull();
  });

  it('a removed feature: muted token, not a link, hidden "(feature removed)"; only once the data has loaded', async () => {
    await mount(FEATURES, [
      user('u1', 'was @gone-feature ok', { mentions: [{ ...m1, id: 'zz', slug: 'gone-feature' }] }),
    ]);
    const gone = body('u1').querySelector('.mention.gone');
    expect(gone.getAttribute('role')).toBeNull();
    expect(gone.getAttribute('tabindex')).toBeNull();
    expect(gone.textContent).toBe('@gone-feature (feature removed)');
  });

  it('before the features load, every token is a live mention', async () => {
    app.now = nowOf(fixture());
    app.data = null;
    const e = user('u1', '@gone-feature', { mentions: [{ ...m1, id: 'zz', slug: 'gone-feature' }] });
    const r = render(ChatText, { item: { entry: e }, ow: {} });
    expect(r.container.querySelector('[role="link"]').textContent).toBe('@gone-feature');
    expect(r.container.querySelector('.gone')).toBeNull();
  });

  it('a waking entry renders its mentions from the context it was sent with', async () => {
    const e = { ...user('u2', 'ask @refunds', null), context: { mentions: [{ kind: 'feature', id: 'f2' }] } };
    await mount(FEATURES, [e]);
    expect(body('u2').querySelector('[role="link"]').textContent).toBe('@refunds');
  });

  it('an Overwatch reply is untouched', async () => {
    await mount(FEATURES, [
      {
        id: 'o1',
        day: '2026-11-02',
        at_ms: Date.now() - 1000,
        author: 'overwatch',
        kind: 'text',
        text: 'See @refunds',
        rev: 1,
      },
    ]);
    expect(c.querySelector('[data-entry="o1"] .mention')).toBeNull();
  });
});
