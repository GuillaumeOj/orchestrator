// P3 pacing (T-10): the helpers, the weekly report, the planner's model
// windows, off-hours and downgrade entries, the AC banner and chip, the
// Board's off-hours action and starter roles, against a mocked core.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import {
  delta,
  downgradeText,
  kpis,
  nextOffHours,
  noData,
  powerChip,
  reportMarkdown,
  stateBar,
  usd,
  weekNo,
  windowQueued,
} from '../src/lib/pacing.js';
import { queueReason } from '../src/lib/status.js';
import { fixture } from './fixture.js';

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

const { default: ReportPane } = await import('../src/panes/ReportPane.svelte');
const { default: PlannerPane } = await import('../src/panes/PlannerPane.svelte');
const { default: BoardPane } = await import('../src/panes/BoardPane.svelte');
const { default: Banner } = await import('../src/components/Banner.svelte');
const { default: StatusBar } = await import('../src/components/StatusBar.svelte');
const { default: TeamCanvasPane } = await import('../src/panes/TeamCanvasPane.svelte');

const at = (d, h, m = 0) => new Date(2026, 9, d, h, m).getTime();
const iso = (ms) => new Date(ms).toISOString();

const row = (key, label, pp, cost, share) => ({ key, label, pp, cost_usd: cost, share });
function report(over = {}) {
  return {
    id: '2026-10-19',
    week_start: iso(at(19, 9)),
    reset_at: iso(at(26, 9)),
    in_progress: false,
    provenance: 'observed',
    totals: { weekly_pct: 93, agents_pp: 81, interactive_pp: 7, outside_pp: 5, reserve_pct: 10 },
    allocations: [
      {
        id: 'a1',
        target: 'shop-api',
        weight: 50,
        tier: 'high',
        planned_pp: 48,
        used_pp: 52,
        delta_pp: 4,
        queued_p50_min: 6,
        queued_max_min: 42,
      },
      {
        id: 'a2',
        target: 'blog',
        weight: 30,
        tier: 'normal',
        planned_pp: 29,
        used_pp: 20,
        delta_pp: -9,
        queued_p50_min: 0,
        queued_max_min: 0,
      },
    ],
    by_project: [row('shop-api', 'shop-api', 52, 350, 0.6)],
    by_feature: [],
    by_role: [row('lead', 'lead', 18, 121, 0.2), row('outside', 'Outside the app', 5, 0, 0.05)],
    by_model: [
      row('opus', 'opus', 61, 412, 0.66),
      row('sonnet', 'sonnet', 27, 96, 0.29),
      row('outside', 'Outside the app', 5, 0, 0.05),
    ],
    states: [
      { state: 'green', minutes: 61 * 60 },
      { state: 'amber', minutes: 23 * 60 },
      { state: 'red', minutes: 31 * 60 },
      { state: 'catch_up', minutes: 2 * 60 },
    ],
    actions: [
      { kind: 'queue', count: 6, undone: 0 },
      { kind: 'defer_after_reset', count: 3, undone: 1 },
      { kind: 'downgrade', count: 2, undone: 0 },
      { kind: 'run_now', count: 2, undone: 0 },
    ],
    alerts: [],
    limit_hits: [],
    model_windows: [{ name: 'seven_day_opus', family: 'opus', peak_pct: 92 }],
    ...over,
  };
}

describe('pacing helpers', () => {
  it('weeks, deltas, $ and the state bar', () => {
    expect(weekNo(at(21, 11))).toBe(43);
    expect(delta(4)).toEqual({ text: '+4 pp', glyph: '▲' });
    expect(delta(-9)).toEqual({ text: '−9 pp', glyph: '▼' });
    expect(usd(row('opus', 'opus', 61, 412.4, 0.6))).toBe('≈ $412');
    expect(usd(row('outside', 'Outside the app', 5, 0, 0))).toBe('—');
    expect(stateBar(report().states).map((s) => [s.id, s.hours])).toEqual([
      ['green', 61],
      ['amber', 23],
      ['red', 31],
      ['catch_up', 2],
    ]);
    const k = kpis(report());
    expect(k.map((x) => x.value)).toEqual(['93%', '0', '31 h', '13']);
    expect(k[3].note).toBe('2 overrides by you · 1 undone');
    expect(noData(report({ totals: { ...report().totals, weekly_pct: 0 }, by_project: [], allocations: [] }))).toBe(
      true,
    );
    const md = reportMarkdown(report());
    expect(md).toContain('# Week 43');
    expect(md).toContain('| shop-api | 50 | 48 pp | 52 pp | +4 pp | 6 min / 42 min |');
    expect(md).toContain('| opus | 61 pp | 66 % | ≈ $412 |');
  });

  it('model windows, off-hours and downgrade copy', () => {
    const resets = iso(at(26, 9));
    expect(windowQueued({ family: 'opus', state: 'red', resets_at: resets })).toBe(
      'Queued: opus week in Red · starts after reset Mon 09:00',
    );
    expect(windowQueued({ family: 'opus', state: 'exhausted', resets_at: resets })).toBe(
      'Queued: opus week exhausted · starts Mon 09:00 · or set a downgrade for this role',
    );
    expect(queueReason({ reason: { kind: 'model_window', model: 'opus', resets_at: resets, exhausted: false } })).toBe(
      'opus week in Red · starts after reset Mon 09:00',
    );
    expect(queueReason({ reason: { kind: 'off_hours' } })).toBe('outside working hours');
    expect(queueReason({ reason: { kind: 'scheduled_off_hours', starts_at: iso(Date.now() + 7 * 3600000) } })).toMatch(
      /^Off-hours · starts \d\d:\d\d \(in [67]h \d\dm\)$/,
    );
    const off = { windows: [{ start: iso(at(21, 18)), end: iso(at(22, 8)) }] };
    expect(nextOffHours(off, at(21, 11))).toBe(at(21, 18));
    expect(nextOffHours(off, at(21, 20))).toBe(at(21, 20));
    expect(nextOffHours({ windows: [] }, at(21, 11))).toBe(null);
    expect(powerChip({ on_ac: true, assertion_held: true, off_hours_pending: 3 })).toEqual({
      text: 'Keeping awake · 3 off-hours',
      warn: false,
    });
    expect(powerChip({ on_ac: false, off_hours_pending: 3 })).toEqual({
      text: 'On battery · 3 off-hours waiting',
      warn: true,
    });
    expect(powerChip({ on_ac: true, off_hours_pending: 0 })).toBe(null);
    expect(downgradeText({ kind: 'downgrade', agent: 'front-2', from: 'opus', to: 'sonnet', why: 'red' })).toBe(
      'Downgraded front-2 opus → sonnet at its task boundary (Red)',
    );
    expect(downgradeText({ kind: 'restore_model', agent: 'front-2', to: 'opus', why: 'green' })).toBe(
      'Restored front-2 to opus (Green)',
    );
  });
});

describe('weekly report pane', () => {
  beforeEach(() => {
    cleanup();
    calls.length = 0;
    for (const k of Object.keys(replies)) delete replies[k];
    app.data = fixture();
    app.overlay = null;
    app.toasts = [];
  });

  const list = {
    reports: [
      { id: 'current', week_start: iso(at(26, 9)), reset_at: iso(at(33, 9)), final_weekly_pct: 12, in_progress: true },
      {
        id: '2026-10-19',
        week_start: iso(at(19, 9)),
        reset_at: iso(at(26, 9)),
        final_weekly_pct: 93,
        in_progress: false,
      },
      {
        id: '2026-10-12',
        week_start: iso(at(12, 9)),
        reset_at: iso(at(19, 9)),
        final_weekly_pct: 94,
        in_progress: false,
      },
    ],
  };

  it('a past week: KPIs, plan vs actual, model breakdown with ≈ API $, tabs and prev / next', async () => {
    replies.report_view = (a) =>
      a.id === '2026-10-12' ? report({ id: a.id, week_start: iso(at(12, 9)), reset_at: iso(at(19, 9)) }) : report();
    replies.reports_list = list;
    app.panes.r1 = { type: 'report', reportId: '2026-10-19', title: 'Weekly report' };
    const { container, getByText } = render(ReportPane, { paneId: 'r1' });
    await waitFor(() => getByText('Week 43'));
    const t = container.textContent;
    expect(t).toContain('93 % at reset');
    expect(t).toContain('Used at reset');
    expect(t).toContain('▲ +4 pp');
    expect(t).toContain('6 min / 42 min');
    expect(t).toContain('≈ $412');
    expect(t).toContain('outside the app');
    expect(container.querySelector('th.usd').getAttribute('title')).toBe(
      'What these tokens would cost at API prices. Your plan is not billed this way.',
    );
    expect(t).toContain('opus week 92 %');
    expect(app.panes.r1.title).toBe('Week 43 report');
    const root = container.querySelector('[data-pane-focus]');
    await fireEvent.keyDown(root, { key: '3' });
    await tick();
    expect(container.querySelector('.tab.on').textContent).toContain('Role');
    expect(container.querySelector('table.rows').textContent).toContain('lead');
    // [ goes to the older report.
    await fireEvent.keyDown(root, { key: '[' });
    await waitFor(() => getByText('Week 42'));
    expect(calls.some(([c, a]) => c === 'report_view' && a.id === '2026-10-12')).toBe(true);
    // e copies the Markdown; ⏎ plans next week.
    await fireEvent.keyDown(root, { key: 'Enter' });
    expect(app.overlay).toEqual({ kind: 'plan' });
    app.overlay = null;
  });

  it('the current week reads so far; an estimated week says so; no data says why', async () => {
    replies.reports_list = list;
    replies.report_view = report({
      id: 'current',
      in_progress: true,
      provenance: 'estimated',
      week_start: iso(at(26, 9)),
    });
    app.panes.r1 = { type: 'report', reportId: 'current', title: 'Weekly report' };
    const r1 = render(ReportPane, { paneId: 'r1' });
    await waitFor(() => r1.getByText('Week 44'));
    expect(r1.container.textContent).toContain('so far ·');
    expect(r1.container.textContent).toContain('Used so far');
    expect(r1.container.textContent).toContain('48 pp so far');
    expect(r1.container.textContent).toContain('Estimated: fewer than 20 usage samples this week');
    cleanup();
    replies.report_view = report({
      totals: { weekly_pct: 0, agents_pp: 0, interactive_pp: 0, outside_pp: 0, reserve_pct: 10 },
      allocations: [],
      by_project: [],
    });
    const r2 = render(ReportPane, { paneId: 'r1' });
    await waitFor(() => r2.getByText('No usage was recorded this week. The app was not running, or no agents ran.'));
  });
});

describe('planner, banner, status bar, board', () => {
  beforeEach(() => {
    cleanup();
    calls.length = 0;
    for (const k of Object.keys(replies)) delete replies[k];
    app.data = fixture();
    app.toasts = [];
    app.panes = {};
  });

  const now = Date.now();
  const planner = (over = {}) => ({
    plan_id: 'p',
    week_start_ms: now - 2 * 86400000,
    reset_at_ms: now + 5 * 86400000,
    now_ms: now,
    shadow: false,
    reserve_pct: 10,
    snapshot: {
      ...fixture().usage.snapshot,
      five_hour_projected: 92,
      eta: null,
      model_windows: [
        { name: 'seven_day_opus', family: 'opus', used_pct: 92, resets_at: iso(now + 4 * 86400000), state: 'red' },
      ],
    },
    curve: [],
    actual: [],
    working: [],
    allocations: [],
    queue: [{ id: 'q1', agent: 'deps-1', tier: 'low', reason: { kind: 'live_cap', cap: 1, state: 'red' } }],
    log: [
      {
        key: 'k1',
        can_undo: true,
        past: false,
        action: {
          id: 7,
          ts: iso(now - 60000),
          reason: 'week in Red',
          action: { kind: 'downgrade', agent: 'front-1', from: 'opus', to: 'sonnet', why: 'red' },
        },
      },
    ],
    next: { target_state: 'red', earliest_state_change: iso(now + 3600000) },
    pending_ask: null,
    sprint: null,
    five_hour_resets_at_ms: null,
    first_run_ms: null,
    plan_set: true,
    off_hours: {
      windows: [{ start: iso(now + 3600000), end: iso(now + 15 * 3600000) }],
      live_cap: 1,
      queued: 3,
      running: 0,
    },
    ...over,
  });

  it('planner: model window rows, off-hours band with the queued count, the downgrade entry, O and R', async () => {
    replies.planner_view = planner();
    replies.queue_off_hours = { starts_at: iso(now + 3600000) };
    replies.governor_act = {};
    const { container, getByText } = render(PlannerPane);
    await waitFor(() => getByText('Model windows'));
    const t = container.textContent;
    expect(t).toContain('opus week 92 %');
    expect(t).toContain('Queued: opus week in Red · starts after reset');
    expect(t).toContain('Downgraded front-1 opus → sonnet at its task boundary (Red)');
    expect(container.querySelector('rect.off')).toBeTruthy();
    expect(t).toContain('☾ 3 queued');
    const root = container.querySelector('[data-pane-focus]');
    await fireEvent.keyDown(root, { key: 'O' });
    await waitFor(() =>
      expect(calls.some(([c, a]) => c === 'queue_off_hours' && a.entry === 'q1' && a.off === true)).toBe(true),
    );
    await waitFor(() => expect(app.toasts.at(-1)?.text).toMatch(/^deps-1 runs off-hours · starts \d\d:\d\d$/));
    await fireEvent.keyDown(root, { key: 'R' });
    expect(Object.values(app.panes).some((p) => p.type === 'report' && p.reportId === 'current')).toBe(true);
  });

  it('planner: no allowance disables Run off-hours', async () => {
    replies.planner_view = planner({ off_hours: { windows: [], live_cap: 0, queued: 0, running: 0 } });
    const { container, getByText } = render(PlannerPane);
    await waitFor(() => getByText('Run off-hours', { exact: false }));
    const b = [...container.querySelectorAll('button')].find((x) => x.textContent.includes('Run off-hours'));
    expect(b.disabled).toBe(true);
    expect(b.getAttribute('title')).toBe('Off-hours allowance is 0 % this week. Set it in the plan (e).');
  });

  it('on battery: one banner and the status bar chip', () => {
    const green = fixture();
    green.usage.snapshot.state = 'green';
    app.data = {
      ...green,
      banners: [{ kind: 'not_on_ac' }],
      power: { on_ac: false, assertion_held: false, off_hours_pending: 3 },
    };
    const b = render(Banner);
    expect(b.container.textContent).toContain('Off-hours work needs AC power.');
    expect(b.container.textContent).toContain("3 off-hours tasks won't run while the Mac sleeps.");
    const s = render(StatusBar);
    expect(s.container.textContent).toContain('On battery · 3 off-hours waiting');
    cleanup();
    app.data = { ...fixture(), power: { on_ac: true, assertion_held: true, off_hours_pending: 3 } };
    expect(render(StatusBar).container.textContent).toContain('Keeping awake · 3 off-hours');
  });

  it('board: ⇧O runs the card off-hours; space opens its menu', async () => {
    replies.board_view = {
      project: { id: 'shop', name: 'shop-api' },
      backend: 'local',
      repo: null,
      last_sync_error: null,
      tasks: [
        {
          id: 14,
          project_id: 'shop',
          backend: 'local',
          external_id: null,
          feature_id: null,
          team_run_id: null,
          title: 'Payment intent endpoint',
          body: '',
          status: 'ready',
          node: null,
          agent_id: null,
          depends_on: [],
          branch: null,
          notes: [],
          created_at_ms: 1,
          updated_at_ms: 1,
        },
      ],
      features: [],
      runs: [],
    };
    replies.planner_view = planner();
    replies.queue_off_hours = { starts_at: iso(now + 3600000) };
    app.panes.b1 = { type: 'board', projectId: 'shop', title: 'shop tasks' };
    const { container, getByText } = render(BoardPane, { paneId: 'b1' });
    await waitFor(() => getByText('Payment intent endpoint'));
    const board = container.querySelector('.board');
    await fireEvent.keyDown(board, { key: ' ' });
    await tick();
    expect(container.querySelector('[aria-label="Task menu"]').textContent).toContain('Run off-hours');
    await fireEvent.keyDown(board, { key: 'O', shiftKey: true });
    await waitFor(() => expect(calls.some(([c, a]) => c === 'queue_off_hours' && a.task === '14' && a.off)).toBe(true));
    await waitFor(() => expect(container.textContent).toContain('☾ Off-hours'));
  });

  it("a starter role's edit asks where to save a copy, then the node uses the copy", async () => {
    const r = (id, over = {}) => ({
      id,
      name: id,
      scope: { kind: 'global' },
      model: null,
      effort: null,
      tools: { edit: true, bash: true, web: false },
      subagents_allowed: false,
      permission_profile: 'autonomous',
      budget_priority: 'normal',
      prompt: '',
      version: 1,
      mcp_refs: [],
      origin: { starter: id, version: 3, fingerprint: 'x' },
      ...over,
    });
    const team = {
      id: 'mine',
      name: 'Mine',
      scope: { kind: 'project', id: 'shop-api' },
      version: 1,
      nodes: [{ id: 'lead', role_id: 'lead', overrides: {}, position: [0, 0], max_concurrency: 1, is_lead: true }],
      edges: [],
    };
    replies.teams_view = { teams: [{ team, issues: [], rules: [], preset: false, starter: null }], roles: [r('lead')] };
    replies.mcp_registry_view = { servers: [], project_overrides: {} };
    replies.role_duplicate = (a) => ({ ...r('lead'), id: 'lead-copy', scope: a.to, origin: null });
    replies.role_save = (a) => a.role;
    replies.team_save = (a) => ({ team: a.team, issues: [] });
    app.panes.c1 = { type: 'canvas', teamId: 'mine', projectId: 'shop-api', title: 'Team' };
    const { container, getByText } = render(TeamCanvasPane, { paneId: 'c1' });
    await waitFor(() => getByText('role lead v1'));
    const selects = [...container.querySelectorAll('.inspector select')].map(
      (x) => /** @type {HTMLSelectElement} */ (x),
    );
    const sel = selects.find((s) => [...s.options].some((o) => o.value === 'high'));
    sel.value = 'high';
    await fireEvent.change(sel);
    expect(container.querySelector('[aria-label="Starter"]').textContent).toContain('Role lead is a starter.');
    expect(calls.some(([c]) => c === 'role_save')).toBe(false);
    await fireEvent.keyDown(container.querySelector('.canvas-pane'), { key: 'Enter' });
    await waitFor(() =>
      expect(calls.some(([c, a]) => c === 'role_save' && a.role.id === 'lead-copy' && a.role.effort === 'high')).toBe(
        true,
      ),
    );
    expect(calls.find(([c]) => c === 'role_duplicate')[1].to).toEqual({ kind: 'project', id: 'shop-api' });
    await waitFor(
      () => expect(calls.some(([c, a]) => c === 'team_save' && a.team.nodes[0].role_id === 'lead-copy')).toBe(true),
      {
        timeout: 2000,
      },
    );
  });
});
