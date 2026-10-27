// Status vocabulary (SPEC §9.5): one label, one icon and one color per state,
// used everywhere. Icons are Material Design glyphs of FiraCode Nerd Font Mono
// (docs/design/README.md).
import { pp, signed } from './format.js';

export const STATUS = {
  working: { label: 'Working', icon: '\u{F040A}', color: 'var(--color-status-working)' },
  waiting_for_you: { label: 'Waiting for you', icon: '\u{F0005}', color: 'var(--color-status-waiting-for-you)' },
  waiting_gate: { label: 'At gate', icon: '\u{F0565}', color: 'var(--color-status-at-gate)' },
  queued: { label: 'Queued', icon: '\u{F051F}', color: 'var(--color-status-queued)' },
  paused: { label: 'Paused', icon: '\u{F03E4}', color: 'var(--color-status-paused)' },
  blocked_by_limit: { label: 'Blocked by limit', icon: '\u{F073A}', color: 'var(--color-status-blocked-by-limit)' },
  idle: { label: 'Idle', icon: '\u{F0766}', color: 'var(--color-status-idle)' },
  suspended: { label: 'Suspended', icon: '\u{F04B2}', color: 'var(--color-status-suspended)' },
  error: { label: 'Error', icon: '\u{F0159}', color: 'var(--color-status-error)' },
  done: { label: 'Done', icon: '\u{F012C}', color: 'var(--color-status-done)' },
};

/**
 * The §9.5 state of an agent row from the runtime's internal state. The
 * runtime has a few states the vocabulary doesn't name: `starting` (before
 * SessionStart) reads Working, `created` reads Queued ("Not started yet"),
 * `killed` reads Suspended (its session can be resumed), `archived` Done.
 * A queued turn held by Pause all, and an agent paused on its own, read Paused.
 */
export function displayState(agent) {
  const s = agent?.state;
  const held = agent?.queue_entry?.reason?.kind;
  if (held === 'paused_all' || held === 'paused_agent') return 'paused';
  // A pause never hides what needs the user (a prompt, a gate, an error) or a running turn.
  if (agent?.paused && !['working', 'starting', 'waiting_for_you', 'waiting_gate', 'error'].includes(s))
    return 'paused';
  switch (s) {
    case 'starting':
      return 'working';
    case 'created':
      return 'queued';
    case 'killed':
      return 'suspended';
    case 'archived':
      return 'done';
    default:
      return STATUS[s] ? s : 'idle';
  }
}

/** Not started yet: queued with no `claude` process (§9.5, FR-809). */
export const notStarted = (agent) => agent?.state === 'created' || (displayState(agent) === 'queued' && !agent?.live);

export const PRESSURE = {
  green: { label: 'Green', icon: '●', color: 'var(--color-pacing-green)' },
  amber: { label: 'Amber', icon: '▲', color: 'var(--color-pacing-amber)' },
  red: { label: 'Red', icon: '■', color: 'var(--color-pacing-red)' },
  catch_up: { label: 'Catch-up', icon: '⇡', color: 'var(--color-pacing-catch-up)' },
};

// Role marks (§9.1 principle 7): solo sessions until teams exist (P2).
export const ROLES = { lead: 1, back: 2, front: 3, ui: 4, ux: 5, solo: 6 };

export function roleOf(agentId) {
  const prefix = String(agentId).split(/[-_]/)[0].toLowerCase();
  const key = { l: 'lead', b: 'back', backend: 'back', f: 'front', frontend: 'front' }[prefix] ?? prefix;
  const n = ROLES[key] ?? ROLES.solo;
  const letter = key in ROLES && key !== 'solo' ? key[0].toUpperCase() : 'S';
  return { letter, color: `var(--color-role-${n})` };
}

/** Why a queued agent waits, in words (FR-809 copy). */
export function queueReason(entry) {
  const r = entry?.reason ?? {};
  switch (r.kind) {
    case 'live_cap':
      return `live-agent cap ${r.cap} (${r.state === 'catch_up' ? 'Catch-up' : r.state})`;
    case 'red_tier':
      return 'low priority (Red)';
    case 'off_hours':
      return 'outside working hours';
    case 'five_hour_guard':
      return '5h guard';
    case 'limit_reached':
      return `${r.window === 'weekly' ? 'weekly' : '5-hour'} limit reached`;
    case 'deferred_until_reset':
      return 'deferred until the weekly reset';
    case 'hard_cap':
      return 'allocation hard cap';
    case 'paused_all':
      return 'Pause all';
    case 'paused_agent':
      return 'paused (p resumes)';
    case 'memory_pressure':
      return `memory pressure (${r.cap} live)`;
    case 'behind_queue':
      return 'behind other queued work';
    default:
      return 'queued';
  }
}

/**
 * Run now's cost in the window that holds the entry (FR-809): % of the 5h
 * window under the 5h guard, pp of the week otherwise. `both` adds the other.
 */
export function runCost(entry, both = false) {
  const week = pp(entry?.estimated_cost_pct ?? 0, 1);
  const f = entry?.estimated_cost_5h_pct;
  if (f == null) return week;
  const five = `${signed(f)} %`;
  if (entry?.binding === 'five_hour') return both ? `${five} of the 5h window (${week} of the week)` : `${five} 5h`;
  return both ? `${week} of the week (${five} of the 5h window)` : week;
}

/** The agent's session usage in pp of the week (cost / cost per weekly %). */
export function sessionPp(agent, snapshot) {
  const k = snapshot?.calibration?.cost_per_weekly_pct;
  return agent?.cost_usd != null && k ? agent.cost_usd / k : null;
}

/** "warm until 16:42" / "cold since 15:10" from the statusline's prompt cache (R6). */
export function cacheState(agent, clock, now = Date.now()) {
  const at = agent?.prompt_cache_expires_at;
  if (!at) return null;
  return at * 1000 > now ? `warm until ${clock(at * 1000)}` : `cold since ${clock(at * 1000)}`;
}
