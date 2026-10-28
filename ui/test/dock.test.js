import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fixture } from './fixture.js';

let nextTerm = 1;
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
  listen: () => Promise.resolve(() => {}),
  invoke: (cmd, args) => {
    if (cmd === 'term_spawn') return Promise.resolve({ id: nextTerm++, cwd: args.req.cwd ?? '/dev/shop-api' });
    if (cmd === 'settings_set') return Promise.resolve(null);
    return Promise.reject(new Error(`not mocked: ${cmd}`));
  },
}));

const { app, closePane, cycleDock, focusDirection, inDock, openShell, toggleDock } = await import(
  '../src/lib/app.svelte.js'
);

beforeEach(() => {
  app.data = fixture();
  app.panes = { p1: { type: 'agent', agentId: 'back-1', title: 'back-1' } };
  app.layout = { kind: 'leaf', id: 'l1', tabs: ['p1'], active: 'p1' };
  app.focused = 'p1';
  app.dock = { open: false, height: 0.3, tabs: [], active: null };
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
