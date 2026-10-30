// lib/mission.js (P4.2 spec §3.2, §5): the lanes from `now`, the stepper,
// the chips' short waits, the queue copy, the filter and the key hints.

import { describe, expect, it } from 'vitest';
import {
  defaultTarget,
  filterLabel,
  footerText,
  hiddenText,
  hints,
  lanes,
  phaseFull,
  phaseText,
  rowCount,
  runNowCost,
  runNowDialog,
  shortWait,
  stepper,
  stops,
  target,
  tasksDetail,
  tasksText,
  toggleProject,
} from '../src/lib/mission.js';
import { decision, rec } from './fixture.js';
import { checkoutRun, MIN, missionNow, today } from './mission-fixture.js';

const NOW = Date.now();
const ids = (lane) => lane.rows.map((r) => r.key);
const laneOf = (m, id) => m.lanes.find((l) => l.id === id);
/** A `now` with these solo records (and runs). */
const world = (agents, runs = []) => ({
  agents: Object.fromEntries(agents.map((r) => [r.agent, r])),
  runs: Object.fromEntries(runs.map((r) => [r.run, r])),
  decisions: {},
  outcomes: [],
});
const solo = (id, status, more = {}) => rec(id, status, { project: 'p', ...more });

describe('lanes(): the design fixture', () => {
  const m = lanes(missionNow(NOW), null, NOW);

  it('shows Needs you, Running, Queued and Done today; empty lanes are hidden (D11)', () => {
    expect(m.lanes.map((l) => l.header)).toEqual([
      'Needs you · 1 run',
      'Running · 1 session',
      'Queued · 2 agents',
      'Done today · 4 outcomes',
    ]);
    expect(m.idle).toBeNull();
  });

  it('a team run is one row in its level lane; members are not rows of their own', () => {
    expect(ids(laneOf(m, 'needs_you'))).toEqual(['run:run-12']);
    expect(ids(laneOf(m, 'running'))).toEqual(['agent:fix-feed']);
  });

  it('the Queued lane is the whole queue: a run member too, in the Governor order then not started', () => {
    expect(ids(laneOf(m, 'queued'))).toEqual(['queue:infra-1', 'queue:ui-1']);
    expect(laneOf(m, 'queued').rows[1].run.feature).toBe('checkout');
  });

  it('Done today: seen and unseen, newest first', () => {
    expect(laneOf(m, 'done').rows.map((r) => r.outcome.id)).toEqual(['ev-13', 'ev-12', 'ev-11', 'ev-10']);
  });
});

describe('lanes(): levels (D3)', () => {
  it.each([
    ['needs_you', 'waiting_for_you', 'needs_you'],
    ['error', 'error', 'needs_you'],
    ['running', 'working', 'running'],
    ['waiting', 'blocked_by_limit', 'waiting'],
    ['queued', 'queued', 'queued'],
  ])('a solo agent at %s is a row in its lane', (level, status, lane) => {
    const m = lanes(world([solo('a', status, { level })]), null, NOW);
    expect(laneOf(m, lane).rows).toHaveLength(1);
  });

  it('idle (suspended too) is the Idle line, done is no row', () => {
    const m = lanes(
      world([solo('a', 'idle'), solo('b', 'suspended', { level: 'idle' }), solo('c', 'done')]),
      null,
      NOW,
    );
    expect(m.idle.header).toBe('Idle · 2 sessions');
    expect(m.lanes.map((l) => l.id)).toEqual(['needs_you']);
    expect(laneOf(m, 'needs_you').header).toBe('Needs you');
  });

  it('errors come after the blocking rows in Needs you, whatever their age', () => {
    const m = lanes(
      world([
        solo('err', 'error', { since_ms: NOW - 60 * MIN }),
        solo('new', 'waiting_for_you', { since_ms: NOW - MIN }),
        solo('old', 'waiting_for_you', { since_ms: NOW - 9 * MIN }),
      ]),
      null,
      NOW,
    );
    expect(ids(laneOf(m, 'needs_you'))).toEqual(['agent:old', 'agent:new', 'agent:err']);
  });

  it('a run in phase done is not a row; a run at error is in Needs you', () => {
    const m = lanes(
      world(
        [],
        [
          checkoutRun(NOW, { run: 'r1', phase: 'done', level: 'done' }),
          checkoutRun(NOW, { run: 'r2', level: 'error' }),
        ],
      ),
      null,
      NOW,
    );
    expect(ids(laneOf(m, 'needs_you'))).toEqual(['run:r2']);
  });

  it('headers count rows with their unit, mixed (D11)', () => {
    const m = lanes(world([solo('a', 'waiting_for_you')], [checkoutRun(NOW)]), null, NOW);
    expect(laneOf(m, 'needs_you').header).toBe('Needs you · 1 run · 1 session');
    expect(rowCount([{ kind: 'queue' }])).toBe('1 agent');
  });

  it('queue order: position, then not started, then paused', () => {
    const q = (position) => ({ entry: 1, position, tier: 'normal', run_now: { binds: 'week' } });
    const m = lanes(
      world([
        solo('paused', 'paused'),
        solo('fresh', 'not_started'),
        solo('second', 'queued', { queue: q(2) }),
        solo('first', 'queued', { queue: q(1) }),
      ]),
      null,
      NOW,
    );
    expect(ids(laneOf(m, 'queued'))).toEqual(['queue:first', 'queue:second', 'queue:fresh', 'queue:paused']);
  });

  it('open decisions with no session make one line in Needs you', () => {
    const n = world([solo('a', 'working')]);
    n.decisions = {
      d1: decision({ id: 'd1', created_ms: NOW - 5 * MIN }),
      d2: decision({ id: 'd2', created_ms: NOW - 9 * MIN, subject: { agent: 'gone-1' } }),
      d3: decision({ id: 'd3', created_ms: NOW - 7 * MIN, subject: { agent: 'a' } }),
      d4: decision({ id: 'd4', lifecycle: { state: 'effect' } }),
    };
    const line = laneOf(lanes(n, null, NOW), 'needs_you').rows.at(-1);
    // a's row is in Running: its decision is loose too (review: no Needs-you row stands for it).
    expect(line).toMatchObject({ kind: 'decisions', ids: ['d2', 'd3', 'd1'], count: 3 });
  });

  it('a decision is not loose while its agent, or its run, has a Needs-you row', () => {
    const n = missionNow(NOW);
    n.decisions.q = decision({ id: 'q', subject: { agent: 'front-1' } });
    expect(laneOf(lanes(n, null, NOW), 'needs_you').rows.map((r) => r.kind)).toEqual(['run']);
  });

  it('a decision with no project shows under any filter', () => {
    const n = world([]);
    n.decisions = { d: decision({ id: 'd' }), e: decision({ id: 'e', subject: { project: 'other' } }) };
    expect(laneOf(lanes(n, ['p'], NOW), 'needs_you').rows[0].ids).toEqual(['d']);
  });

  it('members of a done run are solo rows (review: they vanished)', () => {
    const n = world(
      [solo('m1', 'working', { run: 'r1' }), solo('m2', 'queued', { run: 'r1' })],
      [checkoutRun(NOW, { run: 'r1', phase: 'done', level: 'done', members: ['m1', 'm2'] })],
    );
    const m = lanes(n, null, NOW);
    expect(ids(laneOf(m, 'running'))).toEqual(['agent:m1']);
    expect(laneOf(m, 'queued').rows[0]).toMatchObject({ key: 'queue:m2', run: null });
  });

  it('outcomes of another day are not in Done today', () => {
    const n = world([]);
    n.outcomes = [
      { id: 'a', kind: 'task_done', at_ms: today(9, 0), text: 'a', seen: true },
      { id: 'b', kind: 'task_done', at_ms: today(0, 0) - 60000, text: 'b', seen: false },
    ];
    expect(ids(laneOf(lanes(n, null, NOW), 'done'))).toEqual(['outcome:a']);
  });
});

describe('lanes(): project filter (D8)', () => {
  it('one project', () => {
    const m = lanes(missionNow(NOW), ['blog'], NOW);
    expect(m.lanes.map((l) => l.header)).toEqual(['Needs you', 'Running · 1 session', 'Done today · 1 outcome']);
  });

  it('several projects', () => {
    const m = lanes(missionNow(NOW), ['shop-api', 'infra'], NOW);
    expect(m.lanes.map((l) => l.id)).toEqual(['needs_you', 'queued', 'done']);
    expect(ids(laneOf(m, 'queued'))).toEqual(['queue:infra-1', 'queue:ui-1']);
  });

  it('label, toggle (the last project stays), counts summed over the filter', () => {
    const ps = [
      { id: 'shop-api', name: 'shop-api' },
      { id: 'blog', name: 'blog' },
      { id: 'infra', name: 'infra' },
    ];
    expect(filterLabel(null, ps)).toBe('All projects');
    expect(filterLabel(['blog'], ps)).toBe('blog');
    expect(filterLabel(['blog', 'infra'], ps)).toBe('2 projects of 3');
    const all = ps.map((p) => p.id);
    expect(toggleProject(null, 'shop-api', all)).toEqual(['blog', 'infra']);
    expect(toggleProject(['blog'], 'blog', all)).toEqual(['blog']);
    expect(toggleProject(['blog', 'infra'], 'shop-api', all)).toBeNull();
    const c = missionNow(NOW).counts;
    expect(footerText(c, null, ps)).toBe('1 needs you · 2 running · 1 waiting · 2 queued · live cap 3/2');
    expect(footerText(c, ['shop-api', 'blog'], ps)).toBe(
      '1 needs you · 2 running · 1 waiting · 1 queued · live cap 3/2 · 2 projects of 3',
    );
    // The cap is global: under any filter it reads the global live / cap, never a per-project sum.
    expect(footerText(c, ['blog'], ps)).toBe('1 running · live cap 3/2');
    expect(footerText({ ...c, global: { ...c.global, live: 1 } }, ['shop-api', 'infra'], ps)).toBe(
      '1 needs you · 1 running · 1 waiting · 2 queued · live cap 1/2 · 2 projects of 3',
    );
    expect(footerText(c, null, [])).toBe('No projects');
    expect(hiddenText(c, ['blog', 'infra'], ps)).toBe(
      'Hiding shop-api · 1 needs you · 1 running · 1 waiting · 1 queued',
    );
  });
});

describe('stops (j/k)', () => {
  it('rows of open lanes, one stop per collapsed lane, Show all past 10, the Idle line', () => {
    const n = world([solo('a', 'working'), solo('b', 'working'), solo('z', 'idle')]);
    n.outcomes = Array.from({ length: 12 }, (_, i) => ({
      id: `o${i}`,
      kind: 'task_done',
      at_ms: today(0, i),
      text: `${i}`,
      seen: true,
    }));
    const m = lanes(n, null, NOW);
    const s = stops(m, { collapsed: { running: true } });
    expect(s.slice(0, 2)).toEqual(['lane:running', 'outcome:o11']);
    expect(s.slice(-2)).toEqual(['showall', 'lane:idle']);
    expect(s.filter((k) => k.startsWith('outcome:'))).toHaveLength(10);
    expect(stops(m, { showAll: true, idleOpen: true }).slice(-2)).toEqual(['lane:idle', 'agent:z']);
  });
});

describe('phase stepper (notes §1.2, D12)', () => {
  const states = (s) => s.steps.map((x) => x.state[0]).join('');

  it.each([
    ['executing', undefined, 'ddd' + 'ctt', null, 'phase 4 of 6: executing'],
    ['spec_gate', undefined, 'dcttt' + 't', 'at gate', 'phase 2 of 6: spec gate'],
    ['pr_gate', undefined, 'dddddc', 'at gate', 'phase 6 of 6: PR gate'],
    ['pr_open', undefined, 'dddddc', null, 'phase 6 of 6: PR'],
    ['paused', 'executing', 'dddctt', 'paused', 'phase 4 of 6: executing, paused'],
    ['failed', 'integrating', 'ddddct', 'failed', 'phase 5 of 6: integrating, failed'],
    ['paused', undefined, 'tttttt', 'paused', 'phase: paused'],
  ])('%s (held in %s)', (phase, held_in, want, mark, aria) => {
    const s = stepper({ phase, held_in });
    expect(states(s)).toBe(want);
    expect(s.mark).toBe(mark);
    expect(s.aria).toBe(aria);
  });

  it('phase text: short on the row, full in the tooltip', () => {
    const run = { phase: 'paused', held_in: 'executing', phase_since_ms: NOW - 40 * MIN };
    expect(phaseText(run, NOW)).toBe('paused · 40 min');
    expect(phaseFull(run, NOW)).toBe('paused · executing · 40 min');
    expect(phaseText({ phase: 'executing', phase_since_ms: NOW - 120 * MIN }, NOW)).toBe('executing · 2 h');
    expect(phaseText({ phase: 'deferred', held_in: 'planning' }, NOW)).toBe('deferred');
    expect(phaseText({ phase: 'deferred', until_ms: NOW + 86400000 }, NOW)).toMatch(
      /^deferred · until \w{3} \d\d:\d\d$/,
    );
  });

  it('tasks', () => {
    expect(tasksText({ total: 7, done: 3 })).toBe('3/7 tasks done');
    expect(tasksText({ total: 0 })).toBe('no tasks yet');
    expect(tasksDetail({ total: 7, done: 3, in_progress: 2, review: 1, blocked: 1 })).toBe(
      '7 tasks · 3 done · 2 in progress · 1 in review · 1 blocked',
    );
    expect(tasksDetail({ total: 2, done: 2, in_progress: 0, review: 0, blocked: 0 })).toBe('2 tasks · 2 done');
  });
});

describe('member chips and targets (notes §1.3, D6)', () => {
  const n = missionNow(NOW);

  it('short waits', () => {
    expect(shortWait(n.agents['lead-1'], NOW)).toBe('Edit checkout.ts');
    expect(shortWait(n.agents['back-1'], NOW)).toBe('permission · 2 min');
    expect(shortWait(n.agents['front-1'], NOW)).toBe('→ back-1 · 12 min');
    expect(shortWait(n.agents['ui-1'], NOW)).toBe('not started');
    expect(shortWait(rec('x', 'paused'), NOW)).toBe('paused');
    expect(shortWait(rec('x', 'idle'), NOW)).toBe('idle');
    expect(shortWait(rec('x', 'error', { since_ms: NOW - 2 * MIN }), NOW)).toBe('error · 2 min');
    expect(
      shortWait(rec('x', 'working', { activity: 'Edit a/very/long/path/to/some/file/deep/inside.ts' }), NOW),
    ).toHaveLength(32);
  });

  it('a run row targets its blocker, else its lead; a chip overrides', () => {
    const row = { kind: 'run', run: n.runs['run-12'] };
    expect(target(row)).toBe('back-1');
    expect(target(row, 'front-1')).toBe('front-1');
    // The chip is an agent id: a member that left falls back to the default.
    expect(target({ kind: 'run', run: { ...n.runs['run-12'], members: ['lead-1', 'back-1'] } }, 'front-1')).toBe(
      'back-1',
    );
    // Outcome and decision rows have no target: only o / d act there.
    expect(target({ kind: 'outcome', outcome: { agent: 'back-1' } })).toBeNull();
    expect(defaultTarget({ ...n.runs['run-12'], blocker: undefined })).toBe('lead-1');
  });
});

describe('queue copy (notes §1.5, §1.12, FR-809)', () => {
  it('Run now costs the binding window', () => {
    expect(runNowCost({ run_now: { week_pp: 2.1, five_hour_pp: 6, binds: 'week' } })).toBe('≈ +2.1 pp week');
    expect(runNowCost({ run_now: { week_pp: 2.1, five_hour_pp: 6, binds: 'five_hour' } })).toBe('≈ +6 pp 5h');
    expect(runNowCost({ run_now: { binds: 'week' } })).toBe('');
  });

  it('no cost figures: the slot above the cap', () => {
    const r = { ...missionNow(NOW).agents['infra-1'] };
    r.queue = { ...r.queue, run_now: { binds: 'week' } };
    expect(runNowDialog(r).detail).toBe(
      'It is queued: live cap 2/2 · next · ≈14:20.\nRunning it now uses a slot above the cap.',
    );
  });

  it('the confirmation names both windows and the one that holds it', () => {
    const d = runNowDialog(missionNow(NOW).agents['infra-1']);
    expect(d.title).toBe('Run infra-1 now?');
    expect(d.detail).toBe(
      'It is queued: live cap 2/2 · next · ≈14:20.\nRunning it now costs ≈ +2.1 pp of the week and ≈ +6 pp of the 5h window.\nThe week holds it: that is the window this is counted against.\nIt runs above the live cap.',
    );
    expect(d.yes).toBe('Run now ≈ +2.1 pp week');
    expect(runNowDialog(missionNow(NOW).agents['ui-1']).title).toBe('Start ui-1 now?');
  });
});

describe('toolbar key hints (notes §1.9)', () => {
  const n = missionNow(NOW);
  const text = (h) => h.map(([k, l]) => `${k} ${l}`).join(' · ');

  it('a run row with a blocker', () => {
    const row = { kind: 'run', run: n.runs['run-12'] };
    expect(text(hints(row, { target: 'back-1', rec: n.agents['back-1'], decision: 'inbox-1' }))).toBe(
      'j k move · ⏎ open back-1 · i p s ⇧K back-1 · d decision · h l chips · space details · f filter',
    );
  });

  it('only the actions the target allows (notes §1.9)', () => {
    const row = { kind: 'solo', agent: rec('x', 'working') };
    expect(text(hints(row, { target: 'x', rec: rec('x', 'working', { level: 'running' }) }))).toContain('i p ⇧K x');
    expect(text(hints(row, { target: 'x', rec: rec('x', 'idle', { level: 'idle' }) }))).toContain('p s ⇧K x');
    expect(text(hints(row, { target: 'x', rec: rec('x', 'suspended', { level: 'idle', live: false }) }))).toContain(
      'p r ⇧K x',
    );
  });

  it('a queue row, an outcome, nothing', () => {
    const row = { kind: 'queue', agent: n.agents['infra-1'] };
    expect(text(hints(row, { target: 'infra-1', rec: n.agents['infra-1'] }))).toBe(
      'j k move · ⏎ open infra-1 · n run now · ⌫ cancel · space details · f filter',
    );
    expect(text(hints({ kind: 'outcome' }))).toBe('j k move · o open · f filter');
    expect(text(hints(null))).toBe('j k move · f filter');
  });
});
