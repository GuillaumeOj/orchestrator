// T-322 walkthrough fixes: Add project makes the new project the context (D-1, F1.3), one
// pauseAll for the planner, the palette and the Banner (D-3).
import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fixture } from './fixture.js';

const calls = [];
const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  errText: (e) => String(e?.message ?? e),
  listen: () => Promise.resolve(() => {}),
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    const r = replies[cmd];
    return Promise.resolve(typeof r === 'function' ? r(args) : r);
  },
}));

const A = await import('../src/lib/app.svelte.js');
const { app } = A;
const { default: Dialogs } = await import('../src/components/Dialogs.svelte');

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.toasts = [];
  app.panes = {};
  app.layout = null;
  app.workspace = { project: 'all' };
});
afterEach(() => cleanup());

describe('Add project (D-1)', () => {
  it('the new project becomes the context and no pane opens: the empty state shows', async () => {
    const added = { ...fixture().projects[0], id: 'newproj', name: 'newproj' };
    replies.project_add = added;
    replies.app_state = { ...fixture(), projects: [...fixture().projects, added] };
    app.overlay = { kind: 'add-project' };
    const { container } = render(Dialogs);
    await fireEvent.input(container.querySelector('input.field'), { target: { value: '/dev/newproj' } });
    await fireEvent.submit(container.querySelector('form'));
    await waitFor(() => expect(app.workspace.project).toBe('newproj'));
    expect(Object.values(app.panes).some((s) => s.type === 'project')).toBe(false);
  });
});

describe('Pause all (D-3)', () => {
  it('the palette command and pauseAll send one governor action with one toast', async () => {
    replies.governor_act = {};
    replies.app_state = fixture();
    const cmd = A.commands().find((c) => c.id === 'governor.pause-all');
    await cmd.run();
    await A.pauseAll();
    expect(calls.filter(([c, a]) => c === 'governor_act' && a.action === 'pause_all')).toHaveLength(2);
    expect(app.toasts.map((t) => t.text)).toEqual([
      'Paused all · Resume all from the palette',
      'Paused all · Resume all from the palette',
    ]);
  });
});

describe('T-324 review fixes', () => {
  it('remoteLabel: https, ssh with a port, scp-like', async () => {
    const { remoteLabel } = await import('../src/lib/format.js');
    expect(remoteLabel('https://github.com/acme/shop.git')).toBe('github.com/acme/shop');
    expect(remoteLabel('ssh://git@git.example.com:2222/acme/shop.git')).toBe('git.example.com:2222/acme/shop');
    expect(remoteLabel('git@github.com:acme/shop.git')).toBe('github.com/acme/shop');
  });

  it('a downgrade goes to opus, sonnet or haiku only; the other model pickers keep fable', async () => {
    const { DOWNGRADE_OPTIONS, MODEL_OPTIONS } = await import('../src/lib/options.js');
    expect(DOWNGRADE_OPTIONS.map((o) => o.value)).toEqual(['opus', 'sonnet', 'haiku']);
    expect(MODEL_OPTIONS.map((o) => o.value)).toContain('fable');
  });

  it('opening an open pane from a floating window closes the window and focuses the pane', async () => {
    app.panes = { a1: { type: 'agent', agentId: 'back-1', title: 'back-1' } };
    app.layout = { kind: 'leaf', id: 'l1', tabs: [], active: null };
    app.dock = { open: true, height: 0.45, tabs: ['a1'], active: null, v: 2 };
    app.window = { kind: 'settings', section: 'claude' };
    A.openAgent('back-1');
    expect(app.window).toBeNull();
    expect(app.focused).toBe('a1');
  });
});
