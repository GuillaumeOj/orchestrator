// add-build-pr (T-4): Settings › Builds rows and header, Back to main, the PR
// status chip, palette entries, toasts. Against a mocked core.
import { cleanup, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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

const { app, commands, restartToUpdate, announceWarnings } = await import('../src/lib/app.svelte.js');
const { default: StatusBar } = await import('../src/components/StatusBar.svelte');
const { default: BuildsSettings } = await import('../src/components/BuildsSettings.svelte');
const {
  aboutText,
  backToMain,
  buildsHeader,
  isRestoreWarning,
  prChip,
  prKey,
  prReadyToast,
  restartConfirm,
  rowState,
  updateChip,
} = await import('../src/lib/update.js');

const upd = (o = {}) => ({
  current: { version: '0.1.41', sha: 'bd5797c0000', channel: 'main', pr: null },
  enabled: true,
  disabled_why: null,
  state: 'idle',
  available: null,
  progress: null,
  error: null,
  checked_at: 1793600123,
  apply_on_quit: false,
  pr_install: null,
  ...o,
});
const avail = { version: '0.1.42', sha: 'f9b1e20', published_at: 1793600000, subject: 'x' };
const prRunning = (o = {}) => upd({ current: { version: '0.1.57', sha: 'f9b1e20aaaa', channel: 'pr', pr: 12 }, ...o });
const mk = (number, o = {}) => ({
  number,
  title: `PR ${number}`,
  branch: `b-${number}`,
  author: 'me',
  url: `https://github.com/GuillaumeOj/orchestrator/pull/${number}`,
  draft: false,
  head_sha: 'a17c3d0aaaa',
  updated_at: 1,
  build: null,
  ...o,
});
const bld = (o = {}) => ({ status: 'current', version: '0.1.61', sha: 'a17c3d0aaaa', published_at: 1793600000, ...o });
const inst = (o = {}) => ({
  pr: 14,
  title: 't',
  version: '0.1.61',
  sha: 'a17c3d0aaaa',
  state: 'ready',
  progress: null,
  error: null,
  ...o,
});
const list = (prs, o = {}) => ({ fetched_at: 1793600123, error: null, prs, ...o });

describe('row states', () => {
  const now = 1793600000 * 1000 + 12 * 60000;
  const st = (pr, u = upd()) => rowState(pr, u, now);
  it('current: build sha · age, Install', () => {
    expect(st(mk(14, { build: bld() }))).toMatchObject({
      kind: 'current',
      text: 'build a17c3d0 · 12 min ago',
      action: { label: 'Install', run: 'install' },
    });
  });
  it('outdated: installs the older build', () => {
    expect(st(mk(11, { build: bld({ status: 'outdated', sha: 'old0000' }) }))).toMatchObject({
      kind: 'outdated',
      text: 'built from an older commit',
      note: '(newest commit not built yet)',
      action: { label: 'Install' },
    });
  });
  it('no build: draft and plain, no action', () => {
    expect(st(mk(9, { draft: true }))).toMatchObject({ kind: 'draft', text: 'draft: not built', action: null });
    expect(st(mk(8))).toMatchObject({ kind: 'none', text: 'no build yet', action: null });
  });
  it('running: same sha, or a newer build to Update to', () => {
    expect(st(mk(12, { build: bld({ sha: 'f9b1e20aaaa' }) }), prRunning())).toMatchObject({
      kind: 'running',
      text: 'running',
      action: null,
    });
    expect(st(mk(12), prRunning())).toMatchObject({ kind: 'running', action: null });
    expect(st(mk(12, { build: bld() }), prRunning())).toMatchObject({
      kind: 'running-newer',
      text: 'running · newer build',
      action: { label: 'Update', run: 'install' },
    });
    expect(st(mk(13, { build: bld() }), prRunning()).kind).toBe('current');
  });
  it('installing: progress, and the other rows cannot Install', () => {
    const u = upd({ pr_install: inst({ state: 'downloading', progress: 0.4 }) });
    expect(st(mk(14, { build: bld() }), u)).toMatchObject({
      kind: 'installing',
      text: 'Downloading · 40 %',
      action: null,
    });
    expect(st(mk(11, { build: bld() }), u).action).toMatchObject({ label: 'Install', disabled: true });
    expect(st(mk(14), upd({ pr_install: inst({ state: 'downloading' }) })).text).toBe('Downloading');
  });
  it('ready: Restart to run; error: amber text and Retry', () => {
    expect(st(mk(14, { build: bld() }), upd({ pr_install: inst() }))).toMatchObject({
      kind: 'ready',
      text: 'Ready',
      action: { label: 'Restart to run', run: 'restart', primary: true },
    });
    expect(st(mk(14, { build: bld() }), upd({ pr_install: inst({ state: 'error', error: 'offline' }) }))).toMatchObject(
      { kind: 'error', text: 'offline', tone: 'warn', action: { label: 'Retry', run: 'install' } },
    );
    expect(st(mk(11, { build: bld() }), upd({ pr_install: inst() })).action.disabled).toBeFalsy();
  });
  it('the inline confirm says what restarting does', () => {
    expect(restartConfirm(upd(), 14)).toContain(
      'Restart into PR #14? Agents are suspended and resume. Your data is snapshotted first',
    );
    expect(restartConfirm(prRunning(), 14)).toContain(
      'not snapshotted again (the snapshot from before PR #12 is kept)',
    );
  });
});

describe('header and Back to main', () => {
  it('main: version, sha, main, the update in words, no action', () => {
    const h = buildsHeader(upd());
    expect(h).toMatchObject({ version: '0.1.41', sha: 'bd5797c', channel: 'main', line2: 'up to date' });
    expect(h.back.show).toBe(false);
    expect(buildsHeader(upd({ state: 'ready', available: avail, apply_on_quit: true })).line2).toBe(
      '0.1.42 ready, installs when you quit',
    );
  });
  it('PR: channel with number and title; Back to main enabled only once main is ready', () => {
    expect(buildsHeader(prRunning(), 'Add build on PRs').channel).toBe('PR #12 Add build on PRs');
    expect(backToMain(prRunning({ state: 'ready', available: avail }))).toEqual({
      show: true,
      why: null,
      canCheck: false,
    });
    expect(buildsHeader(prRunning({ state: 'ready', available: avail })).line2).toBe(
      'main 0.1.42 is downloaded: Back to main restarts into it.',
    );
    expect(backToMain(prRunning({ state: 'downloading', available: avail }))).toMatchObject({
      why: 'main build downloading…',
    });
    expect(backToMain(prRunning({ state: 'error', error: 'offline' }))).toMatchObject({
      why: 'offline',
      canCheck: true,
    });
    expect(backToMain(prRunning())).toMatchObject({ why: 'checking main…', canCheck: true });
  });
  it('local build behaves like a PR build toward main', () => {
    const local = upd({ current: { version: '0.1.0', sha: '', channel: 'local' }, state: 'ready', available: avail });
    expect(backToMain(local).why).toBeNull();
    expect(buildsHeader(local).channel).toBe('local build');
  });
  it('is hidden on main and disabled with the reason when updates are off', () => {
    expect(backToMain(upd()).show).toBe(false);
    expect(backToMain(prRunning({ enabled: false, disabled_why: 'gh not logged in' }))).toMatchObject({
      show: true,
      why: 'gh not logged in',
    });
  });
});

describe('PR chip, tooltip and toasts', () => {
  it('chip only while running a PR build', () => {
    expect(prChip(upd())).toBeNull();
    expect(prChip(upd({ current: { version: '0.1.0', sha: '', channel: 'local' } }))).toBeNull();
    expect(prChip(prRunning(), 'Add build on PRs')).toEqual({
      text: 'PR #12',
      title: "You run PR #12's build (f9b1e20): Add build on PRs. Click to change build or go back to main.",
    });
  });
  it('the main chip tooltip says it goes back to main when running a PR build', () => {
    expect(updateChip(prRunning({ state: 'ready', available: avail })).title).toBe(
      'Back to main 0.1.42: restarts into it (agents are suspended and resume).',
    );
  });
  it('PR ready toast once per staged build', () => {
    const u = upd({ pr_install: inst() });
    expect(prReadyToast(u, null)).toBe("PR #14's build is ready.");
    expect(prReadyToast(u, prKey(u.pr_install))).toBeNull();
    expect(prReadyToast(upd({ pr_install: inst({ state: 'downloading' }) }), null)).toBeNull();
  });
  it('restore warnings are recognised and shown as an amber 15 s toast, once', () => {
    const w =
      'Your data was restored from before PR #12 (taken 14:02, 3 Nov). What changed while testing it is kept in /x.';
    expect(isRestoreWarning(w)).toBe(true);
    expect(isRestoreWarning('helpers: x')).toBe(false);
    app.toasts = [];
    announceWarnings([w, 'helpers: x']);
    announceWarnings([w]);
    expect(app.toasts).toHaveLength(1);
    expect(app.toasts[0]).toMatchObject({ kind: 'warn', ms: 15000, text: w });
  });
  it('about shows PR #n', () => {
    expect(aboutText(prRunning())).toContain('· PR #12');
  });
});

describe('Settings › Builds, status bar, palette', () => {
  beforeEach(() => {
    app.data = fixture();
    app.toasts = [];
    app.prTitles = {};
    calls.length = 0;
    for (const k of Object.keys(replies)) delete replies[k];
  });
  afterEach(cleanup);
  const flush = async () => {
    await tick();
    await new Promise((r) => setTimeout(r, 0));
    await tick();
  };
  const btn = (el, label) => [...el.querySelectorAll('button')].find((b) => b.textContent.includes(label));

  it('status bar shows PR #n only while running a PR build', async () => {
    app.data.update = prRunning();
    const { container } = render(StatusBar);
    expect(btn(container, 'PR #12')?.classList.contains('ready')).toBe(true);
    app.data.update = upd();
    await tick();
    expect(container.textContent).not.toContain('PR #12');
  });

  it('palette: Install a PR build… and Back to main follow the state', () => {
    const by = () =>
      Object.fromEntries(
        commands()
          .filter((c) => c.id.startsWith('update.'))
          .map((c) => [c.id, c]),
      );
    app.data.update = upd();
    expect(by()['update.pr'].label).toBe('Install a PR build…');
    expect(by()['update.main'].disabled).toBe('you run main');
    app.data.update = prRunning({ state: 'ready', available: avail });
    expect(by()['update.main'].disabled).toBeNull();
  });

  it('restart keeps working for main and takes a target', async () => {
    await restartToUpdate();
    await restartToUpdate('pr');
    await restartToUpdate(/** @type {any} */ ({ type: 'click' }));
    expect(calls.map((c) => c[1])).toEqual([{ target: 'main' }, { target: 'pr' }, { target: 'main' }]);
  });

  it('lists each row state, and asks before restarting', async () => {
    app.data.update = prRunning({ state: 'ready', available: avail });
    replies.pr_builds_list = list([
      mk(14, { build: bld() }),
      mk(12, { build: bld({ sha: 'f9b1e20aaaa' }) }),
      mk(11, { build: bld({ status: 'outdated' }) }),
      mk(9, { draft: true }),
      mk(8),
    ]);
    const { container } = render(BuildsSettings);
    await flush();
    const state = (n) => /** @type {HTMLElement} */ (container.querySelector(`[data-pr="${n}"]`)).dataset.state;
    expect([14, 12, 11, 9, 8].map(state)).toEqual(['current', 'running', 'outdated', 'draft', 'none']);
    expect(container.querySelector('[data-running]').textContent).toContain('PR #12');
    expect(container.textContent).toContain('refreshed');
    app.data.update = prRunning({ state: 'ready', available: avail, pr_install: inst() });
    await tick();
    expect(state(14)).toBe('ready');
    const row = container.querySelector('[data-pr="14"]');
    btn(row, 'Restart to run').click();
    await tick();
    expect(row.querySelector('[data-confirm]').textContent).toContain('Restart into PR #14?');
    btn(row, 'Cancel').click();
    await tick();
    expect(row.querySelector('[data-confirm]')).toBeNull();
    btn(row, 'Restart to run').click();
    await tick();
    /** @type {HTMLElement} */ (row.querySelector('[data-restart]')).click();
    expect(calls.at(-1)).toEqual(['update_restart', { target: 'pr' }]);
  });

  it('Install calls pr_build_install', async () => {
    app.data.update = upd();
    replies.pr_builds_list = list([mk(14, { build: bld() })]);
    replies.pr_build_install = upd({ pr_install: inst({ state: 'downloading' }) });
    const { container } = render(BuildsSettings);
    await flush();
    btn(container, 'Install').click();
    await flush();
    expect(calls.at(-1)).toEqual(['pr_build_install', { number: 14 }]);
  });

  it('empty, error and off states', async () => {
    app.data.update = upd();
    replies.pr_builds_list = list([]);
    let r = render(BuildsSettings);
    await flush();
    expect(r.container.textContent).toContain('No open pull requests.');
    r.unmount();
    replies.pr_builds_list = list([], { error: 'gh not logged in' });
    r = render(BuildsSettings);
    await flush();
    expect(r.container.querySelector('.warn').textContent).toContain('gh not logged in');
    r.unmount();
    app.data.update = upd({ enabled: false, disabled_why: 'debug build' });
    r = render(BuildsSettings);
    await flush();
    expect(r.container.textContent).toContain('PR builds are off: debug build.');
    expect(r.container.querySelector('[data-pr]')).toBeNull();
  });

  it('Back to main is disabled until main is ready', async () => {
    app.data.update = prRunning();
    replies.pr_builds_list = list([]);
    const { container } = render(BuildsSettings);
    await flush();
    expect(btn(container, 'Back to main').disabled).toBe(true);
    expect(container.textContent).toContain('checking main…');
    app.data.update = prRunning({ state: 'ready', available: avail });
    await tick();
    expect(btn(container, 'Back to main').disabled).toBe(false);
  });
});
