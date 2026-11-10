// Settings › Memory (settings-brief §1.8, spec §3.1, api-contract §3): the bar from the status
// bar's sample (one segment per agent when the sample has it), the pressure badge, Warn above
// (`ui.memory_warn_mb`, default 1600, 256–65 536 validated here) and the warning state.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
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
    return r instanceof Error ? Promise.reject(r) : Promise.resolve(typeof r === 'function' ? r(args) : r);
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
}));

const { default: MemorySettings, check } = await import('../src/components/MemorySettings.svelte');
const text = (el) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
/** The mount's reads are done (the threshold and the pressure). */
const loaded = () =>
  waitFor(() =>
    expect(calls.map(([c]) => c)).toEqual(expect.arrayContaining(['settings_get', 'pacing_settings'])),
  ).then(() => new Promise((r) => setTimeout(r, 0)));
const segs = (c) => [...c.querySelectorAll('.bar [data-seg]')].map((s) => s.dataset.seg);

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.data.memory = { app_mb: 412, agents_mb: 1360, shells_mb: 10, total_mb: 1782, system_mb: 8192 };
  app.data.agents[0].memory_mb = 980;
  app.data.agents[1].memory_mb = 380;
  replies.settings_get = null;
  replies.pacing_settings = { now: { pressure: 'normal', governor_cap: 2, live_autonomous: 1 } };
  replies.settings_set = null;
});

describe('validation', () => {
  it('256–65 536, whole numbers', () => {
    expect(check(1600)).toBe('');
    expect(check(256)).toBe('');
    expect(check(65536)).toBe('');
    for (const bad of [255, 65537, 1.5, NaN, 0]) expect(check(bad)).toBe('Between 256 and 65 536 MB');
  });
});

describe('MemorySettings', () => {
  it('Now: the total, a segment per agent with a sample, then shells; the badge from the pressure level', async () => {
    const { container } = render(MemorySettings);
    expect(text(container)).toContain('1.7 GB in use');
    expect(segs(container)).toEqual(['app', 'back-1', 'front-1', 'shells']);
    expect(container.querySelector('.bar').getAttribute('aria-label')).toMatch(
      /App 412 MB, back-1 980 MB, front-1 380 MB, Shells 10 MB/,
    );
    await waitFor(() =>
      expect(/** @type {any} */ (container.querySelector('[data-pressure]')).dataset.pressure).toBe('normal'),
    );
    expect(text(container.querySelector('[data-pressure]'))).toContain('Normal');
    expect(text(container)).toContain('live cap 2 · 1 live');
  });

  it('without per-agent samples: one agents segment', () => {
    for (const a of app.data.agents) a.memory_mb = 0;
    const { container } = render(MemorySettings);
    expect(segs(container)).toEqual(['app', 'agents', 'shells']);
  });

  it('warn state: above the threshold, the pressure level from the core', async () => {
    replies.settings_get = 1500;
    replies.pacing_settings = { now: { pressure: 'warn', governor_cap: 1, live_autonomous: 1 } };
    const { container } = render(MemorySettings);
    await waitFor(() => expect(text(container.querySelector('[data-warn]'))).toContain('above the warning (1.5 GB)'));
    await waitFor(() => expect(text(container.querySelector('[data-pressure]'))).toContain('Warn'));
    expect(container.querySelector('.bar').classList.contains('warn')).toBe(true);
  });

  it('no sample: not available', () => {
    app.data.memory = null;
    const { container } = render(MemorySettings);
    expect(container.querySelector('[data-unavailable]')).toBeTruthy();
  });

  it('Warn above: default 1600, saves a valid value to ui.memory_warn_mb on blur', async () => {
    const { container } = render(MemorySettings);
    const box = /** @type {HTMLInputElement} */ (container.querySelector('#mem-warn'));
    await loaded();
    expect(box.value).toBe('1600');
    await fireEvent.input(box, { target: { value: '2000' } });
    await fireEvent.blur(box);
    await waitFor(() => expect(calls).toContainEqual(['settings_set', { key: 'ui.memory_warn_mb', value: 2000 }]));
    await waitFor(() => expect(text(container)).toContain('✓ Saved'));
  });

  it('a value out of range says why and saves nothing', async () => {
    const { container } = render(MemorySettings);
    const box = /** @type {HTMLInputElement} */ (container.querySelector('#mem-warn'));
    await loaded();
    await fireEvent.input(box, { target: { value: '100' } });
    await fireEvent.blur(box);
    expect(container.querySelector('[role="alert"]').textContent).toBe('Between 256 and 65 536 MB');
    expect(box.getAttribute('aria-invalid')).toBe('true');
    expect(calls.some(([c]) => c === 'settings_set')).toBe(false);
  });

  it('renders none of the hidden rows', () => {
    const { container } = render(MemorySettings);
    expect(text(container)).not.toContain('Show in the status bar');
    expect(text(container)).not.toContain('Renderer');
    expect(text(container)).not.toContain('WebGL');
    expect([...container.querySelectorAll('.scard')].map((c) => /** @type {HTMLElement} */ (c).dataset.group)).toEqual([
      'now',
      'warning',
      'pressure',
    ]);
  });

  it('the Live agents cap link opens Pacing policy', async () => {
    app.window = { kind: 'settings', section: 'memory' };
    const { getByText } = render(MemorySettings);
    await fireEvent.click(getByText('Pacing policy › Live agents'));
    expect(app.window.section).toBe('pacing');
    app.window = null;
  });
});
