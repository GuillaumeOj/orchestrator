// P4.1: the `now` store (api-contract §3): snapshot, deltas, rev gaps,
// accessors, and topic refetches without polling.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { rec } from './fixture.js';

const calls = [];
let snapshot = null;
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    return cmd === 'now_snapshot' ? Promise.resolve(snapshot) : Promise.reject(new Error(cmd));
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => String(e),
}));

const {
  applyNow,
  attention,
  connectNow,
  counts,
  featureRun,
  loadNow,
  needsYou,
  onTopics,
  openDecisions,
  runAttention,
  since,
} = await import('../src/lib/now.svelte.js');

const tick = () => new Promise((r) => setTimeout(r, 0));
const C = (n) => ({
  needs_you: n,
  decisions: n,
  running: 0,
  waiting: 0,
  queued: 0,
  idle: 0,
  error: 0,
  live: 1,
  done_today: 0,
});
const decision = (id, state = 'open', more = {}) => ({
  id,
  kind: 'permission',
  level: 1,
  lifecycle: { state },
  created_ms: 1,
  ...more,
});

beforeEach(async () => {
  snapshot = {
    rev: 5,
    agents: [
      rec('back-1', 'waiting_for_you', {
        project: 'shop',
        waiting_on: { kind: 'user', ref: 'inbox-1', since_ms: 100 },
      }),
    ],
    runs: [
      {
        run: 'run-1',
        feature: 'checkout',
        project: 'shop',
        phase: 'executing',
        level: 'needs_you',
        members: ['back-1'],
        counts: C(1),
      },
    ],
    counts: { global: { ...C(1), live_cap: 3 }, projects: { shop: C(1) } },
    decisions: [decision('inbox-1')],
    outcomes: [],
  };
  await loadNow();
  calls.length = 0;
});

describe('now store', () => {
  it('loads the snapshot and answers the accessors', () => {
    expect(app.now.rev).toBe(5);
    expect(attention('back-1').status).toBe('waiting_for_you');
    expect(runAttention('run-1').phase).toBe('executing');
    expect(featureRun({ slug: 'checkout', project_id: 'shop' }).run).toBe('run-1');
    expect(featureRun({ slug: 'x', project_id: 'shop', run: { id: 'run-1' } }).run).toBe('run-1');
    expect(counts()).toMatchObject({ needs_you: 1, live_cap: 3 });
    expect(counts({ project: 'shop' }).needs_you).toBe(1);
    expect(counts({ project: 'nope' })).toMatchObject({ needs_you: 0, queued: 0, live: 0 });
    expect(counts({ run: 'run-1' }).decisions).toBe(1);
    expect(since(attention('back-1'))).toBe(100);
    expect(openDecisions().map((d) => d.id)).toEqual(['inbox-1']);
  });

  it('applies deltas: upserts, removals, decisions, outcomes, counts', () => {
    applyNow({
      rev: 6,
      attention: { agents: [rec('front-1', 'working')], runs: [], removed: { agents: ['back-1'], runs: ['run-1'] } },
      counts: { global: C(0), projects: {} },
      decisions: { upsert: [decision('inbox-1', 'sent'), decision('inbox-2')], closed: ['inbox-1'] },
      outcomes: [{ id: 'ev-1', kind: 'run_done', text: 'checkout done' }],
      topics: [],
    });
    expect(app.now.rev).toBe(6);
    expect(attention('back-1')).toBeNull();
    expect(attention('front-1').level).toBe('running');
    expect(runAttention('run-1')).toBeNull();
    expect(counts().needs_you).toBe(0);
    // A closed decision keeps its final lifecycle until the next snapshot, out of the open list.
    expect(app.now.decisions['inbox-1'].lifecycle.state).toBe('sent');
    expect(openDecisions().map((d) => d.id)).toEqual(['inbox-2']);
    expect(app.now.outcomes).toHaveLength(1);
    expect(calls).toEqual([]);
  });

  it('closed decisions go 10 min after they closed, even with no snapshot', () => {
    vi.useFakeTimers({ now: 1_000_000, toFake: ['Date'] });
    try {
      applyNow({ rev: 6, decisions: { upsert: [decision('inbox-1', 'effect')], closed: ['inbox-1'] }, topics: [] });
      vi.setSystemTime(1_000_000 + 9 * 60000);
      applyNow({ rev: 7, topics: [] });
      expect(app.now.decisions['inbox-1']).toBeTruthy();
      vi.setSystemTime(1_000_000 + 11 * 60000);
      applyNow({ rev: 8, topics: [] });
      expect(app.now.decisions['inbox-1']).toBeUndefined();
    } finally {
      vi.useRealTimers();
    }
  });

  it('a missed rev reads the snapshot again; an old rev is ignored', async () => {
    applyNow({ rev: 4, attention: { agents: [rec('x', 'idle')] }, topics: [] });
    expect(attention('x')).toBeNull();
    snapshot = { ...snapshot, rev: 9, decisions: [] };
    applyNow({ rev: 8, attention: { agents: [rec('x', 'idle')] }, topics: [] });
    await tick();
    expect(calls.map(([c]) => c)).toEqual(['now_snapshot']);
    expect(app.now.rev).toBe(9);
    // The snapshot replaces the copy: closed decisions it does not list are gone.
    expect(app.now.decisions).toEqual({});
  });

  it('needs_you agents in waiting order (leader w, palette jump)', () => {
    applyNow({
      rev: 6,
      attention: {
        agents: [rec('a', 'waiting_gate', { waiting_on: { kind: 'user', since_ms: 50 } }), rec('b', 'working')],
      },
      topics: [],
    });
    expect(needsYou().map((r) => r.agent)).toEqual(['a', 'back-1']);
  });
});

describe('connectNow', () => {
  it('a window shown again reads the snapshot and refetches the views', async () => {
    const stop = await connectNow();
    const load = vi.fn(async () => {});
    const stopView = onTopics(['report'], load, { now: false });
    calls.length = 0;
    snapshot = { ...snapshot, rev: 12 };
    Object.defineProperty(document, 'hidden', { value: false, configurable: true });
    document.dispatchEvent(new Event('visibilitychange'));
    await tick();
    expect(calls.map(([c]) => c)).toEqual(['now_snapshot']);
    expect(app.now.rev).toBe(12);
    expect(load).toHaveBeenCalledTimes(1);
    // Hidden: nothing is read.
    Object.defineProperty(document, 'hidden', { value: true, configurable: true });
    document.dispatchEvent(new Event('visibilitychange'));
    await tick();
    expect(calls).toHaveLength(1);
    stop();
    stopView();
    Object.defineProperty(document, 'hidden', { value: false, configurable: true });
  });
});

describe('topics', () => {
  it('loads on subscribe, on its topic, on focus; never on other topics', async () => {
    const load = vi.fn(async () => {});
    const stop = onTopics(['board:shop'], load);
    await tick();
    expect(load).toHaveBeenCalledTimes(1);
    applyNow({ rev: 6, topics: ['board:blog', 'agents'] });
    await tick();
    expect(load).toHaveBeenCalledTimes(1);
    applyNow({ rev: 7, topics: ['board:shop'] });
    await tick();
    expect(load).toHaveBeenCalledTimes(2);
    window.dispatchEvent(new Event('focus'));
    await tick();
    expect(load).toHaveBeenCalledTimes(3);
    stop();
    applyNow({ rev: 8, topics: ['board:shop'] });
    window.dispatchEvent(new Event('focus'));
    await tick();
    expect(load).toHaveBeenCalledTimes(3);
  });

  it('coalesces: one load in flight, one more after it however many topics came', async () => {
    /** @type {(v?: unknown) => void} */
    let release = () => {};
    const load = vi.fn(() => new Promise((r) => (release = r)));
    const stop = onTopics(['report'], load, { now: false });
    applyNow({ rev: 6, topics: ['report'] });
    await tick();
    applyNow({ rev: 7, topics: ['report'] });
    applyNow({ rev: 8, topics: ['report'] });
    applyNow({ rev: 9, topics: ['report'] });
    await tick();
    expect(load).toHaveBeenCalledTimes(1);
    release();
    await tick();
    await tick();
    expect(load).toHaveBeenCalledTimes(2);
    release();
    await tick();
    expect(load).toHaveBeenCalledTimes(2);
    stop();
  });
});
