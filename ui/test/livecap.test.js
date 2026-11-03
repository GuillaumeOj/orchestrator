// Settings › Pacing policy (T-220): the derived caps, the field checks, the section against a mocked core.
import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { checkCap, derivedCaps } from '../src/lib/pacing.js';

const calls = [];
let reply;
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    const r = reply(cmd, args);
    return r instanceof Error ? Promise.reject(r.message) : Promise.resolve(r);
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
}));
const { default: PacingSettings } = await import('../src/components/PacingSettings.svelte');

const view = (o = {}) => ({
  live_cap: 3,
  live_cap_default: 3,
  live_cap_min: 1,
  live_cap_max: 20,
  off_hours_live_cap: 1,
  off_hours_live_cap_default: 1,
  off_hours_live_cap_min: 0,
  off_hours_live_cap_max: 20,
  off_hours_allowance_pct: 10,
  shadow: false,
  derived: derivedCaps(3),
  now: { state: 'amber', governor_cap: 2, live_autonomous: 1, pressure: 'normal', process_cap: 3 },
  ...o,
});

describe('derivedCaps', () => {
  it('follows the contract formulas', () => {
    expect(derivedCaps(3)).toEqual({
      green: 3,
      amber: 2,
      red: 1,
      catch_up: 5,
      memory_normal: 3,
      memory_warn: 2,
      memory_critical: 1,
    });
    expect(derivedCaps(1)).toMatchObject({ amber: 1, red: 1, catch_up: 3, memory_warn: 1 });
    expect(derivedCaps(5)).toMatchObject({ amber: 3, catch_up: 7, memory_warn: 4 });
    expect(derivedCaps(20)).toMatchObject({ amber: 10, memory_normal: 20 });
  });
});

describe('checkCap', () => {
  it('accepts integers in range and returns them', () => {
    expect(checkCap('6', 'live agents', 1, 20)).toBe(6);
    expect(checkCap('0', 'off-hours live agents', 0, 20)).toBe(0);
  });
  it('refuses with the core text', () => {
    expect(checkCap('0', 'live agents', 1, 20)).toEqual({ error: 'live agents 0: use 1–20' });
    expect(checkCap('25', 'off-hours live agents', 0, 20)).toEqual({ error: 'off-hours live agents 25: use 0–20' });
    expect(checkCap('', 'live agents', 1, 20)).toEqual({ error: 'live agents: use 1–20' });
    expect(checkCap('2.5', 'live agents', 1, 20)).toEqual({ error: 'live agents 2.5: use 1–20' });
  });
});

describe('PacingSettings', () => {
  beforeEach(() => {
    calls.length = 0;
    reply = (cmd) => (cmd === 'pacing_settings' ? view() : new Error('unexpected'));
  });
  afterEach(cleanup);

  const text = (c) => c.textContent.replace(/\s+/g, ' ');
  const input = (c, name) => c.querySelector(`input[data-field="${name}"]`);

  it('shows the saved values, the derived caps and the now line', async () => {
    const { container, getByText } = render(PacingSettings);
    await waitFor(() => getByText('Live agents'));
    expect(input(container, 'live').value).toBe('3');
    expect(input(container, 'off').value).toBe('1');
    const t = text(container);
    expect(t).toContain('Green 3');
    expect(t).toContain('Catch-up 5');
    expect(t).toContain('Memory: normal 3 · warn 2 · critical 1');
    expect(t).toContain('Now: ▲ Amber · 2 allowed · 1 live · memory normal, 3 processes');
    expect(getByText('Apply').closest('button').disabled).toBe(true);
  });

  it('recomputes the derived block while typing and marks the change', async () => {
    const { container, getByText } = render(PacingSettings);
    await waitFor(() => getByText('Live agents'));
    await fireEvent.input(input(container, 'live'), { target: { value: '5' } });
    const t = text(container);
    expect(t).toContain('Amber 3');
    expect(t).toContain('Catch-up 7');
    expect(t).toContain('was 3');
    expect(getByText('Apply').closest('button').disabled).toBe(false);
  });

  it('a field error shows the text, keeps the last valid numbers and disables Apply', async () => {
    const { container, getByText } = render(PacingSettings);
    await waitFor(() => getByText('Live agents'));
    await fireEvent.input(input(container, 'live'), { target: { value: '5' } });
    await fireEvent.input(input(container, 'live'), { target: { value: '0' } });
    expect(text(container)).toContain('live agents 0: use 1–20');
    expect(text(container)).toContain('Catch-up 7');
    expect(getByText('Apply').closest('button').disabled).toBe(true);
  });

  it('warns above 6 and when lowering below the live count', async () => {
    reply = (cmd) => (cmd === 'pacing_settings' ? view({ now: { ...view().now, live_autonomous: 3 } }) : null);
    const { container, getByText } = render(PacingSettings);
    await waitFor(() => getByText('Live agents'));
    await fireEvent.input(input(container, 'live'), { target: { value: '8' } });
    expect(text(container)).toContain('More than 6 agents can use 190–255 MB each');
    await fireEvent.input(input(container, 'live'), { target: { value: '2' } });
    expect(text(container)).toContain('Working agents finish their turn; new turns wait for a slot.');
  });

  it('applies with snake_case keys and shows the returned view', async () => {
    reply = (cmd, a) =>
      cmd === 'pacing_settings' ? view() : view({ live_cap: a.live_cap, derived: derivedCaps(a.live_cap) });
    const { container, getByText } = render(PacingSettings);
    await waitFor(() => getByText('Live agents'));
    await fireEvent.input(input(container, 'live'), { target: { value: '6' } });
    await fireEvent.keyDown(input(container, 'live'), { key: 'Enter' });
    await waitFor(() => getByText('Saved'));
    expect(calls.find((c) => c[0] === 'pacing_settings_set')[1]).toEqual({ live_cap: 6, off_hours_live_cap: 1 });
    expect(getByText('Apply').closest('button').disabled).toBe(true);
  });

  it('arrow keys step the value within the bounds', async () => {
    const { container, getByText } = render(PacingSettings);
    await waitFor(() => getByText('Live agents'));
    const f = input(container, 'live');
    await fireEvent.keyDown(f, { key: 'ArrowUp' });
    expect(f.value).toBe('4');
    f.value = '20';
    await fireEvent.input(f);
    await fireEvent.keyDown(f, { key: 'ArrowUp' });
    expect(f.value).toBe('20');
    const o = input(container, 'off');
    await fireEvent.keyDown(o, { key: 'ArrowDown' });
    await fireEvent.keyDown(o, { key: 'ArrowDown' });
    expect(o.value).toBe('0');
  });

  it('a refused save keeps the typed values and shows the core text', async () => {
    reply = (cmd) => (cmd === 'pacing_settings' ? view() : new Error('agents are not running'));
    const { container, getByText } = render(PacingSettings);
    await waitFor(() => getByText('Live agents'));
    await fireEvent.input(input(container, 'live'), { target: { value: '6' } });
    await fireEvent.click(getByText('Apply'));
    await waitFor(() => getByText('Could not save: agents are not running'));
    expect(input(container, 'live').value).toBe('6');
    expect(getByText('Apply').closest('button').disabled).toBe(false);
  });

  it('notes shadow mode and a plan without off-hours allowance', async () => {
    reply = () => view({ shadow: true, off_hours_allowance_pct: 0 });
    const { container, getByText } = render(PacingSettings);
    await waitFor(() => getByText('Live agents'));
    expect(text(container)).toContain(
      'Shadow mode is on: the Governor logs the cap without enforcing it. The memory cap still applies.',
    );
    expect(text(container)).toContain('logged only');
    expect(text(container)).toContain("The plan has no off-hours allowance, so agents don't run off-hours.");
  });

  it('a load error shows Retry, and r retries', async () => {
    let n = 0;
    reply = () => (n++ ? view() : new Error('agents are not running'));
    const { container, getByText } = render(PacingSettings);
    await waitFor(() => getByText('Could not read the pacing settings: agents are not running'));
    await fireEvent.keyDown(container.querySelector('[data-section-focus]'), { key: 'r' });
    await waitFor(() => getByText('Live agents'));
  });

  it('Reset sets the defaults without saving', async () => {
    reply = () => view({ live_cap: 6, off_hours_live_cap: 2 });
    const { container, getByText } = render(PacingSettings);
    await waitFor(() => getByText('Live agents'));
    await fireEvent.click(getByText('Reset to defaults'));
    expect(input(container, 'live').value).toBe('3');
    expect(input(container, 'off').value).toBe('1');
    expect(calls.some((c) => c[0] === 'pacing_settings_set')).toBe(false);
  });
});
