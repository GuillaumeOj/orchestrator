// improve-intake F2 (notes §1.3, §9, §12.5): the Feature pane's source header and Tasks section, the
// Board's removal and where its entry points go. Against a mocked core.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as A from '../src/lib/app.svelte.js';
import { app } from '../src/lib/app.svelte.js';
import { GROUPS } from '../src/lib/shortcuts.js';
import { sourceModel, taskMarks, tasksSummary } from '../src/lib/source.js';
import { fixture, nowOf } from './fixture.js';

const calls = [];
const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    const r = replies[cmd];
    return r instanceof Error ? Promise.reject(r) : Promise.resolve(typeof r === 'function' ? r(args) : (r ?? null));
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => String(e?.message ?? e),
}));
const openWork = vi.fn();
vi.mock('../src/lib/work.js', () => ({ openWork: (...a) => openWork(...a) }));

const { default: FeaturePane } = await import('../src/panes/FeaturePane.svelte');
const { default: WaitingChip } = await import('../src/components/WaitingChip.svelte');
const { default: RunInspector } = await import('../src/components/inspector/RunInspector.svelte');

const SOURCE = {
  node: 'gh:acme/shop#13',
  ref: '#13',
  title: 'Address step',
  url: 'https://github.com/acme/shop/issues/13',
  breadcrumb: [
    { id: 'gh-milestone:acme/shop/3', ref: 'v2', title: 'v2' },
    { id: 'gh:acme/shop#12', ref: '#12', title: 'Checkout v2' },
  ],
  tracker: 'github',
  program: { id: 'prg-1', title: 'Checkout v2', index: 1, total: 3 },
  deleted: false,
  closed_upstream: false,
  write_back: 'on',
};
const task = (id, status, /** @type {any} */ over = {}) => ({
  id,
  project_id: 'shop-api',
  backend: 'github',
  external_id: String(69 + id),
  feature_id: 'f1',
  team_run_id: 'run-1',
  title: `Task ${id}`,
  body: '',
  status,
  node: 'back',
  depends_on: [],
  branch: null,
  remote_url: `https://github.com/acme/shop/issues/${69 + id}`,
  ...over,
});
const view = (over = {}, runOver = {}) => ({
  feature: { id: 'f1', project_id: 'shop-api', slug: 'checkout', integration_worktree: '/w/co' },
  run: {
    id: 'run-1',
    status: 'executing',
    team_id: 't1',
    started_at_ms: Date.now() - 1000,
    source: SOURCE,
    ...runOver,
  },
  team: {
    name: 'Squad',
    version: 1,
    nodes: [
      { id: 'lead', is_lead: true, role_id: 'lead' },
      { id: 'back', role_id: 'backend' },
    ],
  },
  project: { id: 'shop-api', name: 'shop-api', backend: 'github' },
  agents: [],
  tasks: [task(1, 'in_progress'), task(2, 'review'), task(3, 'ready', { depends_on: [2] }), task(4, 'done')],
  docs: [],
  artifacts: [],
  timeline: [],
  ...over,
});

async function pane(v = view(), spec = {}) {
  replies.feature_view = v;
  app.panes = { f1: { type: 'feature', featureId: 'f1', ...spec } };
  const r = render(FeaturePane, { paneId: 'f1' });
  await waitFor(() => expect(r.container.querySelector('[data-tasks]')).toBeTruthy());
  return r;
}
const key = (el, k, extra = {}) => fireEvent.keyDown(el, { key: k, ...extra });

beforeEach(() => {
  cleanup();
  calls.length = 0;
  openWork.mockClear();
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.toasts = [];
  app.panes = {};
  app.layout = null;
});

describe('sourceModel (notes §9.1)', () => {
  it('GitHub item, write-back on: item, tracker, trail with the program, Write-back on', () => {
    const m = sourceModel({ source: SOURCE });
    expect(m).toMatchObject({ kind: 'source', ref: '#13', tracker: 'GitHub', canShowInWork: true });
    expect(m.trail).toBe('in v2 › #12 Checkout v2 · program Checkout v2 (1 of 3)');
    expect(m.writeBack).toEqual({ state: 'on', text: 'Write-back on' });
  });
  it('Linear, write-back off, local tracker', () => {
    expect(sourceModel({ source: { ...SOURCE, tracker: 'linear', write_back: 'off', ref: 'ENG-110' } })).toMatchObject({
      glyph: '◐',
      tracker: 'Linear',
      writeBack: { state: 'off', text: 'Write-back off' },
    });
    expect(sourceModel({ source: { ...SOURCE, tracker: 'local' } }).writeBack).toBeNull();
  });
  it('no source: by you / from the Overwatch brief; no Show in Work', () => {
    expect(sourceModel({})).toMatchObject({
      kind: 'none',
      title: 'No tracker item · started by you',
      canShowInWork: false,
    });
    expect(sourceModel({}, { title: 'Partial refunds' })).toMatchObject({
      kind: 'brief',
      title: "From the Overwatch's brief: Partial refunds",
    });
  });
  it('write-back failed (S15), closed upstream, deleted upstream (S14)', () => {
    expect(
      sourceModel({
        source: { ...SOURCE, writeback: { status: 'failed', message: 'could not move #13 to In progress' } },
      }).writeBack,
    ).toEqual({ state: 'failed', text: 'Write-back failed: could not move #13 to In progress' });
    expect(sourceModel({ source: { ...SOURCE, closed_upstream: true } }).closed).toBe('closed in GitHub');
    expect(sourceModel({ source: { ...SOURCE, deleted: true } })).toMatchObject({
      deleted: '#13 was deleted in GitHub. The run continues.',
      canShowInWork: false,
    });
  });
  it('tasksSummary and task marks', () => {
    const ts = [task(1, 'done'), task(2, 'ready')];
    expect(tasksSummary(SOURCE, ts)).toBe('GitHub sub-issues of #13 · 1 of 2 done');
    expect(tasksSummary({ ...SOURCE, write_back: 'off' }, ts)).toBe('Local · 1 of 2 done');
    expect(tasksSummary(null, ts)).toBe('Local · 1 of 2 done');
    expect(taskMarks({ sync_state: 'pending' })).toEqual({ notSynced: true, conflict: false });
    expect(taskMarks({ remote_won: true }).conflict).toBe(true);
  });
});

describe('Feature pane: source header', () => {
  it('shows the item, trail and write-back; Show in Work opens Work revealed at the item', async () => {
    const { container, getByText } = await pane();
    const src = container.querySelector('[data-source]');
    expect(src.textContent).toContain('#13 Address step');
    expect(src.textContent).toContain('GitHub ↗');
    expect(src.textContent).toContain('program Checkout v2 (1 of 3)');
    expect(src.textContent).toContain('Write-back on');
    await fireEvent.click(getByText('Show in Work'));
    expect(openWork).toHaveBeenCalledWith({ projects: ['shop-api'], reveal: 'gh:acme/shop#13' });
    await key(container.querySelector('.feature'), 'w');
    expect(openWork).toHaveBeenCalledTimes(2);
  });
  it('no source: no Show in Work, `w` does nothing', async () => {
    const { container, queryByText } = await pane(view({}, { source: null }));
    expect(container.querySelector('[data-source="none"]').textContent).toContain('No tracker item · started by you');
    expect(queryByText('Show in Work')).toBeNull();
    await key(container.querySelector('.feature'), 'w');
    expect(openWork).not.toHaveBeenCalled();
  });
  it('write-back failed: the line has Retry, which calls writeback_retry', async () => {
    replies.writeback_retry = null;
    const { container, getByText } = await pane(
      view(
        {},
        { source: { ...SOURCE, writeback: { status: 'failed', message: 'could not move #13 to In progress' } } },
      ),
    );
    expect(container.querySelector('[data-writeback="failed"]').textContent).toContain(
      'Write-back failed: could not move #13 to In progress',
    );
    await fireEvent.click(getByText('Retry'));
    expect(calls.some(([c, a]) => c === 'writeback_retry' && a.node === 'gh:acme/shop#13')).toBe(true);
  });
  it('closed upstream tail; deleted upstream hides Show in Work', async () => {
    const a = await pane(view({}, { source: { ...SOURCE, closed_upstream: true } }));
    expect(a.container.querySelector('[data-source]').textContent).toContain('closed in GitHub');
    cleanup();
    const b = await pane(view({}, { source: { ...SOURCE, deleted: true } }));
    expect(b.container.querySelector('[data-source]').textContent).toContain(
      '#13 was deleted in GitHub. The run continues.',
    );
    expect(b.queryByText('Show in Work')).toBeNull();
  });
});

describe('Feature pane: Tasks', () => {
  it('five columns with the Board order, cards `T-4 · #73`, blocked mark, summary', async () => {
    const { container } = await pane();
    const t = container.querySelector('[data-tasks]');
    expect([...t.querySelectorAll('.colh b')].map((b) => b.textContent)).toEqual([
      'Backlog',
      'Ready',
      'In progress',
      'Review',
      'Done',
    ]);
    expect(t.textContent).toContain('GitHub sub-issues of #13 · 1 of 4 done');
    expect(t.querySelector('[data-task="4"]').textContent).toContain('T-4 · #73');
    expect(t.textContent).toContain('blocked by T-2');
    expect(t.textContent).toContain('No tasks');
  });
  it('not synced and conflict marks; conflict menu has Resolve conflict…', async () => {
    const v = view();
    v.tasks[2].sync_state = 'pending';
    v.tasks[0].sync_state = 'conflict';
    const { container } = await pane(v);
    expect(container.querySelector('[data-task="3"]').textContent).toContain('not synced');
    expect(container.querySelector('[data-task="1"]').textContent).toContain('conflict');
  });

  it('b focuses Tasks; its keys move and change the focused task; Esc returns to the pane', async () => {
    const v = view();
    replies.task_update = (a) => {
      v.tasks.find((t) => t.id === a.id).status = a.status;
    };
    const { container } = await pane(v);
    replies.feature_view = () => structuredClone(v);
    const root = container.querySelector('.feature');
    const tasks = container.querySelector('[data-tasks]');
    await key(root, 'b');
    expect(document.activeElement).toBe(tasks);
    expect(/** @type {HTMLElement} */ (tasks.querySelector('.task.sel')).dataset.task).toBe('3'); // first non-empty column: Ready
    await key(tasks, 'l'); // In progress: T-1
    await tick();
    expect(/** @type {HTMLElement} */ (tasks.querySelector('.task.sel')).dataset.task).toBe('1');
    await key(tasks, '4'); // Review
    await waitFor(() =>
      expect(calls.some(([c, a]) => c === 'task_update' && a.id === 1 && a.status === 'review')).toBe(true),
    );
    await waitFor(() =>
      expect(tasks.querySelector('.task.sel').closest('.tcol').getAttribute('aria-label')).toBe('Review'),
    );
    await key(tasks, 'L', { shiftKey: true });
    await waitFor(() =>
      expect(calls.some(([c, a]) => c === 'task_update' && a.id === 1 && a.status === 'done')).toBe(true),
    );
    await key(tasks, 'Escape');
    expect(document.activeElement).toBe(root);
  });

  it('while Tasks has focus the pane keeps its p / e / m; its own e edits and n opens the new-task form', async () => {
    replies.run_act = {};
    replies.task_update = {};
    const { container, getByText } = await pane();
    const root = container.querySelector('.feature');
    const tasks = container.querySelector('[data-tasks]');
    // Not focused: p pauses the team.
    await key(root, 'p');
    await waitFor(() => expect(calls.some(([c, a]) => c === 'run_act' && a.action === 'pause')).toBe(true));
    calls.length = 0;
    await key(root, 'b');
    await key(tasks, 'p'); // bubbles to the pane, which leaves it to Tasks
    await key(tasks, 'm'); // Tasks' own: Move to menu, never the message box
    await tick();
    expect(calls.some(([c]) => c === 'run_act')).toBe(false);
    expect(document.activeElement).not.toBe(container.querySelector('.msg input'));
    await fireEvent.keyDown(document.activeElement, { key: 'Escape' });
    await key(tasks, 'e');
    await tick();
    expect(getByText('Edit #72')).toBeTruthy();
    const ta = container.querySelector('[data-tasks] textarea');
    await fireEvent.input(ta, { target: { value: 'new body' } });
    await fireEvent.keyDown(ta, { key: 'Enter', metaKey: true });
    await waitFor(() =>
      expect(calls.some(([c, a]) => c === 'task_update' && a.body === 'new body' && a.title === null)).toBe(true),
    );
    await key(tasks, 'n');
    await tick();
    expect(container.querySelector('input[placeholder="Task title"]')).toBeTruthy();
  });

  it('o opens the upstream task; a assigns through the node picker', async () => {
    const opened = [];
    replies.open_url = (a) => opened.push(a);
    const { container, getByText } = await pane();
    const root = container.querySelector('.feature');
    const tasks = container.querySelector('[data-tasks]');
    await key(root, 'b');
    await key(tasks, 'a');
    await tick();
    expect(getByText('Assign #72 to')).toBeTruthy();
  });

  it('landing: task focused and flashed (once), a conflict opens its menu at Resolve conflict…', async () => {
    const v = view();
    v.tasks[0].sync_state = 'conflict';
    const { container } = await pane(v, { land: { task: '1', conflict: true, seq: 1 } });
    const sel = /** @type {HTMLElement} */ (container.querySelector('[data-tasks] .task.sel'));
    expect(sel.dataset.task).toBe('1');
    expect(document.activeElement).toBe(container.querySelector('[data-tasks]'));
    await waitFor(() =>
      expect(document.body.querySelector('[role="menu"][aria-label="Task menu"]')?.textContent).toContain(
        'Resolve conflict…',
      ),
    );
  });
});

describe('the Board is gone: entry points', () => {
  const withFeature = () => {
    app.data = {
      ...fixture(),
      features: [{ id: 'f1', project_id: 'shop-api', slug: 'checkout', run: { id: 'run-1' } }],
    };
  };

  it('openTask lands in the Feature pane of the task’s run', async () => {
    withFeature();
    await A.openTask('shop-api', 'T-12', { run: 'run-1' });
    const p = Object.values(app.panes).find((s) => s.type === 'feature');
    expect(p).toMatchObject({ featureId: 'f1', land: { task: '12', conflict: false } });
    expect(Object.values(app.panes).some((s) => s.type === 'board')).toBe(false);
  });
  it('openTask finds the run by asking the project’s features when only the task is known', async () => {
    withFeature();
    replies.feature_view = view();
    await A.openTask('shop-api', 3);
    expect(Object.values(app.panes).find((s) => s.type === 'feature')?.land.task).toBe('3');
  });
  it('a run that is gone: a toast with Open upstream', async () => {
    withFeature();
    const open = vi.fn();
    replies.open_url = open;
    await A.openTask('shop-api', 12, {
      run: 'run-gone',
      feature: 'checkout-old',
      remote_url: 'https://github.com/acme/shop/issues/70',
      ref: '#70',
      tracker: 'GitHub',
    });
    expect(app.panes).toEqual({});
    const t = app.toasts.at(-1);
    expect(t.text).toBe('T-12 belongs to checkout-old (deleted). Open #70 in GitHub?');
    expect(t.action.label).toBe('Open');
  });
  it('the waiting chip T-n opens the task in its Feature pane', async () => {
    withFeature();
    const rec = {
      id: 'a1',
      project: 'shop-api',
      run: 'run-1',
      level: 'waiting',
      reason: 'review of T-4',
      waiting_on: { kind: 'task', ref: 'T-4' },
    };
    const { container } = render(WaitingChip, { rec });
    const b = container.querySelector('button');
    await fireEvent.click(b);
    await waitFor(() => expect(Object.values(app.panes).find((s) => s.type === 'feature')?.land.task).toBe('4'));
  });
  it('the run inspector: Show tasks (and Show in Work when the run has a source), no Open board', async () => {
    withFeature();
    app.data.features[0].run = { id: 'run-1', source: SOURCE };
    app.now = nowOf(app.data, {
      runs: {
        'run-1': {
          run: 'run-1',
          feature: 'checkout',
          project: 'shop-api',
          phase: 'executing',
          members: [],
          counts: {},
        },
      },
    });
    const { queryByText } = render(RunInspector, { id: 'run-1' });
    expect(queryByText('Open board')).toBeNull();
    expect(queryByText('Show tasks')).toBeTruthy();
    await fireEvent.click(/** @type {HTMLElement} */ (queryByText('Show in Work')));
    expect(openWork).toHaveBeenCalledWith({ projects: ['shop-api'], reveal: 'gh:acme/shop#13' });
  });
  it('the removal toast shows once', async () => {
    let seen = null;
    replies.settings_get = (a) => (a.key === 'work.board_notice_seen' ? seen : null);
    replies.settings_set = (a) => {
      if (a.key === 'work.board_notice_seen') seen = a.value;
    };
    await A.boardNotice();
    expect(app.toasts.at(-1).text).toBe(
      'The Board is gone: tracker items are in Work (w), team tasks are here in Tasks.',
    );
    app.toasts = [];
    await A.boardNotice();
    expect(app.toasts).toEqual([]);
  });
  it('shortcut sheet: no Board group; Feature `b` is Tasks, `w` Show source in Work; the palette has no Board entry', () => {
    expect(GROUPS.some((g) => g.title === 'Board')).toBe(false);
    const f = GROUPS.find((g) => g.id === 'feature');
    expect(f.rows.map((r) => r.label)).toEqual(expect.arrayContaining(['Tasks', 'Show source in Work']));
    expect(f.rows.some((r) => r.label === 'Board')).toBe(false);
    expect(A.commands().some((c) => /\bboard\b/i.test(c.label))).toBe(false);
    expect(A.commands().some((c) => c.id.startsWith('work.open'))).toBe(true);
  });
});
