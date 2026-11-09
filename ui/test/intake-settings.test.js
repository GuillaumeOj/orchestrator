// P4.5 Project settings › Intake and Settings › Pacing › Intake (spec §3.6, notes p4-5 §7),
// against a mocked core.

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

const { default: ProjectSettingsPane } = await import('../src/panes/ProjectSettingsPane.svelte');
const { default: IntakeGlobalSettings } = await import('../src/components/IntakeGlobalSettings.svelte');

const info = (source = null) => ({
  source,
  default_glob: '.orchestrator/backlog/*.md',
  default_branch: 'main',
  known_repo: 'acme/shop',
});
const called = (cmd) => calls.filter(([c]) => c === cmd);
const el = (x) => /** @type {any} */ (x);
const last = (cmd) => called(cmd).at(-1)?.[1];

function open(section = 'intake') {
  app.data = fixture();
  app.panes = { ps: { type: 'project-settings', projectId: 'shop-api', section } };
  return render(ProjectSettingsPane, { paneId: 'ps' });
}

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  replies.project_settings = { settings: {}, backend: { type: 'local' }, track_orchestrator_dir: true };
  replies.board_view = null;
  replies.integrations_view = { items: [] };
  replies.intake_source = info();
  replies.intake_options = { labels: ['ready', 'next'], milestones: [{ title: 'v2.1', open_issues: 8 }] };
  replies.intake_preview = { count: 0, more: 0, items: [], error: null };
});

describe('Project settings › Intake', () => {
  it('is a section after Task backend, and openProjectSettings selects it', async () => {
    const { getAllByRole, findByRole } = open('backend');
    expect(
      getAllByRole('button')
        .map((b) => b.textContent.trim())
        .filter((t) => ['Task backend', 'Intake', 'Allocation'].includes(t)),
    ).toEqual(['Task backend', 'Intake', 'Allocation']);
    openProjectSettings('shop-api', 'intake');
    expect(app.panes.ps.section).toBe('intake');
    cleanup();
    open('intake');
    expect(await findByRole('radiogroup', { name: 'Source' })).toBeTruthy();
  });

  it('None says what the lane will offer', async () => {
    const { findByText } = open();
    expect(await findByText(/Plan with the Overwatch for this project/)).toBeTruthy();
  });

  it('GitHub: the repository is prefilled, a label is added and saved as an IntakeSource', async () => {
    replies.intake_source_set = { preview: { count: 2, more: 0, items: [], error: null } };
    const { findByRole, getByLabelText, getByRole, findByText } = open();
    await fireEvent.click(await findByRole('radio', { name: 'GitHub' }));
    expect(el(getByLabelText('Repository')).value).toBe('acme/shop');
    const label = getByLabelText('Label');
    await fireEvent.input(label, { target: { value: 'ready' } });
    await fireEvent.keyDown(label, { key: 'Enter' });
    await fireEvent.click(getByRole('button', { name: /^Save/ }));
    await waitFor(() => expect(called('intake_source_set')).toHaveLength(1));
    expect(last('intake_source_set')).toEqual({
      project: 'shop-api',
      source: { type: 'github', repo: 'acme/shop', filter: { kind: 'label', names: ['ready'] } },
    });
    expect(await findByText('Saved · updated just now')).toBeTruthy();
  });

  it('a label is required: the field says so and nothing is sent', async () => {
    const { findByRole, getByRole, findByText } = open();
    await fireEvent.click(await findByRole('radio', { name: 'GitHub' }));
    await fireEvent.click(getByRole('button', { name: /^Preview/ }));
    expect(await findByText('Add at least one label.')).toBeTruthy();
    expect(called('intake_preview')).toHaveLength(0);
  });

  it('Linear: team, any project, backlog flag', async () => {
    replies.linear_teams = { teams: [{ id: 't1', key: 'ENG', name: 'Engineering' }] };
    replies.linear_states = { states: [], projects: [{ id: 'lp1', name: 'Checkout' }] };
    replies.intake_options = { labels: ['next'] };
    replies.intake_source_set = { preview: { count: 0, more: 0, items: [], error: null } };
    const { findByRole, findByLabelText, getByLabelText, getByRole } = open();
    await fireEvent.click(await findByRole('radio', { name: 'Linear' }));
    const lteam = await findByLabelText('Team');
    await waitFor(() => expect(picked(lteam)).toBe('ENG'));
    await fireEvent.click(getByLabelText('Include backlog'));
    await fireEvent.click(getByRole('button', { name: /^Save/ }));
    await waitFor(() => expect(called('intake_source_set')).toHaveLength(1));
    expect(last('intake_source_set').source).toEqual({ type: 'linear', team: 'ENG', include_backlog: true });
  });

  it('Linear without a key links to Integrations', async () => {
    replies.linear_teams = new Error('no key');
    const { findByRole, findByText } = open();
    await fireEvent.click(await findByRole('radio', { name: 'Linear' }));
    expect(await findByText(/Linear isn't set up/)).toBeTruthy();
  });

  it('Repo docs: the default glob is the value, matches are counted, the branch is empty', async () => {
    replies.intake_options = { matches: 7 };
    replies.intake_source_set = { preview: { count: 7, more: 0, items: [], error: null } };
    const { findByRole, getByLabelText, findByText, getByRole } = open();
    await fireEvent.click(await findByRole('radio', { name: 'Repo docs' }));
    expect(el(getByLabelText('Glob')).value).toBe('.orchestrator/backlog/*.md');
    expect(await findByText('7 files match')).toBeTruthy();
    expect(el(getByLabelText('Branch')).placeholder).toBe('main (base branch)');
    await fireEvent.click(getByRole('button', { name: /^Save/ }));
    await waitFor(() => expect(called('intake_source_set')).toHaveLength(1));
    expect(last('intake_source_set').source).toEqual({
      type: 'repo_docs',
      glob: '.orchestrator/backlog/*.md',
      branch: null,
    });
  });

  it('drops an options answer older than the latest request', async () => {
    const slow = {};
    replies.intake_options = (a) => new Promise((res) => (slow[a.source.glob] = res));
    const { findByRole, getByLabelText, findByText } = open();
    await fireEvent.click(await findByRole('radio', { name: 'Repo docs' }));
    await waitFor(() => expect(slow['.orchestrator/backlog/*.md']).toBeTruthy(), { timeout: 2000 });
    await fireEvent.input(getByLabelText('Glob'), { target: { value: 'docs/*.md' } });
    await waitFor(() => expect(slow['docs/*.md']).toBeTruthy(), { timeout: 2000 });
    slow['docs/*.md']({ matches: 9 });
    expect(await findByText('9 files match')).toBeTruthy();
    // The first request answers last: it must not overwrite the newer count.
    slow['.orchestrator/backlog/*.md']({ matches: 1 });
    await new Promise((r) => setTimeout(r, 20));
    expect(await findByText('9 files match')).toBeTruthy();
  });

  it('a glob with .. is refused in the field', async () => {
    const { findByRole, getByLabelText, getByRole, findByText } = open();
    await fireEvent.click(await findByRole('radio', { name: 'Repo docs' }));
    await fireEvent.input(getByLabelText('Glob'), { target: { value: '../x/*.md' } });
    await fireEvent.click(getByRole('button', { name: /^Preview/ }));
    expect(await findByText('Use a path inside the repo: no leading `/`, no `..`.')).toBeTruthy();
  });

  it('Preview: items, zero and an error per code', async () => {
    replies.intake_options = { matches: 1 };
    const { findByRole, getByRole, findByText } = open();
    await fireEvent.click(await findByRole('radio', { name: 'Repo docs' }));
    const preview = getByRole('button', { name: /^Preview/ });

    replies.intake_preview = { count: 4, more: 0, items: [{ id: 'a', ref: '#1', title: 'Refunds' }], error: null };
    await fireEvent.click(preview);
    expect(await findByText('4 items')).toBeTruthy();
    expect(await findByText('Refunds')).toBeTruthy();
    expect(called('intake_preview')).toHaveLength(1);
    expect(last('intake_preview')).toEqual({
      project: 'shop-api',
      source: { type: 'repo_docs', glob: '.orchestrator/backlog/*.md', branch: null },
    });

    replies.intake_preview = { count: 0, more: 0, items: [], error: null };
    await fireEvent.click(preview);
    expect(await findByText(/Nothing matches this source right now/)).toBeTruthy();

    replies.intake_preview = {
      count: 0,
      more: 0,
      items: [],
      error: { code: 'gh_scope', message: 'x', fix: 'gh_scope' },
    };
    await fireEvent.click(preview);
    expect(await findByText(/GitHub needs the read:project scope/)).toBeTruthy();
    expect(await findByRole('button', { name: 'Copy command' })).toBeTruthy();

    replies.intake_preview = {
      count: 0,
      more: 0,
      items: [],
      error: { code: 'gh_auth', message: 'x', fix: 'integrations' },
    };
    await fireEvent.click(preview);
    expect(await findByText(/GitHub: not logged in/)).toBeTruthy();
    expect(await findByRole('button', { name: /Settings › Integrations/ })).toBeTruthy();
  });

  it('a bad_request from Save lands under its field', async () => {
    replies.intake_source_set = Object.assign(new Error('x'), {
      code: 'bad_request',
      field: 'repo',
      message: 'not owner/name',
    });
    const { findByRole, getByLabelText, getByRole, findByText } = open();
    await fireEvent.click(await findByRole('radio', { name: 'GitHub' }));
    const label = getByLabelText('Label');
    await fireEvent.input(label, { target: { value: 'ready' } });
    await fireEvent.keyDown(label, { key: 'Enter' });
    await fireEvent.click(getByRole('button', { name: /^Save/ }));
    expect(await findByText('Use owner/name, e.g. acme/shop.')).toBeTruthy();
  });

  it('shows Unsaved changes and Revert, and Revert restores the saved source', async () => {
    replies.intake_source = info({ type: 'repo_docs', glob: 'docs/*.md', branch: null });
    replies.intake_options = { matches: 2 };
    const { findByLabelText, findByText, getByLabelText, getByRole, queryByText } = open();
    const glob = await findByLabelText('Glob');
    expect(el(glob).value).toBe('docs/*.md');
    await fireEvent.input(glob, { target: { value: 'x/*.md' } });
    expect(await findByText('Unsaved changes')).toBeTruthy();
    await fireEvent.click(getByRole('button', { name: 'Revert' }));
    expect(el(getByLabelText('Glob')).value).toBe('docs/*.md');
    expect(queryByText('Unsaved changes')).toBeNull();
  });
});

describe('Settings › Pacing › Intake block', () => {
  const store = {};
  beforeEach(() => {
    for (const k of Object.keys(store)) delete store[k];
    replies.settings_get = ({ key }) => store[key] ?? null;
    replies.settings_set = ({ key, value }) => {
      store[key] = value;
      return null;
    };
  });

  const loaded = async () => {
    await waitFor(() => expect(called('settings_get')).toHaveLength(3));
    await new Promise((r) => setTimeout(r, 0));
  };

  it('reads the defaults and saves what changed under the F16 keys', async () => {
    const { findByLabelText, getByRole } = render(IntakeGlobalSettings);
    const every = await findByLabelText('Refresh every');
    await loaded();
    await waitFor(() => expect(picked(every)).toBe('600'));
    await pick(every, '30 min');
    await fireEvent.input(await findByLabelText('Default estimate, team run'), { target: { value: '8' } });
    await fireEvent.click(getByRole('button', { name: /^Save/ }));
    await waitFor(() => expect(called('settings_set')).toHaveLength(2));
    expect(store).toEqual({ 'intake.refresh_secs': 1800, 'intake.default_team_pp': 8 });
  });

  it("refuses a value outside 0.5–50 with the notes' words", async () => {
    const { findByLabelText, getByRole, findAllByText } = render(IntakeGlobalSettings);
    await findByLabelText('Refresh every');
    await loaded();
    await fireEvent.input(await findByLabelText('Default estimate, solo session'), { target: { value: '80' } });
    expect((await findAllByText('Use a number between 0.5 and 50.')).length).toBe(1);
    expect(el(getByRole('button', { name: /^Save/ })).disabled).toBe(true);
  });
});
