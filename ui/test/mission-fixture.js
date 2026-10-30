// The P4.2 design fixture (ux-brief §4), as `now` holds it: run checkout
// (shop-api) executing with lead-1 running, back-1 at a permission, front-1
// waiting on back-1 and ui-1 not started; solo blog › fix-feed running;
// infra-1 queued behind the live cap; four outcomes today. Times are
// relative to `now` and today's date, so the tests hold any day and hour.

import { decision, rec } from './fixture.js';

export const MIN = 60000;

/** Today at h:m (an outcome's time: today whatever the hour now). */
export const today = (h, m) => new Date().setHours(h, m, 0, 0);

export const counts = (c = {}) => ({
  needs_you: 0,
  decisions: 0,
  running: 0,
  waiting: 0,
  queued: 0,
  idle: 0,
  error: 0,
  live: 0,
  done_today: 0,
  ...c,
});

export function checkoutRun(now = Date.now(), more = {}) {
  return {
    run: 'run-12',
    feature: 'checkout',
    project: 'shop-api',
    phase: 'executing',
    phase_since_ms: now - 120 * MIN,
    level: 'needs_you',
    members: ['lead-1', 'back-1', 'front-1', 'ui-1'],
    lead: 'lead-1',
    started_ms: now - 198 * MIN,
    tasks: { total: 7, done: 3, in_progress: 2, review: 1, blocked: 1 },
    blocker: {
      agent: 'back-1',
      reason: 'permission: Bash pnpm add zod',
      waiting_on: { kind: 'user', ref: 'inbox-1', since_ms: now - 2 * MIN },
    },
    counts: counts({ needs_you: 1, running: 1, waiting: 1, queued: 1, live: 3 }),
    ...more,
  };
}

/** `app.now` for the fixture. */
export function missionNow(now = Date.now()) {
  const member = { project: 'shop-api', run: 'run-12' };
  const agents = [
    rec('lead-1', 'working', { ...member, activity: 'Edit checkout.ts', since_ms: now - 120 * MIN }),
    rec('back-1', 'waiting_for_you', {
      ...member,
      reason: 'permission: Bash pnpm add zod',
      waiting_on: { kind: 'user', ref: 'inbox-1', since_ms: now - 2 * MIN },
    }),
    rec('front-1', 'idle', {
      ...member,
      level: 'waiting',
      reason: '→ back-1 · API contract',
      waiting_on: { kind: 'agent', ref: 'back-1', since_ms: now - 12 * MIN },
    }),
    rec('ui-1', 'not_started', {
      ...member,
      live: false,
      reason: 'live cap 2/2 · ≈14:40',
      waiting_on: { kind: 'governor', ref: 'ui-1', since_ms: now - 2 * MIN },
      queue: {
        entry: 315,
        tier: 'normal',
        eta_ms: now + 40 * MIN,
        run_now: { week_pp: 1, five_hour_pp: 3, binds: 'week' },
      },
    }),
    rec('fix-feed', 'working', { project: 'blog', activity: 'Edit src/feed.ts', since_ms: now - 42 * MIN }),
    rec('infra-1', 'queued', {
      project: 'infra',
      live: false,
      reason: 'live cap 2/2 · next · ≈14:20',
      waiting_on: { kind: 'governor', ref: 'infra-1', since_ms: now - 6 * MIN },
      queue: {
        entry: 314,
        position: 1,
        tier: 'normal',
        eta_ms: now + 20 * MIN,
        run_now: { week_pp: 2.1, five_hour_pp: 6, binds: 'week' },
      },
    }),
  ];
  const perm = decision({
    id: 'inbox-1',
    kind: 'permission',
    title: 'Bash · pnpm add zod',
    subject: { agent: 'back-1', project: 'shop-api', feature: 'checkout' },
    created_ms: now - 2 * MIN,
  });
  const outcomes = [
    {
      id: 'ev-10',
      kind: 'run_phase',
      run: 'run-12',
      project: 'shop-api',
      at_ms: today(11, 40),
      text: 'checkout: planning → executing',
      seen: true,
    },
    {
      id: 'ev-11',
      kind: 'pr_opened',
      project: 'blog',
      agent: 'fix-feed',
      at_ms: today(12, 30),
      text: 'PR opened · blog › fix-feed #12',
      url: 'https://github.com/o/blog/pull/12',
      seen: true,
    },
    {
      id: 'ev-12',
      kind: 'task_done',
      run: 'run-12',
      project: 'shop-api',
      agent: 'back-1',
      at_ms: today(13, 2),
      text: 'T-3 done · checkout API',
      seen: false,
    },
    {
      id: 'ev-13',
      kind: 'merge_done',
      run: 'run-12',
      project: 'shop-api',
      at_ms: today(13, 10),
      text: 'Merged back-1 into integration',
      seen: false,
    },
  ];
  return {
    rev: 1,
    agents: Object.fromEntries(agents.map((r) => [r.agent, r])),
    runs: { 'run-12': checkoutRun(now) },
    decisions: { [perm.id]: perm },
    outcomes,
    counts: {
      global: counts({
        needs_you: 1,
        decisions: 1,
        running: 2,
        waiting: 1,
        queued: 2,
        live: 3,
        live_cap: 2,
        done_today: 4,
      }),
      projects: {
        'shop-api': counts({ needs_you: 1, running: 1, waiting: 1, queued: 1, live: 3, done_today: 3 }),
        blog: counts({ running: 1, live: 1, done_today: 1 }),
        infra: counts({ queued: 1 }),
      },
    },
  };
}

/** `app.data` (app_state) for the fixture: projects, the raw agents (ctx, memory), a feature. */
export function missionData(now = Date.now()) {
  const raw = (id, project, more = {}) => ({
    id,
    state: 'idle',
    live: false,
    project_id: project,
    created_at_ms: now - 90 * MIN,
    ...more,
  });
  return {
    projects: [
      { id: 'shop-api', name: 'shop-api', path: '/dev/shop-api' },
      { id: 'blog', name: 'blog', path: '/dev/blog' },
      { id: 'infra', name: 'infra', path: '/dev/infra' },
    ],
    agents: [
      raw('lead-1', 'shop-api', { context_pct: 62, memory_mb: 412, team: { node: 'lead-1' } }),
      raw('back-1', 'shop-api', { context_pct: 41, memory_mb: 380 }),
      raw('front-1', 'shop-api', { context_pct: 35, memory_mb: 295 }),
      raw('ui-1', 'shop-api'),
      raw('fix-feed', 'blog', { context_pct: 28, memory_mb: 260 }),
      raw('infra-1', 'infra'),
    ],
    features: [{ id: 'f-co', project_id: 'shop-api', slug: 'checkout', run: { id: 'run-12', status: 'executing' } }],
    memory: { total_mb: 1434, system_mb: 8192, app_mb: 300, agents_mb: 1000, shells_mb: 134 },
    usage: { snapshot: { state: 'green' } },
    setup: { done: true },
  };
}
