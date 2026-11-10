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
