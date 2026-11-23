// Launching a team on a PR (allow-launch-team-on-update-pr F1): the pure copy rules, the Project pane's rows
// (states, keys, patch after a launch) and the launch dialog's PR variant (loading, ready, errors).

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { fixture } from './fixture.js';

const calls = [];
const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push({ cmd, args });
    const r = replies[cmd];
    if (typeof r === 'function') return Promise.resolve().then(() => r(args));
    if (r instanceof Error) return Promise.reject(r);
    return Promise.resolve(r ?? null);
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? JSON.stringify(e))),
}));

const P = await import('../src/lib/prlaunch.js');
const { openPrLaunch } = await import('../src/lib/intake.svelte.js');
const { default: ProjectPane } = await import('../src/panes/ProjectPane.svelte');
const { default: Dialogs } = await import('../src/components/Dialogs.svelte');

const text = (el) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const settle = async () => {
  await tick();
  await Promise.resolve();
  await tick();
};
const btn = (c, re) => [...c.querySelectorAll('button')].find((b) => re.test(b.textContent));

const pr = (number, o = {}) => ({
  number,
  title: `Title ${number}`,
  head_ref: `renovate/x-${number}`,
  base_ref: 'main',
  state: 'OPEN',
  url: `https://github.com/acme/shop/pull/${number}`,
  is_draft: false,
  review_decision: null,
  checks: { passing: 5, failing: 1, pending: 0, skipped: 0 },
  author: { login: 'renovate[bot]', is_bot: true },
  cross_repo: false,
  feature: null,
  launch: { ok: true },
  ...o,
});

describe('copy rules', () => {
  it('author: app/ stripped, null is unknown', () => {
    expect(P.authorName({ login: 'app/renovate' })).toBe('renovate');
    expect(P.authorName(null)).toBe('unknown author');
  });
  it('row state, fork reason with and without owner, feature line', () => {
    expect(P.rowState(pr(1))).toBe('launch');
    expect(P.rowState(pr(1, { launch: { ok: false, code: 'fork' } }))).toBe('fork');
    expect(P.rowState(pr(1, { launch: { ok: false, code: 'active_feature' } }))).toBe('active');
    expect(P.rowState({ number: 1 })).toBe('none');
    expect(P.forkReason({ fork_owner: 'mia-dev' })).toBe('Fork PR (from mia-dev): the app pushes to origin only.');
    expect(P.featureLine(pr(1, { feature: { slug: 's', run_status: 'done' } }), 'earlier')).toBe(
      'Earlier team: s · done',
    );
    expect(P.featureLine(pr(1, { feature: { slug: 's' } }), 'active')).toBe('Team on this PR: s');
  });
  it('prefill errors', () => {
    expect(P.prefillError({ code: 'pr_merged' }, 42).text).toContain('was merged');
    expect(
      P.prefillError({ code: 'active_feature', feature: { id: 'f', slug: 'pr-43', run_status: 'executing' } }, 43),
    ).toMatchObject({
      text: 'PR #43 already has a team: pr-43 (executing).',
      open: { id: 'f' },
    });
    expect(P.prefillError({ code: 'offline' }, 1).again).toBe(true);
    expect(P.prefillError({ code: 'weird', message: 'HTTP 502' }, 7).text).toBe("Couldn't read PR #7: HTTP 502");
  });
  it('launch errors: fixable ones keep Launch, the rest do not', () => {
    const row = { head_ref: 'renovate/x', number: 42 };
    expect(P.launchError({ code: 'local_ahead', details: { commits: 2 } }, row)).toMatchObject({
      keep: true,
      text: 'Local renovate/x has 2 commits PR #42 does not have. Rename or delete that local branch first.',
    });
    expect(
      P.launchError({ code: 'branch_checked_out', message: 'm', details: { path: '/h/dev/shop' } }, row, (p) =>
        p.replace('/h', '~'),
      ).text,
    ).toContain('~/dev/shop');
    expect(P.launchError({ code: 'pr_closed' }, row).keep).toBeFalsy();
    expect(P.launchError({ code: 'active_feature', feature: { id: 'f', slug: 'x' } }, row).open).toEqual({
      id: 'f',
      slug: 'x',
    });
    expect(P.launchError({ code: 'fetch_failed', message: 'no route' }, row).text).toContain(
      "Couldn't fetch renovate/x from origin: no route",
    );
  });
});

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.panes = { p1: { type: 'project', projectId: 'shop-api', title: 'shop-api' } };
  app.focused = 'p1';
  app.overlay = null;
  app.toasts = [];
  replies.teams_view = { roles: [], teams: [{ team: { id: 'default', name: 'Default' }, issues: [] }] };
  replies.mcp_registry_view = { servers: [] };
  replies.planner_view = { off_hours: null };
  replies.project_settings = { settings: {} };
});

const rowsFixture = [
  pr(42),
  pr(43, {
    launch: { ok: false, code: 'active_feature', reason: 'x' },
    feature: { id: 'f43', slug: 'pr-43-vite', run_status: 'executing' },
  }),
  pr(44, {
    review_decision: 'APPROVED',
    author: { login: 'app/dependabot', is_bot: true },
    feature: { id: 'f44', slug: 'pr-44-serde', run_status: 'done' },
  }),
  pr(45, {
    cross_repo: true,
    fork_owner: 'mia-dev',
    author: { login: 'mia-dev', is_bot: false },
    launch: { ok: false, code: 'fork', reason: 'Fork PR: the app pushes to origin only' },
  }),
  pr(46, { is_draft: true, author: null }),
];
async function loaded() {
  replies.project_detail = { trusted: true, worktrees: [], prs: rowsFixture };
  const r = render(ProjectPane, { paneId: 'p1' });
  await settle();
  await fireEvent.click(btn(r.container, /Load PRs/));
  await waitFor(() => expect(r.container.querySelector('[data-pr="42"]')).toBeTruthy());
  return r;
}
const rowOf = (c, n) => c.querySelector(`[data-pr="${n}"]`);

describe('PR rows', () => {
  it('renders the five states', async () => {
    const { container: c } = await loaded();
    expect(text(rowOf(c, 42))).toContain('#42 Title 42');
    expect(text(rowOf(c, 42))).toContain('renovate/x-42 → main');
    expect(text(rowOf(c, 42))).toContain('renovate[bot]');
    expect(rowOf(c, 42).querySelector('[aria-label="bot"]')).toBeTruthy();
    expect(text(rowOf(c, 42))).toContain('CI 1 failing');
    expect(btn(rowOf(c, 42), /Launch team…/).disabled).toBe(false);
    expect(text(rowOf(c, 43))).toContain('Team on this PR: pr-43-vite · executing');
    expect(btn(rowOf(c, 43), /Open feature/)).toBeTruthy();
    expect(text(rowOf(c, 44))).toContain('dependabot[bot]'.replace('[bot]', ''));
    expect(text(rowOf(c, 44))).not.toContain('app/');
    expect(text(rowOf(c, 44))).toContain('approved');
    expect(text(rowOf(c, 44))).toContain('Earlier team: pr-44-serde · done');
    expect(btn(rowOf(c, 44), /Launch team…/).disabled).toBe(false);
    const fork = btn(rowOf(c, 45), /Launch team…/);
    expect(fork.getAttribute('aria-disabled')).toBe('true');
    expect(text(rowOf(c, 45))).toContain('Fork PR (from mia-dev): the app pushes to origin only.');
    expect(text(rowOf(c, 46))).toContain('draft');
    expect(text(rowOf(c, 46))).toContain('unknown author');
    expect(text(rowOf(c, 46))).not.toContain('no review');
  });

  it('Launch team… opens the PR dialog; a disabled fork row does nothing', async () => {
    const { container: c } = await loaded();
    await fireEvent.click(btn(rowOf(c, 45), /Launch team…/));
    expect(app.overlay).toBeNull();
    await fireEvent.click(btn(rowOf(c, 42), /Launch team…/));
    expect(app.overlay).toMatchObject({
      kind: 'launch',
      projectId: 'shop-api',
      origin: { kind: 'pr', label: 'PR #42' },
    });
  });

  it('keys: l launches, Enter on the row, ↓ moves, Enter on the fork row does nothing', async () => {
    const { container: c } = await loaded();
    rowOf(c, 42).focus();
    await fireEvent.keyDown(rowOf(c, 42), { key: 'ArrowDown' });
    expect(document.activeElement).toBe(rowOf(c, 43));
    await fireEvent.keyDown(rowOf(c, 45), { key: 'Enter' });
    expect(app.overlay).toBeNull();
    await fireEvent.keyDown(rowOf(c, 42), { key: 'l' });
    expect(app.overlay).toMatchObject({ kind: 'launch', origin: { label: 'PR #42' } });
  });

  it('patches the row from the launch result (F8)', async () => {
    const { container: c } = await loaded();
    await fireEvent.click(btn(rowOf(c, 42), /Launch team…/));
    app.overlay.onLaunched({ id: 'f42', slug: 'pr-42-x', run_status: 'spec' });
    await settle();
    expect(text(rowOf(c, 42))).toContain('Team on this PR: pr-42-x · spec');
    expect(btn(rowOf(c, 42), /Open feature/)).toBeTruthy();
  });
});

const prefill = (o = {}) => ({
  launch: {
    project_id: 'shop-api',
    team_id: 'default',
    slug: 'pr-42-update',
    title: 'PR #42 · Title 42',
    brief: 'Make PR #42 pass CI',
    base: 'main',
    off_hours: false,
    issues: [],
    pr_mode: null,
    pr: { number: 42, url: 'u', head_ref: 'renovate/x-42', base_ref: 'main', head_sha: 'abc', author: 'renovate[bot]' },
  },
  origin: { kind: 'pr', label: 'PR #42', url: 'u' },
  notes: { failing: 1, pending: 0, logs: 'cut' },
  warnings: [],
  ...o,
});
async function openDialog() {
  openPrLaunch('shop-api', pr(42));
  const r = render(Dialogs);
  await settle();
  return r;
}

describe('launch dialog, PR variant', () => {
  it('loads at once (disabled, Works on filled), then fills and focuses Launch', async () => {
    let done;
    replies.pr_prefill = () => new Promise((res) => (done = res));
    const { container: c } = await openDialog();
    expect(calls.find((x) => x.cmd === 'pr_prefill').args).toEqual({ project: 'shop-api', number: 42 });
    expect(text(c)).toContain('From PR #42 · GitHub');
    expect(text(c)).toContain('Works on');
    expect(text(c)).toContain('renovate/x-42 → main');
    expect(c.querySelector('textarea').placeholder).toContain('Reading PR #42');
    expect(btn(c, /^Launch$/).disabled).toBe(true);
    expect(c.querySelectorAll('[data-skeleton]').length).toBe(2);
    done(prefill());
    await waitFor(() => expect(c.querySelector('textarea').value).toBe('Make PR #42 pass CI'));
    expect(btn(c, /^Launch$/).disabled).toBe(false);
    expect(document.activeElement).toBe(btn(c, /^Launch$/));
    const t = text(c);
    expect(t).toContain('Logs are cut to the last 60 lines');
    expect(t).toContain("Checks out renovate/x-42 from origin in the lead's worktree. Nothing is branched from main.");
    expect(t).toContain("The feature docs stay in the app; only the team's commits go to the PR.");
    expect(t).toContain('This text comes from GitHub (the pull request and its check logs).');
    expect(t).not.toContain('New feature');
    expect(t).not.toContain('Base');
    expect(t).not.toContain('Pull requests');
    expect(t).not.toContain('Creates orch/');
  });

  it('shows pre-flight warnings and keeps Launch enabled', async () => {
    replies.pr_prefill = prefill({
      notes: { logs: 'full' },
      warnings: [{ code: 'local_ahead', message: 'm', details: { commits: 2 } }],
    });
    const { container: c } = await openDialog();
    await waitFor(() => expect(c.querySelector('[data-warning]')).toBeTruthy());
    expect(text(c.querySelector('[data-warning]'))).toContain(
      'Local renovate/x-42 has 2 commits PR #42 does not have.',
    );
    expect(btn(c, /^Launch$/).disabled).toBe(false);
    expect(text(c)).not.toContain('Logs are cut');
  });

  it('launch sends `pr`, no base, patches the row and closes', async () => {
    replies.pr_prefill = prefill();
    replies.team_launch = { feature_id: 'f1', feature: { id: 'f1', slug: 'pr-42-update', run_status: 'spec' } };
    const got = [];
    openPrLaunch('shop-api', pr(42), { onLaunched: (f) => got.push(f) });
    const { container: c } = render(Dialogs);
    await waitFor(() => expect(btn(c, /^Launch$/)?.disabled).toBe(false));
    await fireEvent.click(btn(c, /^Launch$/));
    await waitFor(() => expect(app.overlay).toBeNull());
    const sent = calls.find((x) => x.cmd === 'team_launch').args.form;
    expect(sent).toMatchObject({ slug: 'pr-42-update', base: null, pr: { number: 42, head_sha: 'abc' } });
    expect(got).toEqual([{ id: 'f1', slug: 'pr-42-update', run_status: 'spec' }]);
  });

  it('a prefill error replaces the form; Try again re-reads', async () => {
    replies.pr_prefill = () => Promise.reject(JSON.stringify({ code: 'offline', message: 'x' }));
    const { container: c } = await openDialog();
    await waitFor(() => expect(c.querySelector('[data-prefill-error]')).toBeTruthy());
    expect(text(c)).toContain("Couldn't reach GitHub. Check your connection and try again.");
    expect(c.querySelector('textarea')).toBeNull();
    replies.pr_prefill = prefill();
    await fireEvent.click(btn(c, /Try again/));
    await waitFor(() => expect(c.querySelector('textarea')?.value).toBe('Make PR #42 pass CI'));
  });

  it('active_feature prefill error offers Open feature and reloads the list', async () => {
    let stale = 0;
    replies.pr_prefill = () =>
      Promise.reject(
        JSON.stringify({
          code: 'active_feature',
          message: 'm',
          feature: { id: 'f43', slug: 'pr-43-vite', run_status: 'executing' },
        }),
      );
    openPrLaunch('shop-api', pr(43), { onStale: () => stale++ });
    const { container: c } = render(Dialogs);
    await waitFor(() => expect(text(c)).toContain('PR #43 already has a team: pr-43-vite (executing).'));
    expect(btn(c, /Open feature/)).toBeTruthy();
    expect(stale).toBe(1);
  });

  it('a launch error stays inline; fixable ones keep Launch, pr_closed disables it', async () => {
    replies.pr_prefill = prefill();
    replies.team_launch = () =>
      Promise.reject(JSON.stringify({ code: 'local_ahead', message: 'm', details: { commits: 2 } }));
    const { container: c } = await openDialog();
    await waitFor(() => expect(btn(c, /^Launch$/)?.disabled).toBe(false));
    await fireEvent.click(btn(c, /^Launch$/));
    await waitFor(() => expect(c.querySelector('[data-launch-error]')).toBeTruthy());
    expect(text(c.querySelector('[data-launch-error]'))).toContain('Local renovate/x-42 has 2 commits');
    expect(btn(c, /^Launch$/).disabled).toBe(false);
    replies.team_launch = () => Promise.reject(JSON.stringify({ code: 'pr_closed', message: 'm' }));
    await fireEvent.click(btn(c, /^Launch$/));
    await waitFor(() =>
      expect(text(c.querySelector('[data-launch-error]'))).toContain('was closed since this dialog opened'),
    );
    expect(btn(c, /^Launch$/).disabled).toBe(true);
  });

  it('Cancel drops a late prefill', async () => {
    let done;
    replies.pr_prefill = () => new Promise((res) => (done = res));
    const { container: c } = await openDialog();
    await fireEvent.click(btn(c, /Cancel/));
    expect(app.overlay).toBeNull();
    done(prefill());
    await settle();
    expect(app.overlay).toBeNull();
  });
});
