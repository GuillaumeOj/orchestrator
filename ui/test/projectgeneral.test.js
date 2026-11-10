// Project settings › General (settings-brief §2.1, spec §3.1, api-contract §2): rename and
// default branch are the only dirty fields (footer Save / Revert, the Discard guard); their
// refusals show under the field; path actions, Fetch now, Trust, the worktree root, the
// danger zone. "Remove merged worktrees automatically" is not rendered.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app, windowDirty } from '../src/lib/app.svelte.js';
import { fixture } from './fixture.js';
import { pick, shown } from './pick.js';

const calls = [];
const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    const r = replies[cmd];
    return r instanceof Error || (typeof r === 'string' && r.startsWith('!'))
      ? Promise.reject(typeof r === 'string' ? r.slice(1) : r)
      : Promise.resolve(typeof r === 'function' ? r(args) : r);
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
}));

const { default: ProjectSettingsWindow } = await import('../src/components/ProjectSettingsWindow.svelte');
const text = (el) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const btn = (c, re) => [...c.querySelectorAll('button')].find((b) => re.test(b.textContent));

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.data.projects[0].default_branch = 'main';
  app.data.core.user_home = '/Users/me';
  app.data.projects[0].path = '/Users/me/dev/shop-api';
  app.toasts = [];
  app.panes = {};
  replies.project_branches = ['main', 'develop'];
  replies.project_detail = { trusted: false, worktrees: [] };
  replies.project_settings = { settings: {}, backend: { type: 'local' } };
  replies.board_view = null;
  replies.integrations_view = { items: [] };
  replies.rename_project = {};
  replies.set_default_branch = {};
  replies.project_fetch = {};
  replies.reveal_path = null;
});

async function open() {
  app.window = { kind: 'project-settings', projectId: 'shop-api', section: 'general' };
  const r = render(ProjectSettingsWindow);
  await waitFor(() => expect(r.container.querySelector('[data-trust]')).toBeTruthy());
  return r;
}
const foot = (c) => c.querySelector('.foot');

describe('General', () => {
  it('General is listed first in the nav and renders without the placeholder', async () => {
    const { container } = await open();
    const nav = [...container.querySelectorAll('[role="tab"]')].map(text);
    expect(nav[0]).toBe('General');
    expect(text(container)).not.toContain('Not designed yet');
    expect([...container.querySelectorAll('.scard')].map((c) => /** @type {HTMLElement} */ (c).dataset.group)).toEqual([
      'project',
      'git',
      'trust',
      'worktrees',
      'danger',
    ]);
    expect(text(container)).not.toContain('Remove merged worktrees');
  });

  it('shows the record: name, path, id, default branch, fetch, worktree root', async () => {
    const { container } = await open();
    expect(/** @type {any} */ (container.querySelector('#pg-name')).value).toBe('shop-api');
    const t = text(container);
    expect(t).toContain('~/dev/shop-api');
    expect(t).toContain('shop-api');
    expect(t).toContain('~/dev/shop-api.worktrees');
    expect(t).toContain('every 15 min · last not yet');
    expect(shown(container.querySelector('[aria-label="Default branch"]'))).toBe('main');
  });

  it('footer: Save and Revert disabled until a field is dirty; "Unsaved changes" then', async () => {
    const { container } = await open();
    expect(btn(foot(container), /^Save/).disabled).toBe(true);
    expect(btn(foot(container), /^Revert/).disabled).toBe(true);
    expect(foot(container).textContent).not.toContain('Unsaved changes');
    await fireEvent.input(container.querySelector('#pg-name'), { target: { value: 'shop' } });
    expect(foot(container).textContent).toContain('Unsaved changes');
    expect(btn(foot(container), /^Save/).disabled).toBe(false);
    await fireEvent.click(btn(foot(container), /^Revert/));
    expect(/** @type {any} */ (container.querySelector('#pg-name')).value).toBe('shop-api');
    expect(btn(foot(container), /^Save/).disabled).toBe(true);
  });

  it('Save renames, then sets the default branch; only the dirty fields are sent', async () => {
    const { container } = await open();
    await fireEvent.input(container.querySelector('#pg-name'), { target: { value: '  shop  ' } });
    await pick(container.querySelector('[aria-label="Default branch"]'), 'develop');
    await fireEvent.click(btn(foot(container), /^Save/));
    await waitFor(() => expect(calls.some(([c]) => c === 'set_default_branch')).toBe(true));
    expect(calls.filter(([c]) => c === 'rename_project')).toEqual([
      ['rename_project', { id: 'shop-api', name: 'shop' }],
    ]);
    expect(calls.filter(([c]) => c === 'set_default_branch')).toEqual([
      ['set_default_branch', { id: 'shop-api', branch: 'develop' }],
    ]);
    // Name only: no default branch call.
    calls.length = 0;
    cleanup();
    const r = await open();
    await fireEvent.input(r.container.querySelector('#pg-name'), { target: { value: 'again' } });
    await fireEvent.click(btn(foot(r.container), /^Save/));
    await waitFor(() => expect(calls.some(([c]) => c === 'rename_project')).toBe(true));
    expect(calls.some(([c]) => c === 'set_default_branch')).toBe(false);
  });

  it("a refused rename shows the core's text under the field and keeps the edit", async () => {
    replies.rename_project = '!A project named blog already exists';
    const { container } = await open();
    await fireEvent.input(container.querySelector('#pg-name'), { target: { value: 'blog' } });
    await fireEvent.click(btn(foot(container), /^Save/));
    await waitFor(() => expect(container.querySelector('#pg-name-err')).toBeTruthy());
    expect(container.querySelector('#pg-name-err').textContent).toBe('A project named blog already exists');
    expect(/** @type {any} */ (container.querySelector('#pg-name')).value).toBe('blog');
    expect(calls.some(([c]) => c === 'set_default_branch')).toBe(false);
  });

  it('a refused branch shows its text under the Select', async () => {
    replies.set_default_branch = '!No branch develop in this repo';
    const { container } = await open();
    await pick(container.querySelector('[aria-label="Default branch"]'), 'develop');
    await fireEvent.click(btn(foot(container), /^Save/));
    await waitFor(() => expect(container.querySelector('[role="alert"]')).toBeTruthy());
    expect(container.querySelector('[role="alert"]').textContent).toBe('No branch develop in this repo');
  });

  it('a dirty General asks "Discard changes?" before the window closes', async () => {
    const { container } = await open();
    expect(windowDirty()).toBe(false);
    await fireEvent.input(container.querySelector('#pg-name'), { target: { value: 'x' } });
    await tick();
    expect(windowDirty()).toBe(true);
  });

  it('untrusted: the badge and Trust in a pane; trusted: the badge only', async () => {
    const { container } = await open();
    expect(container.querySelector('[data-trust="untrusted"]')).toBeTruthy();
    expect(btn(container, /^Trust in a pane/)).toBeTruthy();
    cleanup();
    replies.project_detail = { trusted: true, worktrees: [] };
    const r = await open();
    expect(r.container.querySelector('[data-trust="trusted"]')).toBeTruthy();
    expect(btn(r.container, /^Trust in a pane/)).toBeUndefined();
  });

  it('Fetch now and Reveal in Finder call the core', async () => {
    const { container } = await open();
    await fireEvent.click(btn(container, /^Fetch now/));
    await waitFor(() => expect(calls).toContainEqual(['project_fetch', { id: 'shop-api' }]));
    await fireEvent.click(container.querySelector('button[aria-label="Reveal in Finder"]'));
    expect(calls).toContainEqual(['reveal_path', { path: '/Users/me/dev/shop-api' }]);
  });

  it('Remove project… opens the project pane and starts the removal there', async () => {
    const { container } = await open();
    await fireEvent.click(btn(container, /^Remove project…/));
    const id = Object.keys(app.panes).find((k) => app.panes[k].type === 'project');
    expect(app.panes[id]).toMatchObject({ projectId: 'shop-api', remove: true });
  });
});
