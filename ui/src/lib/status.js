// Status vocabulary (SPEC §9.5): one label, one icon and one color per state,
// used everywhere. Icons are Material Design glyphs of FiraCode Nerd Font Mono
// (docs/design/README.md).
import { clock, dayHm, duration } from './format.js';

export const STATUS = {
  working: { label: 'Working', icon: '\u{F040A}', color: 'var(--color-status-working)' },
  waiting_for_you: { label: 'Waiting for you', icon: '\u{F0005}', color: 'var(--color-status-waiting-for-you)' },
  waiting_gate: { label: 'At gate', icon: '\u{F0565}', color: 'var(--color-status-at-gate)' },
  queued: { label: 'Queued', icon: '\u{F051F}', color: 'var(--color-status-queued)' },
  not_started: { label: 'Not started yet', icon: '\u{F0150}', color: 'var(--color-status-not-started)' },
  paused: { label: 'Paused', icon: '\u{F03E4}', color: 'var(--color-status-paused)' },
  blocked_by_limit: { label: 'Blocked by limit', icon: '\u{F073A}', color: 'var(--color-status-blocked-by-limit)' },
  idle: { label: 'Idle', icon: '\u{F0766}', color: 'var(--color-status-idle)' },
  suspended: { label: 'Suspended', icon: '\u{F04B2}', color: 'var(--color-status-suspended)' },
  error: { label: 'Error', icon: '\u{F0159}', color: 'var(--color-status-error)' },
  done: { label: 'Done', icon: '\u{F012C}', color: 'var(--color-status-done)' },
};

// Attention levels (api-contract §1, P4.1 palettes): shapes, not glyphs.
export const LEVEL = {
  needs_you: { label: 'Needs you', icon: '●', color: 'var(--color-attention-needs-you)' },
  error: { label: 'Error', icon: '◆', color: 'var(--color-attention-error)' },
  running: { label: 'Running', icon: '▲', color: 'var(--color-attention-running)' },
  waiting: { label: 'Waiting', icon: '◐', color: 'var(--color-attention-waiting)' },
  queued: { label: 'Queued', icon: '□', color: 'var(--color-attention-queued)' },
  idle: { label: 'Idle', icon: '▭', color: 'var(--color-attention-idle)' },
  done: { label: 'Done', icon: '■', color: 'var(--color-attention-done)' },
};

// Decision kinds (Inbox list): their own palette, not a borrowed agent state.
const DECISION = {
  permission: { label: 'Permission', icon: '\u{F0BC5}' },
  question: { label: 'Question', icon: '\u{F0186}' },
  'gate-spec': { label: 'Spec gate', icon: '\u{F1187}' },
  'gate-pr': { label: 'PR gate', icon: '\u{F14CF}' },
  'gate-restart': { label: 'Restart gate', icon: '\u{F0709}' },
  startup: { label: 'Startup', icon: '\u{F0425}' },
  ask: { label: 'Run now?', icon: '\u{F140C}' },
  budget: { label: 'Budget', icon: '\u{F0BDD}' },
  trust: { label: 'Trust dialog', icon: '\u{F0CCC}' },
  wait: { label: 'Prompt in pane', icon: '\u{F07B7}' },
  alert: { label: 'Alert', icon: '\u{F11CF}' },
  error: { label: 'Error', icon: '\u{F11CF}' },
  notion: { label: 'Notion comment', icon: '\u{F0C12}' },
};

/** A decision's kind entry: icon, label and its `color.decision.*` token (a gate by its flavour). */
export function decisionKind(d) {
  const want = d?.kind === 'gate' ? `gate-${d.gate ?? 'restart'}` : d?.kind;
  const key = DECISION[want] ? want : 'alert';
  return { ...DECISION[key], color: `var(--color-decision-${key})` };
}

// Decision lifecycle (the marker): icon and color.lifecycle.<state>.
export const LIFECYCLE = {
  open: { icon: '\u{F009F}', color: 'var(--color-lifecycle-open)' },
  parked: { icon: '\u{F1592}', color: 'var(--color-lifecycle-parked)' },
  decided: { icon: '\u{F1164}', color: 'var(--color-lifecycle-decided)' },
  sent: { icon: '\u{F1165}', color: 'var(--color-lifecycle-sent)' },
  effect: { icon: '\u{F05E1}', color: 'var(--color-lifecycle-effect)' },
  failed: { icon: '\u{F05D6}', color: 'var(--color-lifecycle-failed)' },
  stale: { icon: '\u{F05CE}', color: 'var(--color-lifecycle-stale)' },
  dismissed: { icon: '\u{F015A}', color: 'var(--color-lifecycle-dismissed)' },
  interrupted: { icon: '\u{F033A}', color: 'var(--color-lifecycle-interrupted)' },
};

// Waiting-on kind icons (the chip, text.muted); `running` = the activity icon.
export const WAITING_ON = {
  user: '\u{F0B51}',
  agent: '\u{F06A9}',
  task: '\u{F0756}',
  governor: '\u{F05D1}',
  limit: '\u{F029A}',
  startup: '\u{F14DF}',
  external: '\u{F109B}',
  running: '\u{F018D}',
};

// Outcome kind icons (the digest, text.muted).
export const OUTCOME = {
  task_done: '\u{F08A8}',
  merge_done: '\u{F0F5C}',
  pr_opened: '\u{F14CA}',
  run_phase: '\u{F0D3D}',
  run_done: '\u{F023D}',
  turn_done: '\u{F173E}',
  report_ready: '\u{F1029}',
  sync_conflict: '\u{F04E7}',
};

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

/** D4: work you scheduled for off-hours (P1's off_hours = outside working hours). */
export function offHoursReason(startsAt, now = Date.now()) {
  const t = Date.parse(startsAt);
  return t > now ? `Off-hours · starts ${clock(t, now)} (in ${duration(t - now)})` : 'Off-hours';
}

/** FR-815: "opus week in Red · starts after reset Mon 09:00" / "… exhausted · starts Mon 09:00". */
export function modelWindowReason(model, resetsAt, exhausted) {
  const at = dayHm(Date.parse(resetsAt));
  return exhausted ? `${model} week exhausted · starts ${at}` : `${model} week in Red · starts after reset ${at}`;
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
    case 'scheduled_off_hours':
      return offHoursReason(r.starts_at, now);
    case 'model_window':
      return modelWindowReason(r.model, r.resets_at, r.exhausted);
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
