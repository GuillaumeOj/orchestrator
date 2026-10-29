// The design scenario's shape (Penpot 04), as app_state returns it.
export function fixture() {
  const now = Date.now();
  return {
    now_ms: now,
    core: { ok: true, error: null, home: '/Users/me/Library/Application Support/Orchestrator' },
    warnings: [],
    projects: [
      {
        id: 'shop-api',
        name: 'shop-api',
        path: '/dev/shop-api',
        git: { branch: 'main', ahead: 2, behind: 0, dirty: 3 },
      },
      { id: 'blog', name: 'blog', path: '/dev/blog', git: { branch: 'main', ahead: 0, behind: 0, dirty: 0 } },
    ],
    agents: [
      {
        id: 'back-1',
        state: 'working',
        live: true,
        project_id: 'shop-api',
        context_pct: 62,
        cost_usd: 1.8,
        memory_mb: 412,
        current_tool: 'Bash',
        last_event_at_ms: now - 60000,
        created_at_ms: now - 38 * 60000,
        worktree: '/dev/shop-api.worktrees/back-1',
        branch: 'orch/solo/back-1',
        session_id: 's1',
      },
      {
        id: 'front-1',
        state: 'waiting_for_you',
        live: true,
        project_id: 'shop-api',
        context_pct: 35,
        cost_usd: 0.9,
        memory_mb: 380,
        last_event_at_ms: now - 4 * 60000,
        worktree: '/dev/shop-api.worktrees/front-1',
        session_id: 's2',
      },
      {
        id: 'rss-feed',
        state: 'suspended',
        live: false,
        project_id: 'blog',
        context_pct: 12,
        cost_usd: 0.7,
        last_event_at_ms: now - 47 * 60000,
        worktree: '/dev/blog.worktrees/rss-feed',
        session_id: 's3',
      },
    ],
    usage: {
      reset_at_ms: now + 4 * 86400000,
      shadow: false,
      live_cap: 1,
      five_hour: { used_pct: 61, resets_at: Math.floor(now / 1000) + 7200 },
      snapshot: {
        used: 56,
        governed: 47,
        ideal: 38,
        plan: 47,
        error: 9,
        state: 'red',
        provenance: 'observed',
        five_hour: 61,
        five_hour_guard: true,
        live_cap: 1,
        live_autonomous: 1,
        queued: 2,
        interactive_used: 5,
        unattributed_so_far: 4,
        unattributed_forecast: 4,
        projected: 98,
        reserve_left: 5,
        burn_pct_per_hour: 0.8,
        calibration: { cost_per_weekly_pct: 1, confidence: 0.6 },
      },
    },
    queue: [],
    memory: { app_mb: 96, agents_mb: 792, shells_mb: 10, total_mb: 898, system_mb: 8192 },
    terms: [],
  };
}

const act = (id, label, key) => (key ? { id, label, key } : { id, label });

/** A Decision (api-contract §2) as back's T-22 builds it; `over` replaces fields. */
export function decision(over = {}) {
  return {
    id: 'inbox-1',
    kind: 'permission',
    level: 1,
    subject: {},
    title: '',
    summary: '',
    context: [],
    actions: [],
    lifecycle: { state: 'open' },
    thread: 'x',
    waits_on: 'user',
    created_ms: Date.now() - 60000,
    payload: {},
    ...over,
  };
}

/** front-1's permission prompt, open (the design scenario). */
export const permDecision = (over = {}) =>
  decision({
    id: 'perm-1',
    subject: { agent: 'front-1', role: 'Frontend', project: 'shop-api' },
    title: 'Bash · pnpm add @stripe/stripe-js',
    summary: 'Add the Stripe client SDK',
    context: [
      {
        type: 'tool_call',
        tool: 'Bash',
        input: { command: 'pnpm add @stripe/stripe-js', description: 'Add the Stripe client SDK' },
        cwd: '/Users/me/dev/shop-web.worktrees/front-1',
      },
    ],
    actions: [
      act('approve', 'Allow once', 'a'),
      act('reject', 'Deny', 'r'),
      act('reply', 'Deny with message', 'enter'),
      act('always', 'Always allow'),
      act('in_pane', 'Answer in the pane'),
      act('open', 'Open pane', 'o'),
    ],
    thread: 'front-1',
    created_ms: Date.now() - 240000,
    payload: { always: 'Bash(pnpm add:*)' },
    ...over,
  });

/** A budget ask (level 3, no agent). */
export const budgetDecision = (over = {}) =>
  decision({
    id: 'ask-7',
    kind: 'budget',
    level: 3,
    title: 'Defer "infra" until after the weekly reset',
    summary: 'saves ≈ 6 pp',
    context: [
      { type: 'markdown', text: 'Defer infra' },
      { type: 'cost', week_pp: 6 },
    ],
    actions: [act('approve', 'Apply', 'a'), act('reject', 'Skip', 'r')],
    thread: 'governor',
    ...over,
  });

export { act };

/** Counts (api-contract §1): zeros but for `o`. */
export const countsOf = (o = {}) => ({
  needs_you: 0,
  decisions: 0,
  running: 0,
  waiting: 0,
  queued: 0,
  idle: 0,
  error: 0,
  live: 0,
  done_today: 0,
  ...o,
});

const LEVEL_OF = {
  working: 'running',
  waiting_for_you: 'needs_you',
  waiting_gate: 'needs_you',
  queued: 'queued',
  not_started: 'queued',
  paused: 'queued',
  blocked_by_limit: 'waiting',
  error: 'error',
  done: 'done',
};

/** An attention record (api-contract §1); `status` sets the level unless given. */
export function rec(agent, status, more = {}) {
  return {
    agent,
    level: LEVEL_OF[status] ?? 'idle',
    status,
    reason: '',
    live: status !== 'suspended',
    since_ms: Date.now() - 60000,
    ...more,
  };
}

/** `app.now` for an app_state fixture: one record per agent, and `decisions`
 *  (the design scenario's permission and budget ask by default); other keys replace `app.now`'s.
 *  @param {any} d
 *  @param {{ decisions?: any[], [key: string]: any }} [opts] */
export function nowOf(d, { decisions = [permDecision(), budgetDecision()], ...more } = {}) {
  const agents = (d.agents ?? []).map((a) => rec(a.id, a.state, { project: a.project_id, live: !!a.live }));
  const open = decisions.filter((x) => x.level < 4).length;
  return {
    rev: 1,
    agents: Object.fromEntries(agents.map((r) => [r.agent, r])),
    runs: {},
    decisions: Object.fromEntries(decisions.map((x) => [x.id, x])),
    outcomes: [],
    counts: {
      global: { needs_you: 0, decisions: open, live: agents.filter((r) => r.live).length, queued: 0 },
      projects: {},
    },
    ...more,
  };
}
