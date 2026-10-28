import { fireEvent, render } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fixture } from './fixture.js';

const calls = [];
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
  listen: () => Promise.resolve(() => {}),
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === 'agent_action' && args.action === 'delete_plan')
      return Promise.resolve({
        agent: args.id,
        state: 'suspended',
        live: false,
        shared_with: [],
        worktree: {
          path: '/dev/shop-api.worktrees/back-1',
          branch: 'orch/solo/back-1',
          dirty: 0,
          unmerged_commits: 2,
          safe_to_remove: false,
        },
      });
    if (cmd === 'agent_delete')
      return Promise.resolve({
        deleted: args.id,
        log: ['verified: no process of the agent is left'],
        worktree: { worktree_removed: true, branch_deleted: true },
      });
    if (cmd === 'app_state') {
      // After the delete the core lists the agent as archived.
      const f = fixture();
      if (calls.some(([c]) => c === 'agent_delete')) f.agents.find((a) => a.id === 'back-1').state = 'archived';
      return Promise.resolve(f);
    }
    return Promise.reject(new Error(`not mocked: ${cmd}`));
  },
}));

const { app } = await import('../src/lib/app.svelte.js');
const { default: Dialogs } = await import('../src/components/Dialogs.svelte');
const tick = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  calls.length = 0;
  app.data = fixture();
  app.toasts = [];
  app.panes = { p1: { type: 'agent', agentId: 'back-1', title: 'back-1' } };
  app.layout = { kind: 'leaf', id: 'l1', tabs: ['p1'], active: 'p1' };
  app.focused = 'p1';
});

describe('delete agent', () => {
  it('asks before discarding unmerged work, then kills, deletes and closes its pane', async () => {
    app.overlay = { kind: 'delete-agent', agentId: 'back-1' };
    const { container } = render(Dialogs);
    await tick();
    flushSync();
    const boxes = () => /** @type {HTMLInputElement[]} */ ([...container.querySelectorAll('input[type=checkbox]')]);
    // Unmerged: the worktree is not ticked for removal by default.
    expect(boxes()).toHaveLength(1);
    expect(boxes()[0].checked).toBe(false);
    await fireEvent.click(boxes()[0]);
    expect(container.textContent).toMatch(/2 unmerged commit/);
    await fireEvent.click(boxes()[1]);
    await fireEvent.submit(container.querySelector('form'));
    await tick();
    expect(calls).toContainEqual(['agent_delete', { id: 'back-1', removeWorktree: true, confirmDiscard: true }]);
    await vi.waitFor(() => expect(container.querySelector('.err')?.textContent ?? app.overlay).toBeNull());
    expect(app.panes.p1).toBeUndefined();
    expect(app.toasts.at(-1).text).toMatch(/Deleted back-1 · no process left · worktree removed with its branch/);
  });
});
