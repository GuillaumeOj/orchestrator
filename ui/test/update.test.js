import { render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it } from 'vitest';
import StatusBar from '../src/components/StatusBar.svelte';
import { aboutOrchestrator, app, commands } from '../src/lib/app.svelte.js';
import { aboutText, checkOutcome, readyToast, updateActions, updateChip } from '../src/lib/update.js';
import { fixture } from './fixture.js';

const upd = (o = {}) => ({
  current: { version: '0.1.41', sha: 'bd5797c0000', channel: 'main' },
  enabled: true,
  disabled_why: null,
  state: 'idle',
  available: null,
  progress: null,
  error: null,
  checked_at: 1793600123,
  apply_on_quit: false,
  ...o,
});
const avail = { version: '0.1.42', sha: 'f9b1e20', published_at: 1793600000, subject: 'x' };

describe('update chip', () => {
  it('says nothing when idle, checking or disabled', () => {
    expect(updateChip(upd())).toBeNull();
    expect(updateChip(upd({ state: 'checking' }))).toBeNull();
    expect(
      updateChip(upd({ enabled: false, disabled_why: 'debug build', state: 'ready', available: avail })),
    ).toBeNull();
    expect(updateChip(undefined)).toBeNull();
  });
  it('downloading shows the version, and the percentage when known', () => {
    expect(updateChip(upd({ state: 'downloading', available: avail })).text).toBe('↓ 0.1.42');
    const c = updateChip(upd({ state: 'downloading', available: avail, progress: 0.4 }));
    expect(c.text).toBe('↓ 0.1.42 · 40 %');
    expect(c.title).toBe('Downloading Orchestrator 0.1.42');
  });
  it('ready says when it installs', () => {
    expect(updateChip(upd({ state: 'ready', available: avail, apply_on_quit: true })).title).toContain(
      'Installs when you quit',
    );
    const local = updateChip(upd({ state: 'ready', available: avail }));
    expect(local.text).toBe('⟳ 0.1.42 · Restart');
    expect(local.title).toContain('local');
  });
  it('error shows the reason in the tooltip', () => {
    const c = updateChip(upd({ state: 'error', error: 'offline' }));
    expect(c.text).toBe('⚠ update');
    expect(c.title).toMatch(/^offline · last checked /);
  });
});

describe('update copy', () => {
  it('toasts a staged version once', () => {
    const u = upd({ state: 'ready', available: avail, apply_on_quit: true });
    expect(readyToast(u, null)).toBe('Orchestrator 0.1.42 is ready. It installs when you quit.');
    expect(readyToast(u, '0.1.42')).toBeNull();
    expect(readyToast(upd({ state: 'ready', available: avail }), '0.1.41')).toContain('local build');
    expect(readyToast(upd(), null)).toBeNull();
  });
  it('palette enabled state follows enabled and ready', () => {
    expect(updateActions(upd())).toEqual({ check: null, restart: 'no update ready' });
    expect(updateActions(upd({ enabled: false, disabled_why: 'gh not logged in' })).check).toBe('gh not logged in');
    expect(updateActions(upd({ state: 'ready', available: avail })).restart).toBeNull();
  });
  it('check outcomes and about', () => {
    expect(checkOutcome(upd())).toBe('Orchestrator is up to date (0.1.41)');
    expect(checkOutcome(upd({ state: 'ready', available: avail }))).toBe('Orchestrator 0.1.42 is ready');
    expect(checkOutcome(upd({ state: 'error', error: 'offline' }))).toMatchObject({
      error: expect.stringContaining('offline'),
    });
    expect(checkOutcome(upd({ state: 'checking' }))).toBeNull();
    expect(aboutText(upd())).toMatch(/^Orchestrator 0\.1\.41 · bd5797c · main · checked /);
    expect(aboutText(upd({ current: { version: '0.1.0', sha: '', channel: 'local' } }))).toContain(
      '0.1.0 · local build',
    );
  });
});

describe('status bar and palette', () => {
  beforeEach(() => {
    app.data = fixture();
    app.toasts = [];
  });
  const text = (c) => c.textContent.replace(/\s+/g, ' ');

  it('shows the item for each state', async () => {
    const { container } = render(StatusBar);
    const show = async (u) => {
      app.data = { ...app.data, update: u };
      await tick();
    };
    await show(upd());
    expect(container.querySelector('.upd')).toBeNull();
    await show(upd({ state: 'downloading', available: avail }));
    expect(text(container)).toContain('↓ 0.1.42');
    expect(container.querySelector('button.upd')).toBeNull();
    await show(upd({ state: 'ready', available: avail, apply_on_quit: true }));
    expect(container.querySelector('button.upd.ready')?.textContent).toContain('⟳ 0.1.42 · Restart');
    await show(upd({ state: 'error', error: 'offline' }));
    expect(container.querySelector('button.upd.warn')?.textContent).toContain('⚠ update');
  });

  it('palette entries are enabled per state', () => {
    app.data.update = upd();
    const by = () =>
      Object.fromEntries(
        commands()
          .filter((c) => c.id.startsWith('update.'))
          .map((c) => [c.id, c]),
      );
    expect(by()['update.check'].disabled).toBeNull();
    expect(by()['update.restart'].disabled).toBe('no update ready');
    app.data.update = upd({ enabled: false, disabled_why: 'debug build' });
    expect(by()['update.check'].disabled).toBe('debug build');
    app.data.update = upd({ state: 'ready', available: avail });
    expect(by()['update.restart'].disabled).toBeNull();
  });

  it('About toasts the build', () => {
    app.data.update = upd();
    aboutOrchestrator();
    expect(app.toasts[0].text).toContain('Orchestrator 0.1.41');
  });
});
