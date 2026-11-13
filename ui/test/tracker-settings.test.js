// Project settings › Tracker (improve-intake notes §10, api-contract §1), against a mocked core.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app, openProjectSettings } from '../src/lib/app.svelte.js';
import { fixture } from './fixture.js';
import { pick, picked } from './pick.js';

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

const { default: ProjectSettingsWindow } = await import('../src/components/ProjectSettingsWindow.svelte');
const { readySummary, writeBackLines, noticeInfo, switchKind, stateColumn } = await import('../src/lib/tracker.js');

const wb = { enabled: true, assign_me: false, running_label: 'orch:running', board_status: null, linear_state: null };
const local = () =>
  /** @type {any} */ ({
    tracker: { kind: 'local', backlog_glob: '.orchestrator/backlog/*.md' },
    ready: { kind: 'local' },
    write_back: wb,
    refresh_secs: 600,
    group_by_milestone: true,
    task_states: null,
  });
const github = (over = /** @type {any} */ ({})) => ({
  ...local(),
  tracker: { kind: 'github', repo: null, project_board: null },
  ready: { kind: 'github', labels: ['ready'], milestone: null, board_status: null, issue_type: null },
  ...over,
});
const linear = (over = /** @type {any} */ ({})) => ({
  ...local(),
  tracker: { kind: 'linear', teams: ['ENG'], projects: null },
  ready: { kind: 'linear', state_types: ['unstarted'], label: null, project: null, cycle: 'current' },
  ...over,
});
const GH_OPTS = {
  labels: ['ready', 'next'],
  milestones: ['v2', 'v3'],
  issue_types: ['Feature', 'Bug'],
  boards: [{ owner: 'acme', number: 4, title: 'Roadmap', statuses: ['Todo', 'In progress', 'Done'] }],
};
const LIN_OPTS = {
  teams: [
    { id: 't1', key: 'ENG', name: 'Engineering' },
    { id: 't2', key: 'OPS', name: 'Operations' },
  ],
  projects: [
    { id: 'p1', name: 'Mobile onboarding', team: 't1' },
    { id: 'p2', name: 'Billing revamp', team: 't1' },
  ],
  labels: ['bug'],
  states: [
    { id: 's1', name: 'Backlog', type: 'backlog' },
    { id: 's2', name: 'Todo', type: 'unstarted' },
    { id: 's3', name: 'In Progress', type: 'started' },
    { id: 's4', name: 'In Review', type: 'started' },
    { id: 's5', name: 'Done', type: 'completed' },
  ],
};

const called = (cmd) => calls.filter(([c]) => c === cmd);
const last = (cmd) => called(cmd).at(-1)?.[1];
const el = (x) => /** @type {any} */ (x);
const txt = (c) => c.textContent.replace(/\s+/g, ' ');

function open(settings, notice = null) {
  replies.tracker_settings = { settings, migration_notice: notice };
  app.data = fixture();
  app.window = { kind: 'project-settings', projectId: 'shop-api', section: 'tracker' };
  return render(ProjectSettingsWindow);
}

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  replies.project_settings = { settings: {}, backend: { type: 'local' }, track_orchestrator_dir: true };
  replies.tracker_options = GH_OPTS;
  replies.tracker_settings_set = (a) => ({ settings: a.settings, migration_notice: null });
  replies.tracker_notice_dismiss = undefined;
});

describe('Tracker helpers', () => {
  it('the Ready summary, per kind', () => {
    expect(readySummary(github().ready)).toBe('Open issues labelled ready');
    expect(readySummary({ ...github().ready, milestone: 'v2', board_status: 'Todo' })).toBe(
      'Open issues labelled ready, in milestone v2, with board status Todo',
    );
    expect(readySummary({ ...github().ready, labels: [] })).toMatch(/^Nothing: the lane stays empty/);
    expect(readySummary(linear().ready)).toBe('Unstarted issues in the current cycle');
    expect(readySummary(local().ready)).toBe('Every backlog file not marked done.');
  });

  it('the write-back list follows the options', () => {
    const gh = writeBackLines(github(), 'acme/shop');
    expect(gh[0]).toBe(
      'When you start an item, it gets the label orch:running and a comment "Started in Orchestrator".',
    );
    expect(gh.at(-1)).toBe('Nothing else is changed, and only items of acme/shop.');
    const board = github({
      tracker: { kind: 'github', repo: 'acme/shop', project_board: { owner: 'acme', number: 4 } },
    });
    expect(writeBackLines(board, null, 'In progress')[0]).toBe(
      'When you start an item, it moves to In progress on the board and gets a comment "Started in Orchestrator".',
    );
    expect(writeBackLines({ ...board, write_back: { ...wb, assign_me: true } })[0]).toContain(
      'it is assigned to you, it moves to',
    );
    expect(writeBackLines(linear())[2]).toBe('Pull requests link the issue; merging moves it to Done.');
  });

  it('switching kind resets the Ready filter and keeps write-back and refresh', () => {
    const f = switchKind({ ...github(), refresh_secs: 1800 }, 'linear');
    expect(f.ready).toEqual({
      kind: 'linear',
      state_types: ['unstarted'],
      label: null,
      project: null,
      cycle: 'current',
    });
    expect(f.tracker).toEqual({ kind: 'linear', teams: [], projects: null });
    expect(f.refresh_secs).toBe(1800);
    expect(switchKind(f, 'local').ready).toEqual({ kind: 'local' });
  });

  it('default team task columns by state type', () => {
    expect(LIN_OPTS.states.map(stateColumn)).toEqual(['backlog', 'ready', 'in_progress', 'review', 'done']);
  });

  it('the four notices in the notes words', () => {
    const merged = {
      kind: 'merged',
      from: { intake_source: { type: 'github', filter: { kind: 'label', names: ['ready'] } } },
      kept: github().tracker,
      alternative: null,
    };
    const f = github({ tracker: { kind: 'github', repo: 'acme/shop', project_board: null } });
    expect(noticeInfo({ ...merged, kept: f.tracker }, f).text).toBe(
      'Task backend and Intake are now one setting: the tracker. acme/shop is used for both, and your Ready filter (label ready) is kept.',
    );
  });
});

describe('Project settings › Tracker', () => {
  it('is the third section; Intake and Task backend are gone; opening with no section lands on it', async () => {
    const { getAllByRole, findByRole } = open(local());
    const tabs = getAllByRole('tab').map((b) => b.textContent.trim());
    expect(tabs).toContain('Tracker');
    expect(tabs).not.toContain('Intake');
    expect(tabs).not.toContain('Task backend');
    expect(await findByRole('radiogroup', { name: 'Kind' })).toBeTruthy();
    app.window = null;
    openProjectSettings('shop-api');
    expect(app.window.section).toBe('tracker');
  });

  it('Local: backlog folder with the live match line, no write-back, Save sends the settings', async () => {
    replies.tracker_options = { matches: 3 };
    const { findByLabelText, container, getByRole, queryByText } = open(local());
    const glob = await findByLabelText('Backlog folder');
    await waitFor(() => expect(container.querySelector('[data-matches]')?.textContent).toBe('3 files match'));
    expect(txt(container)).toContain('Local has no hierarchy: Work shows a flat list.');
    expect(txt(container)).toContain('Every backlog file not marked done.');
    expect(queryByText(/as work runs/)).toBeNull();
    await fireEvent.input(glob, { target: { value: 'docs/backlog/*.md' } });
    expect(txt(container)).toContain('Unsaved changes');
    await fireEvent.click(getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(called('tracker_settings_set')).toHaveLength(1));
    expect(last('tracker_settings_set')).toMatchObject({
      project: 'shop-api',
      settings: { tracker: { kind: 'local', backlog_glob: 'docs/backlog/*.md' } },
    });
    await waitFor(() => expect(txt(container)).not.toContain('Unsaved changes'));
    expect(txt(container)).toContain('Saved · updated just now');
  });

  it('Local: an empty folder says only tasks you create; a path outside the repo is refused', async () => {
    const { findByLabelText, container, getByRole } = open(local());
    const glob = await findByLabelText('Backlog folder');
    await fireEvent.input(glob, { target: { value: '' } });
    expect(txt(container)).toContain('No backlog files: only tasks you create.');
    await fireEvent.input(glob, { target: { value: '../x/*.md' } });
    await fireEvent.click(getByRole('button', { name: 'Save' }));
    expect(txt(container)).toContain('Use a path inside the repo: no leading `/`, no `..`.');
    expect(called('tracker_settings_set')).toHaveLength(0);
  });

  it('GitHub: board, Ready filter, write-back with a board, Preview, Save', async () => {
    replies.tracker_preview = {
      roots: 3,
      items: 14,
      ready: 1,
      error: null,
      sample: [
        {
          id: 'gh:acme/shop#13',
          ref: '#13',
          title: 'Address step',
          breadcrumb: [{ id: 'gh:acme/shop#12', ref: '#12', title: 'Checkout v2' }],
          upstream: {},
          ready: false,
        },
        {
          id: 'gh:acme/shop#9',
          ref: '#9',
          title: 'Flaky checkout e2e',
          breadcrumb: [],
          upstream: { milestone: 'v2' },
          ready: true,
        },
      ],
    };
    const { findByLabelText, container, getByRole, findByRole } = open(
      github({ tracker: { kind: 'github', repo: 'acme/shop', project_board: null } }),
    );
    await findByLabelText('Repository');
    await waitFor(() => expect(called('tracker_options').length).toBeGreaterThan(0));
    // Without a board: the running label.
    expect(txt(container)).toContain('Update GitHub as work runs');
    expect(txt(container)).toContain('it gets the label orch:running');
    expect(el(await findByLabelText('Running label')).value).toBe('orch:running');
    // A board: the Status picker takes over and the summary follows.
    await pick(el(container.querySelector('#trk-board')).closest('.col') ?? container, 'acme · #4 Roadmap');
    expect(last('tracker_options')).toBeTruthy();
    expect(txt(container)).toContain('it moves to In progress on the board');
    expect(picked(container.querySelector('[aria-labelledby="trk-mv"]').closest('.col'))).toBe('In progress');
    await pick(container.querySelector('#trk-ms').closest('.row'), 'v2');
    await pick(container.querySelector('#trk-bs').closest('.row'), 'Todo');
    expect(container.querySelector('[data-ready-summary]').textContent).toBe(
      'Open issues labelled ready, in milestone v2, with board status Todo',
    );
    await fireEvent.click(await findByRole('checkbox', { name: 'Assign the item to me when I start it' }));
    expect(txt(container)).toContain('it is assigned to you,');
    // Preview.
    await fireEvent.click(getByRole('button', { name: 'Preview' }));
    await waitFor(() => expect(container.querySelector('[data-preview]')).toBeTruthy());
    const pv = txt(container.querySelector('[data-preview]'));
    expect(pv).toContain('14 items, 3 at the top · 1 ready in acme/shop');
    expect(pv).toContain('#13 Address step in #12 Checkout v2');
    expect(pv).toContain('#9 Flaky checkout e2e in v2 · ready');
    expect(last('tracker_preview').settings.tracker.project_board).toEqual({ owner: 'acme', number: 4 });
    // An edit after the preview marks it stale.
    await fireEvent.click(await findByRole('checkbox', { name: 'Assign the item to me when I start it' }));
    expect(container.querySelector('[data-preview]').classList.contains('stale')).toBe(true);
  });

  it('GitHub: write-back off says Read-only; the label tags add and remove', async () => {
    const { findByLabelText, container, getByRole } = open(github());
    const labels = await findByLabelText('Labels');
    await fireEvent.input(labels, { target: { value: 'next' } });
    await fireEvent.keyDown(labels, { key: 'Enter' });
    expect(container.querySelector('[data-ready-summary]').textContent).toBe('Open issues labelled ready, next');
    await fireEvent.click(getByRole('button', { name: 'Remove ready' }));
    await fireEvent.click(getByRole('button', { name: 'Remove next' }));
    expect(container.querySelector('[data-ready-summary]').textContent).toMatch(/^Nothing: the lane stays empty/);
    await fireEvent.click(getByRole('switch'));
    expect(container.querySelector('[data-write-back-off]').textContent).toBe(
      'Read-only: GitHub stays as it is. Team tasks stay in the app.',
    );
    await fireEvent.click(getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(last('tracker_settings_set').settings.write_back.enabled).toBe(false));
  });

  it('Linear: teams, projects, Advanced states, Ready filter, write-back state; saved as TrackerSettings', async () => {
    replies.tracker_options = LIN_OPTS;
    const { container, getByRole, findByRole } = open(
      linear({ tracker: { kind: 'linear', teams: [], projects: null } }),
    );
    await findByRole('radiogroup', { name: 'Kind' });
    // At least one team.
    await fireEvent.click(getByRole('button', { name: 'Preview' }));
    expect(txt(container)).toContain('Pick at least one team.');
    await waitFor(() => expect(called('tracker_options').length).toBeGreaterThan(0));
    await pick(container.querySelector('[aria-labelledby="trk-teams"]'), 'ENG · Engineering');
    expect(container.querySelector('[aria-labelledby="trk-teams"]').textContent).toContain('ENG');
    // Only these projects.
    await fireEvent.click(getByRole('radio', { name: 'Only these projects' }));
    await pick(container.querySelector('[data-projects]'), 'Billing revamp');
    expect(container.querySelector('[data-projects]').textContent).toContain('Billing revamp');
    // Advanced: the mapping moved here, defaults per type; moving one saves only that.
    el(container.querySelector('details.adv')).open = true;
    await waitFor(() => expect(container.querySelectorAll('.mrow')).toHaveLength(5));
    expect(container.querySelectorAll('.mrow')[3].textContent).toContain('In Review');
    await pick(container.querySelectorAll('.mrow')[2], 'Review');
    // Ready: Backlog too, cycle Next.
    await fireEvent.click(getByRole('checkbox', { name: 'Backlog' }));
    await fireEvent.click(getByRole('radio', { name: 'Next' }));
    expect(container.querySelector('[data-ready-summary]').textContent).toBe(
      'Backlog and Unstarted issues in the next cycle',
    );
    // Write back: Move to a started state.
    expect(txt(container)).toContain('Update Linear as work runs');
    expect(txt(container)).toContain('Pull requests link the issue; merging moves it to Done.');
    await pick(container.querySelector('[aria-labelledby="trk-mv"]').closest('.col'), 'In Review');
    await fireEvent.keyDown(container.querySelector('.tracker'), { key: 'Enter', metaKey: true });
    await waitFor(() => expect(called('tracker_settings_set')).toHaveLength(1));
    expect(last('tracker_settings_set').settings).toMatchObject({
      tracker: { kind: 'linear', teams: ['ENG'], projects: ['p2'] },
      ready: { kind: 'linear', state_types: ['backlog', 'unstarted'], cycle: 'next' },
      write_back: { enabled: true, linear_state: 's4' },
      task_states: { s3: 'review' },
    });
  });

  it('switching kind resets Ready to that kind defaults and keeps write-back', async () => {
    const { findByRole, container } = open(
      github({ ready: { kind: 'github', labels: ['x'], milestone: 'v2', board_status: null, issue_type: null } }),
    );
    await fireEvent.click(await findByRole('radio', { name: 'Linear' }));
    expect(container.querySelector('[data-ready-summary]').textContent).toBe('Unstarted issues in the current cycle');
    await fireEvent.click(await findByRole('radio', { name: 'Local' }));
    expect(txt(container)).toContain('Every backlog file not marked done.');
    await fireEvent.click(await findByRole('radio', { name: 'GitHub' }));
    await waitFor(() =>
      expect(container.querySelector('[data-ready-summary]').textContent).toBe('Open issues labelled ready'),
    );
  });

  it('a core field error stays under its field; Revert goes back to what is saved', async () => {
    replies.tracker_settings_set = () => Promise.reject({ code: 'bad_request', field: 'ready', message: 'x' });
    const { findByLabelText, getByRole, container } = open(github());
    await fireEvent.input(await findByLabelText('Repository'), { target: { value: 'acme/web' } });
    await fireEvent.click(getByRole('button', { name: 'Save' }));
    await waitFor(() =>
      expect(container.querySelector('[role="alert"]')?.textContent).toContain(
        'Ready filter does not match the tracker',
      ),
    );
    await fireEvent.click(getByRole('button', { name: 'Revert' }));
    expect(el(await findByLabelText('Repository')).value).toBe('');
    expect(txt(container)).not.toContain('Unsaved changes');
  });

  it('a switch to Linear with tasks asks (D8) instead of claiming a save; the choice is sent as migrate', async () => {
    replies.tracker_options = LIN_OPTS;
    replies.tracker_settings_set = (a) =>
      a.migrate
        ? { settings: a.settings, migration_notice: null }
        : { confirm: { kind: 'linear_migrate', open: 2, done: 1 } };
    const { findByRole, getByRole, container } = open(local());
    await fireEvent.click(await findByRole('radio', { name: 'Linear' }));
    await waitFor(() => expect(called('tracker_options').length).toBeGreaterThan(0));
    await pick(container.querySelector('[aria-labelledby="trk-teams"]'), 'ENG · Engineering');
    await fireEvent.click(getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(container.querySelector('[data-confirm]')).toBeTruthy());
    expect(txt(container)).toContain('2 open tasks. 1 done tasks are not pushed.');
    expect(txt(container)).toContain('Unsaved changes'); // nothing was stored
    expect(txt(container)).not.toContain('Saved · updated just now');
    await fireEvent.click(getByRole('button', { name: /^Create 2 open tasks as issues in ENG/ }));
    await waitFor(() => expect(called('tracker_settings_set')).toHaveLength(2));
    expect(last('tracker_settings_set')).toMatchObject({
      migrate: 'push_open',
      settings: { tracker: { kind: 'linear' } },
    });
    await waitFor(() => expect(txt(container)).toContain('Saved · updated just now'));
    expect(container.querySelector('[data-confirm]')).toBeNull();
  });

  it('⌘⏎ in the Ready labels field adds the typed label and still saves', async () => {
    const { findByLabelText, container } = open(github());
    const labels = await findByLabelText('Labels');
    await fireEvent.input(labels, { target: { value: 'next' } });
    await fireEvent.keyDown(labels, { key: 'Enter', metaKey: true });
    await waitFor(() => expect(called('tracker_settings_set')).toHaveLength(1));
    expect(last('tracker_settings_set').settings.ready.labels).toEqual(['ready', 'next']);
    expect(container.querySelector('.tracker')).toBeTruthy();
  });

  it('a repository that is not owner/name is refused before Save', async () => {
    const { findByLabelText, getByRole, container } = open(github());
    await fireEvent.input(await findByLabelText('Repository'), { target: { value: 'shop' } });
    await fireEvent.click(getByRole('button', { name: 'Save' }));
    expect(txt(container)).toContain('Use owner/name, e.g. acme/shop.');
    expect(called('tracker_settings_set')).toHaveLength(0);
  });
});

describe('Tracker › errors', () => {
  const run = async (error, settings = github()) => {
    replies.tracker_preview = { roots: 0, items: 0, ready: 0, sample: [], error };
    const r = open(settings);
    await r.findByLabelText(
      settings.tracker.kind === 'local'
        ? 'Backlog folder'
        : settings.tracker.kind === 'github'
          ? 'Repository'
          : 'Teams',
    );
    await fireEvent.click(r.getByRole('button', { name: 'Preview' }));
    await waitFor(() => expect(r.container.querySelector('[data-preview] [role="alert"]')).toBeTruthy());
    return r;
  };

  it('linear_complexity: Fix opens the Projects field', async () => {
    replies.tracker_options = LIN_OPTS;
    const { container, getByRole } = await run({ code: 'linear_complexity', message: 'too big' }, linear());
    expect(txt(container.querySelector('[data-preview]'))).toContain(
      'Linear said the request was too large. Work shows what is cached.',
    );
    await fireEvent.click(getByRole('button', { name: 'Limit the tracker to fewer projects' }));
    expect(container.querySelector('input[name="trk-projects"]:checked').parentElement.textContent).toContain(
      'Only these projects',
    );
  });

  it('gh_write: Turn off write-back switches it off', async () => {
    const { container, getByRole } = await run({ code: 'gh_write', message: 'forbidden' });
    expect(txt(container.querySelector('[data-preview]'))).toContain(
      "GitHub refused the change: your login can't write to this repository.",
    );
    await fireEvent.click(getByRole('button', { name: 'Turn off write-back' }));
    expect(container.querySelector('[data-write-back-off]')).toBeTruthy();
  });

  it('gh_sub_issues: Retry previews again', async () => {
    const { getByRole } = await run({ code: 'gh_sub_issues', message: 'x' });
    await fireEvent.click(getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(called('tracker_preview')).toHaveLength(2));
  });

  it('gh_scope on the board: the command to copy', async () => {
    replies.tracker_options = { error: { code: 'gh_scope', message: 'scope' } };
    const { container } = open(github());
    await waitFor(() => expect(container.querySelector('[data-gh-scope]')).toBeTruthy());
    expect(txt(container.querySelector('[data-gh-scope]'))).toContain('gh auth refresh -s read:project');
  });

  it('not set up: points to Integrations', async () => {
    replies.tracker_options = { error: { code: 'linear_key', message: 'no key' } };
    const { container } = open(linear());
    await waitFor(() =>
      expect(container.querySelector('[data-not-set-up]')?.textContent).toContain("Linear isn't set up."),
    );
  });
});

describe('Tracker › migration notices', () => {
  const ghKept = { kind: 'github', repo: 'acme/shop', project_board: null };
  const from = {
    task_backend: { type: 'github' },
    intake_source: { type: 'github', filter: { kind: 'label', names: ['ready'] } },
  };

  it('same repo: Got it dismisses', async () => {
    const { findByRole, container } = open(github({ tracker: ghKept }), {
      kind: 'merged',
      from,
      kept: ghKept,
      alternative: null,
      dismissed: false,
    });
    await findByRole('radiogroup', { name: 'Kind' });
    expect(txt(container.querySelector('[data-notice]'))).toContain(
      'Task backend and Intake are now one setting: the tracker. acme/shop is used for both, and your Ready filter (label ready) is kept.',
    );
    await fireEvent.click(await findByRole('button', { name: 'Got it' }));
    await waitFor(() => expect(called('tracker_notice_dismiss')).toHaveLength(1));
    expect(container.querySelector('[data-notice]')).toBeNull();
  });

  it('different repo: Use acme/web instead puts it in the form', async () => {
    const alt = { kind: 'github', repo: 'acme/web', project_board: null };
    const { findByRole, container, findByLabelText } = open(github({ tracker: ghKept }), {
      kind: 'different_repo',
      from,
      kept: ghKept,
      alternative: alt,
      dismissed: false,
    });
    await findByRole('radiogroup', { name: 'Kind' });
    expect(txt(container.querySelector('[data-notice]'))).toContain(
      'Task backend used acme/shop and Intake used acme/web. The tracker keeps acme/shop; the Intake label ready is now its Ready filter. acme/web is no longer read.',
    );
    await fireEvent.click(await findByRole('button', { name: 'Use acme/web instead' }));
    expect(el(await findByLabelText('Repository')).value).toBe('acme/web');
    expect(container.querySelector('[data-notice]')).toBeNull();
    expect(txt(container)).toContain('Unsaved changes');
  });

  it('mismatch: pick a tracker, Use this tracker saves it', async () => {
    const kept = { kind: 'linear', teams: ['ENG'], projects: null };
    const alt = { kind: 'github', repo: 'acme/shop', project_board: null };
    const { findByRole, container } = open(linear(), {
      kind: 'mismatch',
      from,
      kept,
      alternative: alt,
      dismissed: false,
    });
    await findByRole('radiogroup', { name: 'Kind' });
    const n = container.querySelector('[data-notice]');
    expect(n.classList.contains('warn')).toBe(true);
    expect(txt(n)).toContain(
      'Intake read GitHub acme/shop but tasks were in Linear ENG. Pick the tracker for shop-api:',
    );
    expect(txt(n)).toContain('Until you pick, Work shows Linear ENG.');
    await fireEvent.click(await findByRole('button', { name: 'Use this tracker' }));
    await waitFor(() => expect(called('tracker_settings_set')).toHaveLength(1));
    expect(last('tracker_settings_set').settings).toMatchObject({ tracker: alt, ready: { kind: 'github' } });
    await waitFor(() => expect(container.querySelector('[data-notice]')).toBeNull());
  });

  it('Local + repo docs', async () => {
    const { container, findByRole } = open(local(), {
      kind: 'backlog',
      from: { task_backend: { type: 'local' }, intake_source: { type: 'repo_docs' } },
      kept: local().tracker,
      alternative: null,
      dismissed: false,
    });
    await findByRole('radiogroup', { name: 'Kind' });
    expect(txt(container.querySelector('[data-notice]'))).toContain(
      "Intake's repo docs folder is now the Local tracker's backlog.",
    );
  });
});
