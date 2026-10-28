// Status vocabulary (SPEC §9.5): one label, one icon and one color per state,
// used everywhere. Icons are Material Design glyphs of FiraCode Nerd Font Mono
// (docs/design/README.md).
import { clock, duration, pp, signed } from './format.js';

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

// Role marks (§9.1 principle 7): color.role.1…8 in the order Lead, Backend,
// Frontend, UI, UX, Solo, Reviewer; the initial is a graphic mark, the name
// carries the meaning (Penpot 02 Agent chip).
export const ROLES = { lead: 1, back: 2, front: 3, ui: 4, ux: 5, solo: 6, reviewer: 7 };
const ROLE_ALIAS = { backend: 'back', frontend: 'front', b: 'back', f: 'front', l: 'lead', review: 'reviewer' };
const ROLE_LETTER = { lead: 'L', back: 'B', front: 'F', ui: 'U', ux: 'X', solo: 'S', reviewer: 'R' };

/**
 * The mark of an agent or a role: from its role id when known (team agents,
 * canvas nodes; `color` = the role's own color.role index), else guessed from
 * the agent id's prefix (fixtures, older agents).
 */
export function roleOf(agentId, role = null, color = null) {
  const raw = String(role ?? String(agentId).split(/[-_]/)[0]).toLowerCase();
  const key = ROLE_ALIAS[raw] ?? raw;
  if (key in ROLES) {
    const n = color ?? ROLES[key];
    return { letter: ROLE_LETTER[key], color: `var(--color-role-${n})` };
  }
  if (!role) return { letter: 'S', color: `var(--color-role-${color ?? ROLES.solo})` };
  // A custom role: its own color, else a stable one of the 8.
  const n = color ?? 1 + ([...key].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7) % 8);
  return { letter: key[0].toUpperCase(), color: `var(--color-role-${n})` };
}

/** Why a queued agent waits, in words (FR-809 copy). */
export function queueReason(entry, now = Date.now()) {
  const r = entry?.reason ?? {};
  switch (r.kind) {
    case 'live_cap':
      return `live-agent cap ${r.cap} (${r.state === 'catch_up' ? 'Catch-up' : r.state})`;
    case 'red_tier':
      return 'low priority (Red)';
    case 'off_hours':
      return 'outside working hours';
    case 'scheduled_off_hours': {
      // D4: work you scheduled for off-hours (P1's off_hours = outside working hours).
      const t = Date.parse(r.starts_at);
      return t > now ? `Off-hours · starts ${clock(t, now)} (in ${duration(t - now)})` : 'Off-hours';
    }
    case 'model_window': {
      const at = clock(Date.parse(r.resets_at), Date.parse(r.resets_at) + 2 * 86400000);
      return r.exhausted
        ? `${r.model} week exhausted · starts ${at}`
        : `${r.model} week in Red · starts after reset ${at}`;
    }
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
