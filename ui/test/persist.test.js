// What the layout setting remembers (design-notes §4.2, §5.4; api-contract):
// the sidebar tree, the workspace's project context, and the migration of a
// layout saved with splits to the single tab strip. (The dock's view, filter
// and per-view active tabs come with the dock views.)
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fixture } from './fixture.js';

const h = vi.hoisted(() => ({ saved: null, saves: [] }));
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  errText: (e) => String(e),
  listen: () => Promise.resolve(() => {}),
  invoke: (cmd, args) => {
    if (cmd === 'settings_get') return Promise.resolve(args.key === 'layout' ? h.saved : { projects: ['blog'] });
    if (cmd === 'settings_set') {
      if (args.key === 'layout') h.saves.push(args.value);
      return Promise.resolve(null);
    }
    if (cmd === 'term_spawn') return Promise.resolve({ id: 1, cwd: args.req.cwd ?? '/dev' });
    return Promise.resolve(null);
  },
}));

const A = await import('../src/lib/app.svelte.js');
const { app } = A;

const feature = (id, project_id) => ({ id, project_id, slug: id, branch: `orch/${id}`, run: null });

beforeEach(() => {
  app.data = { ...fixture(), features: [feature('shop-api-fx', 'shop-api')], setup: { done: true } };
  app.panes = {};
  app.layout = null;
  app.focused = null;
  app.dock = { open: false, height: 0.45, tabs: [], active: null, v: 2 };
  app.workspace = { project: 'all' };
  app.sidebarTree = {};
  h.saved = null;
  h.saves.length = 0;
});
afterEach(() => vi.useRealTimers());

describe('saveLayout', () => {
  it('writes sidebarTree and workspace next to the layout', () => {
    vi.useFakeTimers();
    app.sidebarTree = { 'shop-api': false, 'f:shop-api-fx': true };
    app.workspace = { project: 'blog' };
    A.saveLayout();
    vi.advanceTimersByTime(500);
    const v = h.saves.at(-1);
    expect(v.sidebarTree).toEqual({ 'shop-api': false, 'f:shop-api-fx': true });
    expect(v.workspace).toEqual({ project: 'blog' });
    expect(v).not.toHaveProperty('mission');
  });

  it('a caret click is saved (toggleTree), absent = expanded', () => {
    vi.useFakeTimers();
    expect(A.treeOpen('shop-api')).toBe(true);
    A.toggleTree('shop-api');
    expect(A.treeOpen('shop-api')).toBe(false);
    A.toggleTree('f:shop-api-fx');
    vi.advanceTimersByTime(500);
    expect(h.saves.at(-1).sidebarTree).toEqual({ 'shop-api': false, 'f:shop-api-fx': false });
  });
});

describe('restoreLayout: sidebarTree', () => {
  it('keeps known project and feature keys; prunes gone ones and non-booleans', async () => {
    h.saved = {
      layout: null,
      panes: {},
      sidebarTree: { 'shop-api': false, gone: false, 'f:shop-api-fx': false, 'f:gone-fx': true, blog: 'yes' },
    };
    await A.restoreLayout();
    expect(app.sidebarTree).toEqual({ 'shop-api': false, 'f:shop-api-fx': false });
  });

  it('features never read (null): feature keys stay; absent tree: {}', async () => {
    app.data = { ...app.data, features: null };
    h.saved = { layout: null, panes: {}, sidebarTree: { 'f:any': false } };
    await A.restoreLayout();
    expect(app.sidebarTree).toEqual({ 'f:any': false });
    h.saved = { layout: null, panes: {} };
    await A.restoreLayout();
    expect(app.sidebarTree).toEqual({});
  });
});

describe('restoreLayout: workspace.project (rules 5, 6)', () => {
  const panes = {
    p1: { type: 'mission', title: 'Mission control' },
    p2: { type: 'project', projectId: 'blog', title: 'blog' },
    p3: { type: 'board', projectId: 'shop-api', title: 'shop-api tasks' },
  };
  const layout = { kind: 'leaf', id: 'l1', tabs: ['p1', 'p2', 'p3'], active: 'p3' };

  it('the saved context; the active tab is one it shows', async () => {
    h.saved = { layout, panes, focused: 'p3', workspace: { project: 'blog' } };
    await A.restoreLayout();
    expect(app.workspace.project).toBe('blog');
    expect(A.visibleTabs()).toEqual(['p1', 'p2']);
    expect(app.layout.active).toBe('p1');
    expect(app.focused).toBe('p1');
  });

  it('an unknown saved project → All', async () => {
    h.saved = { layout, panes, focused: 'p3', workspace: { project: 'gone' } };
    await A.restoreLayout();
    expect(app.workspace.project).toBe('all');
    expect(app.focused).toBe('p3');
  });

  it('absent: the focused pane project, else All; the old `mission` key is not read', async () => {
    h.saved = { layout, panes, focused: 'p3' };
    await A.restoreLayout();
    expect(app.workspace.project).toBe('shop-api');
    h.saved = { layout, panes, focused: 'p1' };
    await A.restoreLayout();
    expect(app.workspace.project).toBe('all');
    expect('mission' in app).toBe(false);
  });
});

describe('restoreLayout: a layout saved with splits (design-notes §4.2)', () => {
  // (p1 | p2 tabs p3) / p4, focused p3; p5 an agent moves to the dock.
  const tree = {
    kind: 'split',
    id: 's1',
    dir: 'col',
    sizes: [0.5, 0.5],
    children: [
      {
        kind: 'split',
        id: 's2',
        dir: 'row',
        sizes: [0.5, 0.5],
        children: [
          { kind: 'leaf', id: 'l1', tabs: ['p1'], active: 'p1' },
          { kind: 'leaf', id: 'l2', tabs: ['p2', 'p3', 'p5'], active: 'p3' },
        ],
      },
      { kind: 'leaf', id: 'l3', tabs: ['p4'], active: 'p4' },
    ],
  };
  const panes = {
    p1: { type: 'mission', title: 'Mission control' },
    p2: { type: 'project', projectId: 'shop-api', title: 'shop-api' },
    p3: { type: 'feature', featureId: 'shop-api-fx', projectId: 'shop-api', title: 'fx' },
    p4: { type: 'report', reportId: 'current', title: 'Weekly report' },
    p5: { type: 'agent', agentId: 'back-1', title: 'back-1' },
  };

  it('restores one tab strip in reading order with the focused tab active', async () => {
    h.saved = { layout: tree, panes, focused: 'p3', workspace: { project: 'all' } };
    await A.restoreLayout();
    expect(app.layout.kind).toBe('leaf');
    expect(app.layout.tabs).toEqual(['p1', 'p2', 'p3', 'p4']);
    expect(app.layout.active).toBe('p3');
    expect(app.focused).toBe('p3');
    expect(app.dock.tabs).toEqual(['p5']);
  });

  it('a focused pane that is gone: the first group active tab', async () => {
    h.saved = { layout: tree, panes: { ...panes, p3: { ...panes.p3, featureId: 'gone' } }, focused: 'p3' };
    await A.restoreLayout();
    expect(app.layout.tabs).toEqual(['p1', 'p2', 'p4']);
    expect(app.layout.active).toBe('p1');
  });

  it('the saved layout key keeps its shape: a leaf, which an older build still loads', async () => {
    vi.useFakeTimers();
    h.saved = { layout: tree, panes, focused: 'p3' };
    await A.restoreLayout();
    A.saveLayout();
    vi.advanceTimersByTime(500);
    expect(h.saves.at(-1).layout).toMatchObject({ kind: 'leaf', tabs: ['p1', 'p2', 'p3', 'p4'], active: 'p3' });
  });
});
