// P4.5 Intake: the Ready to start lane (spec §3.3, notes p4-5 §1–§4, §10), its rows, keys and
// start flow through the prefilled dialogs, origin marks, and the pure copy rules of lib/intake.js.
// Tests hold at any date, hour and time zone: times are relative to a fixed `now`.

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { flushSync, tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const calls = [];
const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push({ cmd, args });
    const r = replies[cmd];
    if (r instanceof Error) return Promise.reject(r);
    if (typeof r === 'function') return Promise.resolve(r(args));
    return Promise.resolve(r ?? null);
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? JSON.stringify(e))),
}));

const A = await import('../src/lib/app.svelte.js');
const { app } = A;
const L = await import('../src/lib/layout.js');
const I = await import('../src/lib/intake.js');
const { lanes, readyLane, stops, hints } = await import('../src/lib/mission.js');
const { intakeUi } = await import('../src/lib/intake.svelte.js');
const readyOf = (...a) => lanes(...a).lanes.find((l) => l.id === 'ready');
const { default: MissionPane } = await import('../src/panes/MissionPane.svelte');
const { default: Dialogs } = await import('../src/components/Dialogs.svelte');
const { applyNow } = await import('../src/lib/now.svelte.js');
const { missionData, missionNow } = await import('./mission-fixture.js');

// Relative to the run: the pane's clock is max(30 s tick, Date.now()), and tests hold at any date.
const NOW = Date.now();
const MIN = 60000;
const click = (el) => /** @type {HTMLElement} */ (el).click();
const text = (el) => el.textContent.replace(/\s+/g, ' ');
const row = (c, key) => c.querySelector(`[data-key="${key}"]`);
const key = (el, k, more = {}) => fireEvent.keyDown(el, { key: k, ...more });
const settle = async () => {
  await tick();
  await Promise.resolve();
  await tick();
};

const est = (o = {}) => ({
  team: { low_pp: 4.1, high_pp: 6.8, basis: 'project', samples: 7 },
  solo: { low_pp: 0.9, high_pp: 1.6, basis: 'global', samples: 12 },
  size: 'M',
  room_pp: 3,
  over_room: false,
  room_scope: 'week',
  ...o,
});
const item = (id, project, o = {}) => ({
  id,
  project,
  source: 'github',
  ref: `#${id.slice(3)}`,
  title: `Title of ${id}`,
  url: `https://github.com/acme/shop/issues/${id.slice(3)}`,
  labels: ['ready', 'size:M'],
  updated_at_ms: NOW - 2 * 86400000,
  size: 'M',
  mode: 'team',
  team: 'default-web',
  can_team: true,
  estimate: est(),
  state: 'ready',
  excerpt: 'An excerpt',
  ...o,
});
const source = (project, o = {}) => ({
  project,
  type: 'github',
  label: 'acme/shop · label ready',
  status: 'ok',
  error: null,
  refreshed_at_ms: NOW - 12 * MIN,
  count: 0,
  more: 0,
  hidden: 0,
  url: 'https://github.com/acme/shop/issues?labels=ready',
  ...o,
});
const none = (project) => ({
  project,
  type: 'none',
  label: '',
  status: 'none',
  error: null,
  refreshed_at_ms: null,
  count: 0,
  more: 0,
  hidden: 0,
});
const projects = [
  { id: 'shop-api', name: 'shop-api' },
  { id: 'blog', name: 'blog' },
  { id: 'infra', name: 'infra' },
];

afterEach(cleanup);

describe('lib/intake copy rules (notes §2.1, §4.2, §9)', () => {
  it('rounds ranges to whole pp as the notes say', () => {
    expect(I.ppRange(4.1, 6.8)).toBe('4–7');
    expect(I.ppRange(0.9, 1.6)).toBe('1–2');
    expect(I.ppRange(0.3, 0.6)).toBe('< 1');
    expect(I.ppRange(5.6, 6.4)).toBe('6');
    expect(I.ppRange(0.4, 1.4)).toBe('1');
    expect(I.ppRange(0.4, 2.6)).toBe('< 1–3');
    expect(I.ppRange(null, 3)).toBe('');
  });

  it('writes the row estimate with the unit, the mode and the rough flag', () => {
    const it = item('it_1', 'shop-api');
    expect(I.estimateText(it)).toBe('≈ 4–7 pp week · team');
    expect(I.estimateText(it, 'solo')).toBe('≈ 1–2 pp week · solo');
    expect(I.estimateText(it, 'team', { compact: true })).toBe('≈ 4–7 pp · team');
    const rough = item('it_2', 'shop-api', {
      estimate: est({ team: { low_pp: 4, high_pp: 8, basis: 'default', samples: 0 } }),
    });
    expect(I.estimateText(rough)).toBe('rough ≈ 4–8 pp week · team');
    expect(I.estimateTip(rough)).toBe('No finished runs yet: using the default (6 pp)');
    expect(I.estimateText({ ...it, estimate: undefined })).toBe('no estimate');
  });

  it('warns muted when over the room, per scope, and "none left" under half a pp', () => {
    const over = item('it_3', 'shop-api', { estimate: est({ over_room: true }) });
    expect(I.roomWarning(over)).toBe("over this week's room (3 pp left)");
    expect(I.roomWarning({ estimate: { ...over.estimate, room_scope: 'allocation' } }, 'shop-api')).toBe(
      "over shop-api's room this week (3 pp left)",
    );
    expect(I.roomWarning({ estimate: { ...over.estimate, room_pp: 0.2 } })).toBe("over this week's room (none left)");
    expect(I.roomWarning({ estimate: { ...over.estimate, room_pp: null } })).toBe('');
    expect(I.roomWarning(item('it_4', 'shop-api'))).toBe('');
  });

  it('explains the basis in details by samples and scope', () => {
    expect(I.basisText({ basis: 'project', samples: 7 }, 'team', 'M', 'shop-api')).toBe(
      'from 7 runs in shop-api · size M',
    );
    expect(I.basisText({ basis: 'global', samples: 12 }, 'solo', 'M')).toBe(
      'from 12 sessions across projects · size M',
    );
    expect(I.basisText({ basis: 'default', samples: 0 }, 'team', null)).toBe(
      'rough: no history yet · size M (not set) (default 6 pp)',
    );
    expect(I.basisText({ basis: 'project', samples: 1 }, 'team', 'L', 'blog')).toBe(
      'from 1 run in blog, ± 40 % · size L',
    );
  });

  it('shortens repo-doc refs by the folder the docs share', () => {
    const docs = ['.orchestrator/backlog/checkout-v2.md', '.orchestrator/backlog/search.md'].map((ref) => ({
      source: 'repo_docs',
      ref,
    }));
    expect(I.shortRef(docs[0], docs)).toBe('checkout-v2.md');
    const mixed = [...docs, { source: 'repo_docs', ref: 'docs/ideas/a/b.md' }];
    expect(I.shortRef(mixed[0], mixed)).toBe('…/backlog/checkout-v2.md');
    expect(I.shortRef({ source: 'github', ref: '#123' }, [])).toBe('#123');
  });

  it('hides size and orch labels from the chips and caps them at two', () => {
    const l = I.shownLabels({ labels: ['size:M', 'orch', 'a', 'b', 'c', 'orch:x'] });
    expect(l).toEqual({ chips: ['a', 'b'], more: 1 });
  });

  it('turns a source error into its line and fix (F1–F3)', () => {
    const e = (code, o = {}) =>
      I.errorInfo(
        { code, message: 'raw message', fix: null, ...o },
        { type: 'github', label: 'acme/shop · label ready' },
        NOW,
      );
    expect(e('gh_auth')).toMatchObject({ text: 'GitHub: not logged in', fix: 'integrations', tip: 'raw message' });
    expect(e('gh_scope')).toMatchObject({ fix: 'gh_scope', text: 'GitHub needs the read:project scope' });
    expect(e('not_found')).toMatchObject({ fix: 'settings', text: 'acme/shop not found, or you have no access' });
    expect(e('linear_key').fix).toBe('integrations');
    expect(e('bad_glob').fix).toBe('settings');
    expect(e('failed', { message: 'boom\nstack' })).toMatchObject({ fix: 'retry', text: 'Could not refresh: boom' });
    const rl = e('rate_limited', { retry_at_ms: NOW + 20 * MIN });
    expect(rl.fix).toBeNull();
    expect(rl.text).toMatch(/^GitHub rate limit reached · retry at \d\d:\d\d$/);
    expect(I.rateLimitedUntil({ error: { code: 'rate_limited', retry_at_ms: NOW + MIN } }, NOW)).toBe(NOW + MIN);
    expect(I.rateLimitedUntil({ error: { code: 'rate_limited', retry_at_ms: NOW - MIN } }, NOW)).toBeNull();
  });

  it('prints the report line for every case (notes §9)', () => {
    expect(I.reportLine({ started: 10, planned: 0, untracked: 0 })).toBe('Started from intake: 10 of 10');
    expect(I.reportLine({ started: 9, planned: 0, untracked: 1 })).toBe(
      'Started from intake: 9 of 10 · 1 not from intake',
    );
    expect(I.reportLine({ started: 7, planned: 2, untracked: 2 })).toBe(
      'Started from intake: 9 of 11 · 7 items, 2 briefs · 2 not from intake',
    );
    expect(I.reportLine({ started: 0, planned: 0, untracked: 0 })).toBe('Nothing started this week');
    expect(I.reportLine(undefined)).toBe('');
  });

  it('words the stale errors and the origin marks', () => {
    expect(I.staleBanner({ code: 'stale', started: { kind: 'run', name: 'checkout' } })).toBe(
      'Already started: checkout (team run)',
    );
    expect(I.staleBanner({ code: 'stale', started: { kind: 'agent', name: 'fix-feed' } })).toBe(
      'Already started: fix-feed (solo session)',
    );
    expect(I.staleBanner({ code: 'stale', message: 'brief closed' })).toMatch(/^This brief was closed/);
    expect(I.originText({ kind: 'intake', ref: '#123' })).toBe('from #123');
    expect(I.originText({ kind: 'brief', decision: 'd_1' })).toBe('from brief');
    expect(I.originChip({ kind: 'intake', ref: '#123', source: 'github', url: 'u' })).toMatchObject({
      text: 'From #123 · GitHub',
      url: 'u',
    });
    expect(I.originChip({ kind: 'brief', title: 'Partial refunds', decision: 'd' }).text).toBe(
      "From the Overwatch's brief · Partial refunds",
    );
    expect(I.staleOf({ code: 'stale', message: 'x' })).toMatchObject({ code: 'stale' });
    expect(I.staleOf(new Error('{"code":"stale","message":"not ready"}'))).toMatchObject({ message: 'not ready' });
    expect(I.staleOf(new Error('nope'))).toBeNull();
  });

  it('suggests solo when the item asks for a team the project lacks', () => {
    expect(I.modeOf({ mode: 'team', can_team: false })).toBe('solo');
    expect(I.modeOf({ mode: 'solo', can_team: true })).toBe('solo');
    expect(I.modeOf({ mode: null, can_team: true })).toBe('team');
    expect(I.modeOf({ mode: null, can_team: false })).toBe('solo');
  });
});

describe('the lane model (lib/mission.js readyLane, F9)', () => {
  const intake = (items, sources) => ({ items, sources });

  it('sits after Queued and before Done today, with the count of F9 and the oldest refresh', () => {
    const now = missionNow(NOW);
    now.intake = intake(
      [item('it_1', 'shop-api'), item('it_2', 'shop-api'), item('it_3', 'blog', { source: 'linear', ref: 'ENG-42' })],
      [
        source('shop-api', { count: 2 }),
        source('blog', { type: 'linear', label: 'ENG · next', count: 1, refreshed_at_ms: NOW - 40 * MIN }),
        none('infra'),
      ],
    );
    const m = lanes(now, null, NOW, { projects });
    expect(m.lanes.map((l) => l.id)).toEqual(['needs_you', 'running', 'queued', 'ready', 'done']);
    const ready = m.lanes.find((l) => l.id === 'ready');
    expect(ready.header).toBe('Ready to start · 3 items');
    expect(ready.status).toMatchObject({ refreshing: false, failed: 0, oldest: NOW - 40 * MIN, hidden: 0 });
    // Registered order by project, then the core's order; then infra's no-source line.
    expect(ready.rows.map((r) => r.key)).toEqual(['ready:it_1', 'ready:it_2', 'ready:it_3', 'intake:nosource']);
    expect(ready.rows[3]).toMatchObject({ nameText: 'infra', many: false });
  });

  it('does not enter any attention count or the "moving" state, and has no lane without intake', () => {
    expect(lanes(missionNow(NOW), null, NOW).lanes.some((l) => l.id === 'ready')).toBe(false);
    const now = missionNow(NOW);
    now.intake = intake([item('it_1', 'shop-api')], [source('shop-api', { count: 1 })]);
    const m = lanes(now, null, NOW, { projects });
    expect(m.lanes.find((l) => l.id === 'ready').level).toBe('ready');
  });

  it('merges no-source projects in one line and 3+ empty sources in one', () => {
    const many = [...projects, { id: 'a', name: 'a' }];
    const none4 = readyOf(
      {
        intake: intake(
          [],
          many.map((p) => none(p.id)),
        ),
      },
      null,
      NOW,
      { projects: many },
    );
    expect(none4.rows).toHaveLength(1);
    expect(none4.rows[0]).toMatchObject({ line: 'nosource', many: true, nameText: 'shop-api, blog, infra + 1 more' });
    const three = readyOf(
      {
        intake: intake(
          [],
          projects.map((p) => source(p.id)),
        ),
      },
      null,
      NOW,
      { projects },
    );
    expect(three.rows).toHaveLength(1);
    expect(three.rows[0]).toMatchObject({ line: 'empty', merged: true, count: 3 });
    const two = readyOf(
      {
        intake: intake(
          [],
          projects.slice(0, 2).map((p) => source(p.id)),
        ),
      },
      null,
      NOW,
      { projects },
    );
    expect(two.rows.map((r) => r.line)).toEqual(['empty', 'empty']);
  });

  it('puts a source error above its project items and keeps its rate limit out of the stops', () => {
    const err = { code: 'gh_auth', message: 'x', fix: 'integrations' };
    const ready = readyOf(
      { intake: intake([item('it_1', 'shop-api')], [source('shop-api', { status: 'error', error: err, count: 1 })]) },
      null,
      NOW,
      { projects },
    );
    expect(ready.rows.map((r) => r.key)).toEqual(['intake:error:shop-api', 'ready:it_1']);
    expect(ready.status.failed).toBe(1);
    const rl = readyOf(
      {
        intake: intake(
          [],
          [source('shop-api', { status: 'error', error: { code: 'rate_limited', retry_at_ms: NOW + MIN } })],
        ),
      },
      null,
      NOW,
      { projects },
    );
    expect(rl.rows[0].stop).toBe(false);
    expect(stops({ lanes: [rl], idle: null })).toEqual([]);
    expect(stops({ lanes: [ready], idle: null })).toEqual(['intake:error:shop-api', 'ready:it_1']);
  });

  it('shows "+ n more" from `more`, "Looking for items" on a first launch and hidden items on Show', () => {
    const items = [item('it_1', 'shop-api'), item('it_2', 'shop-api', { state: 'hidden' })];
    const base = { intake: intake(items, [source('shop-api', { count: 1, more: 12, hidden: 1 })]) };
    const closed = readyOf(base, null, NOW, { projects });
    expect(closed.rows.map((r) => r.key)).toEqual(['ready:it_1', 'intake:more:shop-api']);
    expect(closed.status.hidden).toBe(1);
    const open = readyOf(base, null, NOW, { projects, showHidden: true });
    expect(open.rows.map((r) => [r.key, !!r.hidden])).toEqual(
      [
        ['ready:it_1', false],
        ['ready:it_2', true],
        ['intake:more:shop-api', undefined],
      ].map(([k, h]) => [k, !!h]),
    );
    const first = readyOf(
      { intake: intake([], [source('shop-api', { status: 'refreshing', refreshed_at_ms: null })]) },
      null,
      NOW,
      { projects },
    );
    expect(first.header).toBe('Ready to start');
    expect(first.rows.map((r) => r.line)).toEqual(['looking']);
  });

  it('says every item is hidden instead of "Nothing ready"', () => {
    const m = lanes(
      {
        intake: intake([item('it_1', 'shop-api', { state: 'hidden' })], [source('shop-api', { count: 0, hidden: 3 })]),
      },
      null,
      NOW,
      { projects },
    );
    expect(m.lanes.find((l) => l.id === 'ready').rows).toEqual([
      expect.objectContaining({ line: 'allhidden', count: 3 }),
    ]);
  });

  it('follows the project filter: filtered-out projects contribute nothing, not even errors', () => {
    const now = {
      intake: intake(
        [item('it_1', 'shop-api'), item('it_2', 'blog')],
        [
          source('shop-api', { count: 1 }),
          source('blog', { status: 'error', error: { code: 'failed', message: 'x' }, count: 1 }),
        ],
      ),
    };
    const m = readyOf(now, ['shop-api'], NOW, { projects });
    expect(m.rows.map((r) => r.key)).toEqual(['ready:it_1']);
    expect(m.status.failed).toBe(0);
    expect(lanes(now, ['infra'], NOW, { projects }).lanes.some((l) => l.id === 'ready')).toBe(false);
  });

  it('gives each Ready row the keys of the notes in the hint row', () => {
    const r = readyOf({ intake: intake([item('it_1', 'shop-api')], [source('shop-api', { count: 1 })]) }, null, NOW, {
      projects,
    }).rows[0];
    expect(hints(r).map(([k]) => k)).toEqual(['j k', '⏎', 't', 'a', 'space', 'o', 'x', 'f']);
    expect(hints({ ...r, hidden: true }).map(([k]) => k)).toEqual(['j k', 'x', 'space', 'o', 'f']);
  });

  it('keeps `readyLane` empty for a core without intake', () => {
    expect(readyLane(undefined).rows).toEqual([]);
  });
});

describe('applyNow carries intake whole (api-contract §2.1)', () => {
  it('replaces `now.intake` when a delta has it, keeps it when not', () => {
    app.now = missionNow(NOW);
    app.now.rev = 5;
    app.now.intake = { items: [], sources: [] };
    applyNow({ rev: 6, intake: { items: [item('it_1', 'shop-api')], sources: [source('shop-api')] } });
    expect(app.now.intake.items).toHaveLength(1);
    applyNow({ rev: 7 });
    expect(app.now.intake.items).toHaveLength(1);
  });
});

describe('Mission control: the Ready to start lane', () => {
  function seed(extra = {}) {
    app.data = /** @type {any} */ (missionData(NOW));
    app.now = missionNow(NOW);
    app.now.intake = {
      items: [
        item('it_1', 'shop-api', { estimate: est({ over_room: true }) }),
        item('it_2', 'shop-api', { mode: 'solo', labels: ['ready', 'billing', 'urgent'] }),
        item('it_3', 'blog', {
          source: 'linear',
          ref: 'ENG-42',
          url: 'https://linear.app/x/ENG-42',
          can_team: false,
          mode: 'team',
        }),
        item('it_4', 'blog', { state: 'hidden' }),
      ],
      sources: [
        source('shop-api', { count: 2 }),
        source('blog', {
          type: 'linear',
          label: 'ENG · next',
          count: 1,
          hidden: 1,
          refreshed_at_ms: NOW - 3 * 3600000,
        }),
        none('infra'),
      ],
    };
    Object.assign(app.now.intake, extra);
    app.selection = null;
    app.mission = { projects: null };
    app.missionFilterOpen = false;
    app.overlay = null;
    app.toasts = [];
    app.panes = { mc: { type: 'mission', title: 'Mission control' } };
    app.layout = L.leaf('mc');
    app.focused = 'mc';
    intakeUi.showHidden = false;
    intakeUi.starting = null;
    intakeUi.lastHide = null;
    calls.length = 0;
    for (const k of Object.keys(replies)) delete replies[k];
  }
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'], now: NOW });
    seed();
  });
  afterEach(() => vi.useRealTimers());

  it('renders rows, lines and the header from `now` alone: no command is called', async () => {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    const lane = c.querySelector('section[aria-label="Ready to start"]');
    expect(text(lane.querySelector('.lh'))).toContain('Ready to start · 3 items');
    expect(text(lane.querySelector('.lh'))).toContain('updated 3 h ago'); // the oldest refresh (F8)
    expect(text(lane.querySelector('.lh'))).toContain('1 hidden · Show');
    const r1 = text(row(c, 'ready:it_1'));
    expect(r1).toContain('#1');
    expect(r1).toContain('Title of it_1');
    expect(r1).toContain('shop-api');
    expect(r1).toContain('≈ 4–7 pp week · team');
    expect(r1).toContain("over this week's room (3 pp left)");
    expect(r1).toContain('Start team run');
    // Labels: two chips and +n; size labels never.
    const r2 = text(row(c, 'ready:it_2'));
    expect(r2).toContain('ready');
    expect(r2).toContain('billing');
    expect(r2).toContain('+1');
    expect(r2).not.toContain('size:M');
    expect(r2).toContain('≈ 1–2 pp week · solo');
    // A Linear item whose project has no team: the team button is disabled with its reason.
    const team = row(c, 'ready:it_3').querySelector('button[aria-label^="Start team run"]');
    expect(team.disabled).toBe(true);
    expect(team.title).toBe('blog has no team yet');
    // The hidden one is not rendered; the no-source project has its line.
    expect(row(c, 'ready:it_4')).toBeNull();
    expect(text(row(c, 'intake:nosource'))).toContain('infra has no intake source');
    expect(text(row(c, 'intake:nosource'))).toContain('Plan with the Overwatch');
    expect(text(row(c, 'intake:nosource'))).toContain('Set up intake');
    // Ready is not attention: not in the footer's counts, and the "Nothing else is moving" line stays.
    expect(text(c.querySelector('.foot'))).not.toMatch(/ready/i);
    expect(calls).toEqual([]);
  });

  it('names the start buttons after the item', async () => {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    expect(c.querySelector('button[aria-label="Start team run for #1 Title of it_1"]')).toBeTruthy();
    expect(c.querySelector('button[aria-label="Start solo for #1 Title of it_1"]')).toBeTruthy();
  });

  it('t / a / ⏎ open the prefilled dialog through intake_prefill and never start', async () => {
    replies.intake_prefill = (a) => ({
      launch: {
        project_id: 'shop-api',
        team_id: 'default-web',
        slug: 'title-of-it-1',
        title: 'Title of it_1',
        brief: '# Title\n\nbody',
        base: null,
        off_hours: false,
      },
      ...(a.mode === 'solo'
        ? { launch: undefined, session: { project_id: 'shop-api', name: 'title-of-it-1', message: 'text' } }
        : {}),
    });
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    const mc = c.querySelector('.mc');
    key(mc, 'j'); // from the first stop to the first Ready row
    // Move to it_1 (the only stops before are none: no running rows beyond the fixture's).
    const sel = () => c.querySelector('.row.sel')?.getAttribute('data-key');
    for (let i = 0; i < 12 && sel() !== 'ready:it_1'; i++) {
      key(mc, 'j');
      await tick();
    }
    expect(sel()).toBe('ready:it_1');
    key(mc, 't');
    await settle();
    expect(calls.map((x) => [x.cmd, x.args])).toEqual([['intake_prefill', { item: 'it_1', mode: 'team' }]]);
    expect(app.overlay).toMatchObject({
      kind: 'launch',
      intake: { item: 'it_1' },
      origin: { kind: 'intake', ref: '#1' },
    });
    expect(intakeUi.starting).toBe('it_1');
    expect(calls.some((x) => x.cmd === 'team_launch' || x.cmd === 'agent_start')).toBe(false);
    app.overlay = null;
    intakeUi.starting = null;
    key(mc, 'a');
    await settle();
    expect(calls.at(-1)).toMatchObject({ cmd: 'intake_prefill', args: { item: 'it_1', mode: 'solo' } });
    expect(app.overlay).toMatchObject({ kind: 'new-session', intake: { item: 'it_1' } });
  });

  it('a stale item shows a toast and opens no dialog (F11)', async () => {
    replies.intake_prefill = new Error(
      JSON.stringify({ code: 'stale', message: 'already started', started: { kind: 'run', name: 'checkout' } }),
    );
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    click(c.querySelector('button[aria-label="Start team run for #1 Title of it_1"]'));
    await settle();
    expect(app.overlay).toBeNull();
    expect(app.toasts.at(-1).text).toBe('#1 was already started: checkout.');
    expect(intakeUi.starting).toBeNull();
  });

  it('x hides after the core answers, with an Undo toast; u unhides within 5 s', async () => {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    const mc = c.querySelector('.mc');
    const sel = () => c.querySelector('.row.sel')?.getAttribute('data-key');
    for (let i = 0; i < 12 && sel() !== 'ready:it_2'; i++) {
      key(mc, 'j');
      await tick();
    }
    key(mc, 'x');
    await settle();
    expect(calls.map((x) => [x.cmd, x.args])).toEqual([['intake_hide', { item: 'it_2' }]]);
    expect(app.toasts.at(-1).text).toBe('Hidden #2. It comes back if it changes at the source.');
    key(mc, 'u');
    await settle();
    expect(calls.at(-1)).toMatchObject({ cmd: 'intake_unhide', args: { item: 'it_2' } });
    expect(app.toasts.at(-1).text).toBe('#2 is back in Ready to start.');
  });

  it('⇧R refreshes each shown project with a source; the filter is respected', async () => {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    key(c.querySelector('.mc'), 'R', { shiftKey: true });
    await settle();
    expect(calls.map((x) => [x.cmd, x.args])).toEqual([
      ['intake_refresh', { project: 'shop-api' }],
      ['intake_refresh', { project: 'blog' }],
    ]);
    calls.length = 0;
    app.mission = { projects: ['blog'] };
    await settle();
    key(c.querySelector('.mc'), 'R', { shiftKey: true });
    await settle();
    expect(calls.map((x) => x.args)).toEqual([{ project: 'blog' }]);
  });

  it('⇧H toggles the hidden rows, dimmed with Unhide and no Start buttons', async () => {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    key(c.querySelector('.mc'), 'H', { shiftKey: true });
    await settle();
    const h = row(c, 'ready:it_4');
    expect(text(h)).toContain('Hidden');
    expect(text(h)).toContain('Unhide');
    expect(h.querySelector('button[aria-label^="Start"]')).toBeNull();
    expect(text(c.querySelector('section[aria-label="Ready to start"] .lh'))).toContain('Hide the 1');
  });

  it('space opens the details and loads the full text with intake_item', async () => {
    replies.intake_item = {
      item: item('it_1', 'shop-api'),
      body: 'Full body\n\n- one\n- two',
      truncated: true,
      milestone: 'v2.1',
      assignees: ['gu', 'mia'],
      priority: 'High',
    };
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    const mc = c.querySelector('.mc');
    const sel = () => c.querySelector('.row.sel')?.getAttribute('data-key');
    for (let i = 0; i < 12 && sel() !== 'ready:it_1'; i++) {
      key(mc, 'j');
      await tick();
    }
    key(mc, ' ');
    await settle();
    expect(calls.map((x) => [x.cmd, x.args])).toEqual([['intake_item', { item: 'it_1' }]]);
    const d = text(row(c, 'ready:it_1'));
    expect(d).toContain('Full body');
    expect(d).toContain('Shortened to 4 000 characters');
    expect(d).toContain('≈ 4.1–6.8 pp of the week');
    expect(d).toContain('from 7 runs in shop-api · size M');
    expect(d).toContain('from 12 sessions across projects · size M');
    expect(d).toContain('v2.1');
    expect(d).toContain('gu, mia');
    expect(d).toContain('High');
  });

  it('shows a source error above the items with Fix, and Fix goes where `fix` says', async () => {
    seed();
    app.now.intake.sources[0] = source('shop-api', {
      count: 2,
      status: 'error',
      error: { code: 'not_found', message: 'HTTP 404', fix: 'settings' },
    });
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    const lane = c.querySelector('section[aria-label="Ready to start"]');
    expect(text(lane.querySelector('.lh'))).toContain('1 source failed');
    const keys = [...lane.querySelectorAll('[data-key]')]
      .map((e) => e.getAttribute('data-key'))
      .filter((k) => !k.startsWith('lane:'));
    expect(keys.indexOf('intake:error:shop-api')).toBeLessThan(keys.indexOf('ready:it_1'));
    expect(text(row(c, 'intake:error:shop-api'))).toContain(
      'shop-api · acme/shop not found, or you have no access · Fix',
    );
    click(row(c, 'intake:error:shop-api').querySelector('.link'));
    await settle();
    expect(
      Object.values(app.panes).some(
        (p) => p.type === 'project-settings' && p.projectId === 'shop-api' && p.section === 'intake',
      ),
    ).toBe(true);
  });

  it('a collapsed lane keeps the failure visible in its header', async () => {
    seed();
    app.now.intake.sources[0] = source('shop-api', {
      status: 'error',
      error: { code: 'gh_auth', message: 'x', fix: 'integrations' },
    });
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    const head = c.querySelector('section[aria-label="Ready to start"] .lbtn');
    click(head);
    await settle();
    const lh = text(c.querySelector('section[aria-label="Ready to start"] .lh'));
    expect(lh).toContain('1 source failed');
    expect(c.querySelector('[data-key="ready:it_1"]')).toBeNull();
  });

  it('shows the origin mark on a run, a solo row, a queue row and an outcome', async () => {
    seed();
    const from = { kind: 'intake', item: 'it_9', source: 'github', ref: '#123', url: 'https://x/123' };
    app.now.runs['run-12'] = { ...app.now.runs['run-12'], origin: from };
    app.now.agents['fix-feed'] = {
      ...app.now.agents['fix-feed'],
      origin: { kind: 'brief', decision: 'd_1', title: 'Partial refunds' },
    };
    app.now.outcomes = [
      { id: 'ev-9', kind: 'merge_done', text: 'PR merged', at_ms: NOW - MIN, project: 'shop-api', origin: from },
      ...app.now.outcomes,
    ];
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    expect(text(row(c, 'run:run-12'))).toContain('from #123');
    expect(text(row(c, 'agent:fix-feed'))).toContain('from brief');
    expect(row(c, 'agent:fix-feed').querySelector('.origin').title).toBe("From the Overwatch's brief: Partial refunds");
    expect(text(c.querySelector('section[aria-label="Done today"]'))).toContain('PR merged · from #123');
  });

  it('the lane exists with no other work: the board still says nothing is running', async () => {
    seed();
    app.now.agents = {};
    app.now.runs = {};
    app.now.decisions = {};
    app.now.outcomes = [];
    app.now.counts = { global: {}, projects: {} };
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    expect(text(c)).toContain('Nothing is running');
    expect(c.querySelector('section[aria-label="Ready to start"]')).toBeTruthy();
  });
});

describe('the prefilled dialogs (notes §5)', () => {
  const launchOverlay = (more = {}) => ({
    kind: 'launch',
    prefill: {
      project_id: 'shop-api',
      team_id: 'default-web',
      slug: 'refunds',
      title: 'Refunds on partial orders',
      brief: '# Refunds\n\nSee #123',
      base: null,
      off_hours: false,
    },
    intake: { item: 'it_1' },
    origin: {
      kind: 'intake',
      item: 'it_1',
      source: 'github',
      ref: '#123',
      url: 'https://github.com/acme/shop/issues/123',
    },
    item: item('it_1', 'shop-api', { estimate: est({ over_room: true }) }),
    ...more,
  });
  beforeEach(() => {
    app.data = /** @type {any} */ (missionData(NOW));
    app.now = missionNow(NOW);
    app.toasts = [];
    calls.length = 0;
    for (const k of Object.keys(replies)) delete replies[k];
    replies.teams_view = { roles: [], teams: [{ team: { id: 'default-web', name: 'Web team' }, issues: [] }] };
    replies.mcp_registry_view = { servers: [] };
    replies.planner_view = { off_hours: null };
  });

  it('launch: shows the origin, prefilled fields, the estimate and the untrusted note, and submits `intake`', async () => {
    app.overlay = launchOverlay();
    replies.team_launch = { feature_id: 'f1' };
    const { container: c } = render(Dialogs);
    await settle();
    await settle();
    const t = text(c);
    expect(t).toContain('From #123 · GitHub');
    expect(t).toContain("≈ 4–7 pp of the week for a team run · over this week's room (3 pp left)");
    expect(t).toContain('This text comes from GitHub. It goes to the agent as part of the brief you send.');
    expect(c.querySelector('textarea').value).toContain('See #123');
    expect(c.querySelector('textarea').rows).toBe(12);
    // The primary button has the focus: the user reads the text first.
    expect(document.activeElement?.textContent).toContain('Launch');
    click(c.querySelector('button.primary'));
    await settle();
    const sent = calls.find((x) => x.cmd === 'team_launch');
    expect(sent.args.form).toMatchObject({
      project_id: 'shop-api',
      team_id: 'default-web',
      slug: 'refunds',
      title: 'Refunds on partial orders',
      intake: { item: 'it_1' },
    });
    expect(app.overlay).toBeNull();
    expect(intakeUi.starting).toBeNull();
  });

  it('launch: a stale answer keeps the dialog with a banner and Close', async () => {
    app.overlay = launchOverlay();
    replies.team_launch = new Error(
      JSON.stringify({ code: 'stale', message: 'already started', started: { kind: 'run', name: 'checkout' } }),
    );
    const { container: c } = render(Dialogs);
    await settle();
    await settle();
    click(c.querySelector('button.primary'));
    await settle();
    expect(text(c.querySelector('[data-stale]'))).toBe('Already started: checkout (team run)');
    const primary = c.querySelector('button.primary');
    expect(primary.textContent).toContain('Close');
    expect(c.querySelector('textarea').disabled).toBe(true);
    click(primary);
    await settle();
    expect(app.overlay).toBeNull();
  });

  it('new session: prefilled from the item, `intake` goes with agent_start, Cancel leaves no text behind', async () => {
    app.overlay = {
      kind: 'new-session',
      prefill: {
        project_id: 'blog',
        name: 'fix-feed-2',
        message: 'Fix the feed.\n\nWhen done, say so; the item is https://x',
      },
      intake: { item: 'it_3' },
      origin: { kind: 'intake', item: 'it_3', source: 'linear', ref: 'ENG-42', url: 'https://linear.app/x/ENG-42' },
      item: item('it_3', 'blog', { source: 'linear', ref: 'ENG-42' }),
    };
    replies.agent_start = 'fix-feed-2';
    const { container: c } = render(Dialogs);
    await settle();
    await settle();
    expect(text(c)).toContain('From ENG-42 · Linear');
    expect(text(c)).toContain('This text comes from Linear.');
    expect(c.querySelector('textarea').value).toContain('Fix the feed.');
    click(c.querySelector('button.primary'));
    await settle();
    const sent = calls.find((x) => x.cmd === 'agent_start');
    expect(sent.args.req).toMatchObject({ project_id: 'blog', name: 'fix-feed-2', intake: { item: 'it_3' } });
    expect(sent.args.req.message).toContain('Fix the feed.');
  });

  it('cancelling a prefilled session leaves none of the item text for the next plain one', async () => {
    app.overlay = {
      kind: 'new-session',
      prefill: { project_id: 'blog', name: 'fix-feed-2', message: 'Fix the feed.' },
      intake: { item: 'it_3' },
      item: item('it_3', 'blog'),
    };
    const { container: c } = render(Dialogs);
    await settle();
    await settle();
    expect(c.querySelector('textarea').value).toBe('Fix the feed.');
    click([...c.querySelectorAll('button')].find((b) => b.textContent.includes('Cancel')));
    await settle();
    app.overlay = { kind: 'new-session' };
    await settle();
    await settle();
    expect(c.querySelector('textarea').value).toBe('');
  });

  it('a brief opens without the untrusted-text note, with the brief chip, and Edit focuses the brief', async () => {
    app.overlay = {
      kind: 'launch',
      prefill: {
        project_id: 'shop-api',
        team_id: 'default-web',
        slug: 'partial-refunds',
        title: 'Partial refunds',
        brief: '## Goal',
        base: null,
        off_hours: false,
      },
      intake: { brief_decision: 'd_88' },
      origin: { kind: 'brief', decision: 'd_88', title: 'Partial refunds' },
      focus: 'brief',
    };
    const { container: c } = render(Dialogs);
    await settle();
    await settle();
    expect(text(c)).toContain("From the Overwatch's brief · Partial refunds");
    expect(text(c)).not.toContain('This text comes from');
    expect(document.activeElement?.tagName).toBe('TEXTAREA');
  });

  it('cancel clears the row\'s "Starting…" tag', async () => {
    intakeUi.starting = 'it_1';
    app.overlay = launchOverlay();
    const { container: c } = render(Dialogs);
    await settle();
    await settle();
    click([...c.querySelectorAll('button')].find((b) => b.textContent.includes('Cancel')));
    await settle();
    expect(app.overlay).toBeNull();
    expect(intakeUi.starting).toBeNull();
    flushSync();
  });
});
