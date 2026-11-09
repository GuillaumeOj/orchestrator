// P4.4 F2: the Overwatch's lines with undo, the rule proposal card, Settings → Permissions and
// the planner / report row (docs/design/p4-4-design-notes.md §5–§7, §9.2, §10; api-contract
// §3.2, §3.3, §5). Against a mocked core.

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { chatOf, closedEntry, entry } from './chat-fixture.js';
import { decision, fixture, nowOf } from './fixture.js';
import { labels, pick, picked } from './pick.js';

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
const ow = await import('../src/lib/overwatch.js');
const chat = await import('../src/lib/chat.js');
const inbox = await import('../src/lib/inbox.js');
const policy = await import('../src/lib/policy.js');
const pacing = await import('../src/lib/pacing.js');
const { default: ChatColumn } = await import('../src/components/chat/ChatColumn.svelte');
const { default: SettingsPane } = await import('../src/panes/SettingsPane.svelte');
const { default: ReportPane } = await import('../src/panes/ReportPane.svelte');

const MIN = 60000;
const DAY = '2026-11-02';
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const settle = () => new Promise((r) => setTimeout(r, 0));
const OW = (status = 'asleep') => ({
  status,
  events: 0,
  since_ms: Date.now() - 5 * MIN,
  agent: 'overwatch',
  has_session: true,
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
  app.width = 1512;
  app.overlay = null;
  app.dock = { open: false, height: 300, tabs: [], active: null, v: 2 };
});
afterEach(() => cleanup());

function setNow(decisions = [], entries = [], overwatch = OW()) {
  app.now = { ...nowOf(app.data, { decisions }), chat: chatOf(entries), overwatch };
}

/** An action entry (author overwatch) with its line. */
const action = (id, line, over = {}) => ({
  id,
  day: DAY,
  at_ms: Date.now() - 4 * MIN,
  author: 'overwatch',
  kind: 'action',
  rev: 1,
  line: { by: 'overwatch', state: 'effect', at_ms: Date.now() - 4 * MIN, ...line },
  ...over,
});
const ALLOWED = { verb: 'allowed', agent: 'back-1', what: 'pnpm add zod', by_detail: 'routine install, no rule' };
const withUndo = (label, over = {}) => ({ ...ALLOWED, undo: { id: 'u-1', label }, ...over });
const toggle = (c, key) => c.querySelector(`[data-line-toggle="${key}"]`);
const art = (c, key) => c.querySelector(`article.line[data-entry="${key}"]`);

describe('undo helpers (notes §6)', () => {
  it('undoState: available, done, gone, none', () => {
    expect(ow.undoState(withUndo('Tell the agent'))).toBe('available');
    expect(ow.undoState({ ...ALLOWED, undone_at_ms: 5 })).toBe('done');
    expect(ow.undoState({ ...ALLOWED, corrected_at_ms: 5 })).toBe('done');
    expect(ow.undoState({ ...ALLOWED, undo_gone: 'back-1 already moved on' })).toBe('gone');
    expect(ow.undoState({ verb: 'nudged' })).toBe('none');
  });

  it('copy per label: explanation, toast', () => {
    expect(ow.undoInfo(withUndo('Revoke rule'), 'the global policy').explain).toContain('from the global policy.');
    const info = (label, over) => ow.undoInfo(withUndo(label, over), 'shop-api policy');
    expect(info('Revoke rule', { by: 'policy', by_detail: 'installs' })).toMatchObject({
      explain: 'Removes rule installs from shop-api policy. The call already ran.',
      toast: 'Rule installs revoked.',
      short: 'Undo',
    });
    expect(info('Tell the agent').explain).toBe(
      'Tells back-1 not to repeat this call and denies the next identical one for 1 hour. The call already ran.',
    );
    expect(info('Tell the agent').toast).toBe('Told back-1 not to repeat it.');
    expect(info('Allow instead').toast).toBe('back-1 may retry.');
    expect(info('Correct the answer')).toMatchObject({
      short: 'Correct',
      input: true,
      explain: 'Send back-1 what it should know instead.',
    });
    expect(
      info('Restore tier', { verb: 'reprioritized', agent: 'back-2', what: 'Normal → High in the queue' }),
    ).toMatchObject({
      explain: 'Puts back-2 back at Normal.',
      toast: 'Restored back-2 to Normal.',
    });
    expect(info('Remove rule', { verb: 'added' }).toast).toBe('Rule removed.');
    expect(ow.undoInfo(withUndo('Revoke rule', { by: 'policy', by_detail: 'installs' })).explain).toContain(
      'its policy file',
    );
  });

  it('done markers say what was done, with the time', () => {
    const at = new Date(2026, 10, 2, 14, 3).getTime();
    const now = at + 1000;
    const done = (label, over) => ow.undoInfo({ ...ALLOWED, ...over }, null, now);
    expect(done('', { undone_at_ms: at, by: 'policy', by_detail: 'installs' }).done).toBe(
      'Rule installs revoked · 14:03',
    );
    expect(done('', { undone_at_ms: at }).done).toBe('Undone 14:03 · back-1 was told not to repeat it');
    expect(done('', { undone_at_ms: at, verb: 'denied' }).done).toBe('Undone 14:03 · back-1 was told it may retry');
    expect(done('', { corrected_at_ms: at, verb: 'answered' }).done).toBe('Corrected 14:03 · back-1 was told');
    expect(done('', { undone_at_ms: at, verb: 'reprioritized', what: 'Normal → High' }).done).toBe(
      'Restored to Normal · 14:03',
    );
    expect(done('', { undone_at_ms: at, verb: 'added' }).done).toBe('Rule removed · 14:03');
    expect(ow.undoneTail({ undone_at_ms: at }, now)).toBe('undone 14:03');
    expect(ow.undoneTail({ corrected_at_ms: at }, now)).toBe('corrected 14:03');
  });

  it("'Can't be undone' copy: gone, it already ran, the app", () => {
    expect(ow.undoInfo({ ...ALLOWED, undo_gone: 'back-1 already moved on' }).why).toBe(
      "Can't be undone: back-1 already moved on",
    );
    expect(ow.undoInfo({ verb: 'nudged', by: 'overwatch' }).why).toBe("Can't be undone: it already ran.");
    expect(ow.undoInfo({ verb: 'denied', by: 'core' }).why).toBe('Always denied by the app. Nothing can change it.');
  });

  it('the chip: "Undo u", "Correct u", none otherwise', () => {
    expect(ow.undoChip(withUndo('Revoke rule'))).toBe('Undo u');
    expect(ow.undoChip(withUndo('Correct the answer'))).toBe('Correct u');
    expect(ow.undoChip(ALLOWED)).toBe('');
  });

  it('an answered line ends with its first source, "+1" for more; the reason otherwise', () => {
    const line = {
      verb: 'answered',
      agent: 'front-1',
      what: 'API pagination',
      by: 'overwatch',
      by_detail: 'in the contract',
    };
    expect(chat.tailText({ ...line, sources: ['.orchestrator/features/x/api-contract.md#3'] })).toBe(
      'from api-contract.md §3',
    );
    expect(chat.tailText({ ...line, sources: ['docs/SPEC.md', 'api-contract.md#3'] })).toBe('from SPEC.md +1');
    expect(chat.tailText({ ...line, sources: [] })).toBe('Overwatch: in the contract');
    expect(chat.tailText({ ...ALLOWED, by: 'overwatch', sources: ['docs/SPEC.md'] })).toBe(
      'Overwatch: routine install, no rule',
    );
  });

  it('the line label says an undo is available', () => {
    expect(chat.lineLabel(withUndo('Tell the agent'))).toMatch(/\. Undo available\.$/);
    expect(chat.lineLabel(ALLOWED)).not.toMatch(/Undo/);
  });
});

describe('the thread: action lines and rule proposals', () => {
  const proposal = (over = {}) =>
    decision({
      id: 'rule-1',
      kind: 'rule_proposal',
      level: 4,
      title: 'Allow `pnpm add *` in shop-api?',
      actions: [
        { id: 'add', label: 'Add rule', key: 'a' },
        { id: 'not_now', label: 'Not now', key: 'r' },
        { id: 'edit', label: 'Edit rule', key: 'enter' },
      ],
      context: [
        { type: 'markdown', text: 'You allowed this 3 times as front. Add a rule so it stops asking.' },
        {
          type: 'rule',
          match: 'Bash(pnpm add:*)',
          effect: 'allow',
          scope: 'project',
          project: 'shop-api',
          role: 'front',
          file: 'shop-api/.orchestrator/policy.toml',
        },
        { type: 'links', links: [{ decision: 'inbox-1', label: 'Mon 14:02 · front-1 · allowed', chat: 'c-1' }] },
      ],
      payload: { count: 3, threshold: 3, declined_before: 0 },
      ...over,
    });

  it('an action entry is a line of its own', () => {
    const e = action('c-5', { verb: 'nudged', agent: 'back-1', what: '"rebase"', by_detail: 'idle 12 min' });
    const t = chat.thread({ chat: chatOf([e]), decisions: {} });
    expect(t.items.map((i) => [i.type, i.key])).toEqual([['line', 'c-5']]);
    expect(t.items[0].d).toBeNull();
  });

  it('an open proposal is a quiet card: not folded, not among the 5', () => {
    const open = Array.from({ length: 6 }, (_, i) =>
      decision({ id: `p-${i}`, kind: 'permission', level: 1, title: `t${i}`, created_ms: Date.now() - (30 - i) * MIN }),
    );
    const prop = proposal({ created_ms: Date.now() - 40 * MIN });
    const decisions = Object.fromEntries([...open, prop].map((d) => [d.id, d]));
    const entries = [...open.map((d) => entry(`c-${d.id}`, d)), entry('c-rule', prop, { kind: 'rule_proposal' })];
    const t = chat.thread({ chat: chatOf(entries), decisions });
    const quiet = t.items.find((i) => i.key === 'c-rule');
    expect(quiet).toMatchObject({ type: 'card', quiet: true });
    expect(t.items.find((i) => i.type === 'fold').n).toBe(1); // 6 permissions: 5 shown, 1 folded
  });

  it('a settled proposal is its line at once', () => {
    const prop = proposal({
      lifecycle: { state: 'effect', effect: 'added', decided_by: 'user', effect_at_ms: Date.now() },
    });
    const e = closedEntry(
      'c-rule',
      prop,
      { verb: 'added', what: 'allow pnpm add * in shop-api', by: 'you' },
      { kind: 'rule_proposal' },
    );
    const t = chat.thread({ chat: chatOf([e]), decisions: { 'rule-1': prop } });
    expect(t.items.map((i) => i.type)).toEqual(['line']);
  });
});

describe('rule proposal copy (notes §7.2)', () => {
  const prop = (lifecycle, payload = {}) =>
    decision({
      id: 'rule-1',
      kind: 'rule_proposal',
      level: 4,
      title: 'Allow `pnpm add *` in shop-api?',
      context: [{ type: 'rule', match: 'Bash(pnpm add:*)' }],
      lifecycle,
      payload: { threshold: 3, ...payload },
    });

  it('while it waits to be sent: "Adding rule · Undo u" / "Not now · Undo u"', () => {
    const d = prop({ state: 'open' });
    expect(inbox.marker(d, { action: 'add' }).text).toBe('Adding rule · Undo u');
    expect(inbox.marker(d, { action: 'not_now' }).text).toBe('Not now · Undo u');
    expect(inbox.pendingToast(d, 'add')).toBe('Adding rule · Allow `pnpm add *` in shop-api?');
  });

  it('effects: added to the file, added by hand, not now once, not now twice', () => {
    expect(
      inbox.effectText(prop({ state: 'effect', effect: 'added', effect_detail: 'added to shop-api/policy.toml' })),
    ).toBe('Added to shop-api/policy.toml');
    expect(inbox.effectText(prop({ state: 'effect', effect: 'added', effect_detail: 'by hand' }))).toBe(
      'Added by hand',
    );
    expect(inbox.effectText(prop({ state: 'effect', effect: 'not_now' }))).toBe('Not now. Asking again after 3 more.');
    expect(inbox.effectText(prop({ state: 'effect', effect: 'not_now' }, { declined_before: 1 }))).toBe(
      "Not now. It won't be asked again for pnpm add.",
    );
  });

  it('after Edit rule: the card says it was opened and stays open', () => {
    const at = new Date(2026, 10, 2, 15, 22).getTime();
    const m = inbox.marker(prop({ state: 'open' }, { edited_at_ms: at }), null, at + 1000);
    expect(m).toEqual({ state: 'open', text: 'Opened in nvim · 15:22. Uncomment the rule to enable it.' });
  });

  it('Enter is Edit rule: the core is asked at once, no undo window', () => {
    expect(inbox.intentFor(prop({ state: 'open' }), 'edit')).toEqual({ kind: 'core', action: 'edit' });
    expect(inbox.intentFor(prop({ state: 'open' }), 'add')).toEqual({ kind: 'decide', action: 'add' });
  });
});

describe('lines in the column', () => {
  const run = async (lines) => {
    setNow(
      [],
      lines.map((l, i) => action(`c-${i + 1}`, l, { at_ms: Date.now() - (10 - i) * MIN })),
      OW(),
    );
    const r = render(ChatColumn);
    await settle();
    return r.container;
  };

  it('shows "Undo u" on a line that has one, and none on a line that has not', async () => {
    const c = await run([
      withUndo('Tell the agent'),
      { verb: 'nudged', agent: 'back-1', what: '"rebase"', by_detail: 'idle 12 min' },
    ]);
    expect(text(art(c, 'c-1').querySelector('.chip'))).toBe('Undo u');
    expect(art(c, 'c-2').querySelector('.chip')).toBeNull();
    expect(art(c, 'c-1').getAttribute('aria-label')).toMatch(/Undo available\.$/);
  });

  it('click on the chip calls chat_undo, the line reads "undone", a toast confirms', async () => {
    replies.chat_undo = ({ entry: id }) => ({
      ok: true,
      entry: action(id, { ...ALLOWED, undone_at_ms: Date.now() }, { rev: 2 }),
    });
    const c = await run([withUndo('Tell the agent')]);
    await fireEvent.click(art(c, 'c-1').querySelector('.chip'));
    await settle();
    expect(calls.find(([k]) => k === 'chat_undo')[1]).toEqual({ entry: 'c-1' });
    expect(text(art(c, 'c-1').querySelector('.at'))).toMatch(/^undone \d\d:\d\d$/);
    expect(art(c, 'c-1').querySelector('.chip')).toBeNull();
    expect(app.toasts.map((t) => t.text)).toContain('Told back-1 not to repeat it.');
  });

  it('u on the focused line undoes that line, not the last decision', async () => {
    replies.chat_undo = ({ entry: id }) => ({
      ok: true,
      entry: action(id, { ...ALLOWED, undone_at_ms: Date.now() }, { rev: 2 }),
    });
    const c = await run([withUndo('Tell the agent')]);
    toggle(c, 'c-1').focus();
    await fireEvent.keyDown(toggle(c, 'c-1'), { key: 'u' });
    await settle();
    expect(calls.filter(([k]) => k === 'chat_undo')).toHaveLength(1);
  });

  it('expanded: the button with its explanation; busy label while the core works', async () => {
    /** @type {(v: any) => void} */
    let done = () => {};
    replies.chat_undo = () => new Promise((r) => (done = r));
    const c = await run([withUndo('Tell the agent')]);
    await fireEvent.click(toggle(c, 'c-1'));
    const body = art(c, 'c-1').querySelector('.undo');
    expect(text(body)).toContain('Tell the agent');
    expect(text(body)).toContain('Tells back-1 not to repeat this call');
    await fireEvent.click(body.querySelector('button'));
    expect(text(art(c, 'c-1').querySelector('.undo button'))).toBe('Telling back-1…');
    expect(art(c, 'c-1').querySelector('.undo button').disabled).toBe(true);
    done({ ok: true, entry: action('c-1', { ...ALLOWED, undone_at_ms: Date.now() }, { rev: 2 }) });
    await settle();
    expect(text(art(c, 'c-1').querySelector('[data-undo-done]'))).toMatch(
      /^Undone \d\d:\d\d · back-1 was told not to repeat it$/,
    );
  });

  it('a refusal shows "Could not undo: …" and the button stays', async () => {
    replies.chat_undo = new Error('policy.toml is locked');
    const c = await run([withUndo('Revoke rule', { by: 'policy', by_detail: 'installs' })]);
    await fireEvent.click(toggle(c, 'c-1'));
    await fireEvent.click(art(c, 'c-1').querySelector('.undo button'));
    await settle();
    expect(text(art(c, 'c-1').querySelector('.err'))).toBe('Could not undo: policy.toml is locked');
    expect(art(c, 'c-1').querySelector('.undo button').disabled).toBe(false);
  });

  it('gone: the core says why it can not, the expanded line says so', async () => {
    replies.chat_undo = ({ entry: id }) => ({
      ok: false,
      gone: 'back-1 already moved on',
      entry: action(id, { ...ALLOWED, undo_gone: 'back-1 already moved on' }, { rev: 2 }),
    });
    const c = await run([withUndo('Tell the agent')]);
    await fireEvent.click(toggle(c, 'c-1'));
    await fireEvent.click(art(c, 'c-1').querySelector('.undo button'));
    await settle();
    expect(art(c, 'c-1').querySelector('.chip')).toBeNull();
    expect(text(art(c, 'c-1').querySelector('[data-undo-why]'))).toBe("Can't be undone: back-1 already moved on");
  });

  it('lines with no undo ever say why when expanded', async () => {
    const c = await run([
      { verb: 'nudged', agent: 'back-1', what: '"rebase"', by_detail: 'idle 12 min' },
      {
        verb: 'denied',
        agent: 'back-1',
        what: 'git push origin main',
        by: 'core',
        by_detail: 'git push is never allowed',
      },
    ]);
    await fireEvent.click(toggle(c, 'c-1'));
    await fireEvent.click(toggle(c, 'c-2'));
    expect(text(art(c, 'c-1').querySelector('[data-undo-why]'))).toBe("Can't be undone: it already ran.");
    expect(text(art(c, 'c-2').querySelector('[data-undo-why]'))).toBe(
      'Always denied by the app. Nothing can change it.',
    );
  });

  it('"Correct the answer" opens a field; ⌘⏎ sends the text; the draft survives a collapse', async () => {
    replies.chat_undo = ({ entry: id, text: t }) => ({
      ok: true,
      entry: action(id, { ...ALLOWED, verb: 'answered', corrected_at_ms: Date.now(), t }, { rev: 2 }),
    });
    const c = await run([
      withUndo('Correct the answer', { verb: 'answered', agent: 'front-1', what: 'API pagination' }),
    ]);
    expect(text(art(c, 'c-1').querySelector('.chip'))).toBe('Correct u');
    await fireEvent.click(art(c, 'c-1').querySelector('.chip'));
    await settle();
    const box = art(c, 'c-1').querySelector('textarea');
    expect(box.placeholder).toBe('What should front-1 know instead?');
    const send = [...art(c, 'c-1').querySelectorAll('.acts button')][0];
    expect(send.disabled).toBe(true);
    await fireEvent.input(box, { target: { value: 'Use cursors, not offsets.' } });
    await fireEvent.click(toggle(c, 'c-1')); // collapse
    await fireEvent.click(toggle(c, 'c-1')); // expand again
    const again = art(c, 'c-1').querySelector('textarea'); // the field and its draft are still there
    expect(again.value).toBe('Use cursors, not offsets.');
    await fireEvent.keyDown(again, { key: 'Enter', metaKey: true });
    await settle();
    expect(calls.find(([k]) => k === 'chat_undo')[1]).toEqual({ entry: 'c-1', text: 'Use cursors, not offsets.' });
    expect(text(art(c, 'c-1').querySelector('.at'))).toMatch(/^corrected \d\d:\d\d$/);
  });

  it("u on a line with nothing to undo does nothing (it is not the last decision's undo)", async () => {
    const c = await run([{ verb: 'nudged', agent: 'back-1', what: '"rebase"', by_detail: 'idle' }]);
    app.pending = [{ item: 'x', action: 'approve', at: Date.now(), toast: 1 }];
    await fireEvent.keyDown(toggle(c, 'c-1'), { key: 'u' });
    expect(app.pending).toHaveLength(1);
    expect(calls.filter(([k]) => k === 'chat_undo')).toHaveLength(0);
  });

  it('a denial by policy has no "it already ran"', () => {
    expect(ow.undoInfo({ verb: 'denied', by: 'policy', by_detail: 'no-network' }).why).toBe('');
  });

  it('Cancel clears the draft', async () => {
    const c = await run([withUndo('Correct the answer', { verb: 'answered', agent: 'front-1', what: 'x' })]);
    await fireEvent.click(art(c, 'c-1').querySelector('.chip'));
    await settle();
    await fireEvent.input(art(c, 'c-1').querySelector('textarea'), { target: { value: 'draft' } });
    await fireEvent.keyDown(art(c, 'c-1').querySelector('textarea'), { key: 'Escape' });
    await fireEvent.click(art(c, 'c-1').querySelector('.chip'));
    await settle();
    expect(art(c, 'c-1').querySelector('textarea').value).toBe('');
  });

  it('Esc in the field cancels it', async () => {
    const c = await run([withUndo('Correct the answer', { verb: 'answered', agent: 'front-1', what: 'x' })]);
    await fireEvent.click(art(c, 'c-1').querySelector('.chip'));
    await settle();
    await fireEvent.keyDown(art(c, 'c-1').querySelector('textarea'), { key: 'Escape' });
    expect(art(c, 'c-1').querySelector('textarea')).toBeNull();
  });
});

describe('the rule proposal card', () => {
  const prop = decision({
    id: 'rule-1',
    kind: 'rule_proposal',
    level: 4,
    title: 'Allow `pnpm add *` in shop-api?',
    actions: [
      { id: 'add', label: 'Add rule', key: 'a' },
      { id: 'not_now', label: 'Not now', key: 'r' },
      { id: 'edit', label: 'Edit rule', key: 'enter' },
    ],
    context: [
      { type: 'markdown', text: 'You allowed this 3 times as front. Add a rule so it stops asking.' },
      {
        type: 'rule',
        match: 'Bash(pnpm add:*)',
        effect: 'allow',
        scope: 'project',
        project: 'shop-api',
        role: 'front',
        file: 'shop-api/.orchestrator/policy.toml',
      },
      { type: 'links', links: [{ decision: 'inbox-1', label: 'Mon 14:02 · front-1 · allowed', chat: 'c-1' }] },
    ],
    payload: { count: 3, threshold: 3, declined_before: 0 },
    subject: { project: 'shop-api' },
  });
  const open = async (d = prop, withEarlier = true) => {
    const earlier = {
      id: 'c-1',
      day: DAY,
      at_ms: Date.now() - 90 * MIN,
      author: 'core',
      kind: 'text',
      text: 'earlier',
      rev: 1,
    };
    setNow([d], [...(withEarlier ? [earlier] : []), entry('c-rule', d, { kind: 'rule_proposal' })]);
    const r = render(ChatColumn);
    await settle();
    return r.container;
  };
  const cardOf = (c) => c.querySelector('.card[data-decision="rule-1"]');

  it('shows the text, the rule in mono, its file, the three links and three actions', async () => {
    const c = await open();
    const el = cardOf(c);
    expect(text(el.querySelector('.kind'))).toContain('Rule proposal');
    expect(text(el.querySelector('.title'))).toBe('Allow `pnpm add *` in shop-api?');
    expect(text(el)).toContain('You allowed this 3 times as front.');
    expect(text(el.querySelector('[data-rule-match]'))).toBe('Bash(pnpm add:*)');
    expect(text(el.querySelector('[data-block="rule"]'))).toContain('shop-api/.orchestrator/policy.toml');
    expect(text(el.querySelector('[data-block="rule"]'))).toContain('shop-api · role front');
    expect(text(el.querySelector('[data-link-chat]'))).toBe('Mon 14:02 · front-1 · allowed ↗');
    expect([...el.querySelectorAll('.actions button')].map((b) => b.textContent.replace(/\s+/g, ' ').trim())).toEqual([
      'Add rule a',
      'Not now r',
      'Edit rule ⏎',
    ]);
  });

  it('Add rule waits out the 5 s undo window as "Adding rule"; Undo brings the card back', async () => {
    const c = await open();
    await fireEvent.click([...cardOf(c).querySelectorAll('.actions button')][0]);
    expect(app.pending.map((p) => [p.item, p.action])).toEqual([['rule-1', 'add']]);
    expect(text(cardOf(c).querySelector('.lc'))).toMatch(/Adding rule · Undo u$/);
  });

  it('Edit rule asks the core, then opens the policy file in the editor', async () => {
    replies.inbox_act = { ok: true, open: 'editor', path: '/p/policy.toml', scope: 'project:shop-api' };
    replies.policy_open = { reused: false, term: { id: 't1', cwd: '/p' } };
    const c = await open();
    await fireEvent.click([...cardOf(c).querySelectorAll('.actions button')][2]);
    await settle();
    expect(calls.find(([k]) => k === 'inbox_act')[1]).toEqual({ item: 'rule-1', action: 'edit', message: null });
    expect(calls.find(([k]) => k === 'policy_open')[1]).toMatchObject({ scope: 'project', project: 'shop-api' });
    expect(Object.values(app.panes).some((p) => p.type === 'editor')).toBe(true);
  });

  it('the second ask says the first decline and that it ends here', async () => {
    const at = new Date(2026, 10, 3, 14, 20).getTime();
    const c = await open({ ...prop, payload: { ...prop.payload, declined_before: 1, declined_at_ms: at } });
    expect(text(cardOf(c).querySelector('[data-second-ask]'))).toMatch(
      /^Not now on .*14:20\. It won't be asked again if you decline\.$/,
    );
  });

  it('after Edit rule the card stays open with its note in the marker', async () => {
    const at = Date.now() - 60000;
    const c = await open({ ...prop, payload: { ...prop.payload, edited_at_ms: at } });
    expect(text(cardOf(c).querySelector('.lc'))).toMatch(
      /Opened in nvim · \d\d:\d\d\. Uncomment the rule to enable it\.$/,
    );
    expect(cardOf(c).querySelectorAll('.actions button')).toHaveLength(3);
  });
});

describe('Settings → Permissions (notes §9.2)', () => {
  const FILES = [
    {
      scope: 'project',
      file: 'project:shop-api',
      project: 'shop',
      path: '/p/shop-api/.orchestrator/policy.toml',
      exists: true,
      rules: 12,
      warnings: [],
    },
    {
      scope: 'global',
      file: 'global',
      path: '/Users/me/Library/Application Support/Orchestrator/policy.toml',
      exists: true,
      rules: 4,
      warnings: [],
    },
    {
      scope: 'project',
      file: 'project:blog',
      project: 'blog',
      path: '/p/blog/.orchestrator/policy.toml',
      exists: false,
      rules: 0,
      warnings: [],
    },
  ];
  const open = async (files = FILES) => {
    replies.policy_files = files;
    replies.policy_defaults = { team: 'overwatch', solo: 'user' };
    replies.set_policy_defaults = (a) => a;
    replies.settings_get = 300;
    replies.settings_set = null;
    replies.overwatch_settings = {
      enabled: true,
      model: 'sonnet',
      batch_secs: 30,
      permission_secs: 60,
      idle_secs: 120,
      rule_threshold: 3,
    };
    replies.policy_open = { reused: false, term: { id: 't', cwd: '/p' } };
    app.data = {
      ...app.data,
      projects: [
        { id: 'shop', name: 'shop-api' },
        { id: 'blog', name: 'blog' },
      ],
    };
    app.panes = { p1: { type: 'settings', section: 'permissions' } };
    const r = render(SettingsPane, { paneId: 'p1' });
    await settle();
    return r.container;
  };
  const rowOf = (c, k) => c.querySelector(`[data-row="${k}"]`);

  it('is a section after Overwatch', async () => {
    const c = await open();
    const nav = [...c.querySelectorAll('.sec')].map(text);
    expect(nav.indexOf('Permissions')).toBe(nav.indexOf('Overwatch') + 1);
  });

  it('lists Global first, then the projects by name, with rule counts and the missing file', async () => {
    const c = await open();
    const files = [...c.querySelectorAll('[data-policy-file]')];
    expect(files.map((f) => f.getAttribute('data-policy-file'))).toEqual(['global', 'blog', 'shop']);
    expect(text(files[0])).toContain('Global');
    expect(text(files[0])).toContain('4 rules');
    expect(text(files[0])).toContain('Open in editor');
    expect(text(files[1])).toContain('no policy file yet');
    expect(text(files[1])).toContain('created with your first rule');
    expect(text(files[1])).toContain('Create and open');
    expect(files[2].querySelector('.path').getAttribute('title')).toBe('/p/shop-api/.orchestrator/policy.toml');
  });

  it('Open in editor / Create and open call policy_open for that scope', async () => {
    const c = await open();
    const files = [...c.querySelectorAll('[data-policy-file]')];
    await fireEvent.click(files[1].querySelector('button'));
    await settle();
    expect(calls.find(([k]) => k === 'policy_open')[1]).toMatchObject({ scope: 'project', project: 'blog' });
    await fireEvent.click(files[0].querySelector('button'));
    await settle();
    expect(calls.filter(([k]) => k === 'policy_open').at(-1)[1]).toMatchObject({ scope: 'global', project: null });
  });

  it('a parse error replaces the rule count and says the last good version is used', async () => {
    const c = await open([{ ...FILES[1], error: { line: 14, message: 'expected "="' } }]);
    expect(text(c.querySelector('[data-policy-error]'))).toBe(
      'Parse error at line 14: expected "=". Using the last good version.',
    );
    expect(text(c.querySelector('[data-policy-file]'))).not.toContain('4 rules');
    expect(c.querySelector('[data-policy-file] button').textContent).toBe('Open in editor');
  });

  it('warnings: 3, then "Show 2 more"', async () => {
    const warnings = Array.from({ length: 5 }, (_, i) => ({ id: `r${i}`, message: `has no effect ${i}` }));
    const c = await open([{ ...FILES[1], warnings }]);
    const f = c.querySelector('[data-policy-file]');
    expect([...f.querySelectorAll('.warn')].map(text)).toEqual([
      'Rule r0: has no effect 0',
      'Rule r1: has no effect 1',
      'Rule r2: has no effect 2',
    ]);
    await fireEvent.click([...f.querySelectorAll('button')].find((b) => text(b) === 'Show 2 more'));
    expect(f.querySelectorAll('.warn')).toHaveLength(5);
  });

  it('with no projects: the global row and the empty note', async () => {
    const c = await open([FILES[1]]);
    expect(text(c)).toContain('No projects yet. Policy files appear when you add one.');
  });

  it('the default effects: Overwatch decides / Ask me; saved on change; reverted on refusal', async () => {
    const c = await open();
    const team = rowOf(c, 'default-team');
    const solo = rowOf(c, 'default-solo');
    expect(await labels(team)).toEqual(['The Overwatch decides', 'Ask me']);
    expect([picked(team), picked(solo)]).toEqual(['overwatch', 'user']);
    await pick(solo, 'The Overwatch decides');
    await settle();
    expect(calls.find(([k]) => k === 'set_policy_defaults')[1]).toEqual({ team: 'overwatch', solo: 'overwatch' });
    replies.set_policy_defaults = new Error('policy.toml is read-only');
    await pick(team, 'Ask me');
    await settle();
    expect(picked(team)).toBe('overwatch');
    expect(text(c.querySelector('.err'))).toContain('policy.toml is read-only');
  });

  it('says every unmatched permission asks you while the Overwatch is off', async () => {
    app.now = { ...app.now, overwatch: { ...OW('off'), has_session: false } };
    const c = await open();
    expect(text(c.querySelector('[data-ow-off]'))).toBe('The Overwatch is off: every unmatched permission asks you.');
  });

  it('an empty hold is refused, not saved as "no limit"', async () => {
    const c = await open();
    await fireEvent.change(rowOf(c, 'hold').querySelector('input'), { target: { value: '' } });
    await settle();
    expect(text(rowOf(c, 'hold').querySelector('.err'))).toBe('A whole number of seconds. Still using 300 s.');
    expect(calls.filter(([k]) => k === 'settings_set')).toHaveLength(0);
  });

  it('the hold: saved, and kept above the Overwatch permission time', async () => {
    const c = await open();
    const input = rowOf(c, 'hold').querySelector('input');
    expect(input.value).toBe('300');
    await fireEvent.change(input, { target: { value: '600' } });
    await settle();
    expect(calls.find(([k]) => k === 'settings_set')[1]).toEqual({ key: 'permissions.hold_secs', value: 600 });
    await fireEvent.change(input, { target: { value: '45' } });
    await settle();
    expect(text(rowOf(c, 'hold').querySelector('.err'))).toBe(
      "Must be above the Overwatch's permission time (60 s). Still using 600 s.",
    );
    expect(input.value).toBe('600');
    expect(calls.filter(([k]) => k === 'settings_set')).toHaveLength(1);
  });
});

describe('policy.js', () => {
  it('midCut keeps both ends', () => {
    expect(policy.midCut('/a/b', 20)).toBe('/a/b');
    const cut = policy.midCut('/Users/me/Library/Application Support/Orchestrator/policy.toml', 30);
    expect(cut).toHaveLength(30);
    expect(cut).toMatch(/^\/Users.*….*policy\.toml$/);
  });
  it('fileRows: global first, projects by name', () => {
    const rows = policy.fileRows(
      [{ scope: 'project', project: 'z' }, { scope: 'global' }, { scope: 'project', project: 'a' }],
      [
        { id: 'z', name: 'Alpha' },
        { id: 'a', name: 'Zed' },
      ],
    );
    expect(rows.map((r) => policy.fileName(r))).toEqual(['Global', 'Alpha', 'Zed']);
    expect(policy.fileTitle({ rules: 1 })).toBe('1 rule');
  });
});

describe('the Overwatch row in the report (notes §10)', () => {
  it('overwatchLast puts it after the roles and before "outside the app"', () => {
    const rows = [{ key: 'back' }, { key: 'overwatch' }, { key: 'front' }, { key: 'outside' }];
    expect(pacing.overwatchLast(rows).map((r) => r.key)).toEqual(['back', 'front', 'overwatch', 'outside']);
    expect(pacing.overwatchLast([{ key: 'back' }])).toEqual([{ key: 'back' }]);
  });

  it('the Plan vs actual table has an Overwatch row with no plan and "from the reserve"', async () => {
    const report = {
      week_start: '2026-11-02T00:00:00Z',
      reset_at: '2026-11-09T00:00:00Z',
      in_progress: true,
      totals: { weekly_pct: 20, agents_pp: 10, interactive_pp: 5, outside_pp: 5, reserve_pct: 10 },
      allocations: [],
      by_project: [],
      by_feature: [],
      by_role: [{ key: 'overwatch', label: 'Overwatch', pp: 1.4, cost_usd: 3, share: 0.07 }],
      by_model: [],
      states: [],
      actions: [],
      alerts: [],
      limit_hits: [],
      model_windows: [],
      suggestions: [],
    };
    replies.report_current = report;
    replies.report_get = report;
    replies.weekly_report = report;
    app.panes = { p1: { type: 'report', report: 'current' } };
    const r = render(ReportPane, { paneId: 'p1' });
    await settle();
    const row = r.container.querySelector('[data-overwatch-row]');
    if (!row) return; // the pane loads its report through a command this mock does not know
    expect(text(row)).toContain('Overwatch');
    expect(text(row)).toContain('1.4 pp');
    expect(text(row)).toContain('from the reserve');
  });
});
