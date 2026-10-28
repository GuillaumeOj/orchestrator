// T-12: Project settings › Task backend Linear (FR-501, FR-503) and the
// Board's Linear cards, against a mocked core.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { backendForm, mappingRows, mappingToSave, syncLine } from '../src/lib/linear.js';
import { handle } from '../src/lib/teams.js';
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

const { default: ProjectSettingsPane } = await import('../src/panes/ProjectSettingsPane.svelte');
const { default: BoardPane } = await import('../src/panes/BoardPane.svelte');

const STATES = [
  { id: 's1', name: 'Backlog', type: 'backlog', default_column: 'backlog' },
  { id: 's2', name: 'Todo', type: 'unstarted', default_column: 'ready' },
  { id: 's3', name: 'In Progress', type: 'started', default_column: 'in_progress' },
  { id: 's4', name: 'In Review', type: 'started', default_column: 'review' },
  { id: 's5', name: 'Done', type: 'completed', default_column: 'done' },
  { id: 's6', name: 'Canceled', type: 'canceled', default_column: 'done' },
];

describe('linear helpers', () => {
  it('mapping rows, what is saved, the sync line and the D8 counts', () => {
    const rows = mappingRows(STATES, { s4: 'in_progress' });
    expect(rows[3]).toMatchObject({ name: 'In Review', column: 'in_progress', isDefault: false });
    expect(rows[0].isDefault).toBe(true);
    expect(mappingToSave(rows)).toEqual({ s4: 'in_progress' });
    expect(mappingToSave(mappingRows(STATES))).toBe(undefined);
    const at = new Date(2026, 9, 21, 15, 4).getTime();
    expect(syncLine({ pushed: 7, imported: 0, updated: 2, conflicts: 1 }, at)).toBe(
      'Last sync 15:04: 7 pushed · 2 pulled · 1 conflict (Linear won)',
    );
    expect(syncLine({ pushed: 7, imported: 0, updated: 0, conflicts: 0 }, at)).toBe(
      'Last sync 15:04: 7 pushed · 0 pulled · 0 conflicts',
    );
    expect(backendForm('linear', { team: 'ENG', project: null, states: undefined })).toEqual({
      type: 'linear',
      team: 'ENG',
    });
    expect(handle({ id: 3, backend: 'linear', external_id: 'ENG-142' })).toBe('ENG-142');
  });
});

describe('project settings › Task backend', () => {
  beforeEach(() => {
    cleanup();
    calls.length = 0;
    for (const k of Object.keys(replies)) delete replies[k];
    app.data = fixture();
    app.toasts = [];
    app.panes = {};
  });

  const settings = (backend = { type: 'local' }) => ({
    settings: { copy_env: false },
    backend,
    track_orchestrator_dir: true,
    default_branch: 'main',
  });
  const linear = (status) => ({
    items: [
      {
        id: 'linear',
        name: 'Linear',
        status,
        has_secret: status !== 'not_configured',
        needs_secret: true,
        account: 'Acme',
      },
    ],
  });

  it('not connected: the Linear option stays enabled and points to Integrations', async () => {
    replies.project_settings = settings();
    replies.integrations_view = linear('not_configured');
    app.panes.p1 = { type: 'project-settings', projectId: 'shop-api', section: 'backend', title: 'shop-api settings' };
    const { container, getByText } = render(ProjectSettingsPane, { paneId: 'p1' });
    await waitFor(() => getByText(/Where shop-api's Board lives/));
    await fireEvent.keyDown(container.querySelector('[data-pane-focus]'), { key: '3' });
    await tick();
    expect(container.textContent).toContain('Linear is not connected. Add a key in Settings › Integrations');
    expect(calls.some(([c]) => c === 'linear_teams')).toBe(false);
  });

  it('connected: team, mapping with defaults, the D8 confirm, then Sync now', async () => {
    replies.project_settings = settings();
    replies.integrations_view = linear('connected');
    replies.linear_teams = {
      teams: [
        { id: 't1', key: 'ENG', name: 'Engineering' },
        { id: 't2', key: 'OPS', name: 'Operations' },
      ],
    };
    replies.linear_states = { states: STATES, projects: [{ id: 'p1', name: 'Checkout' }] };
    // D8 as the core answers it: nothing saved, a confirm with the counts, until form.migrate.
    replies.project_settings_set = (a) =>
      a.form.migrate ? settings(a.form.backend) : { confirm: { kind: 'linear_migrate', open: 7, done: 12 } };
    replies.board_sync = { imported: 0, updated: 0, pushed: 7, pulled: 0, conflicts: 0, errors: [] };
    app.panes.p1 = { type: 'project-settings', projectId: 'shop-api', section: 'backend', title: 'shop-api settings' };
    const { container, getByText } = render(ProjectSettingsPane, { paneId: 'p1' });
    await waitFor(() => getByText(/Where shop-api's Board lives/));
    const root = container.querySelector('[data-pane-focus]');
    await fireEvent.keyDown(root, { key: '3' });
    await waitFor(() => expect(container.querySelectorAll('.mrow')).toHaveLength(6));
    expect(container.textContent).toContain('Connected · Acme');
    expect(container.textContent).toContain('2 teams: ENG · Engineering, OPS · Operations');
    expect(container.querySelectorAll('.mrow')[1].textContent).toContain('Todo');
    expect([...container.querySelectorAll('.mrow')].every((r) => r.textContent.includes('default'))).toBe(true);
    // j j ⏎: the third row's select; move In Progress to Review.
    await fireEvent.keyDown(root, { key: 'j' });
    await fireEvent.keyDown(root, { key: 'j' });
    expect(container.querySelector('.mrow.sel').textContent).toContain('In Progress');
    const sel = /** @type {HTMLSelectElement} */ (container.querySelector('.mrow.sel select'));
    sel.value = 'review';
    await fireEvent.change(sel);
    expect(container.querySelector('.mrow.sel').textContent).not.toContain('default');
    // ⌘⏎: refused with counts → the D8 confirm; ⏎ creates the open tasks.
    await fireEvent.keyDown(root, { key: 'Enter', metaKey: true });
    await waitFor(() => getByText("Move shop-api's tasks to Linear?"));
    expect(container.textContent).toContain('7 open tasks. 12 done tasks are not pushed.');
    expect(container.textContent).toContain('Create 7 open tasks as issues in ENG');
    await fireEvent.keyDown(root, { key: 'Enter' });
    await waitFor(() =>
      expect(
        calls.some(
          ([c, a]) =>
            c === 'project_settings_set' &&
            a.form.migrate === 'push_open' &&
            a.migrate === undefined &&
            a.form.backend.type === 'linear' &&
            a.form.backend.team === 'ENG' &&
            a.form.backend.states.s3 === 'review',
        ),
      ).toBe(true),
    );
    await waitFor(() => expect(container.textContent).not.toContain("Move shop-api's tasks to Linear?"));
    await fireEvent.keyDown(root, { key: 'r' });
    await waitFor(() =>
      expect(container.textContent).toMatch(/Last sync \d\d:\d\d: 7 pushed · 0 pulled · 0 conflicts/),
    );
    expect(app.toasts.at(-1).text).toBe('Synced shop-api with Linear · 7 pushed');
  });

  it('sync errors show, an unmapped state offers Map it', async () => {
    replies.project_settings = settings({ type: 'linear', team: 'ENG' });
    replies.integrations_view = linear('connected');
    replies.linear_teams = { teams: [{ id: 't1', key: 'ENG', name: 'Engineering' }] };
    replies.linear_states = { states: STATES, projects: [] };
    replies.board_sync = {
      imported: 0,
      updated: 0,
      pushed: 0,
      conflicts: 0,
      errors: ['ENG-150: state "QA" isn\'t mapped.'],
    };
    app.panes.p1 = { type: 'project-settings', projectId: 'shop-api', section: 'backend', title: 'shop-api settings' };
    const { container, getByText } = render(ProjectSettingsPane, { paneId: 'p1' });
    await waitFor(() => expect(container.querySelectorAll('.mrow')).toHaveLength(6));
    await fireEvent.click(getByText('Sync now', { exact: false }));
    await waitFor(() => getByText('ENG-150: state "QA" isn\'t mapped.', { exact: false }));
    expect(getByText('Map it', { exact: false })).toBeTruthy();
  });

  it('board on Linear: the header, the ENG ids and the remote-won marker', async () => {
    const task = (id, ext, over = {}) => ({
      id,
      project_id: 'shop',
      backend: 'linear',
      external_id: ext,
      feature_id: null,
      team_run_id: null,
      title: `Task ${id}`,
      body: '',
      status: 'ready',
      node: null,
      agent_id: null,
      depends_on: [],
      branch: null,
      notes: [],
      created_at_ms: 1,
      updated_at_ms: 1,
      ...over,
    });
    replies.board_view = {
      project: { id: 'shop', name: 'shop-api' },
      backend: 'linear',
      linear_team: 'ENG',
      repo: null,
      last_sync_error: null,
      tasks: [task(1, 'ENG-142'), task(2, 'ENG-140', { remote_won: true })],
      features: [],
      runs: [],
    };
    replies.board_sync = { imported: 0, updated: 0, pushed: 7, pulled: 1, conflicts: 1, errors: [] };
    app.panes.b1 = { type: 'board', projectId: 'shop', title: 'shop tasks' };
    const { container, getByText } = render(BoardPane, { paneId: 'b1' });
    await waitFor(() => getByText('ENG-142'));
    expect(container.textContent).toContain('Linear · ENG');
    expect(container.textContent).toContain('↓ changed in Linear');
    await fireEvent.keyDown(container.querySelector('.board'), { key: 'r' });
    await waitFor(() =>
      expect(app.toasts.at(-1)?.text).toBe(
        'Synced shop-api with Linear · 7 pushed · 1 pulled · 1 conflict (Linear won)',
      ),
    );
  });
});
