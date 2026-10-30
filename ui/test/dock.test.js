import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fixture } from './fixture.js';

let nextTerm = 1;
const h = vi.hoisted(() => ({ saved: null }));
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
  listen: () => Promise.resolve(() => {}),
  invoke: (cmd, args) => {
    if (cmd === 'term_spawn') return Promise.resolve({ id: nextTerm++, cwd: args.req.cwd ?? '/dev/shop-api' });
    if (cmd === 'settings_set') return Promise.resolve(null);
    if (cmd === 'settings_get') return Promise.resolve(h.saved);
    if (cmd === 'trust_command') return Promise.resolve('claude');
    return Promise.reject(new Error(`not mocked: ${cmd}`));
  },
}));

const {
  app,
  closePane,
  closePaneByUser,
  cycleDock,
  dockFills,
  focusDirection,
  focusPane,
  inDock,
  openAgent,
  openShell,
  openTrust,
  reopenClosed,
  restoreLayout,
  runInPane,
  toggleDock,
  toggleZoom,
} = await import('../src/lib/app.svelte.js');

beforeEach(() => {
  app.data = fixture();
  app.panes = { p1: { type: 'agent', agentId: 'back-1', title: 'back-1' } };
  app.layout = { kind: 'leaf', id: 'l1', tabs: ['p1'], active: 'p1' };
  app.focused = 'p1';
  app.dock = { open: false, height: 0.45, tabs: [], active: null, v: 2 };
  app.zoomed = null;
  h.saved = null;
});

describe('shell dock', () => {
  it('opens shells as dock tabs, never as panes', async () => {
    await openShell();
    await openShell({ cwd: '/tmp' });
    expect(app.dock.tabs).toHaveLength(2);
    expect(app.dock.open).toBe(true);
    expect(app.layout.tabs).toEqual(['p1']);
    expect(app.focused).toBe(app.dock.tabs[1]);
    expect(app.dock.active).toBe(app.dock.tabs[1]);
    // A split still makes a pane.
    await openShell({ how: 'right' });
    expect(app.layout.kind).toBe('split');
    expect(app.dock.tabs).toHaveLength(2);
  });

  it('cycles, hides and shows, and moves focus in and out', async () => {
    await openShell();
    await openShell();
    const [a, b] = app.dock.tabs;
    cycleDock(1);
    expect(app.focused).toBe(a);
    focusDirection('up');
    expect(app.focused).toBe('p1');
    focusDirection('down');
    expect(app.focused).toBe(a);
    toggleDock();
    expect(app.dock.open).toBe(false);
    expect(app.focused).toBe('p1');
    toggleDock();
    expect(app.dock.open).toBe(true);
    expect(app.focused).toBe(a);
    expect(inDock(b)).toBe(true);
  });

  it('closing the last shell closes the dock', async () => {
    await openShell();
    const [a] = app.dock.tabs;
    closePane(a);
    expect(app.dock.tabs).toEqual([]);
    expect(app.dock.open).toBe(false);
    expect(app.focused).toBe('p1');
    expect(app.panes[a]).toBeUndefined();
  });
});

const agentTab = (agentId) => app.dock.tabs.find((t) => app.panes[t].agentId === agentId);

describe('agents in the dock', () => {
  it('opens an agent as a focused dock tab and focuses its tab when opened again', async () => {
    openAgent('front-1');
    const t = agentTab('front-1');
    expect(app.panes[t]).toMatchObject({ type: 'agent', agentId: 'front-1', projectId: 'shop-api' });
    expect(app.dock).toMatchObject({ open: true, active: t });
    expect(app.focused).toBe(t);
    expect(app.layout.tabs).toEqual(['p1']); // the workspace is unchanged
    await openShell();
    const shell = app.dock.active;
    openAgent('front-1');
    expect(app.dock.tabs).toHaveLength(2);
    expect(app.dock.active).toBe(t);
    expect(app.focused).toBe(t);
    expect(inDock(shell)).toBe(true);
  });

  it('puts agents and shells side by side, in the order they were opened', async () => {
    openAgent('front-1');
    await openShell();
    openAgent('rss-feed');
    expect(app.dock.tabs.map((t) => (app.panes[t].type === 'agent' ? app.panes[t].agentId : 'shell'))).toEqual([
      'front-1',
      'shell',
      'rss-feed',
    ]);
    cycleDock(1);
    expect(app.panes[app.focused].agentId).toBe('front-1');
  });

  it('opens an agent already in the workspace by focusing it there', () => {
    openAgent('back-1'); // p1 is back-1's pane in the workspace
    expect(app.dock.tabs).toEqual([]);
    expect(app.focused).toBe('p1');
  });

  it('closing an agent tab keeps the agent; reopening brings the tab back', () => {
    openAgent('front-1');
    const t = agentTab('front-1');
    closePaneByUser(t);
    expect(app.dock.tabs).toEqual([]);
    expect(app.dock.open).toBe(false);
    expect(app.panes[t]).toBeUndefined();
    return reopenClosed().then(() => {
      expect(app.panes[agentTab('front-1')]).toMatchObject({ type: 'agent', agentId: 'front-1' });
      expect(app.dock.open).toBe(true);
      expect(app.layout.tabs).toEqual(['p1']);
    });
  });

  it('runInPane and the trust shell open as dock tabs', async () => {
    await runInPane('gh auth login', 'gh auth login', 'shop-api');
    await openTrust('shop-api');
    expect(app.dock.tabs.map((t) => app.panes[t].title)).toEqual(['gh auth login', 'trust · shop-api']);
    expect(app.layout.kind).toBe('leaf');
  });
});

describe('zoom and the dock filling the column', () => {
  it('zooms a dock tab over the column and restores it', async () => {
    await openShell();
    const [a] = app.dock.tabs;
    expect(dockFills()).toBe(false);
    toggleZoom();
    expect(app.zoomed).toBe(a);
    expect(dockFills()).toBe(true);
    toggleZoom();
    expect(app.zoomed).toBe(null);
    expect(dockFills()).toBe(false);
  });

  it('keeps the zoom across dock tabs and ends it on the other side', async () => {
    await openShell();
    openAgent('front-1');
    const [a, b] = app.dock.tabs;
    toggleZoom();
    expect(app.zoomed).toBe(b);
    cycleDock(1);
    expect(app.focused).toBe(a);
    expect(app.zoomed).toBe(a);
    focusPane('p1');
    expect(app.zoomed).toBe(null);
    toggleZoom(); // a workspace zoom
    expect(app.zoomed).toBe('p1');
    focusPane(a);
    expect(app.zoomed).toBe(null);
  });

  it('hiding the dock ends its zoom', async () => {
    await openShell();
    toggleZoom();
    toggleDock();
    expect(app.zoomed).toBe(null);
    expect(app.focused).toBe('p1');
  });

  it('fills the column with no workspace pane, and gives it back with the last tab', async () => {
    await openShell();
    app.layout = null;
    expect(dockFills()).toBe(true);
    closePane(app.dock.tabs[0]);
    expect(dockFills()).toBe(false);
  });
});

describe('restore', () => {
  const agent = (agentId) => ({ type: 'agent', agentId, title: agentId });
  const leaf = (...tabs) => ({ kind: 'leaf', id: 'l9', tabs, active: tabs[0] });
  const saved = (over = {}) => ({
    layout: { kind: 'split', id: 's1', dir: 'row', sizes: [0.5, 0.5], children: [leaf('p1'), leaf('p2', 'p3')] },
    panes: {
      p1: { type: 'project', projectId: 'shop-api' },
      p2: agent('back-1'),
      p3: agent('front-1'),
      p4: agent('rss-feed'),
    },
    focused: 'p3',
    dock: { open: false, height: 0.3, tabs: ['p4'], active: 'p4' },
    ...over,
  });
  beforeEach(() => {
    app.data = { ...fixture(), setup: { done: true } };
  });

  it('moves the agents of the workspace into the dock, after its tabs, and opens it', async () => {
    h.saved = saved();
    await restoreLayout();
    expect(app.dock.tabs.map((t) => app.panes[t].agentId)).toEqual(['rss-feed', 'back-1', 'front-1']);
    expect(app.dock.open).toBe(true);
    expect(app.panes[app.dock.active].agentId).toBe('front-1'); // the focused one
    expect(app.focused).toBe(app.dock.active);
    expect(app.layout.kind).toBe('leaf');
    expect(app.layout.tabs).toEqual(['p1']);
  });

  it('keeps a dock with tabs and no workspace pane, with no Overview', async () => {
    h.saved = saved({
      layout: null,
      focused: 'p4',
      dock: { open: true, height: 0.45, v: 2, tabs: ['p4'], active: 'p4' },
    });
    await restoreLayout();
    expect(app.layout).toBe(null);
    expect(app.dock).toMatchObject({ open: true, active: 'p4', tabs: ['p4'] });
    expect(Object.values(app.panes).some((p) => p.type === 'overview')).toBe(false);
    expect(app.focused).toBe('p4');
  });

  it('adds the Overview only when the workspace and the dock are both empty', async () => {
    h.saved = saved({ layout: null, panes: {}, dock: { open: false, tabs: [] } });
    app.panes = {};
    await restoreLayout();
    expect(Object.values(app.panes).map((p) => p.type)).toEqual(['overview']);
  });

  it('moves a saved 0.3 height to 0.45 once, and keeps a height saved under v 2', async () => {
    h.saved = saved();
    await restoreLayout();
    expect(app.dock).toMatchObject({ height: 0.45, v: 2 });
    h.saved = saved({ dock: { open: true, height: 0.6, tabs: ['p4'], active: 'p4' } });
    await restoreLayout();
    expect(app.dock.height).toBe(0.6);
    h.saved = saved({ dock: { open: true, height: 0.3, v: 2, tabs: ['p4'], active: 'p4' } });
    await restoreLayout();
    expect(app.dock.height).toBe(0.3);
  });
});
