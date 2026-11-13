import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { app, claudeCheck, claudeUpdate, commands } from '../src/lib/app.svelte.js';
import {
  account,
  bannerClaude,
  checkToast,
  claudeChip,
  claudeToast,
  cmpVersion,
  codeSpans,
  compat,
  elapsed,
  hint,
  installedHeader,
  isVersionWarning,
  liveKeys,
  navBadge,
  paletteActions,
  updateBlock,
} from '../src/lib/claude.js';
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

const { default: ClaudeSettings } = await import('../src/components/ClaudeSettings.svelte');
const { default: Banner } = await import('../src/components/Banner.svelte');
const { default: StatusBar } = await import('../src/components/StatusBar.svelte');
const { default: SetupPane } = await import('../src/panes/SetupPane.svelte');

const upd = (o = {}) => ({
  state: 'idle',
  latest: '2.1.330',
  available: true,
  checked_at: 1793000000,
  error: null,
  error_from: null,
  started_at: null,
  updated: null,
  log: [],
  ...o,
});
const claude = (o = {}, u = {}) => ({
  path: '/Users/me/.local/bin/claude',
  version: '2.1.317',
  untested: false,
  logged_in: true,
  auth_method: 'claude.ai',
  subscription: 'max',
  problem: null,
  note: null,
  problem_kind: null,
  install: 'native',
  channel: 'latest',
  supported: { min: '2.1.302', tested_max: '2.1.317' },
  update: upd(u),
  ...o,
});
const old = (o = {}, u = {}) =>
  claude(
    {
      version: '2.1.288',
      problem: 'Claude Code 2.1.288 is too old: update Claude Code (Settings › Claude Code, or `claude update`).',
      problem_kind: 'version',
      ...o,
    },
    u,
  );
const stateWith = (c) => ({ ...fixture(), usage: null, setup: { done: true, claude: c } });
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();

describe('versions', () => {
  it('compares dotted numbers', () => {
    expect(cmpVersion('2.1.9', '2.1.10')).toBe(-1);
    expect(cmpVersion('2.1.330', '2.1.317')).toBe(1);
    expect(cmpVersion('2.1.317', '2.1.317')).toBe(0);
    expect(cmpVersion(null, '2.1.1')).toBeNull();
    expect(cmpVersion('2.1.317 (Claude Code)', '2.1.317')).toBe(0);
  });
  it('formats the elapsed time', () => {
    expect(elapsed(1000, 1000 * 1042)).toBe('0:42');
    expect(elapsed(1000, 1000 * (1000 + 3725))).toBe('1:02:05');
    expect(elapsed(null)).toBe('0:00');
  });
});

describe('update block', () => {
  const kind = (c) => updateBlock(c).kind;
  it('picks the first matching state', () => {
    expect(kind(null)).toBe('pending');
    expect(kind(claude({ problem: 'x', problem_kind: 'not_found' }))).toBe('not-found');
    expect(kind(claude({}, { state: 'updating', started_at: 1 }))).toBe('updating');
    expect(kind(claude({}, { state: 'error', error: 'boom', error_from: 'update' }))).toBe('update-failed');
    expect(kind(claude({}, { state: 'error', error: 'offline', error_from: 'check' }))).toBe('check-failed');
    expect(kind(claude({}, { state: 'checking' }))).toBe('checking');
    expect(kind(claude({}, { updated: { from: '2.1.317', to: '2.1.330', at: 1 }, available: false }))).toBe('updated');
    expect(kind(claude())).toBe('available');
    expect(kind(claude({}, { available: false }))).toBe('up-to-date');
    expect(kind(claude({}, { available: false, latest: null }))).toBe('never');
  });
  it('shows only the core’s text when Claude Code is not found, with no buttons', () => {
    const b = updateBlock(
      claude({ problem: 'Claude Code not found (claude: not found). Install it.', problem_kind: 'not_found' }),
    );
    expect(b.lead).toBe('Claude Code not found (claude: not found). Install it.');
    expect(b.buttons).toEqual([]);
    expect(b.setup).toBe(true);
  });
  it('updating: elapsed time, log open, no buttons', () => {
    const b = updateBlock(claude({}, { state: 'updating', started_at: 100 }), 142_000);
    expect(b.lead).toBe('Updating to 2.1.330…');
    expect(b.rest).toBe('0:42');
    expect(b.buttons).toEqual([]);
    expect(b.log).toBe('open');
  });
  it('failures: Retry first, Update anyway after a failed check', () => {
    const f = updateBlock(claude({}, { state: 'error', error: 'EACCES', error_from: 'update' }));
    expect(f.lead).toBe('Update failed: EACCES');
    expect(f.buttons.map((b) => [b.id, b.def])).toEqual([
      ['check', undefined],
      ['update', true],
    ]);
    expect(f.buttons[1]).toMatchObject({ label: 'Retry', primary: true });
    const k = updateBlock(claude({}, { state: 'error', error: 'offline', error_from: 'check' }));
    expect(k.buttons.map((b) => b.label)).toEqual(['Update anyway', 'Check now']);
    expect(k.line2).toMatch(/^Last known: 2\.1\.330 \(checked /);
  });
  it('a newer release after an update keeps the "Updated" line above', () => {
    const b = updateBlock(claude({}, { updated: { from: '2.1.317', to: '2.1.324', at: 1 } }));
    expect(b.kind).toBe('available');
    expect(b.before).toBe('Updated 2.1.317 → 2.1.324.');
    expect(b.buttons.map((x) => x.label)).toEqual(['Check now', 'Update to 2.1.330']);
    expect(b.buttons[1].primary).toBe(true);
  });
  it('a failed check right after an update keeps the Updated block with a muted line', () => {
    const upOk = { updated: { from: '2.1.317', to: '2.1.330', at: 1 }, available: false };
    for (const extra of [{ state: 'error', error: 'offline', error_from: 'check' }, { state: 'idle' }]) {
      const b = updateBlock(claude({}, { ...upOk, ...extra }));
      expect(b.kind).toBe('updated');
      expect(b.lead).toBe('Updated 2.1.317 → 2.1.330.');
      expect(b.buttons.map((x) => x.label)).toEqual(['Check now']);
      expect(b.line3).toBe(extra.error ? "Couldn't check for newer versions: offline" : null);
    }
    // a failed update still wins
    expect(updateBlock(claude({}, { ...upOk, state: 'error', error: 'x', error_from: 'update' })).kind).toBe(
      'update-failed',
    );
  });
  it('too old: always offers Update (primary, default), never "Up to date"', () => {
    for (const u of [
      { available: false },
      { available: false, latest: null },
      { available: false, latest: '2.1.288' },
    ]) {
      const b = updateBlock(old({}, u));
      expect(b.kind).toBe('too-old');
      expect(b.buttons.at(-1)).toMatchObject({ id: 'update', primary: true, def: true });
    }
  });
  it('no default button in up to date / updated', () => {
    expect(updateBlock(claude({}, { available: false })).buttons.some((b) => b.def)).toBe(false);
    expect(
      updateBlock(claude({}, { available: false, updated: { from: '1', to: '2', at: 1 } })).buttons.some((b) => b.def),
    ).toBe(false);
  });
  it('the u hint names the button', () => {
    expect(liveKeys(claude({}, { state: 'error', error: 'x', error_from: 'update' })).uLabel).toBe('retry');
    expect(liveKeys(claude({}, { state: 'error', error: 'x', error_from: 'check' })).uLabel).toBe('update anyway');
    expect(liveKeys(claude()).uLabel).toBe('update');
  });
  it('keys follow the buttons', () => {
    expect(liveKeys(claude())).toMatchObject({ u: true, r: true });
    expect(liveKeys(claude({}, { available: false }))).toMatchObject({ u: false, r: true });
    expect(liveKeys(claude({}, { state: 'updating' }))).toMatchObject({ u: false, r: false });
  });
});

describe('copy', () => {
  it('header, hints, compatibility and account', () => {
    expect(installedHeader(claude())).toMatchObject({ version: '2.1.317', meta: 'native installer · channel latest' });
    expect(installedHeader(claude({ problem_kind: 'not_found' })).version).toBe('—');
    expect(hint(claude({ install: 'homebrew' }))).toMatch(/^Homebrew installs update through brew/);
    expect(hint(claude({ install: 'unknown' }))).toMatch(/the way you installed it/);
    expect(hint(claude({ install: 'npm' }))).toBeNull();
    expect(account(claude())).toEqual({ login: 'Logged in · claude.ai · max', note: null });
    expect(account(claude({ logged_in: false }))?.login).toBe('Not logged in');
  });
  it('compat notes: too old (red), newer installed, newer latest (amber)', () => {
    expect(compat(claude()).supported).toBe('2.1.302 (minimum) – 2.1.317 (last tested)');
    expect(compat(claude()).notes).toEqual([
      {
        tone: 'amber',
        text: '2.1.330 is newer than the last tested version. Updating is fine: agents still run. Verify after updating with `make contract`.',
      },
    ]);
    expect(compat(claude({ version: '2.1.320' }, { available: false })).notes[0].text).toBe(
      '2.1.320 is newer than the last tested version. Agents still run.',
    );
    const o = compat(old());
    expect(o.notes[0]).toEqual({ tone: 'red', text: 'Too old. The app is read-only until Claude Code is updated.' });
    expect(o.notes[1].tone).toBe('amber');
  });
});

const W =
  'claude 2.1.323 is newer than the last tested version (2.1.302). Agents still run. To verify it, run `make contract` in an Orchestrator checkout (`docs/app/CONTRACT-TESTS.md`).';

describe('version warning helpers', () => {
  it('matches only the untested-version warning', () => {
    expect(isVersionWarning(W)).toBe(true);
    expect(isVersionWarning('claude 2.1.1 is older than the supported minimum (2.1.302)')).toBe(false);
    expect(isVersionWarning('something claude 2.1.323 is newer than the last tested version')).toBe(false);
    expect(isVersionWarning(null)).toBe(false);
  });
  it('codeSpans marks backticked text and drops the ticks', () => {
    expect(codeSpans('run `make contract` (`a/b.md`).')).toEqual([
      { text: 'run ', code: false },
      { text: 'make contract', code: true },
      { text: ' (', code: false },
      { text: 'a/b.md', code: true },
      { text: ').', code: false },
    ]);
  });
  it('verify row: both untested cases, from supported.verify with a fallback; hidden in range or too old', () => {
    const verify = { command: 'make contract', doc: 'docs/app/CONTRACT-TESTS.md' };
    const v = compat(claude({ version: '2.1.320' }, { available: false })).verify;
    expect(v).toMatchObject(verify);
    expect(v.line).toMatch(/a few cents of plan quota/);
    expect(compat(claude()).verify).toMatchObject(verify); // only the latest is newer
    const custom = claude(
      {
        version: '2.1.320',
        supported: { min: '2.1.302', tested_max: '2.1.317', verify: { command: 'just c', doc: 'd.md' } },
      },
      { available: false },
    );
    expect(compat(custom).verify).toMatchObject({ command: 'just c', doc: 'd.md' });
    expect(compat(claude({}, { available: false })).verify).toBeNull();
    expect(compat(old()).verify).toBeNull();
  });
});

describe('badge, chip, toasts, palette', () => {
  it('nav badge: spinner › error › amber › ↑ › none', () => {
    expect(navBadge(claude({}, { state: 'updating' }))?.kind).toBe('spinner');
    expect(navBadge(old())?.kind).toBe('error');
    expect(navBadge(claude({}, { state: 'error', error: 'x', error_from: 'update' }))?.kind).toBe('warn');
    expect(navBadge(claude({}, { state: 'error', error: 'x', error_from: 'check', available: false }))).toBeNull();
    expect(navBadge(claude())?.kind).toBe('up');
    expect(navBadge(claude({}, { available: false }))).toBeNull();
    expect(navBadge(null)).toBeNull();
  });
  it('chip: updating › failed › available; quiet for a check failure or a too-old banner', () => {
    expect(claudeChip(claude())).toMatchObject({ kind: 'available', version: '2.1.330', text: '↑ Claude Code' });
    expect(claudeChip(claude()).title).toBe(
      'Claude Code 2.1.330 is available (you run 2.1.317). Click to open Settings.',
    );
    expect(claudeChip(claude({}, { state: 'updating' }))?.text).toBe('Updating Claude Code…');
    expect(claudeChip(claude({}, { state: 'error', error: 'x', error_from: 'update' }))?.text).toBe(
      '⚠ Claude Code update',
    );
    expect(claudeChip(claude({}, { state: 'error', error: 'x', error_from: 'check', available: false }))).toBeNull();
    expect(claudeChip(old())).toBeNull();
    expect(claudeChip(null)).toBeNull();
  });
  it('toasts only for changes of this run', () => {
    const done = claude(
      { version: '2.1.330' },
      { available: false, updated: { from: '2.1.317', to: '2.1.330', at: 5 } },
    );
    expect(claudeToast(claude({}, { state: 'updating' }), done)).toEqual({
      text: 'Claude Code updated to 2.1.330.',
      ms: 3000,
    });
    expect(claudeToast(done, done)).toBeNull();
    expect(claudeToast(null, done)).toBeNull();
    const failed = claude({}, { state: 'error', error: 'x', error_from: 'update' });
    expect(claudeToast(claude({}, { state: 'updating' }), failed)).toMatchObject({
      text: 'Claude Code update failed.',
      kind: 'warn',
      sticky: true,
      details: true,
    });
    // Seen first while the check after the failed update ran, or not at all while updating.
    expect(claudeToast(claude({}, { state: 'checking' }), failed)?.text).toBe('Claude Code update failed.');
    expect(claudeToast(claude(), failed)?.text).toBe('Claude Code update failed.');
    expect(claudeToast(failed, failed)).toBeNull();
    expect(claudeToast(claude(), claude({}, { state: 'checking' }))).toBeNull();
  });
  it('palette hints', () => {
    expect(paletteActions(null)).toEqual({
      check: 'Claude Code is being checked',
      update: 'Claude Code is being checked',
    });
    expect(paletteActions(claude({ problem_kind: 'not_found' })).update).toBe('Claude Code not found');
    expect(paletteActions(claude({}, { state: 'updating' }))).toEqual({
      check: 'an update is running',
      update: 'an update is running',
    });
    expect(paletteActions(claude({}, { available: false }))).toEqual({ check: null, update: 'no update available' });
    // the same cases as the Update / Update anyway buttons
    expect(paletteActions(old({}, { available: false })).update).toBeNull();
    expect(
      paletteActions(claude({}, { state: 'error', error: 'x', error_from: 'check', available: false })).update,
    ).toBeNull();
    expect(paletteActions(claude())).toEqual({ check: null, update: null });
  });
  it('check outcome toasts', () => {
    expect(checkToast(claude())?.text).toBe('Claude Code 2.1.330 is available.');
    expect(checkToast(claude({}, { available: false }))?.text).toBe('Claude Code is up to date.');
    expect(checkToast(claude({}, { state: 'error', error: 'offline', error_from: 'check' }))).toMatchObject({
      kind: 'error',
    });
  });
  it('banner clause only for a version problem', () => {
    expect(bannerClaude(claude({ problem: 'x', problem_kind: 'login' }))).toBeNull();
    expect(bannerClaude(old())?.button.label).toBe('Update Claude Code');
    expect(bannerClaude(old({}, { state: 'updating' }))?.button).toMatchObject({ label: 'Updating…', disabled: true });
    const f = bannerClaude(old({}, { state: 'error', error: 'EACCES', error_from: 'update' }));
    expect(f).toMatchObject({ fail: 'Update failed: EACCES', details: true });
    expect(f.button.label).toBe('Retry update');
  });
});

describe('components', () => {
  beforeEach(() => {
    calls.length = 0;
    for (const k of Object.keys(replies)) delete replies[k];
    app.data = fixture();
    app.data.usage = null; // no limit banners in the way
    app.data.setup = { done: true, claude: claude() };
    app.toasts = [];
  });
  afterEach(cleanup);

  it('Settings: shows both versions and Update runs claude_update_run', async () => {
    replies.claude_update_run = { started: true };
    replies.app_state = stateWith(claude({}, { state: 'updating', started_at: 1 }));
    const { container, getByText } = render(ClaudeSettings);
    const t = text(container);
    expect(t).toContain('2.1.317');
    expect(t).toContain('native installer · channel latest');
    expect(t).toContain('~/.local/bin/claude'.replace('~', '')); // the path (home unknown in the fixture)
    expect(t).toContain('2.1.330 is available');
    expect(t).toContain('2.1.302 (minimum) – 2.1.317 (last tested)');
    expect(t).toContain('Logged in · claude.ai · max');
    expect(t).not.toContain('back to sections'); // the keys are in the buttons' tooltips
    await fireEvent.click(getByText(/Update to 2.1.330/));
    await waitFor(() => expect(calls.some(([c]) => c === 'claude_update_run')).toBe(true));
  });

  it('Settings: keys u and r', async () => {
    replies.claude_update_run = { started: true };
    replies.claude_update_check = claude();
    replies.app_state = stateWith(claude());
    const { container } = render(ClaudeSettings);
    const sec = container.querySelector('[data-section-focus]');
    await fireEvent.keyDown(sec, { key: 'r' });
    await waitFor(() => expect(calls.some(([c]) => c === 'claude_update_check')).toBe(true));
    await fireEvent.keyDown(sec, { key: 'u' });
    await waitFor(() => expect(calls.some(([c]) => c === 'claude_update_run')).toBe(true));
  });

  it('Settings: a failed update shows the error, Retry and the log', () => {
    app.data.setup.claude = claude(
      {},
      { state: 'error', error: 'exit 1', error_from: 'update', log: ['Checking…', 'Error: EACCES'] },
    );
    const { container } = render(ClaudeSettings);
    const t = text(container);
    expect(t).toContain('Update failed: exit 1');
    expect(t).toContain('Output of claude update');
    expect(t).toContain('Error: EACCES');
    expect(t).toContain('Retry');
  });

  it('Settings: the updated log is collapsed behind Show output', async () => {
    app.data.setup.claude = claude(
      { version: '2.1.330' },
      { available: false, updated: { from: '2.1.317', to: '2.1.330', at: 1 }, log: ['Done'] },
    );
    const { container, getByText } = render(ClaudeSettings);
    expect(text(container)).toContain('Updated 2.1.317 → 2.1.330.');
    expect(container.querySelector('.log')).toBeNull();
    await fireEvent.click(getByText('Show output'));
    expect(container.querySelector('.log')?.textContent).toContain('Done');
  });

  it('Settings: before the start-up check lands', () => {
    app.data.setup = { done: true, claude: null };
    const { container } = render(ClaudeSettings);
    expect(text(container)).toContain('Checking Claude Code…');
    expect(container.querySelectorAll('[data-update-block] button')).toHaveLength(0);
  });

  it('status bar: the chip opens nothing but Settings and carries the words Claude Code', () => {
    const { container } = render(StatusBar);
    const chip = container.querySelector('.chip.claude');
    expect(text(chip)).toBe('↑ Claude Code 2.1.330');
  });

  it('banner: Update Claude Code beside Set up for a version problem; none for a login problem', async () => {
    replies.claude_update_run = { started: true };
    replies.app_state = stateWith(claude());
    app.data.setup.claude = old();
    const { container, getByText } = render(Banner);
    expect(text(container)).toContain('Read-only:');
    expect(text(container)).toContain('Set up');
    await fireEvent.click(getByText('Update Claude Code'));
    await waitFor(() => expect(calls.some(([c]) => c === 'claude_update_run')).toBe(true));
    cleanup();
    app.data.setup.claude = claude({ problem: 'Not logged in', problem_kind: 'login' });
    expect(text(render(Banner).container)).not.toContain('Update Claude Code');
  });

  it('banner: after a failed update, Retry update, the clause and Details replace Set up', () => {
    app.data.setup.claude = old({}, { state: 'error', error: 'EACCES', error_from: 'update' });
    const { container } = render(Banner);
    const t = text(container);
    expect(t).toContain('Update failed: EACCES');
    expect(t).toContain('Retry update');
    expect(t).toContain('Details');
    expect(t).not.toContain('Set up');
  });

  it('banner: a dismissal does not hide a later failure', async () => {
    app.data.setup.claude = old();
    const { container, getByLabelText } = render(Banner);
    await fireEvent.click(getByLabelText('Dismiss'));
    expect(container.querySelector('.banner')).toBeNull();
    app.data.setup = { ...app.data.setup, claude: old({}, { state: 'error', error: 'EACCES', error_from: 'update' }) };
    await waitFor(() => expect(container.querySelector('.banner')).not.toBeNull());
  });

  it('banner: a dismissal survives background checks and returns with a failed update', async () => {
    app.data.setup.claude = old();
    const { container, getByLabelText } = render(Banner);
    await fireEvent.click(getByLabelText('Dismiss'));
    for (const state of ['checking', 'idle']) {
      app.data.setup = { ...app.data.setup, claude: old({}, { state }) };
      await tick();
      expect(container.querySelector('.banner')).toBeNull();
    }
    app.data.setup = { ...app.data.setup, claude: old({}, { state: 'error', error: 'EACCES', error_from: 'update' }) };
    await waitFor(() => expect(container.querySelector('.banner')).not.toBeNull());
  });

  it('banner: the untested-version warning shows mono chips and Details opens Claude Code settings', async () => {
    app.data.warnings = [W];
    const { container, getByText } = render(Banner);
    expect(text(container)).toContain(
      'Check: claude 2.1.323 is newer than the last tested version (2.1.302). Agents still run.',
    );
    expect([...container.querySelectorAll('.code')].map((e) => e.textContent)).toEqual([
      'make contract',
      'docs/app/CONTRACT-TESTS.md',
    ]);
    expect(text(container)).not.toContain('`');
    await fireEvent.click(getByText('Details'));
    expect(app.window).toMatchObject({ kind: 'settings', section: 'claude' });
  });

  it('banner: other warnings have no Details', () => {
    app.data.warnings = ['something else happened'];
    const { container } = render(Banner);
    expect(text(container)).toContain('something else happened');
    expect(text(container)).not.toContain('Details');
  });

  it('Settings: the To verify row copies the command', async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.assign(navigator, { clipboard: { writeText } });
    app.data.setup.claude = claude({ version: '2.1.320' }, { available: false });
    const { container } = render(ClaudeSettings);
    expect(text(container)).toContain('To verify make contract');
    expect(text(container)).toContain('(a few cents of plan quota). docs/app/CONTRACT-TESTS.md');
    await fireEvent.click(container.querySelector('[data-verify]'));
    expect(writeText).toHaveBeenCalledWith('make contract');
    await waitFor(() => expect(app.toasts.some((t) => t.text === 'Copied')).toBe(true));
    cleanup();
    app.data.setup.claude = claude({}, { available: false });
    expect(text(render(ClaudeSettings).container)).not.toContain('To verify');
  });

  it('Setup step 1: an untested version names the tested one and the command', () => {
    app.data.setup.claude = claude({ version: '2.1.320', untested: true });
    app.data.setup.trust = [];
    expect(text(render(SetupPane).container)).toContain(
      '2.1.320 (newer than the last tested version 2.1.317: agents still run; verify with make contract)',
    );
  });

  it('Setup step 1: the version problem gets the update state line', () => {
    app.data.setup.claude = old();
    app.data.setup.trust = [];
    const { container } = render(SetupPane);
    expect(container.querySelector('[data-update-block]')).not.toBeNull();
    expect(text(container)).toContain('2.1.330 is available');
  });

  it('a stale check reply does not overwrite a running update', async () => {
    app.data.setup.claude = claude({}, { state: 'checking' });
    replies.claude_update_check = () => {
      app.data.setup = { ...app.data.setup, claude: claude({}, { state: 'updating', started_at: 1 }) };
      return claude({}, { available: false });
    };
    await claudeCheck();
    expect(app.data.setup.claude.update.state).toBe('updating');
  });

  it('a second click while one command is in flight is dropped', async () => {
    const gate = { release: (_v) => {} };
    replies.claude_update_run = () => new Promise((r) => (gate.release = r));
    replies.app_state = stateWith(claude());
    const first = claudeUpdate();
    await claudeUpdate();
    expect(calls.filter(([c]) => c === 'claude_update_run')).toHaveLength(1);
    gate.release({ started: true });
    await first;
  });

  it('banner: a second failure after a retry is not covered by the first dismissal', async () => {
    const failed = () => old({}, { state: 'error', error: 'EACCES', error_from: 'update' });
    app.data.setup.claude = failed();
    const { container, getByLabelText } = render(Banner);
    await fireEvent.click(getByLabelText('Dismiss'));
    expect(container.querySelector('.banner')).toBeNull();
    app.data.setup = { ...app.data.setup, claude: old({}, { state: 'updating', started_at: 2 }) };
    await tick();
    app.data.setup = { ...app.data.setup, claude: failed() };
    await waitFor(() => expect(container.querySelector('.banner')).not.toBeNull());
  });

  it('palette: both commands, with their hints', () => {
    app.data.setup.claude = claude({}, { available: false });
    const list = commands().filter((c) => c.id.startsWith('claude.'));
    expect(list.map((c) => c.label)).toEqual(['Check for Claude Code updates', 'Update Claude Code']);
    expect(list.map((c) => c.disabled)).toEqual([null, 'no update available']);
  });
});
