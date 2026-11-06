// P4.5 fork B: the brief card (spec §3.5, notes §8), Plan with the Overwatch (palette, chooser,
// planning tag, chat_send context), routeOpen `launch` and the report line (notes §9).
// Against a mocked core.

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { chatOf, entry } from './chat-fixture.js';
import { decision, fixture, nowOf } from './fixture.js';

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

const A = await import('../src/lib/app.svelte.js');
const { app } = A;
const I = await import('../src/lib/intake.js');
const IS = await import('../src/lib/intake.svelte.js');
const chat = await import('../src/lib/chat.js');
const inbox = await import('../src/lib/inbox.js');
const { decisionKind } = await import('../src/lib/status.js');
const { default: ChatColumn } = await import('../src/components/chat/ChatColumn.svelte');
const { default: ReportPane } = await import('../src/panes/ReportPane.svelte');

const MIN = 60000;
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
  app.chatDraft = '';
  app.width = 1512;
  app.overlay = null;
  app.mission = { projects: null };
  IS.intakeUi.plan = null;
});
afterEach(() => cleanup());

const EST = {
  team: { low_pp: 4.1, high_pp: 6.8, basis: 'project', samples: 7 },
  solo: { low_pp: 0.9, high_pp: 1.6, basis: 'global', samples: 12 },
  size: 'M',
  room_pp: 3,
  over_room: true,
};
const brief = (over = {}) =>
  decision({
    id: 'd_88',
    kind: 'brief',
    level: 4,
    title: 'Partial refunds',
    project: 'shop-api',
    mode: 'team',
    team: 'default-web',
    waits_on: 'user',
    actions: [
      { id: 'launch', label: 'Launch', key: '⏎' },
      { id: 'edit', label: 'Edit', key: 'e' },
      { id: 'dismiss', label: 'Dismiss', key: 'r' },
    ],
    context: [
      { type: 'markdown', text: '## Goal\nRefund part of an order line.' },
      { type: 'text', text: 'shop-api · team run · default-web' },
      { type: 'cost', ...EST },
    ],
    ...over,
  });

function setNow(decisions, entries, overwatch = OW()) {
  app.now = { ...nowOf(app.data, { decisions }), chat: chatOf(entries), overwatch };
}
async function open(d = brief(), overwatch = OW()) {
  setNow([d], [entry('c-brief', d)], overwatch);
  const r = render(ChatColumn);
  await settle();
  return r.container;
}
const cardOf = (c) => c.querySelector('.card[data-decision="d_88"]');
const buttons = (el) => [...el.querySelectorAll('.actions button')];

describe('the brief card', () => {
  it('is a level-4 quiet kind with its own label', () => {
    expect(decisionKind(brief()).label).toBe('Brief');
    expect(chat.quietKind(brief())).toBe(true);
    const t = chat.thread({ chat: chatOf([entry('c-brief', brief())]), decisions: { d_88: brief() } });
    expect(t.cards.map((c) => c.id)).toEqual(['d_88']);
    expect(t.cards[0].quiet).toBe(true);
    expect(t.counts.all).toBe(0); // not in the level chips
  });

  it('shows title · project, the brief, the mode line, the estimate, the room warning and three actions', async () => {
    const c = await open();
    const el = cardOf(c);
    expect(text(el.querySelector('.kind'))).toContain('Brief');
    expect(text(el.querySelector('.title'))).toBe('Partial refunds · shop-api');
    expect(text(el)).toContain('Refund part of an order line.');
    expect(text(el.querySelector('[data-brief-mode]'))).toBe('shop-api · team run · default-web');
    expect(text(el.querySelector('[data-brief-estimate]'))).toBe('≈ 4–7 pp week · from 7 runs in shop-api · size M');
    expect(text(el.querySelector('[data-over-room]'))).toContain("over this week's room (3 pp left)");
    expect(buttons(el).map(text)).toEqual(['Launch ⏎', 'Edit e', 'Dismiss r']);
    expect(text(el.querySelector('[data-brief-foot]'))).toBe(
      'Want changes? Answer in the chat and the Overwatch sends a new brief.',
    );
    expect(text(el.querySelector('.lc'))).toMatch(/Waiting for you/);
  });

  it('the footer says when the Overwatch is off or down', async () => {
    let c = await open(brief(), OW('off'));
    expect(text(cardOf(c).querySelector('[data-brief-foot]'))).toBe(
      'The Overwatch is off. Turn it on in Settings to ask for changes.',
    );
    cleanup();
    c = await open(brief(), OW('down'));
    expect(text(cardOf(c).querySelector('[data-brief-foot]'))).toBe(
      'The Overwatch is down. Messages are kept until it is back.',
    );
  });

  it('Launch and Edit open the prefilled dialog through intake_prefill and decide nothing', async () => {
    replies.intake_prefill = {
      launch: {
        project_id: 'shop-api',
        team_id: 'default-web',
        slug: 'partial-refunds',
        title: 'Partial refunds',
        brief: 'x',
      },
    };
    const c = await open();
    await fireEvent.click(buttons(cardOf(c))[0]);
    await settle();
    expect(calls.find(([k]) => k === 'intake_prefill')[1]).toEqual({ briefDecision: 'd_88', mode: 'team' });
    expect(app.overlay).toMatchObject({
      kind: 'launch',
      intake: { brief_decision: 'd_88' },
      origin: { kind: 'brief', decision: 'd_88' },
    });
    expect(app.overlay.focus).toBeNull();
    expect(text(cardOf(c).querySelector('.lc'))).toContain('Opening dialog…');
    expect(buttons(cardOf(c)).every((b) => b.disabled)).toBe(true);
    expect(app.pending).toEqual([]);
    expect(calls.some(([k]) => k === 'inbox_act')).toBe(false);
  });

  it('Edit focuses the brief; a solo brief opens the session dialog', async () => {
    replies.intake_prefill = { session: { project_id: 'shop-api', name: 'partial-refunds', message: 'x' } };
    const c = await open(brief({ mode: 'solo' }));
    await fireEvent.click(buttons(cardOf(c))[1]);
    await settle();
    expect(calls.find(([k]) => k === 'intake_prefill')[1]).toEqual({ briefDecision: 'd_88', mode: 'solo' });
    expect(app.overlay).toMatchObject({ kind: 'new-session', focus: 'brief' });
  });

  it('a stale brief does not open the dialog and says so', async () => {
    replies.intake_prefill = Object.assign(new Error('x'), { code: 'stale', message: 'brief closed' });
    replies.intake_prefill = new Error(JSON.stringify({ code: 'stale', message: 'brief closed' }));
    const c = await open();
    await fireEvent.click(buttons(cardOf(c))[0]);
    await settle();
    expect(app.overlay).toBeNull();
    expect(app.toasts.at(-1).text).toBe('This brief was closed.');
  });

  it('Dismiss decides, with the 5 s undo window and clears the planning tag', async () => {
    IS.intakeUi.plan = 'shop-api';
    const c = await open();
    await fireEvent.click(buttons(cardOf(c))[2]);
    expect(app.pending.map((p) => [p.item, p.action])).toEqual([['d_88', 'dismiss']]);
    expect(app.toasts.at(-1).text).toBe('Dismissing · Partial refunds');
    expect(IS.intakeUi.plan).toBeNull();
  });

  it('keys: ⏎ launches, e edits, r dismisses', () => {
    const d = brief();
    expect(inbox.keyIntent('⏎', d)).toEqual({ kind: 'brief', edit: false });
    expect(inbox.keyIntent('enter', d)).toEqual({ kind: 'brief', edit: false });
    expect(inbox.keyIntent('e', d)).toEqual({ kind: 'brief', edit: true });
    expect(inbox.keyIntent('r', d)).toEqual({ kind: 'decide', action: 'dismiss' });
  });

  it('lifecycle lines: launched, dismissed, revised and the stale card', async () => {
    const lc = (state, extra = {}) => ({ state, decided_by: 'user', effect_at_ms: Date.now(), ...extra });
    expect(chat.lineOfDecision(brief({ lifecycle: lc('effect', { effect: 'launched partial-refunds' }) })).verb).toBe(
      'launched',
    );
    expect(chat.lineOfDecision(brief({ lifecycle: lc('dismissed', { effect: 'dismissed' }) })).verb).toBe('dismissed');
    expect(
      chat.lineOfDecision(brief({ lifecycle: lc('effect', { effect: 'revised', decided_by: 'overwatch' }) })).verb,
    ).toBe('revised');
    expect(inbox.effectText(brief({ lifecycle: lc('effect', { effect: 'launched partial-refunds' }) }))).toBe(
      'Launched partial-refunds',
    );
    expect(inbox.marker(brief({ lifecycle: lc('effect', { effect: 'revised' }) })).text).toContain(
      'Revised · a new brief replaces this one',
    );
    expect(inbox.marker(brief({ lifecycle: { state: 'stale', stale_why: 'shop-api was removed' } })).text).toBe(
      'Stale · shop-api was removed',
    );
    // A settled brief is a line, not a card.
    const d = brief({ lifecycle: lc('effect', { effect: 'launched partial-refunds' }) });
    const c = await open(d);
    expect(cardOf(c)).toBeNull();
    expect(c.querySelector('article.line')).not.toBeNull();
  });

  it('a brief ending clears a planning tag set before it', () => {
    const since = Date.now() - 1000;
    expect(
      I.briefEnded(brief({ lifecycle: { state: 'effect', effect: 'launched x', effect_at_ms: Date.now() } }), since),
    ).toBe(true);
    expect(
      I.briefEnded(brief({ lifecycle: { state: 'effect', effect: 'revised', effect_at_ms: Date.now() } }), since),
    ).toBe(false);
    expect(I.briefEnded(brief({ lifecycle: { state: 'dismissed', decided_at_ms: since - 5000 } }), since)).toBe(false);
  });
});

describe('routeOpen launch', () => {
  it('opens the prefilled dialog with the brief as its origin', () => {
    const ok = A.routeOpen({
      open: 'launch',
      decision: 'd_88',
      title: 'Partial refunds',
      prefill: { launch: { project_id: 'shop-api', slug: 's' } },
    });
    expect(ok).toBe(true);
    expect(app.overlay).toMatchObject({
      kind: 'launch',
      prefill: { slug: 's' },
      intake: { brief_decision: 'd_88' },
      origin: { kind: 'brief', decision: 'd_88', title: 'Partial refunds' },
    });
  });
});

describe('Plan with the Overwatch', () => {
  const entryOf = (label) => A.commands().find((c) => c.id === label);

  it('palette entries exist; one project goes straight to the draft', () => {
    app.data = { ...app.data, projects: [app.data.projects[0]] };
    expect(entryOf('intake.refresh').label).toBe('Refresh Ready to start');
    expect(entryOf('intake.setup').label).toMatch(/^Set up intake for /);
    entryOf('intake.plan').run();
    expect(app.chatDraft).toBe('Plan a feature for shop-api: ');
    expect(IS.intakeUi.plan).toBe('shop-api');
    expect(app.chatFocus).toEqual({ ask: true });
  });

  it('several projects open the chooser, the Mission filter first; Enter chooses', async () => {
    app.mission = { projects: ['shop-api', 'blog'] };
    entryOf('intake.plan').run();
    expect(app.overlay).toEqual({ kind: 'plan-project', ids: ['shop-api', 'blog'] });
    const { default: PlanChooser } = await import('../src/components/PlanChooser.svelte');
    const { getByRole } = render(PlanChooser);
    await settle();
    const box = getByRole('dialog');
    await fireEvent.keyDown(box, { key: 'j' });
    await fireEvent.keyDown(box, { key: 'Enter' });
    expect(app.chatDraft).toBe('Plan a feature for blog: ');
    expect(app.overlay).toBeNull();
  });

  it('the palette hint says when the Overwatch is off', () => {
    app.now = { ...app.now, overwatch: OW('off') };
    expect(entryOf('intake.plan').detail).toBe('off: your message waits');
  });

  it('Refresh calls intake_refresh for the shown projects', async () => {
    app.mission = { projects: ['blog'] };
    entryOf('intake.refresh').run();
    await settle();
    expect(calls.filter(([k]) => k === 'intake_refresh').map(([, a]) => a)).toEqual([{ project: 'blog' }]);
  });

  it('the input shows the tag, chat_send carries the context, the tag stays, ✕ clears it', async () => {
    replies.chat_send = (a) => ({
      entry: { id: 'c-u1', day: '2026-11-02', at_ms: Date.now(), author: 'user', kind: 'text', text: a.text, rev: 1 },
    });
    setNow([], [], OW());
    IS.planWithOverwatch('shop-api');
    const { container } = render(ChatColumn);
    await settle();
    expect(text(container.querySelector('[data-plan-tag]'))).toContain('Planning · shop-api');
    const ta = container.querySelector('textarea[aria-label="Message the Overwatch"]');
    await fireEvent.keyDown(ta, { key: 'Enter' });
    await settle();
    expect(calls.find(([k]) => k === 'chat_send')[1]).toEqual({
      text: 'Plan a feature for shop-api:',
      context: { plan: 'shop-api' },
    });
    expect(IS.intakeUi.plan).toBe('shop-api');
    expect(text(container.querySelector('[data-planning]'))).toBe('planning · shop-api');
    await fireEvent.click(container.querySelector('[data-plan-tag] button'));
    expect(IS.intakeUi.plan).toBeNull();
    app.chatDraft = 'hello';
    await fireEvent.keyDown(ta, { key: 'Enter' });
    await settle();
    expect(calls.filter(([k]) => k === 'chat_send').at(-1)[1]).toEqual({ text: 'hello' });
  });
});

describe('the report line (notes §9)', () => {
  const line = I.reportLine;
  it('every case', () => {
    expect(line({ started: 10, planned: 0, untracked: 0 })).toBe('Started from intake: 10 of 10');
    expect(line({ started: 9, planned: 0, untracked: 1 })).toBe('Started from intake: 9 of 10 · 1 not from intake');
    expect(line({ started: 7, planned: 2, untracked: 0 })).toBe('Started from intake: 9 of 9 · 7 items, 2 briefs');
    expect(line({ started: 7, planned: 2, untracked: 2 })).toBe(
      'Started from intake: 9 of 11 · 7 items, 2 briefs · 2 not from intake',
    );
    expect(line({ started: 0, planned: 0, untracked: 0 })).toBe('Nothing started this week');
    expect(line(undefined)).toBe('');
    expect(I.reportTip({ untracked_items: [{ kind: 'agent', ref: 'fix-ci-3' }] })).toBe(
      'Not from intake: agent fix-ci-3',
    );
  });

  const at = (d, h) => new Date(2026, 9, d, h).getTime();
  const report = (intake) => ({
    id: 'current',
    week_start: new Date(at(19, 9)).toISOString(),
    reset_at: new Date(at(26, 9)).toISOString(),
    in_progress: false,
    provenance: 'observed',
    totals: { weekly_pct: 93, agents_pp: 81, interactive_pp: 7, outside_pp: 5, reserve_pct: 10 },
    allocations: [],
    by_project: [],
    by_feature: [],
    by_role: [],
    by_model: [],
    states: [{ state: 'green', minutes: 60 }],
    actions: [],
    alerts: [],
    limit_hits: [],
    model_windows: [],
    intake,
  });
  it('the pane shows it as plain text with the untracked names in the tooltip, and nothing without intake', async () => {
    replies.reports_list = { reports: [] };
    replies.report_view = report({
      started: 9,
      planned: 0,
      untracked: 1,
      untracked_items: [{ kind: 'agent', ref: 'fix-ci-3' }],
    });
    app.panes.r1 = { type: 'report', reportId: 'current', title: 'Weekly report' };
    let r = render(ReportPane, { paneId: 'r1' });
    await settle();
    await settle();
    const el = r.container.querySelector('[data-intake-line]');
    expect(text(el)).toBe('Started from intake: 9 of 10 · 1 not from intake');
    expect(el.getAttribute('title')).toBe('Not from intake: agent fix-ci-3');
    cleanup();
    replies.report_view = report(undefined);
    r = render(ReportPane, { paneId: 'r1' });
    await settle();
    await settle();
    expect(r.container.querySelector('[data-intake-line]')).toBeNull();
  });
});
