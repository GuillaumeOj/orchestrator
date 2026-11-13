// improve-intake F5 (notes §6–§7, §12.2–§12.4, api-contract §4): programs. The pure rules of
// lib/program.js; the plan dialog (edits, issues and fixes, submit, states); the Programs lane and its
// row; the Program view and its actions; program cards, the spec / PR tag and "next spec".

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
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

const { app } = await import('../src/lib/app.svelte.js');
const A = await import('../src/lib/app.svelte.js');
const P = await import('../src/lib/program.js');
const { lanes } = await import('../src/lib/mission.js');
const { applyNow } = await import('../src/lib/now.svelte.js');
const { keyIntent, intentFor, pendingToast, effectText } = await import('../src/lib/inbox.js');
const { decisionKind } = await import('../src/lib/status.js');
const { originText, staleOf } = await import('../src/lib/intake.js');
const { openProgram, openProgramPlan } = await import('../src/lib/work.js');
const { default: ProgramPlanDialog } = await import('../src/components/ProgramPlanDialog.svelte');
const { default: MissionPane } = await import('../src/panes/MissionPane.svelte');
const { default: ProgramPane } = await import('../src/panes/ProgramPane.svelte');
const { default: DecisionCard } = await import('../src/components/chat/DecisionCard.svelte');
const { missionData, missionNow } = await import('./mission-fixture.js');

const NOW = Date.now();
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const key = (el, k, more = {}) => fireEvent.keyDown(el, { key: k, ...more });
const called = (cmd) => calls.filter((c) => c.cmd === cmd).map((c) => c.args);
const settle = async () => {
  for (let i = 0; i < 4; i++) {
    await tick();
    await Promise.resolve();
  }
  await tick();
};

// ---------------------------------------------------------------- fixtures (notes §0.2)

const N = (n) => `gh:acme/shop#${n}`;
const feature = (key, items, o = {}) => ({
  key,
  items,
  slug: o.slug ?? key,
  title: o.title ?? key,
  mode: 'team',
  team: 'default-web',
  brief: `# ${o.title ?? key}`,
  base: 'main',
  depends_on: [],
  skip: false,
  estimate: { low_pp: 4, high_pp: 7, basis: 'project', rough: false },
  state: 'planned',
  phase: null,
  run: null,
  agent: null,
  pr: null,
  merged_at_ms: null,
  actual_pp: null,
  flags: [],
  ...o,
});
/** Program Checkout v2 (draft): pf-1 address (#13 +2), pf-2 payment (#14 +1, after pf-1), pf-3 summary (#15, solo). */
function checkout(o = {}) {
  return {
    id: 'prg-draft',
    project: 'shop-api',
    title: 'Checkout v2',
    root: [N(12)],
    root_ref: '#12',
    status: 'draft',
    grouping: 'child',
    max_parallel: 2,
    sequential: false,
    features: [
      feature('pf-1', [N(13), N(16), N(17)], { slug: 'checkout-address', title: 'Address step' }),
      feature('pf-2', [N(14), N(18)], {
        slug: 'checkout-payment',
        title: 'Payment step',
        depends_on: ['pf-1'],
        estimate: { low_pp: 5, high_pp: 8, basis: 'project' },
      }),
      feature('pf-3', [N(15)], {
        slug: 'checkout-summary',
        title: 'Order summary',
        mode: 'solo',
        team: null,
        estimate: { low_pp: 1, high_pp: 2, basis: 'global' },
      }),
    ],
    edges: [{ from: 'pf-1', to: 'pf-2', source: 'tracker', items: [N(13), N(14)] }],
    cut: [],
    new_items: [],
    estimate: { low_pp: 10, high_pp: 17, room_pp: 12, room_scope: 'week', over_room: 'high' },
    left_out: { closed: [], running: [] },
    created_at_ms: NOW,
    started_at_ms: null,
    done_at_ms: null,
    ...o,
  };
}
const OVER = { kind: 'over_room', blocking: false, features: [], items: [], feature: null, edges: [], message: '' };
const TRACKER = {
  settings: {
    tracker: { kind: 'github', repo: 'acme/shop', project_board: null },
    write_back: { enabled: true, running_label: 'orch:running' },
  },
};
const TEAMS = { teams: [{ team: { id: 'default-web', name: 'default-web' }, issues: [] }], roles: [] };
const ROOT = {
  id: N(12),
  project: 'shop-api',
  kind: 'issue',
  ref: '#12',
  title: 'Checkout v2',
  url: 'https://github.com/acme/shop/issues/12',
  upstream: { state: 'open' },
  breadcrumb: [{ id: 'gh-milestone:acme/shop/3', ref: 'v2', title: 'v2' }],
  child_nodes: [{ id: N(13), children: { total: 2 } }],
};

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = /** @type {any} */ (missionData(NOW));
  app.now = missionNow(NOW);
  app.overlay = null;
  app.toasts = [];
  app.panes = {};
});
afterEach(() => cleanup());

// ---------------------------------------------------------------- lib/program.js

describe('plan rules (§6.2, §6.4)', () => {
  it('order text, its note and tooltip; refs from node ids', () => {
    const p = checkout();
    expect(P.refOfId(N(13))).toBe('#13');
    expect(P.refOfId('lin:ENG-42')).toBe('ENG-42');
    expect(P.refOfId('local:dark-mode.md')).toBe('dark-mode.md');
    expect(P.orderText(p, p.features[0])).toBe('first');
    expect(P.orderText(p, p.features[1])).toBe('after 1');
    expect(P.orderNote(p, p.features[1])).toBe('⛓ #13 blocks #14');
    expect(P.orderTip(p, p.features[1])).toBe('#13 blocks #14 in GitHub');
    expect(P.fromText(p.features[0])).toBe('#13 +2');
    expect(P.fromText(p.features[2])).toBe('#15');
    const seq = checkout({ edges: [{ from: 'pf-1', to: 'pf-2', source: 'sequential', items: [] }] });
    expect(P.orderNote(seq, seq.features[1])).toBe('Sequential');
  });

  it('a dependency on a left-out running item (B5 external edge): "after #13 merges"', () => {
    const ext = { from: 'x:run:run-elsewhere', to: 'pf-2', source: 'external', items: [N(13), N(14)] };
    const p = checkout({ edges: [ext] });
    expect(P.orderText(p, p.features[1])).toBe('after #13 merges');
    expect(P.orderNote(p, p.features[1])).toBe('⛓ #13 blocks #14');
    expect(P.orderTip(p, p.features[1])).toBe('#13 already runs outside this program: waits for its merge');
    const both = checkout({
      edges: [
        { from: 'pf-1', to: 'pf-3', source: 'tracker', items: [N(13), N(15)] },
        { ...ext, to: 'pf-3' },
      ],
    });
    expect(P.orderText(both, both.features[2])).toBe('after 1, #13 merges');
    // Cut like any edge.
    expect(P.orderText(P.cutEdge(p, ext.from, 'pf-2'), p.features[1])).toBe('first');
  });

  it('skip: the total drops, dependents note the skip, the label counts what starts', () => {
    const p = P.toggleSkip(checkout(), 'pf-1');
    expect(p.features[0].skip).toBe(true);
    expect(P.orderText(p, p.features[0])).toBe('first (skipped)');
    expect(P.orderText(p, p.features[1])).toBe('after 1 (skipped)');
    expect(p.estimate).toMatchObject({ low_pp: 6, high_pp: 10, over_room: 'none' });
    expect(P.startLabel(p)).toBe('Start program · 2 features');
    expect(P.footLine(p)).toBe('Skipped: checkout-address · Left out: none');
  });

  it('merge into previous: items and brief join, edges follow, self-edges drop', () => {
    const p = P.mergeIntoPrevious(checkout(), 'pf-2');
    expect(p.features.map((f) => f.key)).toEqual(['pf-1', 'pf-3']);
    expect(p.features[0].items).toEqual([N(13), N(16), N(17), N(14), N(18)]);
    expect(p.features[0].brief).toContain('# Payment step');
    expect(p.features[0].estimate).toMatchObject({ low_pp: 9, high_pp: 15 });
    expect(p.edges).toEqual([]);
    expect(P.mergeIntoPrevious(checkout(), 'pf-1').features).toHaveLength(3); // no row above
  });

  it('a loop is a blocking issue until an edge is cut; no team until Make it solo', () => {
    const loop = checkout({
      edges: [
        { from: 'pf-2', to: 'pf-3', source: 'tracker', items: [N(14), N(15)] },
        { from: 'pf-3', to: 'pf-2', source: 'tracker', items: [N(15), N(14)] },
      ],
    });
    let list = P.liveIssues(loop, [], ['default-web']);
    expect(list.map((i) => i.kind)).toEqual(['cycle', 'over_room']);
    expect(P.issuesHeader(list)).toBe('1 thing to fix before starting');
    expect(P.issueText(list[0], loop, { tracker: 'GitHub' })).toBe(
      'Order loop: checkout-payment and checkout-summary wait on each other (#14 ↔ #15 in GitHub).',
    );
    expect(P.issueFixes(list[0]).map((f) => f.label)).toEqual(['Remove a dependency…', 'Use tree order']);
    expect([...P.flaggedKeys(list)].sort()).toEqual(['pf-2', 'pf-3']);
    const cut = P.cutEdge(loop, 'pf-3', 'pf-2');
    list = P.liveIssues(cut, [], ['default-web']);
    expect(list.map((i) => i.kind)).toEqual(['over_room']);
    expect(P.issuesHeader(list)).toBe('Nothing to fix · 1 note');
    expect(P.orderText(cut, cut.features[1])).toBe('first');
    // No team in the project: both team rows are blocking; Make it solo fixes one.
    list = P.liveIssues(checkout(), [], []);
    expect(list.filter((i) => i.kind === 'no_team').map((i) => i.feature)).toEqual([
      'checkout-address',
      'checkout-payment',
    ]);
    expect(P.issueText(list[0], checkout(), { project: 'shop-api' })).toBe(
      'checkout-address needs a team: shop-api has no team.',
    );
    const solo = P.setMode(checkout(), 'pf-1', 'solo');
    expect(P.liveIssues(solo, [], []).filter((i) => i.kind === 'no_team')).toHaveLength(1);
  });

  it('advisory issues: already running, closed, over the room (high / low end)', () => {
    const p = checkout();
    const run = {
      kind: 'already_running',
      blocking: false,
      items: [N(13)],
      feature: 'checkout-address',
      features: [],
      edges: [],
    };
    expect(P.issueText(run, p)).toBe(
      '#13 is already running as checkout-address. It is left out; features that depend on it wait for its merge.',
    );
    expect(P.issueFixes(run)[0]).toMatchObject({ id: 'feature', label: 'Show feature', key: 'g' });
    expect(P.issueText({ kind: 'closed', items: [N(16)] }, p, { tracker: 'GitHub' })).toBe(
      '#16 is closed in GitHub. It is left out.',
    );
    expect(P.issueText(OVER, p, { project: 'shop-api' })).toBe(
      "≈ 10–17 pp is over this week's room for shop-api at the high end (12 pp left). Features wait for room under the Governor.",
    );
    const low = checkout({ estimate: { ...checkout().estimate, room_pp: 8, over_room: 'low' } });
    expect(P.issueText(OVER, low, { project: 'shop-api' })).toBe(
      "≈ 10–17 pp is over this week's room for shop-api (8 pp left). Features wait for room under the Governor.",
    );
    expect(P.totalsParts(p, 'shop-api')).toEqual({
      total: 'Total ≈ 10–17 pp of the week',
      room: '12 pp left this week for shop-api',
      over: 'over at the high end',
    });
    expect(P.totalsParts(low, 'shop-api').over).toBe("over this week's room");
    const lo = checkout({ left_out: { closed: [N(16)], running: [{ id: N(13), feature: 'checkout-address' }] } });
    expect(P.footLine(lo)).toBe('Closed sub-issues left out: #16 · Already running, left out: #13 (checkout-address)');
  });

  it('write-back line names the per-feature start (D4) and the root closure', () => {
    expect(P.writeBackLine(TRACKER.settings, '#12')).toBe(
      'When each feature starts, its item gets the label orch:running and a comment in GitHub. #12 closes when all are done.',
    );
    expect(P.writeBackLine(TRACKER.settings, null)).toBe(
      'When each feature starts, its item gets the label orch:running and a comment in GitHub.',
    );
    const linear = { tracker: { kind: 'linear', teams: ['ENG'] }, write_back: { enabled: true } };
    expect(P.writeBackLine(linear, 'Mobile onboarding')).toBe(
      'When each feature starts, its item moves to a started state and gets a comment in Linear. Mobile onboarding moves to Done when all are done.',
    );
    expect(P.writeBackLine({ ...TRACKER.settings, write_back: { enabled: false } }, '#12')).toBe('');
    expect(P.writeBackLine({ tracker: { kind: 'local' }, write_back: { enabled: true } }, 'x')).toBe('');
  });

  it('graph strip: hidden without edges; levels by order; skipped dashed', () => {
    expect(P.graphOf(checkout({ edges: [] }))).toBeNull();
    const g = P.graphOf(P.toggleSkip(checkout(), 'pf-3'));
    expect(g.edges).toEqual([{ from: 'pf-1', to: 'pf-2', fromN: 1, toN: 2 }]);
    expect(g.nodes.map((n) => [n.n, n.level, n.skipped])).toEqual([
      [1, 0, false],
      [2, 1, false],
      [3, 2, true],
    ]);
  });

  it('a stale refusal is read from the error text', () => {
    const s = staleOf(new Error('{"code":"stale","message":"already in program Checkout v2","program":"prg-1"}'));
    expect(s).toMatchObject({ code: 'stale', program: 'prg-1' });
    expect(P.staleText(s, '#12')).toBe('#12 is already in program Checkout v2.');
    expect(staleOf(new Error('boom'))).toBeNull();
  });
});

// ---------------------------------------------------------------- lane rows (§7.1)

/** A ProgramRow (contract §4) as `now.programs` carries it. */
function row(o = {}) {
  return {
    id: 'prg-1',
    project: 'shop-api',
    title: 'Checkout v2',
    root_ref: '#12',
    status: 'running',
    counts: { merged: 1, running: 1, pr: 1, waiting: 1, queued: 1, failed: 0, skipped: 0 },
    estimate: { low_pp: 10, high_pp: 17 },
    features: [
      {
        key: 'pf-1',
        slug: 'checkout-address',
        title: 'Address step',
        state: 'merged',
        mode: 'team',
        team: 'default-web',
        pr: { number: 64 },
        flags: [],
      },
      {
        key: 'pf-2',
        slug: 'checkout-payment',
        title: 'Payment step',
        state: 'running',
        mode: 'team',
        team: 'default-web',
        phase: 'executing',
        flags: [],
      },
      {
        key: 'pf-4',
        slug: 'checkout-wallets',
        title: 'Wallets',
        state: 'pr',
        mode: 'team',
        team: 'default-web',
        pr: { number: 66 },
        flags: [],
      },
      {
        key: 'pf-5',
        slug: 'checkout-cards',
        title: 'Cards',
        state: 'waiting',
        mode: 'team',
        team: 'default-web',
        waiting_on: ['checkout-address'],
        flags: ['new_dependency'],
      },
      {
        key: 'pf-3',
        slug: 'checkout-summary',
        title: 'Order summary',
        state: 'queued',
        mode: 'solo',
        team: null,
        flags: [],
      },
    ],
    ...o,
  };
}

describe('Programs lane rules (§7.1)', () => {
  it('counts in the shared word order, only non-zero; failed is error text', () => {
    expect(
      P.countParts(row())
        .map((x) => x.text)
        .join(' · '),
    ).toBe('1 of 5 merged · 1 running · 1 PR open · 1 waiting · 1 queued');
    const f = row({ counts: { merged: 1, running: 0, pr: 0, waiting: 0, queued: 0, failed: 1, skipped: 1 } });
    f.features[4].state = 'skipped';
    expect(
      P.countParts(f)
        .map((x) => x.text)
        .join(' · '),
    ).toBe('1 of 4 merged · 1 failed · 1 skipped');
    expect(P.countParts(f).find((x) => x.key === 'failed').error).toBe(true);
    // The 1280 rule: "1/5 merged · 1 running · 3 more"; two counts or fewer stay whole.
    const short = (r) =>
      P.countParts(r, { short: true })
        .map((x) => x.text)
        .join(' · ');
    expect(short(row())).toBe('1/5 merged · 1 running · 3 more');
    expect(short(f)).toBe('1/4 merged · 1 failed · 1 skipped');
    expect(P.progressOf(row())).toEqual([true, false, false, false, false]);
  });

  it('sub-rows per state, with flags', () => {
    const [m, r, pr, w, q] = row().features;
    expect(P.subRowOf(m)).toMatchObject({ glyph: '✓', text: 'merged #64', tone: 'muted' });
    expect(P.subRowOf(r)).toMatchObject({ glyph: '●', text: 'executing · default-web' });
    expect(P.subRowOf(pr)).toMatchObject({ glyph: '◐', text: 'PR #66 open' });
    expect(P.subRowOf(w)).toMatchObject({
      glyph: '○',
      text: 'waiting for checkout-address to merge',
      flag: 'new dependency',
    });
    expect(P.subRowOf(q, 'GitHub', { liveCap: '2/2' })).toMatchObject({ text: 'queued · live cap 2/2' });
    expect(P.subRowOf({ ...q, state: 'failed' })).toMatchObject({ glyph: '✕', tone: 'error', open: true });
    expect(P.subRowOf({ ...q, state: 'skipped' })).toMatchObject({ glyph: '–', text: 'skipped' });
    expect(P.subRowOf({ ...r, flags: ['closed_upstream'] }, 'GitHub').flag).toBe('closed in GitHub');
  });

  it('the lane sits between Running and Waiting on, filtered by project; header counts running and paused', () => {
    const now = {
      ...missionNow(NOW),
      programs: { 'prg-1': row(), 'prg-2': row({ id: 'prg-2', title: 'Blog', project: 'blog', status: 'paused' }) },
    };
    const ids = lanes(now, null, NOW).lanes.map((l) => l.id);
    expect(ids.indexOf('programs')).toBeGreaterThan(ids.indexOf('needs_you'));
    expect(ids.indexOf('programs')).toBeLessThan(ids.indexOf('done'));
    // Between Running and Waiting on whenever those lanes show (the lane order of notes §7.1).
    const all = lanes({ ...now, runs: {}, agents: {} }, null, NOW).lanes.map((l) => l.id);
    expect(all).toEqual(['needs_you', 'programs', 'done'].filter((x) => all.includes(x)));
    const lane = lanes(now, null, NOW).lanes.find((l) => l.id === 'programs');
    expect(lane.header).toBe('Programs · 1 running · 1 paused');
    expect(lane.rows.map((r) => r.key)).toEqual(['program:prg-2', 'program:prg-1']);
    expect(lanes(now, ['shop-api'], NOW).lanes.find((l) => l.id === 'programs').rows).toHaveLength(1);
    expect(lanes({ ...now, programs: {} }, null, NOW).lanes.some((l) => l.id === 'programs')).toBe(false);
  });

  it('now.programs: whole in the snapshot, upsert / remove in deltas', () => {
    app.now = { ...missionNow(NOW), rev: 5, programs: {} };
    applyNow({ rev: 6, programs: { upsert: [row()], remove: [] } });
    expect(app.now.programs['prg-1'].title).toBe('Checkout v2');
    applyNow({ rev: 7, programs: { upsert: [], remove: ['prg-1'] } });
    expect(app.now.programs).toEqual({});
  });

  it('origin chips name the program', () => {
    expect(originText({ kind: 'intake', ref: '#14', program: { id: 'prg-1', title: 'Checkout v2' } })).toBe(
      'from #14 · Checkout v2',
    );
  });
});

// ---------------------------------------------------------------- the plan dialog

/** Opens the dialog on #12 with a plan reply (a function of the args, or a value). */
/**
 * @param {any} [plan]
 * @param {any} [teams]
 */
async function openPlan(plan = { program: checkout(), issues: [OVER] }, teams = TEAMS) {
  replies.program_plan = plan;
  replies.work_node = ROOT;
  replies.teams_view = teams;
  replies.tracker_settings = TRACKER;
  openProgramPlan([N(12)], { title: '#12 Checkout v2' });
  const r = render(ProgramPlanDialog);
  await settle();
  const dlg = /** @type {HTMLElement} */ (document.body.querySelector('[data-plan-phase]'));
  return { ...r, dlg };
}
const rows = () => [...document.body.querySelectorAll('[data-row]')];
const startBtn = () => /** @type {HTMLButtonElement} */ (document.body.querySelector('[data-start]'));

describe('plan dialog (§6)', () => {
  it('"in …" on the From line only under an issue; "Start program" while the first plan loads', async () => {
    replies.program_plan = () => new Promise(() => {});
    replies.work_node = {
      ...ROOT,
      id: N(13),
      ref: '#13',
      title: 'Address step',
      breadcrumb: [{ id: N(12), ref: '#12', title: 'Checkout v2' }],
    };
    replies.teams_view = TEAMS;
    replies.tracker_settings = TRACKER;
    openProgramPlan([N(13)], { title: '#13 Address step' });
    render(ProgramPlanDialog);
    await settle();
    expect(text(document.body.querySelector('[data-from]'))).toContain(
      'From #13 Address step · in #12 Checkout v2 · acme/shop · GitHub',
    );
    expect(text(startBtn())).toBe('Start program');
  });

  it('plans on open and shows rows, order, issues, totals, gates and write-back lines', async () => {
    const { dlg } = await openPlan();
    expect(called('program_plan')[0]).toEqual({
      roots: [N(12)],
      grouping: 'child',
      opts: { sequential: false, max_parallel: 2, cut: [] },
    });
    expect(dlg.dataset.planPhase).toBe('planned');
    expect(text(dlg.querySelector('[data-from]'))).toContain('From #12 Checkout v2 · acme/shop · GitHub');
    expect(rows()).toHaveLength(3);
    expect(text(rows()[0])).toContain('checkout-address');
    expect(text(rows()[0])).toContain('#13 +2');
    expect(text(rows()[0])).toContain('≈ 4–7 pp');
    expect(text(rows()[1])).toContain('after 1');
    expect(text(rows()[1])).toContain('⛓ #13 blocks #14');
    expect(text(dlg.querySelector('[data-issues]'))).toContain('Nothing to fix · 1 note');
    expect(text(dlg.querySelector('[data-totals]'))).toBe(
      'Total ≈ 10–17 pp of the week · 12 pp left this week for shop-api · over at the high end',
    );
    expect(text(dlg)).toContain('Each feature still asks you to approve its spec and its PR.');
    expect(text(dlg.querySelector('[data-writeback]'))).toBe(
      'When each feature starts, its item gets the label orch:running and a comment in GitHub. #12 closes when all are done.',
    );
    expect(text(dlg.querySelector('[data-foot]'))).toBe('Skipped: none · Left out: none');
    expect(dlg.querySelector('[data-graph]')).toBeTruthy();
    expect(text(startBtn())).toBe('Start program · 3 features');
    expect(startBtn().disabled).toBe(false);
  });

  it('keys: j/k rows, x skips, m toggles mode, ⇧M merges, e edits the brief (⌘⏎ done)', async () => {
    const { dlg } = await openPlan();
    key(dlg, 'x'); // row 1 skipped
    await settle();
    expect(rows()[0].classList.contains('skipped')).toBe(true);
    expect(text(startBtn())).toBe('Start program · 2 features');
    expect(text(rows()[1])).toContain('after 1 (skipped)');
    expect(text(dlg.querySelector('[data-foot]'))).toBe('Skipped: checkout-address · Left out: none');
    key(dlg, 'x'); // unskip
    key(dlg, 'j');
    key(dlg, 'j');
    key(dlg, 'm'); // row 3: solo → team
    await settle();
    expect(text(rows()[2])).toContain('Team');
    key(dlg, 'k');
    key(dlg, 'M', { shiftKey: true }); // row 2 into row 1
    await settle();
    expect(rows()).toHaveLength(2);
    expect(text(rows()[0])).toContain('#13 +4');
    expect(dlg.querySelector('[data-graph]')).toBeNull(); // no edge left
    key(dlg, 'e');
    await settle();
    const ta = /** @type {HTMLTextAreaElement} */ (document.body.querySelector('[data-brief-editor]'));
    expect(ta.value).toContain('# Address step');
    await fireEvent.input(ta, { target: { value: 'My brief' } });
    key(ta, 'Enter', { metaKey: true });
    await settle();
    expect(document.body.querySelector('[data-brief-editor]')).toBeNull();
    // Start sends the edited draft as is.
    replies.program_start = (a) => ({ program: { ...a.program, id: 'prg-9', status: 'running' } });
    key(dlg, 'Enter', { metaKey: true });
    await settle();
    const sent = called('program_start')[0].program;
    expect(sent.features.map((f) => f.key)).toEqual(['pf-1', 'pf-3']);
    expect(sent.features[0].brief).toBe('My brief');
    expect(sent.features[1].mode).toBe('team');
    expect(sent.max_parallel).toBe(2);
    expect(app.overlay).toBeNull();
    expect(app.toasts.at(-1).text).toBe('Started Checkout v2 · 2 features');
  });

  it('a skipped row is not merged into the row above', async () => {
    const { dlg } = await openPlan();
    key(dlg, 'j');
    key(dlg, 'x'); // row 2 skipped
    key(dlg, 'M', { shiftKey: true });
    await settle();
    expect(rows()).toHaveLength(3);
  });

  it('a row is not merged into a skipped previous row', async () => {
    const { dlg } = await openPlan();
    key(dlg, 'x'); // row 1 skipped
    key(dlg, 'j');
    key(dlg, 'M', { shiftKey: true });
    await settle();
    expect(rows()).toHaveLength(3);
  });

  it('a loop blocks Start; Remove a dependency… cuts one edge and Start comes back', async () => {
    const loop = checkout({
      edges: [
        { from: 'pf-2', to: 'pf-3', source: 'tracker', items: [N(14), N(15)] },
        { from: 'pf-3', to: 'pf-2', source: 'tracker', items: [N(15), N(14)] },
      ],
    });
    const { dlg } = await openPlan({
      program: loop,
      issues: [
        {
          kind: 'cycle',
          blocking: true,
          features: ['pf-2', 'pf-3'],
          items: [],
          edges: [
            ['pf-2', 'pf-3'],
            ['pf-3', 'pf-2'],
          ],
          message: '',
        },
      ],
    });
    expect(text(dlg.querySelector('[data-issues] b'))).toBe('1 thing to fix before starting');
    expect(startBtn().disabled).toBe(true);
    /** @type {HTMLElement} */ (dlg.querySelector('[data-fix="cut"]')).click();
    await settle();
    const pop = dlg.querySelector('[data-cut]');
    expect(text(pop)).toContain('3 after 2 (#14 blocks #15)');
    expect(text(pop)).toContain('2 after 3 (#15 blocks #14)');
    /** @type {HTMLElement} */ ([...pop.querySelectorAll('button')].find((b) => text(b) === 'Apply')).click();
    await settle();
    expect(dlg.querySelector('[data-issue="cycle"]')).toBeNull();
    expect(startBtn().disabled).toBe(false);
  });

  it('closed item: Include anyway adds it to include, and program_start sends it', async () => {
    const closed = {
      kind: 'closed',
      blocking: false,
      items: [N(16)],
      features: [],
      feature: null,
      edges: [],
      message: '',
    };
    const { dlg } = await openPlan({ program: checkout(), issues: [closed] });
    expect(text(dlg.querySelector('[data-issue="closed"]'))).toContain('It is left out.');
    /** @type {HTMLElement} */ (dlg.querySelector('[data-fix="include"]')).click();
    await settle();
    expect(text(dlg.querySelector('[data-issue="closed"]'))).toContain('It stays in its feature.');
    expect(dlg.querySelector('[data-fix="include"]')).toBeNull();
    replies.program_start = (a) => ({ program: { ...a.program, id: 'prg-9', status: 'running' } });
    key(dlg, 'Enter', { metaKey: true });
    await settle();
    expect(called('program_start')[0].program.include).toEqual([N(16)]);
  });

  it('no team: Make it solo fixes the row; Create a team opens the canvas', async () => {
    const { dlg } = await openPlan(undefined, { teams: [] });
    expect(startBtn().disabled).toBe(true);
    expect(text(dlg.querySelector('[data-issues] b'))).toBe('2 things to fix before starting');
    /** @type {HTMLElement} */ (dlg.querySelector('[data-fix="solo"]')).click();
    await settle();
    expect(text(dlg.querySelector('[data-issues] b'))).toBe('1 thing to fix before starting');
    /** @type {HTMLElement} */ (dlg.querySelector('[data-fix="team"]')).click();
    await settle();
    expect(app.overlay).toBeNull();
    expect(Object.values(app.panes).some((s) => s.type === 'canvas')).toBe(true);
  });

  it('program_start returns new issues: the dialog stays with them, no toast', async () => {
    const { dlg } = await openPlan();
    replies.program_start = {
      issues: [
        {
          kind: 'cycle',
          blocking: true,
          features: ['pf-2', 'pf-3'],
          items: [],
          edges: [['pf-2', 'pf-3']],
          message: '',
        },
      ],
    };
    /** @type {HTMLElement} */ (startBtn()).click();
    await settle();
    expect(app.overlay?.kind).toBe('program-plan');
    expect(app.toasts).toEqual([]);
    // The client re-checks loops on the draft: this one has no loop, so only advisories remain…
    expect(dlg.querySelector('[data-issues]')).toBeTruthy();
  });

  it('changing Split into after edits asks to re-plan; Esc after edits asks to discard', async () => {
    const { dlg } = await openPlan();
    key(dlg, 'x');
    await settle();
    const per = /** @type {HTMLInputElement} */ (dlg.querySelector('input[name="grouping"][value="single"]'));
    await fireEvent.change(per);
    await settle();
    expect(text(document.body.querySelector('[data-ask]'))).toContain('Re-plan and lose your edits to 1 feature?');
    /** @type {HTMLElement} */ (
      [...document.body.querySelectorAll('[data-ask] button')].find((b) => text(b) === 'Re-plan')
    ).click();
    await settle();
    expect(called('program_plan').at(-1)).toMatchObject({ grouping: 'single' });
    key(dlg, 'x');
    await settle();
    key(dlg, 'Escape');
    await settle();
    expect(text(document.body.querySelector('[data-ask]'))).toContain('Discard this plan?');
  });

  it('sequential forces At once to 1 and re-plans', async () => {
    const { dlg } = await openPlan();
    await fireEvent.change(/** @type {HTMLInputElement} */ ([...dlg.querySelectorAll('input[name="order"]')][1]));
    await settle();
    expect(called('program_plan').at(-1).opts).toMatchObject({ sequential: true });
    expect(text(dlg)).toContain('One after the other runs one at a time');
  });

  it('states: plan failed with Retry; stale with Open program; nothing to plan', async () => {
    let { dlg } = await openPlan(new Error('GitHub timed out.'));
    expect(text(dlg.querySelector('[data-plan-failed]'))).toContain(
      'Could not plan #12 Checkout v2: GitHub timed out.',
    );
    replies.program_plan = { program: checkout(), issues: [] };
    /** @type {HTMLElement} */ (dlg.querySelector('[data-plan-failed] button')).click();
    await settle();
    expect(dlg.dataset.planPhase).toBe('planned');
    cleanup();

    ({ dlg } = await openPlan(
      new Error('{"code":"stale","message":"already in program Checkout v2","program":"prg-1"}'),
    ));
    expect(text(dlg.querySelector('[data-stale]'))).toContain('#12 is already in program Checkout v2.');
    expect(startBtn().disabled).toBe(true);
    /** @type {HTMLElement} */ (
      [...dlg.querySelectorAll('[data-stale] button')].find((b) => text(b) === 'Open program')
    ).click();
    await settle();
    expect(Object.values(app.panes).some((s) => s.type === 'program' && s.programId === 'prg-1')).toBe(true);
    cleanup();

    ({ dlg } = await openPlan({
      program: checkout({ features: [], left_out: { closed: [N(13), N(14)], running: [{ id: N(15) }] } }),
      issues: [],
    }));
    expect(text(dlg.querySelector('[data-nothing]'))).toBe('Nothing left to run under #12: 2 closed, 1 running.');
  });
});

// ---------------------------------------------------------------- Mission control (§7.1, §12.2)

describe('Programs lane in Mission control', () => {
  async function mount(r = row()) {
    app.now.programs = { [r.id]: r };
    app.panes = { mc: { type: 'mission' } };
    const c = render(MissionPane, { paneId: 'mc' }).container;
    await settle();
    const mc = c.querySelector('.mc');
    for (let i = 0; i < 20 && c.querySelector('.row.sel')?.getAttribute('data-key') !== `program:${r.id}`; i++) {
      key(mc, 'j');
      await tick();
    }
    return { c, mc };
  }

  it('the row: title, origin, counts, sub-rows (≤ 5 open), estimate', async () => {
    const { c } = await mount();
    const lane = c.querySelector('section[aria-label="Programs"]');
    expect(text(lane.querySelector('.lh'))).toContain('Programs · 1 running');
    // The program glyph, in the running colour (not ▲).
    expect(text(lane.querySelector('.lh'))).toContain('\u{F0328}');
    expect(text(lane.querySelector('.lh'))).not.toContain('▲');
    const r = lane.querySelector('[data-program="prg-1"]');
    expect(text(r)).toContain('Checkout v2');
    expect(text(r)).toContain('from #12 · shop-api');
    expect(text(r.querySelector('[data-counts]'))).toBe('1 of 5 merged · 1 running · 1 PR open · 1 waiting · 1 queued');
    expect(text(r)).toContain('≈ 10–17 pp week');
    expect(r.querySelectorAll('[data-feature]')).toHaveLength(5);
    expect(text(r.querySelector('[data-feature="pf-5"]'))).toBe(
      '○ checkout-cards · waiting for checkout-address to merge · new dependency',
    );
  });

  it('more than 5 features: collapsed with "6 features ▸"; space expands', async () => {
    const big = row({
      features: [...row().features, { key: 'pf-6', slug: 'six', state: 'queued', mode: 'solo', flags: [] }],
    });
    const { c, mc } = await mount(big);
    expect(c.querySelectorAll('[data-feature]')).toHaveLength(0);
    expect(text(c.querySelector('[data-program] .more'))).toBe('6 features ▸');
    key(mc, ' ');
    await settle();
    expect(c.querySelectorAll('[data-feature]')).toHaveLength(6);
  });

  it('paused tag; p pauses / resumes with the toasts; ⏎ opens the Program view; ⇧K confirms', async () => {
    replies.program_update = (a) => ({ ...row(), status: a.patch.pause ? 'paused' : 'running' });
    const { c, mc } = await mount(row({ status: 'paused' }));
    expect(c.querySelector('[data-paused]')).toBeTruthy();
    key(mc, 'p');
    await settle();
    expect(called('program_update')[0]).toEqual({ id: 'prg-1', patch: { resume: true } });
    expect(app.toasts.at(-1).text).toBe('Checkout v2 resumed.');
    key(mc, 'K', { shiftKey: true });
    await settle();
    expect(app.overlay).toMatchObject({
      kind: 'confirm',
      text: 'Cancel Checkout v2?',
      yes: 'Cancel program',
      no: 'Keep',
    });
    app.overlay.onYes();
    await settle();
    expect(called('program_update').at(-1)).toEqual({ id: 'prg-1', patch: { cancel: true } });
    app.overlay = null;
    key(mc, 'Enter');
    await settle();
    expect(Object.values(app.panes).find((s) => s.type === 'program')).toMatchObject({ programId: 'prg-1' });
  });

  it('a done program is one outcome line in Done today; its o opens the Program view', () => {
    app.now.outcomes = [
      {
        id: 'o1',
        kind: 'program',
        program: 'prg-1',
        text: 'Program Checkout v2 done · 3 merged',
        at_ms: NOW,
        project: 'shop-api',
      },
    ];
    const done = lanes(app.now, null, NOW).lanes.find((l) => l.id === 'done');
    expect(done.rows[0].outcome.text).toBe('Program Checkout v2 done · 3 merged');
  });
});

// ---------------------------------------------------------------- Program view (§7.2, §12.4)

function running(o = {}) {
  const p = checkout({ id: 'prg-1', status: 'running', started_at_ms: new Date(2026, 10, 12, 9, 40).getTime(), ...o });
  p.features[0] = { ...p.features[0], state: 'running', phase: 'executing', actual_pp: 2.1, run: 'run-1' };
  p.features[1] = { ...p.features[1], state: 'waiting' };
  p.features[2] = { ...p.features[2], state: 'queued' };
  return p;
}

describe('Program view', () => {
  async function mount(p = running()) {
    replies.program = p;
    replies.work_node = (a) =>
      a.id === N(19)
        ? { id: N(19), ref: '#19', title: 'Gift cards', parent: N(12), upstream: { blocked_by: [{ id: N(14) }] } }
        : ROOT;
    replies.teams_view = TEAMS;
    app.now.counts.global.live = 2;
    app.now.counts.global.live_cap = 2;
    app.panes = { pv: { type: 'program', programId: p.id } };
    const c = render(ProgramPane, { paneId: 'pv' }).container;
    await settle();
    return { c, pv: /** @type {HTMLElement} */ (c.querySelector('.pv')) };
  }

  it('header, subline, rows with live states, actual, graph', async () => {
    const { c } = await mount();
    expect(text(c.querySelector('h2'))).toBe('Checkout v2 · program');
    expect(text(c.querySelector('[data-status]'))).toBe('running · 0 of 3 merged');
    expect(text(c.querySelector('[data-subline]'))).toBe(
      'From #12 Checkout v2 · acme/shop ↗ · started 12 Nov 09:40 · 2 at once · by dependencies',
    );
    const r = [...c.querySelectorAll('[data-row]')];
    expect(text(r[0])).toContain('running · executing');
    expect(text(r[0])).toContain('2.1 pp so far');
    expect(text(r[1])).toContain('waiting for checkout-address to merge');
    expect(text(r[2])).toContain('queued · live cap 2/2');
    expect(c.querySelector('[data-graph] .node.working')).toBeTruthy();
  });

  it('p pauses (toast); paused rows say so; ⇧K and x confirm before editing', async () => {
    replies.program_update = (a) => {
      const p = running({ status: a.patch.pause ? 'paused' : 'running' });
      if (a.patch.skip) p.features[2].state = 'skipped';
      return p;
    };
    const { c, pv } = await mount();
    key(pv, 'p');
    await settle();
    expect(called('program_update')[0]).toEqual({ id: 'prg-1', patch: { pause: true } });
    expect(app.toasts.at(-1).text).toBe('Checkout v2 paused. Running features continue.');
    expect(text(c.querySelector('[data-status]'))).toBe('paused · 0 of 3 merged');
    expect(text(c.querySelectorAll('[data-row]')[1])).toContain('waiting · program paused');
    key(pv, 'j');
    key(pv, 'j');
    key(pv, 'x');
    await settle();
    expect(app.overlay).toMatchObject({
      kind: 'confirm',
      text: 'Skip checkout-summary?',
      detail: 'Features after it no longer wait for it.',
    });
    app.overlay.onYes();
    await settle();
    expect(called('program_update').at(-1)).toEqual({ id: 'prg-1', patch: { skip: 'pf-3' } });
    app.overlay = null;
    key(pv, 'k'); // a running feature can't be skipped
    key(pv, 'k');
    key(pv, 'x');
    expect(app.overlay).toBeNull();
    key(pv, 'K', { shiftKey: true });
    expect(app.overlay).toMatchObject({ text: 'Cancel Checkout v2?' });
  });

  it('a new upstream item: Add to program opens the row editor prefilled; Add sends add_item; Ignore', async () => {
    replies.program_update = () => running();
    const { c, pv } = await mount(running({ new_items: [N(19)] }));
    await settle();
    expect(text(c.querySelector('[data-new-item]'))).toContain('+ New in GitHub: #19 Gift cards (sub-issue of #12)');
    key(pv, 'a');
    await settle();
    const ed = c.querySelector('[data-add-editor]');
    expect(text(ed)).toContain('Add #19 Gift cards as feature 4');
    expect(text(ed)).toContain('checkout-payment (2)'); // after the feature its item is blocked by
    /** @type {HTMLElement} */ ([...ed.querySelectorAll('button')].find((b) => text(b) === 'Add')).click();
    await settle();
    expect(called('program_update').at(-1)).toEqual({
      id: 'prg-1',
      patch: { add_item: N(19), mode: 'team', team: 'default-web', after: ['pf-2'] },
    });
    expect(app.toasts.at(-1).text).toBe('Added #19 to Checkout v2');
  });

  it('a new item shows when it was added, and the row editor sends a typed slug', async () => {
    replies.program_update = () => running();
    const at = new Date(2026, 10, 13, 12).getTime();
    const { c, pv } = await mount(running({ new_items: [N(19)], new_items_added: { [N(19)]: at } }));
    await settle();
    expect(text(c.querySelector('[data-new-item]'))).toContain('sub-issue of #12, added 13 Nov)');
    key(pv, 'a');
    await settle();
    const ed = c.querySelector('[data-add-editor]');
    await fireEvent.input(ed.querySelector('input[aria-label="Slug"]'), { target: { value: 'gift-cards' } });
    /** @type {HTMLElement} */ ([...ed.querySelectorAll('button')].find((b) => text(b) === 'Add')).click();
    await settle();
    expect(called('program_update').at(-1).patch.slug).toBe('gift-cards');
  });

  it('refetches only when a programs delta upserts or removes its id', async () => {
    const { c } = await mount(running());
    await settle();
    const n = () => called('program').length;
    const before = n();
    app.now.rev = 10;
    applyNow({ rev: 11, topics: ['programs'], programs: { upsert: [row({ id: 'prg-other' })], remove: [] } });
    await settle();
    expect(n()).toBe(before);
    applyNow({ rev: 12, topics: ['programs'], programs: { upsert: [row()], remove: [] } });
    await settle();
    expect(n()).toBe(before + 1);
    expect(c).toBeTruthy();
  });

  it('done and cancelled are read-only', async () => {
    const done = checkout({ id: 'prg-1', status: 'done', done_at_ms: new Date(2026, 10, 13, 18, 20).getTime() });
    for (const f of done.features) f.state = 'merged';
    let { c } = await mount(done);
    expect(text(c.querySelector('[data-status]'))).toBe('done · 3 merged · 13 Nov 18:20');
    expect([...c.querySelectorAll('button')].some((b) => text(b) === 'Pause')).toBe(false);
    cleanup();
    const can = checkout({ id: 'prg-1', status: 'cancelled' });
    can.features[0].state = 'merged';
    can.features[1].state = 'skipped';
    can.features[2].state = 'skipped';
    ({ c } = await mount(can));
    expect(text(c.querySelector('[data-status]'))).toBe('cancelled · 1 merged');
    expect(text(c.querySelectorAll('[data-row]')[1])).toContain('dropped');
    expect(text(c.querySelector('[data-over]'))).toBe(
      'Running features kept running as normal features; waiting ones were dropped.',
    );
  });
});

// ---------------------------------------------------------------- cards (§7.3–§7.5)

const card = (kind, actions, o = {}) => ({
  id: 'prog-1',
  kind,
  level: 3,
  title: 'New sub-issue in Checkout v2',
  summary: '#19 Gift cards was added under #12 in GitHub.',
  subject: { project: 'shop-api' },
  context: [],
  actions,
  lifecycle: { state: 'open' },
  created_ms: NOW,
  payload: { program: 'prg-1' },
  ...o,
});

describe('program cards', () => {
  it('keys map to the card actions: ⏎ Add opens at once, r Ignore decides, x Skip on a failed feature', () => {
    const add = card('program_new_item', [
      { id: 'add', label: 'Add to program', key: '⏎' },
      { id: 'ignore', label: 'Ignore', key: 'r' },
    ]);
    expect(keyIntent('enter', add)).toEqual({ kind: 'core', action: 'add' });
    expect(keyIntent('r', add)).toEqual({ kind: 'decide', action: 'ignore' });
    const failed = card('program_feature_failed', [
      { id: 'open', label: 'Open feature', key: '⏎' },
      { id: 'skip', label: 'Skip it', key: 'x' },
    ]);
    expect(keyIntent('x', failed)).toEqual({ kind: 'decide', action: 'skip' });
    expect(keyIntent('enter', failed)).toEqual({ kind: 'core', action: 'open' });
    expect(intentFor(card('rule_proposal', []), 'add')).toEqual({ kind: 'decide', action: 'add' });
    // I1: a dependent of a failed run outside the program: ⏎ Start anyway, x Skip it.
    const dep = card('program_feature_failed', [
      { id: 'start_anyway', label: 'Start anyway', key: '⏎' },
      { id: 'skip', label: 'Skip it', key: 'x' },
    ]);
    expect(keyIntent('enter', dep)).toEqual({ kind: 'decide', action: 'start_anyway' });
    expect(keyIntent('x', dep)).toEqual({ kind: 'decide', action: 'skip' });
    expect(pendingToast(dep, 'start_anyway')).toBe('Starting anyway · New sub-issue in Checkout v2');
    expect(
      effectText({ ...dep, lifecycle: { state: 'decided', effect: 'started anyway' } }, { action: 'start_anyway' }),
    ).toBe('Started anyway · New sub-issue in Checkout v2');
  });

  it('the card shows its kind label and the ⏎ action as primary', async () => {
    const d = card('program_new_item', [
      { id: 'add', label: 'Add to program', key: '⏎' },
      { id: 'ignore', label: 'Ignore', key: 'r' },
    ]);
    const c = render(DecisionCard, { d }).container;
    expect(text(c.querySelector('.kind'))).toContain('Program · new item');
    const add = [...c.querySelectorAll('.actions button')].find((b) => text(b).startsWith('Add to program'));
    expect(add.classList.contains('primary')).toBe(true);
  });

  it('routeOpen: program, program_add (the row editor) and the failed feature', () => {
    A.routeOpen({ open: 'program', program: 'prg-1' });
    expect(Object.values(app.panes).find((s) => s.type === 'program')).toMatchObject({ programId: 'prg-1' });
    A.routeOpen({ open: 'program_add', program: 'prg-1', item: N(19) });
    expect(Object.values(app.panes).find((s) => s.type === 'program').land).toMatchObject({ add: N(19) });
    A.routeOpen({ open: 'feature', program: 'prg-1', feature: 'checkout' }, 'shop-api');
    expect(Object.values(app.panes).some((s) => s.type === 'feature' && s.featureId === 'f-co')).toBe(true);
    // The reply's own project wins over the card's.
    for (const k of Object.keys(app.panes)) delete app.panes[k];
    A.routeOpen({ open: 'feature', program: 'prg-1', feature: 'checkout', project: 'blog' }, 'shop-api');
    expect(Object.values(app.panes).some((s) => s.type === 'feature')).toBe(false);
  });

  it('spec / PR cards of a program feature carry the tag; approving one chains to the next spec', async () => {
    const gate = (id, index, feature) => ({
      ...card('gate', [{ id: 'approve', label: 'Approve', key: 'a' }], {
        id,
        title: `Approve the spec of ${feature}?`,
        subject: { project: 'shop-api', feature },
      }),
      gate: 'spec',
      program: { id: 'prg-1', title: 'Checkout v2', index, total: 3 },
    });
    const c = render(DecisionCard, { d: gate('g-2', 2, 'checkout-payment') }).container;
    expect(text(c.querySelector('[data-program-tag]'))).toContain('Checkout v2 · 2 of 3');
    const decisions = {
      'g-2': gate('g-2', 2, 'checkout-payment'),
      'g-3': gate('g-3', 3, 'checkout-summary'),
      x: { ...gate('x', 1, 'other'), program: { id: 'prg-9', title: 'Other', index: 1, total: 1 } },
    };
    const next = P.nextSpec(decisions, decisions['g-2']);
    expect(next.id).toBe('g-3');
    expect(P.nextSpecText(next)).toBe("Next: checkout-summary's spec");
    expect(P.nextSpec(decisions, decisions.x)).toBeNull();
    expect(P.programTag({ kind: 'gate' })).toBe('');
  });

  it('palette: Program, Pause program and Run as program entries', () => {
    app.now.programs = { 'prg-1': row() };
    app.now.intake = {
      items: [{ id: 'it_1', node: N(12), ref: '#12', title: 'Checkout v2', actions: { program: 'ok' } }],
      sources: [],
    };
    const labels = A.commands().map((x) => x.label);
    expect(labels).toContain('Program: Checkout v2');
    expect(labels).toContain('Pause program Checkout v2');
    expect(labels).toContain('Run as program: #12 Checkout v2');
  });

  it('openProgram reuses its tab', () => {
    openProgram('prg-1', 'Checkout v2');
    openProgram('prg-1', 'Checkout v2');
    expect(Object.values(app.panes).filter((s) => s.type === 'program')).toHaveLength(1);
  });
});

describe('F6 follow-ups (X1)', () => {
  it('writeback_failing: its own kind label; Open settings is answered at once and lands on the Write back block', async () => {
    expect(decisionKind({ kind: 'writeback_failing' }).label).toBe('Write-back failing');
    expect(intentFor({ kind: 'writeback_failing', actions: [] }, 'settings')).toEqual({
      kind: 'core',
      action: 'settings',
    });
    expect(intentFor({ kind: 'writeback_failing', actions: [] }, 'turn_off')).toEqual({
      kind: 'decide',
      action: 'turn_off',
    });
    replies.inbox_act = () => ({ open: 'settings' });
    await A.coreAct({ id: 'd-1', kind: 'writeback_failing', subject: { project: 'shop-api' } }, 'settings');
    expect(app.window).toMatchObject({
      kind: 'project-settings',
      projectId: 'shop-api',
      section: 'tracker',
      block: 'write_back',
    });
  });

  it('palette: Start team run for a ready item', () => {
    app.now.intake = {
      items: [{ id: 'it_1', node: N(12), ref: '#12', title: 'Checkout v2', actions: { team: 'ok', program: 'ok' } }],
      sources: [],
    };
    expect(A.commands().map((x) => x.label)).toContain('Start team run: #12 Checkout v2');
  });
});
