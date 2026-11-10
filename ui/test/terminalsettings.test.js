// Settings › Terminal & editor (settings-brief §1.6, spec §3.1): the theme Select calls
// setTheme, the kitty-colours switch calls setKittyTheme, the Terminal group reads app.kitty
// (read-only) with Open kitty.conf, and a missing kitty.conf says so. No Renderer, Editor or
// Shells group is rendered.

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { fixture } from './fixture.js';
import { pick, shown } from './pick.js';

const calls = [];
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => (calls.push([cmd, args]), Promise.resolve(null)),
  listen: () => Promise.resolve(() => {}),
  errText: (e) => String(e?.message ?? e),
}));

const { default: TerminalSettings } = await import('../src/components/TerminalSettings.svelte');
const text = (el) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();

const KITTY = {
  sources: ['/Users/me/.config/kitty/kitty.conf'],
  options: { fontSize: 13, cursorStyle: 'block', cursorBlink: true },
  padding: [6, 6, 6, 6],
  opacity: 0.96,
  font_family: 'FiraCode Nerd Font Mono',
  unmapped: [
    ['foo', 'bar', 'x'],
    ['baz', '1', 'x'],
    ['qux', '2', 'x'],
  ],
};

beforeEach(() => {
  cleanup();
  calls.length = 0;
  app.data = fixture();
  app.theme = null;
  app.kittyTheme = false;
  app.kitty = KITTY;
  app.toasts = [];
});

describe('TerminalSettings', () => {
  it('Appearance: the theme Select calls setTheme, the switch calls setKittyTheme', async () => {
    const { container } = render(TerminalSettings);
    const sel = container.querySelector('[role="combobox"]');
    expect(shown(sel)).toBe('Follow the system');
    await pick(sel, 'Dark');
    expect(app.theme).toBe('dark');
    expect(app.kittyTheme).toBe(false);
    expect(text(container)).toContain('✓ Saved');
    const sw = container.querySelector('input[role="switch"]');
    await fireEvent.click(sw);
    expect(app.kittyTheme).toBe(true);
    expect(app.theme).toBeNull();
    // A Dark / Light choice ends the kitty-derived theme: the switch follows.
    await pick(container.querySelector('[role="combobox"]'), 'Light');
    expect(app.kittyTheme).toBe(false);
    expect(app.theme).toBe('light');
  });

  it('Terminal: read-only from the import, Open kitty.conf, unmapped list', async () => {
    const { container, getByText } = render(TerminalSettings);
    const t = text(container);
    expect(t).toContain('FiraCode Nerd Font Mono · 13 · from kitty.conf');
    expect(t).toContain('block · blink');
    expect(t).toContain('6 px · 0.96');
    expect(t).toContain('3 options not applied');
    expect(container.textContent).not.toContain('foo bar');
    await fireEvent.click(getByText('Show'));
    expect(text(container)).toContain('foo bar');
    expect(getByText('Open kitty.conf')).toBeTruthy();
  });

  it('no kitty.conf: says so, defaults, no Open kitty.conf', () => {
    app.kitty = { sources: [], options: {}, padding: [0, 0, 0, 0], opacity: 1, font_family: 'Menlo', unmapped: [] };
    const { container, queryByText } = render(TerminalSettings);
    expect(container.querySelector('[data-no-kitty]').textContent).toBe('No kitty.conf found. Defaults are used.');
    expect(text(container)).toContain('Menlo (default)');
    expect(queryByText('Open kitty.conf')).toBeNull();
  });

  it('renders none of the hidden groups', () => {
    const { container } = render(TerminalSettings);
    const t = text(container);
    for (const hidden of [
      'Renderer',
      'Editor command',
      'Reuse the open nvim',
      'New shell opens in',
      'Not designed yet',
    ])
      expect(t).not.toContain(hidden);
    expect([...container.querySelectorAll('.scard')].map((c) => /** @type {HTMLElement} */ (c).dataset.group)).toEqual([
      'appearance',
      'terminal',
    ]);
  });
});
