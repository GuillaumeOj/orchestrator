// A dock shell's live title (design-notes §6): OSC 7, the `term_cwd` poll, a
// path-like OSC 0/2 title, the opening path; left truncation; fixed titles.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fixture } from './fixture.js';

const h = vi.hoisted(() => ({ cwd: new Map(), calls: [], saves: 0 }));
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  errText: (e) => String(e),
  listen: () => Promise.resolve(() => {}),
  invoke: (cmd, args) => {
    if (cmd === 'term_cwd') {
      h.calls.push(args.id);
      return Promise.resolve(h.cwd.get(args.id) ?? null);
    }
    if (cmd === 'settings_set') h.saves++;
    if (cmd === 'term_spawn') return Promise.resolve({ id: 99, cwd: args.req.cwd, shell: 'zsh' });
    return Promise.resolve(null);
  },
}));

const S = await import('../src/lib/shellcwd.js');
const A = await import('../src/lib/app.svelte.js');
const { app } = A;

describe('shellcwd helpers', () => {
  it('parseOsc7 decodes the path and reports the host', () => {
    expect(S.parseOsc7('file://mac.local/Users/me/my%20dir')).toEqual({ host: 'mac.local', path: '/Users/me/my dir' });
    expect(S.parseOsc7('file:///tmp')).toEqual({ host: '', path: '/tmp' });
    expect(S.parseOsc7('http://x/y')).toBe(null);
    expect(S.parseOsc7('nonsense')).toBe(null);
    expect(S.parseOsc7('file://h/%E0%A4%A')).toBe(null);
  });

  it('a foreign host is not local', () => {
    expect(S.isLocalHost('', [])).toBe(true);
    expect(S.isLocalHost('localhost', [])).toBe(true);
    expect(S.isLocalHost('mac.local', ['mac.local'])).toBe(true);
    expect(S.isLocalHost('prod-1', ['mac.local'])).toBe(false);
  });

  it('only a path-like OSC 0/2 title counts', () => {
    expect(S.looksLikePath('~/dev')).toBe(true);
    expect(S.looksLikePath('/tmp')).toBe(true);
    expect(S.looksLikePath('me@mac:~/dev')).toBe(false);
    expect(S.looksLikePath('zsh')).toBe(false);
    expect(S.looksLikePath('nvim README.md')).toBe(false);
  });

  it('the title is `<shell> · <cwd>`, home as ~, cut from the left at ~32 characters', () => {
    expect(S.liveTitle('zsh', '/Users/me/dev/orch/ui/src/components', '/Users/me')).toBe(
      'zsh · ~/dev/orch/ui/src/components',
    );
    expect(S.liveTitle(undefined, '/tmp', '/Users/me')).toBe('zsh · /tmp');
    expect(S.liveTitle('zsh', '/Users/me', '/Users/me')).toBe('zsh · ~');
    const long = S.shownCwd('/Users/me/dev/orchestrator.worktrees/improve-orchestrator-ui-ux/ui-1', '/Users/me');
    expect(long).toHaveLength(S.TITLE_MAX);
    expect(long.startsWith('…')).toBe(true);
    expect(long.endsWith('improve-orchestrator-ui-ux/ui-1')).toBe(true);
  });

  it('the tooltip is two lines: the title and the full path', () => {
    expect(S.shellTip('zsh · ~/dev', '/Users/me/dev')).toEqual({ label: 'zsh · ~/dev', detail: '/Users/me/dev' });
    expect(S.shellTip('gh auth login', null)).toBe('gh auth login');
  });
});

describe('live shells in the app', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    h.cwd = new Map();
    h.calls = [];
    h.saves = 0;
    const d = fixture();
    d.core = /** @type {any} */ ({ user_home: '/Users/me' });
    app.data = d;
    app.panes = {
      p1: { type: 'shell', termId: 11, shell: 'zsh', live: true, cwd: '/Users/me/dev', title: 'zsh · ~/dev' },
      p2: { type: 'shell', termId: 12, title: 'gh auth login', cwd: '/Users/me' },
    };
    app.layout = null;
    app.dock = {
      open: true,
      height: 0.45,
      tabs: ['p1', 'p2'],
      active: 'p1',
      v: 2,
      view: 'shells',
      agentProject: 'all',
      activeAgent: null,
      activeShell: 'p1',
    };
    app.focused = 'p1';
    Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
  });
  afterEach(() => {
    for (const id of [...app.dock.tabs]) A.closePane(id); // forgets what each shell reported
    app.panes = {};
    vi.advanceTimersByTime(S.POLL_MS * 2); // the poll stops itself
    vi.useRealTimers();
  });

  it('OSC 7 sets the cwd and the title at once, and ends the poll for that shell', async () => {
    A.onShellOsc7('p1', 'file:///Users/me/dev/orch/src');
    expect(app.panes.p1.cwd).toBe('/Users/me/dev/orch/src');
    expect(app.panes.p1.title).toBe('zsh · ~/dev/orch/src');
    h.cwd.set(11, '/elsewhere');
    await A.readShellCwd('p1');
    expect(h.calls).toEqual([]);
    expect(app.panes.p1.cwd).toBe('/Users/me/dev/orch/src');
  });

  it('ignores the OSC 7 of an ssh session (a host other than the first one the shell reported)', () => {
    A.onShellOsc7('p1', 'file://mac.local/Users/me/a');
    expect(app.panes.p1.cwd).toBe('/Users/me/a');
    A.onShellOsc7('p1', 'file://prod-1/srv/app');
    expect(app.panes.p1.cwd).toBe('/Users/me/a');
    A.onShellOsc7('p1', 'file://mac.local/Users/me/b');
    expect(app.panes.p1.cwd).toBe('/Users/me/b');
  });

  it('polls term_cwd every 1.5 s, updates the title only on change', async () => {
    A.trackShells();
    h.cwd.set(11, '/Users/me/dev/orch');
    await vi.advanceTimersByTimeAsync(S.POLL_MS);
    expect(h.calls).toEqual([11]); // the command shell (no `live`) is never read
    expect(app.panes.p1.title).toBe('zsh · ~/dev/orch');
    await vi.advanceTimersByTimeAsync(400); // the throttled save
    const saves = h.saves;
    await vi.advanceTimersByTimeAsync(S.POLL_MS);
    expect(h.calls).toEqual([11, 11]);
    expect(h.saves).toBe(saves); // unchanged: no write
    h.cwd.set(11, '/Users/me/dev/orch/ui');
    await vi.advanceTimersByTimeAsync(S.POLL_MS + 400);
    expect(app.panes.p1.title).toBe('zsh · ~/dev/orch/ui');
    expect(h.saves).toBeGreaterThan(saves);
  });

  it('does not poll while the window is hidden, reads once when it shows', async () => {
    A.trackShells();
    h.cwd.set(11, '/Users/me/dev/x');
    Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
    await vi.advanceTimersByTimeAsync(S.POLL_MS * 3);
    expect(h.calls).toEqual([]);
    Object.defineProperty(document, 'visibilityState', { value: 'visible', configurable: true });
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(0);
    expect(h.calls).toEqual([11]);
    expect(app.panes.p1.cwd).toBe('/Users/me/dev/x');
  });

  it('one read on focus', async () => {
    h.cwd.set(11, '/Users/me/focus');
    A.focusPane('p1');
    await vi.advanceTimersByTimeAsync(0);
    expect(h.calls).toEqual([11]);
    expect(app.panes.p1.title).toBe('zsh · ~/focus');
  });

  it('stops polling a closed shell', async () => {
    A.trackShells();
    A.closePane('p1');
    await vi.advanceTimersByTimeAsync(S.POLL_MS * 2);
    expect(h.calls).toEqual([]);
  });

  it('a path-like OSC 0/2 title is the fallback; not after the core answered', async () => {
    A.onShellTitle('p1', 'me@mac:~/dev');
    expect(app.panes.p1.cwd).toBe('/Users/me/dev');
    A.onShellTitle('p1', '~/dev/ui');
    expect(app.panes.p1.cwd).toBe('/Users/me/dev/ui');
    A.onShellTitle('p1', 'zsh');
    expect(app.panes.p1.cwd).toBe('/Users/me/dev/ui');
    h.cwd.set(11, '/Users/me/real');
    await A.readShellCwd('p1');
    A.onShellTitle('p1', '/tmp');
    expect(app.panes.p1.cwd).toBe('/Users/me/real');
  });

  it('command shells keep their fixed title whatever the sources say', async () => {
    A.onShellOsc7('p2', 'file:///tmp');
    A.onShellTitle('p2', '/tmp');
    h.cwd.set(12, '/tmp');
    await A.readShellCwd('p2');
    expect(app.panes.p2.title).toBe('gh auth login');
    expect(app.panes.p2.cwd).toBe('/Users/me');
  });

  it('a restored or new interactive shell is live, with the shell the core spawned', async () => {
    await A.openShell({ cwd: '/Users/me/dev' });
    const id = app.dock.active;
    expect(app.panes[id].live).toBe(true);
    expect(app.panes[id].title).toMatch(/^zsh · /);
  });
});
