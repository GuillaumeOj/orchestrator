// allow-launch-team-on-update-pr F2 (notes §3, §5): the Feature pane of a feature linked to someone else's PR,
// and its delete dialog. Against a mocked core.
import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { flushSync, tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { fixture } from './fixture.js';

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

const { default: FeaturePane } = await import('../src/panes/FeaturePane.svelte');
const { default: Dialogs } = await import('../src/components/Dialogs.svelte');
const flush = () => new Promise((r) => setTimeout(r, 0));

const link = {
  number: 42,
  url: 'https://github.com/acme/shop/pull/42',
  head_ref: 'renovate/tokio-1.x',
  base_ref: 'main',
  head_sha_at_link: '9f2c1ab',
  author: 'renovate[bot]',
};
const feature = (over = {}) => ({
  id: 'f1',
  project_id: 'shop-api',
  slug: 'pr-42-update',
  branch: 'renovate/tokio-1.x',
  pr_link: link,
  integration_worktree: '/w/pr-42',
  ...over,
});
const view = (runOver = {}, featOver = {}) => ({
  feature: feature(featOver),
  run: {
    id: 'run-1',
    status: 'executing',
    team_id: 't1',
    started_at_ms: Date.now() - 1000,
    pr_number: 42,
    pr_url: link.url,
    pr: null,
    ...runOver,
  },
  team: { name: 'Default', version: 3, nodes: [{ id: 'lead', is_lead: true, role_id: 'lead' }] },
  project: { id: 'shop-api', name: 'shop-api', backend: 'github' },
  agents: [],
  tasks: [],
  docs: [],
  artifacts: [],
  timeline: [],
});
const pr = (over = {}) => ({
  state: 'open',
  base: 'main',
  head_sha: '9f2c1ab',
  linked: true,
  foreign_head: false,
  foreign_head_sha: null,
  ci: { overall: 'running', done: 1, total: 2, jobs: [] },
  position: { status: 'up_to_date' },
  unpushed: 0,
  checked_at_ms: Date.now(),
  ...over,
});

async function pane(v) {
  replies.feature_view = v;
  app.panes = { f1: { type: 'feature', featureId: 'f1' } };
  const r = render(FeaturePane, { paneId: 'f1' });
  await waitFor(() => expect(r.container.querySelector('.fh')).toBeTruthy());
  return r;
}

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.now = { ...app.now, decisions: {} };
  app.toasts = [];
  app.panes = {};
  app.layout = null;
  app.overlay = null;
});

describe('Feature pane, linked', () => {
  it('shows the real branch and the PR in the header, the linked line and the pre-push line', async () => {
    const { container } = await pane(view());
    const row2 = container.querySelector('.row2');
    expect(row2.textContent).toContain('renovate/tokio-1.x');
    expect(row2.textContent).not.toContain('orch/pr-42-update/integration');
    expect(row2.querySelector('button').getAttribute('aria-label')).toBe('Open PR #42 on GitHub');
    expect(container.querySelector('[data-linked-line]').textContent).toMatch(
      /Works on PR #42.*renovate\/tokio-1\.x → main.*opened by renovate\[bot\]/,
    );
    expect(container.querySelector('[data-prepush]').textContent).toMatch(/^Nothing pushed to PR #42 yet/);
    expect(container.textContent).not.toContain('No PR yet');
  });

  it('says a push is asked, with Show card, when a gate is pending', async () => {
    app.now.decisions = {
      d1: {
        id: 'd1',
        kind: 'gate',
        lifecycle: { state: 'open' },
        payload: { action: 'push_pr', run: 'run-1', linked: { number: 42 } },
      },
    };
    const { container } = await pane(view());
    expect(container.querySelector('[data-prepush]').textContent).toContain('The lead asked to push to PR #42.');
    expect(container.querySelector('[data-prepush] button').textContent).toBe('Show card');
  });

  it('after the push there is no pre-push line', async () => {
    const { container } = await pane(view({ status: 'pr_open', pr: pr() }));
    expect(container.querySelector('[data-prepush]')).toBeNull();
    expect(container.querySelector('[data-linked-line]')).toBeTruthy();
  });

  it('warns about a head moved outside the app', async () => {
    const { container } = await pane(
      view({ status: 'pr_open', pr: pr({ foreign_head: true, foreign_head_sha: 'a41d7e0123' }) }),
    );
    const w = container.querySelector('[data-foreign-head]');
    expect(w.textContent).toContain("PR #42's head moved:");
    expect(w.querySelector('button').textContent).toBe('a41d7e0');
    expect(w.textContent).toContain('probably by renovate[bot]');
  });

  it('closed: the closed line replaces the pre-push line', async () => {
    const { container } = await pane(view({ status: 'integrating', pr: pr({ state: 'closed' }) }));
    expect(container.querySelector('[data-closed]').textContent).toMatch(/PR #42 was closed on GitHub/);
    expect(container.querySelector('[data-prepush]')).toBeNull();
  });

  it('merged: only the linked line stays', async () => {
    const { container } = await pane(view({ status: 'done', pr: pr({ state: 'merged' }) }));
    expect(container.querySelector('[data-linked-line]')).toBeTruthy();
    expect(container.querySelector('[data-closed]')).toBeNull();
    expect(container.querySelector('[data-prepush]')).toBeNull();
  });

  it('an unlinked feature keeps its integration branch and no linked line', async () => {
    const { container } = await pane(view({}, { pr_link: null, branch: 'orch/pr-42-update/integration' }));
    expect(container.querySelector('.row2').textContent).toContain('orch/pr-42-update/integration');
    expect(container.querySelector('[data-linked-line]')).toBeNull();
  });
});

describe('Delete feature dialog, linked', () => {
  const open = async (plan, over = {}) => {
    app.data = { ...fixture(), features: [feature(over)] };
    replies.feature_delete_plan = plan;
    app.overlay = { kind: 'delete-feature', featureId: 'f1' };
    const r = render(Dialogs);
    await flush();
    flushSync();
    return r;
  };

  it('names the real branch, D2 and the kept commits', async () => {
    const { container } = await open({ branch: 'renovate/tokio-1.x', linked: true, unpushed: 2 });
    expect(container.querySelector('[data-pr-stays]').textContent).toContain('PR #42 stays open on GitHub');
    expect(container.textContent).toContain('is removed only if it is already pushed to PR #42');
    expect(container.querySelector('[data-kept]').textContent).toBe(
      'renovate/tokio-1.x has 2 commits that PR #42 does not have: it will be kept.',
    );
  });

  it('toasts the kept branch without an error tone', async () => {
    replies.feature_delete = {
      deleted: 'pr-42-update',
      agents: [],
      worktrees: [],
      kept: [{ branch: 'renovate/tokio-1.x', commits: 2 }],
    };
    replies.app_state = fixture();
    const { container } = await open({ branch: 'renovate/tokio-1.x', linked: true, unpushed: 2 });
    await fireEvent.submit(container.querySelector('form'));
    await tick();
    await waitFor(() => expect(app.toasts.length).toBeGreaterThan(0));
    const t = app.toasts.at(-1);
    expect(t.text).toBe(
      'Deleted pr-42-update · 0 agent(s), no process left · kept renovate/tokio-1.x: 2 commits not pushed to PR #42',
    );
    expect(t.kind).not.toBe('error');
  });

  it('keeps the unlinked copy, with the real branch', async () => {
    const { container } = await open(
      { branch: 'orch/pr-42-update/integration', linked: false, unpushed: 0 },
      { pr_link: null, branch: 'orch/pr-42-update/integration' },
    );
    expect(container.textContent).toContain(
      'Remove its worktrees and branches, orch/pr-42-update/integration included',
    );
    expect(container.querySelector('[data-pr-stays]')).toBeNull();
  });
});
