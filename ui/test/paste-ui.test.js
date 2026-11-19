// Pasted long texts in the Overwatch input and thread (allow-long-text-in-overwatch spec §3.2,
// §3.3, §5 UI part, design-notes): the paste event, the placeholder, ⌫ / ⌦, the limits and their
// errors, what a send carries, a retry, the chips of a sent / unsent entry, the panel (fetch, cache,
// missing file, load error, Copy, keys). Against a mocked core. No date or clock in any assertion.

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
const { resetPastes } = await import('../src/lib/paste-store.svelte.js');
const { placeholder } = await import('../src/lib/pastes.js');
const { default: ChatColumn } = await import('../src/components/chat/ChatColumn.svelte');

const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const settle = () => new Promise((r) => setTimeout(r, 0));
const FEATURES = [
  {
    id: 'f2',
    project_id: 'shop-api',
    slug: 'refunds',
    title: 'Refunds',
    branch: 'orch/refunds/integration',
    archived: false,
    run: null,
    tasks: { done: 0, total: 0 },
  },
];
const BIG = (n = 5000) => 'x'.repeat(n);
const TRACE = Array.from({ length: 120 }, (_, i) => `at frame ${i}`).join('\n');

let c;
let ta;
const sends = () => calls.filter(([k]) => k === 'chat_send').map(([, a]) => a);
const fetches = () => calls.filter(([k]) => k === 'chat_paste').map(([, a]) => a);
const live = () => c.querySelector('[aria-live="polite"]').textContent;
const alert = () => c.querySelector('p.warn[role="alert"]');
async function type(value, caret = value.length) {
  ta.value = value;
  ta.setSelectionRange(caret, caret);
  await fireEvent.input(ta);
}
const key = (k, o = {}) => fireEvent.keyDown(ta, { key: k, ...o });
/** A `beforeinput` of `inputType`; false when it was cancelled (the field's own deletion did not run). */
const beforeInput = (inputType) =>
  ta.dispatchEvent(new InputEvent('beforeinput', { inputType, bubbles: true, cancelable: true }));
/** A paste event; true when the default (the native insertion) was left alone. */
async function paste(data) {
  const ev = new Event('paste', { bubbles: true, cancelable: true });
  Object.defineProperty(ev, 'clipboardData', { value: { getData: (t) => (t === 'text/plain' ? data : '') } });
  ta.dispatchEvent(ev);
  await settle();
  return !ev.defaultPrevented;
}
async function mount(entries = []) {
  app.data = { ...fixture(), features: FEATURES };
  app.now = {
    ...nowOf(app.data),
    chat: chatOf(entries),
    overwatch: { status: 'asleep', events: 0, since_ms: Date.now(), agent: 'overwatch', has_session: true },
  };
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
  app.chatPastes = [];
  app.width = 1512;
  app.overlay = null;
  app.workspace = { project: 'all' };
  IS.intakeUi.plan = null;
  resetPastes();
});
afterEach(() => {
  vi.useRealTimers();
  cleanup();
  vi.restoreAllMocks();
});

describe('a paste in the input', () => {
  it('a short paste is left to the field (nothing prevented, nothing kept)', async () => {
    await mount();
    expect(await paste('a short line')).toBe(true);
    expect(await paste('x'.repeat(800))).toBe(true);
    expect(await paste('a\n'.repeat(10))).toBe(true);
    expect(app.chatPastes).toEqual([]);
    expect(ta.value).toBe('');
  });

  it('over 800 characters: a placeholder at the caret, the text kept aside, announced', async () => {
    await mount();
    await type('why: ', 5);
    expect(await paste(BIG(5000))).toBe(false);
    expect(ta.value).toBe('why: [Pasted text #1 · 5 000 chars]');
    expect(app.chatDraft).toBe(ta.value);
    expect(ta.selectionStart).toBe(ta.value.length);
    expect(app.chatPastes).toEqual([{ n: 1, text: BIG(5000), chars: 5000, lines: 1 }]);
    expect(live()).toBe('Pasted 5 000 characters as pasted text 1');
  });

  it('counts like the core: code points, str::lines over CRLF (a trailing newline adds no line)', async () => {
    await mount();
    await paste(`${Array.from({ length: 12 }, () => '😀').join('\r\n')}\r\n`);
    expect(app.chatPastes.map(({ chars, lines }) => ({ chars, lines }))).toEqual([{ chars: 12 + 11 + 1, lines: 12 }]);
  });

  it('801 characters and 11 lines are both over the threshold', async () => {
    await mount();
    expect(await paste('x'.repeat(801))).toBe(false);
    expect(await paste('a\n'.repeat(11))).toBe(false);
    expect(ta.value).toBe('[Pasted text #1 · 801 chars][Pasted text #2 · 22 chars]');
    expect(app.chatPastes.map((p) => p.lines)).toEqual([1, 11]);
  });

  it('goes in at the caret, in the middle of the text, with no spaces added', async () => {
    await mount();
    await type('ab', 1);
    await paste(BIG(900));
    expect(ta.value).toBe('a[Pasted text #1 · 900 chars]b');
    expect(ta.selectionStart).toBe(1 + '[Pasted text #1 · 900 chars]'.length);
  });

  it('replaces the selection', async () => {
    await mount();
    await type('keep REPLACE keep');
    ta.setSelectionRange(5, 12);
    await paste(BIG(900));
    expect(ta.value).toBe('keep [Pasted text #1 · 900 chars] keep');
  });

  it('is normalised (CRLF and lone CR to LF) before it is counted and kept', async () => {
    await mount();
    await paste(`${'a\r\n'.repeat(11)}b\rc`);
    expect(app.chatPastes[0].text).toBe(`${'a\n'.repeat(11)}b\nc`);
    expect(app.chatPastes[0].lines).toBe(13);
    expect(ta.value).toBe(`[Pasted text #1 · ${app.chatPastes[0].chars} chars]`);
  });

  it("inserts through the field's own editing, so ⌘Z undoes it", async () => {
    await mount();
    const exec = vi.fn((_cmd, _ui, s) => {
      ta.setRangeText(s, ta.selectionStart, ta.selectionEnd, 'end');
      ta.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText' }));
      return true;
    });
    document.execCommand = exec;
    try {
      await paste(BIG(900));
      expect(exec).toHaveBeenCalledWith('insertText', false, '[Pasted text #1 · 900 chars]');
      expect(app.chatDraft).toBe('[Pasted text #1 · 900 chars]');
    } finally {
      delete document.execCommand;
    }
  });

  it('numbers 1, 2, 3 and never reuses a number while a removed paste is kept', async () => {
    await mount();
    await paste(BIG(900));
    await paste(BIG(901));
    await type('');
    expect(app.chatPastes.map((p) => p.n)).toEqual([1, 2]);
    await paste(BIG(902));
    expect(ta.value).toBe('[Pasted text #3 · 902 chars]');
  });

  it('numbering restarts at 1 after a send', async () => {
    await mount();
    await paste(BIG(900));
    await key('Enter');
    await settle();
    expect(app.chatPastes).toEqual([]);
    await paste(BIG(950));
    expect(ta.value).toBe('[Pasted text #1 · 950 chars]');
  });

  it('a paste over 200 000 characters is not inserted: the draft is unchanged, the error says how many', async () => {
    await mount();
    await type('draft');
    expect(await paste(BIG(312450))).toBe(false);
    expect(ta.value).toBe('draft');
    expect(app.chatPastes).toEqual([]);
    expect(text(alert())).toBe('Not pasted: 312 450 characters, at most 200 000 in a pasted text.');
    expect(await paste(BIG(200000))).toBe(false);
    expect(app.chatPastes[0].chars).toBe(200000);
    expect(alert()).toBeNull();
  });

  it('an 11th long paste is refused; a short one is never refused', async () => {
    await mount();
    for (let i = 0; i < 10; i++) await paste(BIG(900 + i));
    expect(app.chatPastes.length).toBe(10);
    await paste(BIG(950));
    expect(text(alert())).toBe('Not pasted: at most 10 pasted texts in one message.');
    expect(app.chatPastes.length).toBe(10);
    expect(await paste('short')).toBe(true);
    expect(alert()).toBeNull(); // a paste clears the line
  });

  it('a paste error stays until the next keystroke, then the over-length line shows if it applies', async () => {
    await mount();
    await paste(BIG(300000));
    expect(alert()).not.toBeNull();
    await key('Shift'); // a modifier alone is not a keystroke
    expect(alert()).not.toBeNull();
    await key('a');
    expect(alert()).toBeNull();
  });

  it('the paste error shows before the over-length one', async () => {
    await mount();
    await type('x'.repeat(2001));
    await paste(BIG(300000));
    expect(text(alert())).toMatch(/^Not pasted:/);
    await key('a');
    expect(text(alert())).toBe(
      'Too long for a message. A paste of over 800 characters or 10 lines is sent as a file; shorter text stays in the message.',
    );
  });

  it('the counter counts a placeholder as its text', async () => {
    await mount();
    await type('x'.repeat(1790));
    await paste(BIG(900));
    expect(text(c.querySelector('[data-count]'))).toBe('1 818 / 2 000');
  });
});

describe('review fixes', () => {
  it('a paste over a selection of 10 placeholders is not refused: the selection goes away', async () => {
    await mount();
    for (let i = 0; i < 10; i++) await paste(BIG(900 + i));
    ta.setSelectionRange(0, ta.value.length);
    await paste(BIG(950));
    expect(alert()).toBeNull();
    expect(ta.value).toBe('[Pasted text #11 · 950 chars]');
  });

  it('picking a mention mid-draft closes the popup and does not reopen it on the inserted token', async () => {
    await mount();
    await type('hello @re world', 9);
    expect(c.querySelector('[data-mention-popup]')).not.toBeNull();
    await key('Enter');
    await settle();
    expect(ta.value).toBe('hello @refunds world');
    expect(c.querySelector('[data-mention-popup]')).toBeNull();
  });

  it('an @query typed inside a placeholder opens no popup', async () => {
    await mount();
    await paste(BIG(900));
    const v = ta.value;
    await type(`${v.slice(0, 8)}@x${v.slice(8)}`, 10);
    expect(c.querySelector('[data-mention-popup]')).toBeNull();
  });
});

describe('Backspace and Delete next to a placeholder', () => {
  const PH = placeholder(1, 900);
  async function withPaste(before = 'ab', after = 'cd') {
    await mount();
    await type(`${before}${after}`, before.length);
    await paste(BIG(900));
    return before.length + PH.length;
  }

  it('Backspace just after removes the whole placeholder; the paste is kept for ⌘Z', async () => {
    await withPaste();
    expect(beforeInput('deleteContentBackward')).toBe(false); // preventDefault: the native deletion did not run
    await settle();
    expect(ta.value).toBe('abcd');
    expect(ta.selectionStart).toBe(2);
    expect(app.chatPastes.length).toBe(1);
    expect(live()).toBe('Removed pasted text 1');
  });

  it('Delete just before removes it whole', async () => {
    await withPaste();
    ta.setSelectionRange(2, 2);
    beforeInput('deleteContentForward');
    expect(ta.value).toBe('abcd');
  });

  it('deleteWordBackward (⌥⌫) just after removes it whole too', async () => {
    await withPaste();
    beforeInput('deleteWordBackward');
    expect(ta.value).toBe('abcd');
  });

  it('deleteWordForward (⌥⌦) just before removes it whole too', async () => {
    await withPaste();
    ta.setSelectionRange(2, 2);
    beforeInput('deleteWordForward');
    expect(ta.value).toBe('abcd');
  });

  it('a plain keydown Backspace is left alone: the beforeinput handles it, once', async () => {
    await withPaste();
    expect(await key('Backspace')).toBe(true); // not prevented, nothing removed by the key handler
    expect(ta.value).toContain(PH);
    expect(live()).not.toBe('Removed pasted text 1');
  });

  it("elsewhere, and with a selection, the deletion is the field's own", async () => {
    await withPaste();
    ta.setSelectionRange(1, 1);
    expect(beforeInput('deleteContentBackward')).toBe(true);
    ta.setSelectionRange(2 + PH.length, 2 + PH.length);
    expect(beforeInput('deleteContentForward')).toBe(true);
    ta.setSelectionRange(0, 2 + PH.length);
    expect(beforeInput('deleteContentBackward')).toBe(true);
    expect(beforeInput('insertText')).toBe(true);
    expect(ta.value).toContain(PH);
  });

  it('an edited placeholder is plain text: the deletion is native', async () => {
    await withPaste();
    await type(`ab${PH.replace('900', '90')}cd`, 2 + PH.length - 1);
    expect(beforeInput('deleteContentBackward')).toBe(true);
  });

  it('after the removal the paste is not sent', async () => {
    await withPaste('see ', '!');
    beforeInput('deleteContentBackward');
    await key('Enter');
    await settle();
    expect(sends()).toEqual([{ text: 'see !' }]);
  });
});

describe('@ inside a placeholder', () => {
  it('opens no mention popup; an @ elsewhere still does', async () => {
    await mount();
    await paste(BIG(900));
    const v = ta.value;
    await type(`${v.slice(0, 8)}@${v.slice(8)}`, 9);
    expect(c.querySelector('[data-mention-popup]')).toBeNull();
    await type(`${v} @`);
    expect(c.querySelector('[data-mention-popup]')).not.toBeNull();
  });
});

describe('what a send carries', () => {
  it('context.pastes beside plan and mentions for the placeholders still whole; the draft and pastes are cleared', async () => {
    await mount();
    IS.planWithOverwatch('shop-api');
    await settle();
    await type('about @re');
    await key('Enter'); // picks @refunds
    await settle();
    await type(`${ta.value}see `);
    await paste('A'.repeat(900));
    await type(`${ta.value} and `);
    await paste('B'.repeat(901));
    await key('Enter');
    await settle();
    expect(sends()).toEqual([
      {
        text: 'about @refunds see [Pasted text #1 · 900 chars] and [Pasted text #2 · 901 chars]',
        context: {
          plan: 'shop-api',
          mentions: [{ kind: 'feature', id: 'f2' }],
          pastes: [
            { n: 1, text: 'A'.repeat(900) },
            { n: 2, text: 'B'.repeat(901) },
          ],
        },
      },
    ]);
    expect(app.chatPastes).toEqual([]);
    expect(app.chatDraft).toBe('');
  });

  it('a message made only of a placeholder is valid and carries its paste', async () => {
    await mount();
    await paste(TRACE);
    await key('Enter');
    await settle();
    expect(sends()).toEqual([
      { text: `${placeholder(1, TRACE.length)}`, context: { pastes: [{ n: 1, text: TRACE }] } },
    ]);
  });

  it('an edited placeholder drops its paste; no context at all when none is left', async () => {
    await mount();
    await paste(BIG(900));
    await type(`${ta.value.replace('900', '90')} tail`);
    await key('Enter');
    await settle();
    expect(sends()).toEqual([{ text: '[Pasted text #1 · 90 chars] tail' }]);
  });

  it('the same token twice is one paste', async () => {
    await mount();
    await paste(BIG(900));
    await type(`${ta.value} ${ta.value}`);
    await key('Enter');
    await settle();
    expect(sends()[0].context.pastes).toEqual([{ n: 1, text: BIG(900) }]);
  });

  it('the over-length block counts the placeholders as text', async () => {
    await mount();
    await type('x'.repeat(1990));
    await paste(BIG(900));
    await key('Enter');
    expect(sends()).toEqual([]);
    expect(ta.getAttribute('aria-invalid')).toBe('true');
  });

  it('a refused send keeps its pastes: the unsent entry shows a chip, a retry sends them again', async () => {
    await mount();
    replies.chat_send = new Error('bad_request: busy');
    await type('look ');
    await paste(TRACE);
    await key('Enter');
    await settle();
    const un = c.querySelector('[data-entry^="unsent"]');
    expect(un.querySelector('[data-paste-chip="1"]')).not.toBeNull();
    // the panel opens at once from memory: no fetch
    await fireEvent.click(un.querySelector('[data-paste-chip="1"]'));
    expect(un.querySelector('[role="region"]').textContent).toBe(TRACE);
    expect(fetches()).toEqual([]);
    replies.chat_send = (a) => ({
      entry: { id: 'c-r', day: '2026-11-02', at_ms: Date.now(), author: 'user', kind: 'text', text: a.text, rev: 1 },
    });
    await fireEvent.click(un.querySelector('button.link'));
    await settle();
    expect(sends().at(-1)).toEqual({
      text: `look ${placeholder(1, TRACE.length)}`,
      context: { pastes: [{ n: 1, text: TRACE }] },
    });
  });
});

describe('the chip in the thread', () => {
  const PH1 = placeholder(1, 5000);
  const user = (id, t, pastes, over = {}) => ({
    id,
    day: '2026-11-02',
    at_ms: Date.now() - 60000,
    author: 'user',
    kind: 'text',
    text: t,
    rev: 1,
    delivery: 'delivered',
    meta: { pastes },
    ...over,
  });
  const P1 = { n: 1, chars: 5000, lines: 120, path: '/r/overwatch/pastes/d/k-1.md' };
  const chip = (id, n = 1) => c.querySelector(`[data-entry="${id}"] [data-paste-chip="${n}"]`);
  const panel = (id, n = 1) => c.querySelector(`[data-entry="${id}"] [data-paste-panel="${n}"]`);

  it('renders the placeholder as a chip button with its counts, the text around it untouched', async () => {
    await mount([user('u1', `before ${PH1} after`, [P1])]);
    const el = chip('u1');
    expect(el.getAttribute('role')).toBe('button');
    expect(el.getAttribute('tabindex')).toBe('0');
    expect(el.getAttribute('aria-expanded')).toBe('false');
    expect(el.getAttribute('aria-label')).toBe('Pasted text 1, 5 000 characters, 120 lines');
    expect(el.getAttribute('aria-controls')).toBeNull();
    expect(text(el)).toBe('Pasted text #1 · 5 000 chars · 120 lines');
    expect(c.querySelector('[data-entry="u1"] .body').textContent).toBe(
      'before Pasted text #1 · 5 000 chars · 120 lines after',
    );
    expect(panel('u1')).toBeNull();
  });

  it('a placeholder with no paste in meta stays plain text', async () => {
    await mount([user('u1', `x ${placeholder(2, 900)} y`, [P1])]);
    expect(c.querySelector('[data-entry="u1"] [data-paste-chip]')).toBeNull();
    expect(c.querySelector('[data-entry="u1"] .body').textContent).toBe(`x ${placeholder(2, 900)} y`);
  });

  it('the same placeholder twice: two chips, one paste', async () => {
    await mount([user('u1', `${PH1} and ${PH1}`, [P1])]);
    expect(c.querySelectorAll('[data-entry="u1"] [data-paste-chip="1"]').length).toBe(2);
  });

  it('a click opens the panel and fetches the paste once; closing and reopening is instant', async () => {
    replies.chat_paste = (a) => ({ n: a.n, text: TRACE, chars: 5000, lines: 120 });
    await mount([user('u1', PH1, [P1])]);
    await fireEvent.click(chip('u1'));
    expect(chip('u1').getAttribute('aria-expanded')).toBe('true');
    expect(chip('u1').getAttribute('aria-controls')).toBe(panel('u1').id);
    await settle();
    expect(fetches()).toEqual([{ entry: 'u1', n: 1 }]);
    expect(text(panel('u1').querySelector('.lab'))).toBe('#1 · 5 000 chars · 120 lines');
    const region = panel('u1').querySelector('[role="region"]');
    expect(region.getAttribute('aria-label')).toBe('Pasted text 1');
    expect(region.textContent).toBe(TRACE); // one text node, no Markdown
    expect(region.childNodes.length).toBe(1);
    await fireEvent.click(chip('u1'));
    expect(panel('u1')).toBeNull();
    await fireEvent.click(chip('u1'));
    expect(panel('u1').querySelector('[role="region"]').textContent).toBe(TRACE);
    expect(fetches().length).toBe(1);
  });

  it('⏎ and ⎵ toggle', async () => {
    replies.chat_paste = () => ({ n: 1, text: 'T', chars: 5000, lines: 1 });
    await mount([user('u1', PH1, [P1])]);
    await fireEvent.keyDown(chip('u1'), { key: 'Enter' });
    expect(chip('u1').getAttribute('aria-expanded')).toBe('true');
    await fireEvent.keyDown(chip('u1'), { key: ' ' });
    expect(chip('u1').getAttribute('aria-expanded')).toBe('false');
  });

  it('several panels stack in the order of n, whatever the click order; the entry takes its full width', async () => {
    replies.chat_paste = (a) => ({ n: a.n, text: `text ${a.n}`, chars: 1, lines: 1 });
    const P2 = { n: 2, chars: 900, lines: 4, path: '/p2' };
    await mount([user('u1', `${PH1} ${placeholder(2, 900)}`, [P1, P2])]);
    await fireEvent.click(chip('u1', 2));
    await fireEvent.click(chip('u1', 1));
    await settle();
    expect([...c.querySelectorAll('[data-entry="u1"] [data-paste-panel]')].map((p) => p.dataset.pastePanel)).toEqual([
      '1',
      '2',
    ]);
    expect(c.querySelector('[data-entry="u1"]').classList.contains('wide')).toBe(true);
    await fireEvent.click(chip('u1', 1));
    await fireEvent.click(chip('u1', 2));
    expect(c.querySelector('[data-entry="u1"]').classList.contains('wide')).toBe(false);
  });

  it('the panel sits under the bubble, before the planning line', async () => {
    replies.chat_paste = () => ({ n: 1, text: 'T', chars: 5000, lines: 1 });
    await mount([user('u1', PH1, [P1], { meta: { pastes: [P1], plan: { project: 'shop-api' } } })]);
    await fireEvent.click(chip('u1'));
    const kids = [...c.querySelector('[data-entry="u1"]').children].map((k) => k.className.split(' ')[0] || k.tagName);
    expect(kids.indexOf('paste-panel')).toBeGreaterThan(kids.indexOf('body'));
    expect(c.querySelector('[data-entry="u1"] [data-planning]')).not.toBeNull();
    expect(
      panel('u1').compareDocumentPosition(c.querySelector('[data-entry="u1"] [data-planning]')) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it('while the fetch is in flight: a muted loading line, busy, Copy disabled', async () => {
    replies.chat_paste = () => new Promise(() => {});
    await mount([user('u1', PH1, [P1])]);
    await fireEvent.click(chip('u1'));
    expect(text(panel('u1'))).toContain('Loading pasted text…');
    expect(panel('u1').querySelector('[aria-busy="true"]')).not.toBeNull();
    expect(panel('u1').querySelector('button.act').disabled).toBe(true);
  });

  it('a missing file: the panel says so, no Copy, the chip is muted; still toggles', async () => {
    replies.chat_paste = new Error('not_found: paste file is gone');
    await mount([user('u1', PH1, [P1])]);
    await fireEvent.click(chip('u1'));
    await settle();
    expect(text(panel('u1'))).toContain('Pasted text no longer available');
    expect(panel('u1').querySelector('[role="status"]')).not.toBeNull();
    expect([...panel('u1').querySelectorAll('button')].map((b) => b.textContent.trim())).toEqual(['✕']);
    expect(chip('u1').classList.contains('missing')).toBe(true);
    expect(chip('u1').getAttribute('aria-label')).toBe(
      'Pasted text 1, 5 000 characters, 120 lines, no longer available',
    );
    await fireEvent.click(chip('u1'));
    expect(panel('u1')).toBeNull();
  });

  it('another error: the message with Retry, Copy disabled, the chip unchanged; Retry asks again', async () => {
    replies.chat_paste = new Error('io: disk on fire');
    await mount([user('u1', PH1, [P1])]);
    await fireEvent.click(chip('u1'));
    await settle();
    expect(text(panel('u1'))).toContain('Could not load pasted text: io: disk on fire · Retry');
    expect(chip('u1').classList.contains('missing')).toBe(false);
    expect(panel('u1').querySelector('button.act').disabled).toBe(true);
    replies.chat_paste = () => ({ n: 1, text: 'back', chars: 4, lines: 1 });
    await fireEvent.click(panel('u1').querySelector('button.link'));
    await settle();
    expect(panel('u1').querySelector('[role="region"]').textContent).toBe('back');
    expect(fetches().length).toBe(2);
  });

  it('Copy puts the full text on the clipboard, says Copied, announces it, then goes back to Copy', async () => {
    try {
      const write = vi.fn(() => Promise.resolve());
      Object.defineProperty(navigator, 'clipboard', { value: { writeText: write }, configurable: true });
      replies.chat_paste = () => ({ n: 1, text: TRACE, chars: 5000, lines: 120 });
      await mount([user('u1', PH1, [P1])]);
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
      await fireEvent.click(chip('u1'));
      await vi.advanceTimersByTimeAsync(5);
      const copy = panel('u1').querySelector('button.act');
      expect(copy.textContent).toBe('Copy');
      await fireEvent.click(copy);
      await vi.advanceTimersByTimeAsync(5);
      expect(write).toHaveBeenCalledWith(TRACE);
      expect(copy.textContent).toBe('Copied');
      expect(live()).toBe('Copied pasted text 1');
      await vi.advanceTimersByTimeAsync(1500);
      expect(copy.textContent).toBe('Copy');
    } finally {
      vi.useRealTimers();
    }
  });

  it('Copy failing says Copy failed', async () => {
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: () => Promise.reject(new Error('no')) },
      configurable: true,
    });
    replies.chat_paste = () => ({ n: 1, text: 'T', chars: 5000, lines: 1 });
    await mount([user('u1', PH1, [P1])]);
    await fireEvent.click(chip('u1'));
    await settle();
    await fireEvent.click(panel('u1').querySelector('button.act'));
    await settle();
    expect(panel('u1').querySelector('button.act').textContent).toBe('Copy failed');
  });

  it('✕ and Esc inside the panel collapse it and give the focus back to the chip; Esc does not reach the column', async () => {
    replies.chat_paste = () => ({ n: 1, text: 'T', chars: 5000, lines: 1 });
    await mount([user('u1', PH1, [P1])]);
    await fireEvent.click(chip('u1'));
    await settle();
    await fireEvent.click(panel('u1').querySelector('.x'));
    expect(panel('u1')).toBeNull();
    expect(document.activeElement).toBe(chip('u1'));
    await fireEvent.click(chip('u1'));
    await settle();
    app.focused = 'keep';
    panel('u1').querySelector('[role="region"]').focus();
    await fireEvent.keyDown(panel('u1').querySelector('[role="region"]'), { key: 'Escape' });
    expect(panel('u1')).toBeNull();
    expect(document.activeElement).toBe(chip('u1'));
    expect(app.focused).toBe('keep'); // the column's own Esc (leave) did not run
  });

  it("the column's shortcuts do not see the keys typed in the panel", async () => {
    replies.chat_paste = () => ({ n: 1, text: 'T', chars: 5000, lines: 1 });
    await mount([user('u1', PH1, [P1])]);
    await fireEvent.click(chip('u1'));
    await settle();
    const region = panel('u1').querySelector('[role="region"]');
    for (const k of ['j', 'k', 'f', '/', 'ArrowDown', ' ', '[']) {
      const notPrevented = await fireEvent.keyDown(region, { key: k });
      expect(notPrevented).toBe(true); // nothing called preventDefault: the scroll box keeps its key
    }
    expect(c.querySelector('.search')).toBeNull();
  });

  it('a waking entry renders its chip from the context it was sent with, opening from memory', async () => {
    const e = { ...user('u3', PH1, undefined), meta: undefined, context: { pastes: [{ n: 1, text: 'from memory' }] } };
    await mount([e]);
    expect(chip('u3')).not.toBeNull();
    await fireEvent.click(chip('u3'));
    expect(panel('u3').querySelector('[role="region"]').textContent).toBe('from memory');
    expect(fetches()).toEqual([]);
  });

  it('an Overwatch reply is untouched', async () => {
    await mount([
      {
        id: 'o1',
        day: '2026-11-02',
        at_ms: Date.now() - 1000,
        author: 'overwatch',
        kind: 'text',
        text: PH1,
        rev: 1,
        meta: { pastes: [P1] },
      },
    ]);
    expect(c.querySelector('[data-entry="o1"] [data-paste-chip]')).toBeNull();
  });
});
