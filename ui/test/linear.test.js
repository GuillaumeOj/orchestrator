// T-12: Project settings › Task backend Linear (FR-501, FR-503) and the
// Board's Linear cards, against a mocked core.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { mappingRows, mappingToSave, syncLine } from '../src/lib/linear.js';
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
    expect(syncLine({ pushed: 7, pulled: 2, imported: 0, updated: 1, conflicts: 1 }, at)).toBe(
      'Last sync 15:04: 7 pushed · 2 pulled · 1 conflict (Linear won)',
    );
    expect(syncLine({ pushed: 7, pulled: 0, imported: 0, updated: 0, conflicts: 0 }, at)).toBe(
      'Last sync 15:04: 7 pushed · 0 pulled · 0 conflicts',
    );
    expect(handle({ id: 3, backend: 'linear', external_id: 'ENG-142' })).toBe('ENG-142');
  });
});

describe('Board › Linear', () => {
  beforeEach(() => {
    cleanup();
    calls.length = 0;
    for (const k of Object.keys(replies)) delete replies[k];
    app.data = fixture();
    app.toasts = [];
    app.panes = {};
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
