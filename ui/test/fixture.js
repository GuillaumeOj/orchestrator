// The design scenario's shape (Penpot 04), as app_state returns it.
export function fixture() {
  const now = Date.now();
  return {
    now_ms: now,
    core: { ok: true, error: null, home: '/Users/me/Library/Application Support/Orchestrator' },
    warnings: [],
    projects: [
      { id: 'shop-api', name: 'shop-api', path: '/dev/shop-api', git: { branch: 'main', ahead: 2, behind: 0, dirty: 3 } },
      { id: 'blog', name: 'blog', path: '/dev/blog', git: { branch: 'main', ahead: 0, behind: 0, dirty: 0 } },
    ],
    agents: [
      { id: 'back-1', state: 'working', live: true, project_id: 'shop-api', context_pct: 62, cost_usd: 1.8, memory_mb: 412, current_tool: 'Bash', last_event_at_ms: now - 60000, created_at_ms: now - 38 * 60000, worktree: '/dev/shop-api.worktrees/back-1', branch: 'orch/solo/back-1', session_id: 's1' },
      { id: 'front-1', state: 'waiting_for_you', live: true, project_id: 'shop-api', context_pct: 35, cost_usd: 0.9, memory_mb: 380, last_event_at_ms: now - 4 * 60000, worktree: '/dev/shop-api.worktrees/front-1', session_id: 's2' },
      { id: 'rss-feed', state: 'suspended', live: false, project_id: 'blog', context_pct: 12, cost_usd: 0.7, last_event_at_ms: now - 47 * 60000, worktree: '/dev/blog.worktrees/rss-feed', session_id: 's3' },
    ],
    usage: {
      reset_at_ms: now + 4 * 86400000,
      shadow: false,
      live_cap: 1,
      five_hour: { used_pct: 61, resets_at: Math.floor(now / 1000) + 7200 },
      snapshot: { used: 56, governed: 47, ideal: 38, plan: 47, error: 9, state: 'red', provenance: 'observed', five_hour: 61, five_hour_guard: true, live_cap: 1, live_autonomous: 1, queued: 2, interactive_used: 5, unattributed_so_far: 4, unattributed_forecast: 4, projected: 98, reserve_left: 5, burn_pct_per_hour: 0.8, calibration: { cost_per_weekly_pct: 1, confidence: 0.6 } },
    },
    queue: [],
    inbox: {
      badge: 2,
      items: [
        { id: 'perm-1', level: 1, kind: 'permission', agent_id: 'front-1', title: 'Bash: Add the Stripe client SDK', detail: 'front-1 asks to run Bash', preview: 'pnpm add @stripe/stripe-js', created_ms: now - 240000, actions: ['a', 'r', 'enter', 'o', 'e'] },
        { id: 'ask-7', level: 3, kind: 'budget', agent_id: null, title: 'Defer "infra" until after the weekly reset', detail: 'saves ≈ 6 pp', preview: null, created_ms: now - 60000, actions: ['a', 'r'] },
      ],
    },
    permissions: [{ id: 'perm-1', agent_id: 'front-1', tool: 'Bash', summary: 'pnpm add @stripe/stripe-js', created_ms: now - 240000 }],
    memory: { app_mb: 96, agents_mb: 792, shells_mb: 10, total_mb: 898, system_mb: 8192 },
    terms: [],
  };
}
