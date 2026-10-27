import { describe, expect, it } from 'vitest';
import { duration, mb, pp, when } from '../src/lib/format.js';
import { filter, score } from '../src/lib/fuzzy.js';
import { displayState, notStarted, queueReason, roleOf, runCost, STATUS } from '../src/lib/status.js';

describe('status vocabulary §9.5', () => {
  it('maps every runtime state onto the vocabulary', () => {
    for (const s of [
      'created',
      'starting',
      'working',
      'waiting_for_you',
      'waiting_gate',
      'queued',
      'blocked_by_limit',
      'idle',
      'suspended',
      'archived',
      'error',
      'killed',
    ]) {
      expect(STATUS[displayState({ state: s })]).toBeTruthy();
    }
    expect(displayState({ state: 'starting' })).toBe('working');
    expect(displayState({ state: 'killed' })).toBe('suspended');
    expect(displayState({ state: 'idle', queue_entry: { reason: { kind: 'paused_all' } } })).toBe('paused');
    expect(notStarted({ state: 'created' })).toBe(true);
    expect(Object.keys(STATUS)).toHaveLength(10);
  });

  it('roles and queue reasons', () => {
    expect(roleOf('back-1').letter).toBe('B');
    expect(roleOf('fix-tax').letter).toBe('S');
    expect(queueReason({ reason: { kind: 'live_cap', cap: 1, state: 'red' } })).toBe('live-agent cap 1 (red)');
    expect(queueReason({ reason: { kind: 'memory_pressure', cap: 1 } })).toBe('memory pressure (1 live)');
  });

  it('paused agents read Paused, except while the user has them working', () => {
    expect(displayState({ state: 'idle', queue_entry: { reason: { kind: 'paused_agent' } } })).toBe('paused');
    expect(displayState({ state: 'idle', paused: true })).toBe('paused');
    expect(displayState({ state: 'working', paused: true })).toBe('working');
  });

  it('Run now costs the window that holds the entry (FR-809)', () => {
    const weekly = { estimated_cost_pct: 1.25, estimated_cost_5h_pct: 6.2, binding: 'weekly' };
    expect(runCost(weekly)).toBe('+1.3 pp');
    expect(runCost(weekly, true)).toBe('+1.3 pp of the week (+6 % of the 5h window)');
    const five = { ...weekly, binding: 'five_hour' };
    expect(runCost(five)).toBe('+6 % 5h');
    expect(runCost(five, true)).toBe('+6 % of the 5h window (+1.3 pp of the week)');
    expect(runCost({ estimated_cost_pct: 0.5, estimated_cost_5h_pct: null, binding: 'five_hour' }, true)).toBe(
      '+0.5 pp',
    );
  });
});

describe('fuzzy and format', () => {
  it('ranks word starts and contiguous runs first', () => {
    expect(score('xyz', 'abc')).toBe(0);
    const items = [{ label: 'Toggle sidebar' }, { label: 'Split right' }, { label: 'Open Week planner' }];
    expect(filter('wp', items)[0].label).toBe('Open Week planner');
    expect(filter('sp', items)[0].label).toBe('Split right');
    expect(filter('', items)).toHaveLength(3);
  });

  it('formats like the copy rules', () => {
    expect(pp(9)).toBe('+9 pp');
    expect(pp(-3.2, 1)).toBe('−3.2 pp');
    expect(duration(72 * 60000)).toBe('1h 12m');
    expect(mb(1434)).toBe('1.4 GB');
    expect(when(Date.now() + 3600_000 * 2 + 60000)).toMatch(/\(in 2h 0[01]m\)/);
  });
});

describe('signed', () => {
  it('formats the title bar delta', async () => {
    const { signed } = await import('../src/lib/format.js');
    expect(signed(9)).toBe('+9');
    expect(signed(-2.6)).toBe('−3');
    expect(signed(null)).toBe('—');
  });
});
