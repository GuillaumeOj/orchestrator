// The P1 wiring: the project pane (FR-102/104, J11), the first-run checklist
// (J0), the plan editor (FR-802), the memory watch and read-only banners.

import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
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

const { default: ProjectPane } = await import('../src/panes/ProjectPane.svelte');
const { default: SetupPane } = await import('../src/panes/SetupPane.svelte');
const { default: PlanEditor } = await import('../src/components/PlanEditor.svelte');
const { default: StatusBar } = await import('../src/components/StatusBar.svelte');
const { default: Banner } = await import('../src/components/Banner.svelte');
const { default: PlannerPane } = await import('../src/panes/PlannerPane.svelte');

function plannerView(over = {}) {
  const now = Date.now();
  const ws = now - 2 * 86400000;
  const action = (id, ts, action, mode = 'shadow') => ({
    id,
    ts: new Date(ts).toISOString(),
    state: 'green',
    action,
    reason: 'why',
    expected_effect: 'effect',
    mode,
    outcome: 'logged',
    ask: null,
    undo: null,
    undone_at: null,
  });
  return {
    plan_id: 'w',
    week_start_ms: ws,
    reset_at_ms: ws + 7 * 86400000,
    now_ms: now,
    shadow: true,
    reserve_pct: 10,
    snapshot: {
      state: 'green',
      provenance: 'observed',
      used: 20,
      governed: 12,
      ideal: 15,
      plan: 23,
      error: -3,
      eta: null,
      five_hour: 30,
      five_hour_projected: 40,
      five_hour_guard: false,
      interactive_used: 5,
      live_autonomous: 0,
      live_cap: 3,
      burn_pct_per_hour: 0.4,
      calibration: { confidence: 0.5 },
      reserve_left: 5,
      unattributed_so_far: 3,
      unattributed_forecast: 4,
    },
    curve: [
      { t_ms: ws, plan_total: 0, forecast_total: null, with_governor_total: null },
      { t_ms: now, plan_total: 20, forecast_total: 20, with_governor_total: 20 },
    ],
    actual: [
      { t_ms: ws + 3600000, used: 2, governed: 1, five_hour: null },
      { t_ms: now, used: 20, governed: 12, five_hour: 30 },
    ],
    working: [],
    allocations: [],
    queue: [],
    log: [
      {
        key: 'n1',
        can_undo: false,
        past: false,
        action: action(1, now - 60000, { kind: 'state_change', from: 'green', to: 'amber' }),
      },
      { key: 'p0', can_undo: false, past: true, action: action(1, ws + 7200000, { kind: 'pause_all' }, 'auto') },
    ],
    next: { target_state: 'green', earliest_state_change: new Date(now).toISOString(), pending_ask: null },
    pending_ask: null,
    sprint: null,
    five_hour_resets_at_ms: null,
    first_run_ms: now - 2.5 * 86400000,
    plan_set: true,
    ...over,
  };
}

const text = (c) => c.textContent.replace(/\s+/g, ' ');

const detail = {
  trusted: false,
  worktrees: [
    {
      path: '/dev/shop-api',
      branch: 'main',
      verdict: 'main',
      safe_to_remove: false,
      dirty: 0,
      unmerged_commits: 0,
      in_progress: [],
      locked: false,
    },
    {
      path: '/dev/shop-api.worktrees/back-1',
      branch: 'orch/solo/back-1',
      verdict: 'ok',
      safe_to_remove: false,
      dirty: 2,
      unmerged_commits: 1,
      in_progress: [],
      locked: true,
      row: { agent_id: 'back-1' },
    },
    {
      path: '/dev/shop-api.worktrees/old',
      branch: 'orch/solo/old',
      verdict: 'ok',
      safe_to_remove: true,
      dirty: 0,
      unmerged_commits: 0,
      in_progress: [],
      locked: false,
    },
    {
      path: '/dev/shop-api/.claude/worktrees/mine',
      branch: 'claude/mine',
      verdict: 'external',
      safe_to_remove: false,
      dirty: 1,
      unmerged_commits: 3,
      in_progress: [],
      locked: false,
    },
  ],
  branches: [
    { name: 'orch/solo/old', merged: true, orch: true },
    { name: 'claude/mine', merged: false, orch: false },
  ],
};

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.data.projects[0] = {
    ...app.data.projects[0],
    default_branch: 'main',
    last_fetch_at_ms: null,
    git: {
      ...app.data.projects[0].git,
      upstream: 'origin/main',
      staged: 1,
      untracked: 1,
      stash: 1,
      last_commit: 'Add checkout',
      last_commit_s: Math.floor(Date.now() / 1000) - 600,
    },
  };
  app.panes = { p1: { type: 'project', projectId: 'shop-api', title: 'shop-api' } };
  app.focused = 'p1';
  app.overlay = null;
  app.toasts = [];
});

describe('project pane', () => {
  it('shows the git state, trust, worktrees and keeps unmerged work behind its own confirmation', async () => {
    replies.project_detail = detail;
    const { container, getByText, getAllByText } = render(ProjectPane, { paneId: 'p1' });
    await waitFor(() => expect(text(container)).toContain('locked'));
    const t = text(container);
    expect(t).toContain('main → origin/main · ↑2 ↓0');
    expect(t).toContain('3 files (1 staged, 1 untracked) · 1 stashed');
    expect(t).toContain('Add checkout · 10m ago');
    expect(t).toContain('Not trusted yet');
    expect(t).toContain('unmerged work · back-1 · locked');
    expect(t).not.toContain('main checkout'); // the main checkout is not listed for removal
    expect(t).toContain('1 other worktree outside the sibling folder (yours, not managed by the app)');
    expect(t).toContain('orch/ branches: orch/solo/old (merged)');
    expect(t).not.toContain('claude/mine (');
    expect(getAllByText('Remove')).toHaveLength(2); // never offered for the user's own worktree
    // Removing the unmerged worktree asks to discard it.
    await fireEvent.click(getAllByText('Remove')[0]);
    expect(app.overlay.text).toMatch(/^Discard unmerged work in orch\/solo\/back-1/);
    app.overlay = null;
    await fireEvent.click(getAllByText('Remove')[1]);
    expect(app.overlay.text).toBe('Remove worktree orch/solo/old?');
    app.overlay.onYes();
    await waitFor(() =>
      expect(calls.some(([c, a]) => c === 'worktree_remove' && a.discard === false && a.path.endsWith('/old'))).toBe(
        true,
      ),
    );
    expect(getByText('Trust in a pane', { exact: false })).toBeTruthy();
  });

  it('J11: archive by default, unmerged worktrees kept unless ticked', async () => {
    replies.project_detail = detail;
    replies.project_removal_plan = { plan: { worktrees: detail.worktrees.slice(1) }, agents: [{ id: 'back-1' }] };
    replies.project_remove = { kept: [['/dev/shop-api.worktrees/back-1', 'unmerged']] };
    const { container, getByText } = render(ProjectPane, { paneId: 'p1' });
    await fireEvent.keyDown(container.querySelector('[data-pane-focus]'), { key: 'x' });
    await waitFor(() => expect(text(container)).toContain('Agents stopped and archived: back-1.'));
    expect(text(container)).toContain('Discard unmerged work in orch/solo/back-1 (otherwise kept)');
    await fireEvent.click(container.querySelector('.danger-zone .btn.danger'));
    expect(app.overlay.text).toBe('Remove project shop-api?');
    expect(app.overlay.detail).toContain('It is hidden; everything is kept.');
    app.overlay.onYes();
    await waitFor(() => expect(calls.find(([c]) => c === 'project_remove')).toBeTruthy());
    const [, args] = calls.find(([c]) => c === 'project_remove');
    expect(args.choices).toEqual({ remove_merged: true, discard: [], history: 'archive' });
    expect(getByText).toBeTruthy();
  });
});

describe('first run (J0)', () => {
  it('is read-only until Claude Code is fixed, and counts the steps left', async () => {
    app.data.setup = {
      done: false,
      claude: {
        path: '/u/.local/bin/claude',
        version: '2.1.302',
        logged_in: false,
        problem: 'Claude Code is not logged in: run `claude` …',
      },
      trust: [
        { id: 'shop-api', trusted: true },
        { id: 'blog', trusted: false },
      ],
      kitty: { sources: ['/u/.config/kitty/kitty.conf'], font: 'FiraCode Nerd Font Mono', mapped: 12, unmapped: 3 },
      editor: 'nvim +{line} {file}',
      plan_confirmed: false,
    };
    replies.plan_form = { days: ['mon'], start: '09:00', end: '19:00', reserve_pct: 10, allocations: [] };
    const { container } = render(SetupPane);
    const t = text(container);
    expect(t).toContain('not logged in');
    expect(t).toContain('read-only mode');
    expect(t).toContain('3 steps left'); // claude, trust (blog), plan
    expect(t).toContain('trusted');
    expect(t).toContain('Trust…');
    expect(t).toContain('shadow mode');
    const b = render(Banner);
    expect(text(b.container)).toContain('Read-only:');
  });

  it('the plan editor proposes the core’s form and saves it', async () => {
    replies.plan_form = {
      days: ['mon', 'tue', 'wed', 'thu', 'fri'],
      start: '09:00',
      end: '19:00',
      reserve_pct: 10,
      allocations: [
        { id: 'shop-api', target: 'shop-api', weight: 1, tier: 'normal' },
        { id: 'blog', target: 'blog', weight: 1, tier: 'normal' },
      ],
    };
    replies.plan_set = null;
    replies.app_state = fixture();
    const saved = vi.fn();
    const { container } = render(PlanEditor, { onSaved: saved });
    const { getByText, getByLabelText } = within(container);
    await waitFor(() => expect(text(container)).toContain('blog'));
    expect(text(container)).toContain('50%');
    await fireEvent.input(getByLabelText('Weight of blog'), { target: { value: '3' } });
    await fireEvent.click(getByText('sat'));
    await tick();
    expect(text(container)).toContain('75%');
    await fireEvent.click(getByText('Save plan'));
    await waitFor(() => expect(saved).toHaveBeenCalled());
    const [, { form }] = calls.find(([c]) => c === 'plan_set');
    expect(form.days).toEqual(['mon', 'tue', 'wed', 'thu', 'fri', 'sat']);
    expect(form.allocations.map((a) => a.weight)).toEqual([1, 3]);
    // Empty means no cap.
    expect(form.allocations.map((a) => a.hard_cap_pct)).toEqual([null, null]);
    expect(form.off_hours_allowance_pct).toBe(0);
  });

  it('the plan editor saves hard caps and the off-hours allowance (FR-811, FR-816)', async () => {
    replies.plan_form = {
      days: ['mon'],
      start: '09:00',
      end: '19:00',
      reserve_pct: 10,
      off_hours_allowance_pct: 0,
      allocations: [{ id: 'blog', target: 'blog', weight: 1, tier: 'low', hard_cap_pct: null }],
    };
    replies.plan_set = null;
    replies.app_state = fixture();
    const saved = vi.fn();
    const { container } = render(PlanEditor, { onSaved: saved });
    const { getByText, getByLabelText } = within(container);
    await waitFor(() => expect(text(container)).toContain('blog'));
    await fireEvent.input(getByLabelText('Hard cap of blog'), { target: { value: '12' } });
    await fireEvent.input(getByLabelText('Off-hours allowance (%)'), { target: { value: '5' } });
    await fireEvent.click(getByText('Save plan'));
    await waitFor(() => expect(saved).toHaveBeenCalled());
    const [, { form }] = calls.filter(([c]) => c === 'plan_set').at(-1);
    expect(form.allocations[0].hard_cap_pct).toBe(12);
    expect(form.off_hours_allowance_pct).toBe(5);
  });
});

describe('week planner', () => {
  it('lists the actions of this start and of earlier ones (same ids), counts the shadow week', async () => {
    replies.planner_view = plannerView();
    const { container } = render(PlannerPane);
    await waitFor(() => expect(text(container)).toContain('green → amber'));
    const t = text(container);
    expect(t).toContain('pause all · why');
    expect(t).toContain('Shadow mode · day 3 of 7');
    expect(t).not.toContain('Enforce…');
    expect(t).toContain('Edit plan');
  });

  it('draws the week usage: legend and the now tag', async () => {
    replies.planner_view = plannerView();
    const { container } = render(PlannerPane);
    await waitFor(() => expect(text(container)).toContain('Week used'));
    const t = text(container);
    for (const l of ['Week used', 'Agents', 'Plan', 'Forecast', 'W/ governor']) expect(t).toContain(l);
    expect(t).toMatch(/now \d\d:\d\d · week 20% \(plan 23%, .*3 pp\) · agents 12%/);
    expect(t).toContain('reserve 10% for you');
    const d = (c) => container.querySelector(`svg path.${c}`).getAttribute('d');
    expect(d('actual')).toMatch(/^M.*L/);
    expect(d('agents')).not.toBe(d('fill'));
    expect(d('ideal')).toMatch(/^M.*L/);
    expect(container.querySelector('svg').getAttribute('aria-label')).toBe('Week usage so far against the plan');
  });

  it('offers to enforce once the shadow week is over, and to go back', async () => {
    replies.planner_view = plannerView({ first_run_ms: Date.now() - 8 * 86400000 });
    const { container, getByText } = render(PlannerPane);
    await waitFor(() => expect(text(container)).toContain('Shadow week done · Enforce…'));
    await fireEvent.click(getByText('Shadow week done · Enforce…'));
    expect(app.overlay.text).toBe('Enforce the Governor?');
    replies.governor_enforce = null;
    app.overlay.onYes();
    await waitFor(() => expect(calls.some(([c, a]) => c === 'governor_enforce' && a.enforce === true)).toBe(true));
    cleanup();
    replies.planner_view = plannerView({ shadow: false });
    const r = render(PlannerPane);
    await waitFor(() => expect(text(r.container)).toContain('Enforced'));
  });
});

describe('memory watch', () => {
  it('warns in the status bar when a WebKit helper runs away', () => {
    app.data.memory_watch = { alert: 'WebContent 2800 MB', warn_mb: 600, log: '/h/logs/memory.log' };
    const { container } = render(StatusBar);
    expect(text(container)).toContain('⚠ WebContent 2800 MB');
  });
});

describe('path and token formatting', async () => {
  const { shortPath, expandHome, tokens } = await import('../src/lib/format.js');
  it('shortens only paths under home, expands ~', () => {
    expect(shortPath('/Users/g/dev/app', '/Users/g')).toBe('~/dev/app');
    expect(shortPath('/Users/gx/dev', '/Users/g')).toBe('/Users/gx/dev');
    expect(expandHome(' ~/dev/app ', '/Users/g')).toBe('/Users/g/dev/app');
    expect(expandHome('~', '/Users/g')).toBe('/Users/g');
    expect(tokens(63197)).toBe('63 k');
    expect(tokens(1_250_000)).toBe('1.3 M');
  });
});
