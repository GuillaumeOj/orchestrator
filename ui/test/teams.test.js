// P2 panes: the pure helpers, and the Board, Feature and Team canvas panes
// rendered against a mocked core.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { roleOf } from '../src/lib/status.js';
import {
  blockedBy,
  COLUMNS,
  handle,
  moveCursor,
  runStateLabel,
  shiftStatus,
  slugify,
  timelineFilter,
  timelineText,
} from '../src/lib/teams.js';
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

const { default: BoardPane } = await import('../src/panes/BoardPane.svelte');
const { default: FeaturePane } = await import('../src/panes/FeaturePane.svelte');
const { default: TeamCanvasPane } = await import('../src/panes/TeamCanvasPane.svelte');
const { default: Dialogs } = await import('../src/components/Dialogs.svelte');

const task = (id, status, over = {}) => ({
  id,
  project_id: 'shop',
  backend: 'local',
  external_id: null,
  feature_id: 'shop-fx',
  team_run_id: 'run-1',
  title: `Task ${id}`,
  body: '',
  status,
  node: 'back',
  agent_id: null,
  depends_on: [],
  branch: null,
  notes: [],
  created_at_ms: 1,
  updated_at_ms: 1,
  ...over,
});

describe('team helpers', () => {
  it('board columns, moves and handles', () => {
    expect(COLUMNS.map((c) => c.id)).toEqual(['backlog', 'ready', 'in_progress', 'review', 'done']);
    expect(shiftStatus('ready', 1)).toBe('in_progress');
    expect(shiftStatus('backlog', -1)).toBe(null);
    expect(shiftStatus('done', 1)).toBe(null);
    expect(handle(task(3, 'ready'))).toBe('T-3');
    expect(handle(task(3, 'ready', { backend: 'github', external_id: '14' }))).toBe('#14');
    const tasks = [task(1, 'done'), task(2, 'review'), task(3, 'ready', { depends_on: [1, 2] })];
    expect(blockedBy(tasks[2], tasks)).toEqual([2]);
    const cols = [[1], [1, 2, 3], []];
    expect(moveCursor(cols, { col: 1, row: 0 }, 'j')).toEqual({ col: 1, row: 1 });
    expect(moveCursor(cols, { col: 1, row: 2 }, 'l')).toEqual({ col: 2, row: 0 });
    expect(moveCursor(cols, { col: 0, row: 0 }, 'h')).toEqual({ col: 0, row: 0 });
    expect(slugify('Search v2: Facets!')).toBe('search-v2-facets');
    expect(runStateLabel('pr_gate')).toBe('PR gate');
    expect(runStateLabel('failed')).toBe('Stopped');
  });

  it('timeline copy and filters', () => {
    const ev = (kind, detail) => ({ id: 1, ts_ms: 0, agent_id: 'fx-back-1', kind, detail });
    expect(timelineText(ev('message', { to: 'lead', body: 'hi' }))).toBe('→ lead: hi');
    expect(timelineText(ev('message', { to: 'lead', kind: 'report', body: 'done' }))).toBe('report → lead: done');
    expect(timelineText(ev('run.state', { from: 'spec_gate', to: 'planning', why: 'spec approved' }))).toBe(
      'Spec gate → Planning (spec approved)',
    );
    expect(
      timelineText(ev('git.merge', { branch: 'orch/fx/back-1', result: { outcome: 'conflict', files: ['a.rs'] } })),
    ).toBe('merge of orch/fx/back-1: conflicts in a.rs');
    expect(timelineFilter('git')(ev('git.merge', {}))).toBe(true);
    expect(timelineFilter('tasks')(ev('message', {}))).toBe(false);
    expect(timelineFilter('gates')(ev('run.state', { from: 'spec', to: 'spec_gate' }))).toBe(true);
  });

  it('role marks come from the role when known', () => {
    expect(roleOf('fx-back-1', 'backend').letter).toBe('B');
    expect(roleOf('fx-ux-1', 'ux').letter).toBe('X');
    expect(roleOf('fx-review-1', 'reviewer').color).toBe('var(--color-role-7)');
    expect(roleOf('x', 'lead', 3).color).toBe('var(--color-role-3)');
    const custom = roleOf('fx-data-1', 'data');
    expect(custom.letter).toBe('D');
    expect(custom).toEqual(roleOf('fx-data-2', 'data'));
    expect(roleOf('solo-1').letter).toBe('S');
  });
});

describe('P2 panes', () => {
  beforeEach(() => {
    cleanup();
    calls.length = 0;
    for (const k of Object.keys(replies)) delete replies[k];
    app.data = fixture();
  });

  it('the board groups tasks by status and moves the selected card with ⇧l', async () => {
    replies.board_view = {
      project: { id: 'shop', name: 'shop-api' },
      backend: 'github',
      repo: 'me/shop',
      last_sync_error: null,
      tasks: [task(1, 'ready'), task(2, 'ready', { depends_on: [1] }), task(3, 'done', { branch: 'orch/fx/back-1' })],
      features: [{ id: 'shop-fx', slug: 'fx' }],
      runs: [
        {
          id: 'run-1',
          feature_id: 'shop-fx',
          slug: 'fx',
          status: 'executing',
          nodes: [
            { id: 'lead', role: 'lead', lead: true },
            { id: 'back', role: 'backend', lead: false },
          ],
        },
      ],
    };
    replies.task_update = (a) => task(a.id, a.status);
    app.panes.b1 = { type: 'board', projectId: 'shop', title: 'shop tasks' };
    const { container, getByText } = render(BoardPane, { paneId: 'b1' });
    await waitFor(() => getByText('Task 1'));
    expect(getByText(/GitHub Issues · me\/shop/)).toBeTruthy();
    expect(getByText(/blocked by T-1/)).toBeTruthy();
    const board = container.querySelector('.board');
    await fireEvent.keyDown(board, { key: 'L', shiftKey: true });
    await waitFor(() =>
      expect(calls.some(([c, a]) => c === 'task_update' && a.status === 'in_progress' && a.id === 1)).toBe(true),
    );
    await fireEvent.keyDown(board, { key: 'n' });
    await tick();
    expect(container.querySelector('input[placeholder="Task title"]')).toBeTruthy();
  });

  it('the feature pane shows the run, the rendered doc, the team and the timeline', async () => {
    replies.feature_view = {
      feature: {
        id: 'shop-fx',
        project_id: 'shop',
        slug: 'fx',
        title: 'Fix X',
        integration_worktree: '/w/fx/integration',
      },
      run: {
        id: 'run-1',
        team_id: 'bugfix-squad',
        status: 'spec_gate',
        started_at_ms: Date.now() - 60000,
        pr_url: null,
      },
      team: {
        name: 'Bugfix squad',
        version: 1,
        nodes: [
          { id: 'lead', role_id: 'lead', is_lead: true },
          { id: 'back', role_id: 'backend', is_lead: false },
        ],
      },
      project: { id: 'shop', name: 'shop-api', backend: 'local' },
      agents: [{ id: 'fx-lead-1', node: 'lead', state: 'idle', task: null, tool: null, flag: null }],
      tasks: [task(1, 'in_progress')],
      docs: ['spec.md'],
      artifacts: [{ id: 1, kind: 'design', uri: 'penpot://drawer', from_node: 'ux', to_node: 'ui' }],
      timeline: [
        {
          id: 9,
          ts_ms: Date.now(),
          agent_id: 'fx-lead-1',
          kind: 'run.state',
          detail: { from: 'spec', to: 'spec_gate', why: 'spec gate asked' },
        },
      ],
    };
    replies.feature_doc = '# Fix X\n\nThe **plan**.';
    replies.message_send = { ids: [1] };
    app.panes.f1 = { type: 'feature', featureId: 'shop-fx', title: 'fx' };
    const { container, getByText, getAllByText } = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => getByText('Spec gate'));
    await waitFor(() => expect(container.querySelector('.rendered b')?.textContent).toBe('plan'));
    expect(getAllByText(/Spec → Spec gate/).length).toBeGreaterThan(0);
    expect(getByText('penpot://drawer')).toBeTruthy();
    const input = container.querySelector('input[placeholder^="Message"]');
    await fireEvent.input(input, { target: { value: 'Keep v1' } });
    await fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() =>
      expect(calls.some(([c, a]) => c === 'message_send' && a.to === 'lead' && a.body === 'Keep v1')).toBe(true),
    );
  });

  it('the launch dialog loads the teams once per opening, whatever the poll does', async () => {
    replies.teams_view = {
      teams: [{ team: { id: 'duo', name: 'Duo', nodes: [], edges: [], version: 1 }, issues: [], rules: [] }],
      roles: [],
      presets: [],
    };
    replies.team_launch = { id: 'run-1', feature_id: 'demo-fx' };
    app.overlay = { kind: 'launch', teamId: 'duo', projectId: 'shop' };
    const { container } = render(Dialogs);
    await tick();
    const title = container.querySelector('input[placeholder="Search v2"]');
    await fireEvent.input(title, { target: { value: 'Fix login' } });
    // Polls replace app.data: the form keeps what was typed, and no new load starts.
    for (let i = 0; i < 3; i++) {
      app.data = fixture();
      await tick();
    }
    expect(calls.filter(([c]) => c === 'teams_view')).toHaveLength(1);
    expect(/** @type {HTMLInputElement} */ (container.querySelector('input[placeholder="Search v2"]')).value).toBe(
      'Fix login',
    );
    expect(container.textContent).toContain('orch/fix-login/integration');
    await fireEvent.click(container.querySelector('button[type=submit]'));
    await waitFor(() =>
      expect(
        calls.some(([c, a]) => c === 'team_launch' && a.form.slug === 'fix-login' && a.form.team_id === 'duo'),
      ).toBe(true),
    );
  });

  it('the launch dialog lists the teams of the project picked', async () => {
    const view = (...ids) => ({
      teams: ids.map((id) => ({ team: { id, name: id, nodes: [], edges: [], version: 1 }, issues: [], rules: [] })),
      roles: [],
    });
    replies.teams_view = (a) => (a.project === 'blog' ? view('blog-team') : view('duo'));
    app.overlay = { kind: 'launch', teamId: 'duo', projectId: 'shop-api' };
    const { container } = render(Dialogs);
    await waitFor(() => expect(container.textContent).toContain('duo'));
    const [projectSel, teamSel] = container.querySelectorAll('select');
    projectSel.value = 'blog';
    await fireEvent.change(projectSel);
    await waitFor(() => expect(/** @type {HTMLSelectElement} */ (teamSel).value).toBe('blog-team'));
    expect(calls.filter(([c]) => c === 'teams_view').map(([, a]) => a.project)).toEqual(['shop-api', 'blog']);
    expect(container.textContent).not.toContain('duo');
  });

  it('the team canvas edits with keys and saves through the core', async () => {
    const team = {
      id: 'duo',
      name: 'Duo',
      scope: { kind: 'global' },
      version: 1,
      nodes: [
        { id: 'lead', role_id: 'lead', overrides: {}, position: [0, 0], max_concurrency: 1, is_lead: true },
        { id: 'back', role_id: 'backend', overrides: {}, position: [0, 200], max_concurrency: 1, is_lead: false },
      ],
      edges: [{ from: 'back', to: 'lead', kind: 'reports-to' }],
    };
    const role = (id, name, color) => ({
      id,
      name,
      color,
      model: null,
      effort: null,
      tools: { edit: true, bash: true, web: false },
      subagents_allowed: false,
      permission_profile: 'gated',
      budget_priority: 'normal',
      prompt: '',
      version: 1,
    });
    replies.teams_view = {
      teams: [
        {
          team,
          issues: [],
          rules: [
            ['lead', 'messages everyone'],
            ['back', 'reports to lead'],
          ],
        },
      ],
      roles: [role('lead', 'Lead', 1), role('backend', 'Backend', 2), role('ux', 'UX', 5)],
      presets: [],
    };
    replies.team_save = (a) => ({ team: { ...a.team, version: 2 }, issues: [] });
    app.panes.c1 = { type: 'canvas', teamId: 'duo', title: 'Team: duo' };
    const { container, getByText } = render(TeamCanvasPane, { paneId: 'c1' });
    await waitFor(() => getByText('reports to lead'));
    const pane = container.querySelector('.canvas-pane');
    expect(container.querySelectorAll('.node')).toHaveLength(2);
    // n then 3: add a UX node; the canvas shows it and flags it unreachable until connected.
    await fireEvent.keyDown(pane, { key: 'n' });
    await tick();
    await fireEvent.keyDown(pane, { key: '3' });
    await tick();
    expect(container.querySelectorAll('.node')).toHaveLength(3);
    expect(container.querySelector('.node.bad')).toBeTruthy();
    // c, target 1 (the lead), type 1 (reports-to).
    await fireEvent.keyDown(pane, { key: 'c' });
    await tick();
    await fireEvent.keyDown(pane, { key: '1' });
    await tick();
    await fireEvent.keyDown(pane, { key: '1' });
    await tick();
    expect(container.querySelector('.node.bad')).toBeNull();
    // u undoes the edge, ⌃r redoes it.
    await fireEvent.keyDown(pane, { key: 'u' });
    await tick();
    expect(container.querySelector('.node.bad')).toBeTruthy();
    await fireEvent.keyDown(pane, { key: 'r', ctrlKey: true });
    await tick();
    expect(container.querySelector('.node.bad')).toBeNull();
    // ⇧R opens the library with a node selected (⇧L moves the node).
    await fireEvent.keyDown(pane, { key: 'R', shiftKey: true });
    await tick();
    expect(container.querySelector('[aria-label="Library"]')).toBeTruthy();
    await fireEvent.keyDown(pane, { key: 'Escape' });
    await tick();
    expect(container.querySelector('[aria-label="Library"]')).toBeNull();
    await waitFor(() => expect(calls.some(([c, a]) => c === 'team_save' && a.team.nodes.length === 3)).toBe(true), {
      timeout: 2000,
    });
    // ⌘⏎ opens the launch dialog once saved and valid.
    await waitFor(() => getByText('valid'));
    await fireEvent.keyDown(pane, { key: 'Enter', metaKey: true });
    expect(app.overlay).toEqual({ kind: 'launch', teamId: 'duo', projectId: null });
    app.overlay = null;
  });
});
