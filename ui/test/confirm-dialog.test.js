// Confirms through the real Dialogs (never `app.overlay.onYes()`): the dialog
// ran `close(), o.onYes()` and `o` is derived from the overlay it had just
// cleared, so no confirm in the app ever ran its action.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
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

const { app, confirm, confirmKill } = await import('../src/lib/app.svelte.js');
const { default: Dialogs } = await import('../src/components/Dialogs.svelte');
const { default: ProjectPane } = await import('../src/panes/ProjectPane.svelte');

const text = (c) => c.textContent.replace(/\s+/g, ' ');
const btn = (c, label) => [...c.querySelectorAll('button')].find((b) => b.textContent.trim() === label);

const detail = {
  trusted: false,
  worktrees: [
    {
      path: '/dev/shop-api',
      branch: 'main',
      verdict: 'main',
      safe_to_remove: false,
      dirty: 0,
      unmerged_commits: 0,
      in_progress: [],
      locked: false,
    },
    {
      path: '/dev/shop-api.worktrees/back-1',
      branch: 'orch/solo/back-1',
      verdict: 'ok',
      safe_to_remove: false,
      dirty: 2,
      unmerged_commits: 1,
      in_progress: [],
      locked: true,
      row: { agent_id: 'back-1' },
    },
    {
      path: '/dev/shop-api.worktrees/old',
      branch: 'orch/solo/old',
      verdict: 'ok',
      safe_to_remove: true,
      dirty: 0,
      unmerged_commits: 0,
      in_progress: [],
      locked: false,
    },
    {
      path: '/dev/shop-api/.claude/worktrees/mine',
      branch: 'claude/mine',
      verdict: 'external',
      safe_to_remove: false,
      dirty: 1,
      unmerged_commits: 3,
      in_progress: [],
      locked: false,
    },
  ],
  branches: [
    { name: 'orch/solo/old', merged: true, orch: true },
    { name: 'claude/mine', merged: false, orch: false },
  ],
};

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.data.projects[0] = { ...app.data.projects[0], default_branch: 'main' };
  app.panes = { p1: { type: 'project', projectId: 'shop-api', title: 'shop-api' } };
  app.focused = 'p1';
  app.overlay = null;
  app.toasts = [];
  app.chatFocus = null;
});

describe('the confirm dialog', () => {
  const open = async (onYes) => {
    confirm('Do it?', 'Really.', onYes, 'Do it');
    const r = render(Dialogs);
    await tick();
    return r;
  };

  it('the button runs the callback once and closes', async () => {
    const onYes = vi.fn();
    const { container } = await open(onYes);
    await fireEvent.click(btn(container, 'Do it'));
    expect(onYes).toHaveBeenCalledTimes(1);
    expect(app.overlay).toBeNull();
  });

  it('⏎ on the focused button is its native click: once, closed', async () => {
    const onYes = vi.fn();
    const { container } = await open(onYes);
    // The primary button has the focus; a browser turns Enter on it into a click.
    expect(document.activeElement).toBe(btn(container, 'Do it'));
    await fireEvent.click(document.activeElement);
    expect(onYes).toHaveBeenCalledTimes(1);
    expect(app.overlay).toBeNull();
  });

  it('⌘⏎ (the keydown path) runs it once and closes', async () => {
    const onYes = vi.fn();
    const { container } = await open(onYes);
    await fireEvent.keyDown(btn(container, 'Do it'), { key: 'Enter', metaKey: true });
    expect(onYes).toHaveBeenCalledTimes(1);
    expect(app.overlay).toBeNull();
  });

  it('Cancel and Esc do not run it', async () => {
    const onYes = vi.fn();
    const { container } = await open(onYes);
    await fireEvent.click(btn(container, 'Cancel'));
    expect(app.overlay).toBeNull();
    confirm('Do it?', 'Really.', onYes, 'Do it');
    await tick();
    await fireEvent.keyDown(document.activeElement, { key: 'Escape' });
    expect(onYes).not.toHaveBeenCalled();
    expect(app.overlay).toBeNull();
  });

  it('another caller: confirmKill kills the agent', async () => {
    replies.agent_action = {};
    confirmKill('back-1');
    const { container } = render(Dialogs);
    await tick();
    await fireEvent.click(btn(container, 'Kill'));
    await waitFor(() =>
      expect(calls).toContainEqual(['agent_action', { id: 'back-1', action: 'kill', message: null }]),
    );
  });

  it('the origin chip link of a brief focuses its card', async () => {
    app.overlay = {
      kind: 'launch',
      prefill: {
        project_id: 'shop-api',
        team_id: 'default-web',
        slug: 's',
        title: 'T',
        brief: 'b',
        base: null,
        off_hours: false,
      },
      origin: { kind: 'brief', decision: 'd_1', title: 'Partial refunds' },
    };
    const { container } = render(Dialogs);
    await tick();
    await fireEvent.click(container.querySelector('button[aria-label="Show the brief card"]'));
    expect(app.chatFocus).toMatchObject({ id: 'd_1' });
    expect(app.overlay).toBeNull();
  });
});

describe('removing a worktree through the pane and the dialog', () => {
  const setup = async () => {
    replies.project_detail = detail;
    replies.worktree_remove = { worktree_removed: true };
    const pane = render(ProjectPane, { paneId: 'p1' });
    const dlg = render(Dialogs);
    await waitFor(() => expect(text(pane.container)).toContain('shop-api.worktrees/old'));
    return { pane: pane.container, dlg: dlg.container };
  };
  const bins = (c) => [...c.querySelectorAll('button[aria-label="Remove worktree…"]')];

  it('merged: Remove removes the row and toasts', async () => {
    const { pane, dlg } = await setup();
    await fireEvent.click(bins(pane)[1]);
    await fireEvent.click(btn(dlg, 'Remove'));
    await waitFor(() =>
      expect(calls).toContainEqual([
        'worktree_remove',
        { project: 'shop-api', path: '/dev/shop-api.worktrees/old', deleteBranch: true, discard: false },
      ]),
    );
    await waitFor(() => expect(text(pane)).not.toContain('shop-api.worktrees/old'));
    expect(JSON.stringify(app.toasts)).toContain('Removed ');
    expect(app.overlay).toBeNull();
  });

  it('unmerged: Discard and remove sends discard: true', async () => {
    const { pane, dlg } = await setup();
    await fireEvent.click(bins(pane)[0]);
    await fireEvent.click(btn(dlg, 'Discard and remove'));
    await waitFor(() =>
      expect(calls).toContainEqual([
        'worktree_remove',
        { project: 'shop-api', path: '/dev/shop-api.worktrees/back-1', deleteBranch: true, discard: true },
      ]),
    );
    await waitFor(() => expect(text(pane)).not.toContain('shop-api.worktrees/back-1'));
    expect(JSON.stringify(app.toasts)).toContain('Removed ');
  });

  it('an approved removal that meets a running action says so', async () => {
    const { pane, dlg } = await setup();
    /** @type {((v: unknown) => void) | undefined} */
    let release;
    replies.project_detail = () => new Promise((r) => (release = r));
    await fireEvent.click(bins(pane)[1]); // the confirm is open...
    await fireEvent.click(btn(pane, 'Load PRs')); // ...when the PR load starts
    await waitFor(() => expect(release).toBeTypeOf('function'));
    await fireEvent.click(btn(dlg, 'Remove'));
    expect(JSON.stringify(app.toasts)).toContain('Wait for the PR load to finish');
    expect(calls.some(([c]) => c === 'worktree_remove')).toBe(false);
    /** @type {any} */ (release)(detail);
    await waitFor(() => expect(btn(pane, 'Load PRs')?.disabled).toBe(false)); // the pending action settled
  });
});
