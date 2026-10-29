import { describe, expect, it } from 'vitest';
import { duration, mb, pp, when } from '../src/lib/format.js';
import { filter, score } from '../src/lib/fuzzy.js';
import { decisionKind, LEVEL, queueReason, roleOf, runCost, STATUS } from '../src/lib/status.js';

describe('status vocabulary §9.5', () => {
  it('has one entry per status the core writes, not_started included', () => {
    for (const s of [
      'working',
      'waiting_for_you',
      'waiting_gate',
      'queued',
      'not_started',
      'paused',
      'blocked_by_limit',
      'idle',
      'suspended',
      'error',
      'done',
    ])
      expect(STATUS[s]?.label).toBeTruthy();
    expect(STATUS.not_started.label).toBe('Not started yet');
    expect(Object.keys(LEVEL)).toEqual(['needs_you', 'error', 'running', 'waiting', 'queued', 'idle', 'done']);
  });

  it('decision kinds have their own palette entry (a gate by its flavour)', () => {
    expect(decisionKind({ kind: 'permission' }).color).toBe('var(--color-decision-permission)');
    expect(decisionKind({ kind: 'gate', gate: 'pr' })).toMatchObject({
      label: 'PR gate',
      color: 'var(--color-decision-gate-pr)',
    });
    expect(decisionKind({ kind: 'error' }).color).toBe('var(--color-decision-error)');
    expect(decisionKind({ kind: 'mystery' }).color).toBe('var(--color-decision-alert)');
  });

  it('roles and queue reasons', () => {
    expect(roleOf('back-1').letter).toBe('B');
    expect(roleOf('fix-tax').letter).toBe('S');
    expect(queueReason({ reason: { kind: 'live_cap', cap: 1, state: 'red' } })).toBe('live-agent cap 1 (red)');
    expect(queueReason({ reason: { kind: 'memory_pressure', cap: 1 } })).toBe('memory pressure (1 live)');
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
