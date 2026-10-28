// P3 pacing in the UI (FR-815, 816, 818, 819; p3-design-notes §5.9–5.12):
// model windows, off-hours, the weekly report and the downgrade log copy.
// Pure: tested in test/pacing.test.js. Usage is pp of the week; $ appears
// only as the muted "≈ API $" column (D7).
import { clock, dayTime, duration } from './format.js';

/** ISO week number of a date (the report's "Week 43"). */
export function weekNo(ms) {
  const d = new Date(ms);
  const t = new Date(Date.UTC(d.getFullYear(), d.getMonth(), d.getDate()));
  const day = t.getUTCDay() || 7;
  t.setUTCDate(t.getUTCDate() + 4 - day);
  return Math.ceil(((t.getTime() - Date.UTC(t.getUTCFullYear(), 0, 1)) / 86400000 + 1) / 7);
}

// ---------------------------------------------------------------- model windows

/** "opus week" from a window name ("seven_day_opus") and its family. */
export const windowLabel = (w) => `${w.family ?? w.name} week`;

/** The pill state of a model window: exhausted reads as its own label. */
export const WINDOW_STATE = {
  green: { label: 'Green', icon: '●', color: 'var(--color-pacing-green)' },
  amber: { label: 'Amber', icon: '▲', color: 'var(--color-pacing-amber)' },
  red: { label: 'Red', icon: '■', color: 'var(--color-pacing-red)' },
  exhausted: { label: 'Exhausted', icon: '⊘', color: 'var(--color-status-blocked-by-limit)' },
};

/** "Mon 09:00": a reset within the week. */
export const dayHm = (ms) => clock(ms, ms + 2 * 86400000);

/** "Queued: opus week in Red · starts after reset Mon 09:00" (§5.10). */
export function windowQueued(w) {
  const at = dayHm(Date.parse(w.resets_at));
  return w.state === 'exhausted'
    ? `Queued: ${windowLabel(w)} exhausted · starts ${at} · or set a downgrade for this role`
    : `Queued: ${windowLabel(w)} in Red · starts after reset ${at}`;
}

/** Windows that hold work (title bar compact: "· opus 92 %"). */
export const hotWindows = (windows) => (windows ?? []).filter((w) => w.state === 'red' || w.state === 'exhausted');

// ---------------------------------------------------------------- off-hours

/** The next off-hours window start after `now` (ms), else null. */
export function nextOffHours(off, now = Date.now()) {
  if (!offHoursAllowed(off)) return null;
  const starts = (off?.windows ?? []).map((w) => Date.parse(w.start)).filter((t) => t > now);
  const inside = (off?.windows ?? []).some((w) => Date.parse(w.start) <= now && Date.parse(w.end) > now);
  if (inside) return now;
  return starts.length ? Math.min(...starts) : null;
}

/** Off-hours scheduling needs this week's allowance (plan `off_hours_allowance_pct` > 0). */
export const offHoursAllowed = (off) => (off?.allowance_pct ?? 0) > 0;

export const OFF_DISABLED = 'Off-hours allowance is 0 % this week. Set it in the plan (e).';

/** "Off-hours · starts 18:00 (in 7h)" (D4/D5: the time comes from starts_at). */
export function offHoursReason(startsAt, now = Date.now()) {
  const t = Date.parse(startsAt);
  return t > now
    ? `Off-hours · starts ${clock(t, now)} (in ${duration(t - now)})`
    : 'Off-hours · running in the window';
}

/** The status bar's power chip (§5.11); null when nothing is pending. */
export function powerChip(power) {
  const n = power?.off_hours_pending ?? 0;
  if (!n) return null;
  if (power.on_ac === false) return { text: `On battery · ${n} off-hours waiting`, warn: true };
  return { text: power.assertion_held ? `Keeping awake · ${n} off-hours` : `${n} off-hours queued`, warn: false };
}

// ---------------------------------------------------------------- downgrade

/** Governor log copy for the P3 actions (§5.12). */
export function downgradeText(k) {
  const who = k.agent ?? k.agent_id ?? 'an agent';
  const why = k.why ?? k.when ?? k.state;
  if (k.kind === 'downgrade')
    return `Downgraded ${who} ${k.from ?? ''} → ${k.to} at its task boundary${why ? ` (${label(why)})` : ''}`.replace(
      '  ',
      ' ',
    );
  if (k.kind === 'restore_model') return `Restored ${who} to ${k.to}${why ? ` (${label(why)})` : ''}`;
  return null;
}

const label = (s) =>
  ({ red: 'Red', amber: 'Amber', green: 'Green', model_window: 'model window', catch_up: 'Catch-up' })[s] ?? s;

// ---------------------------------------------------------------- weekly report

export const TABS = [
  { id: 'project', label: 'Project', rows: 'by_project' },
  { id: 'feature', label: 'Feature', rows: 'by_feature' },
  { id: 'role', label: 'Role', rows: 'by_role' },
  { id: 'model', label: 'Model', rows: 'by_model' },
];

export const API_TIP = 'What these tokens would cost at API prices. Your plan is not billed this way.';

/** "≈ $412"; the outside row has no price. */
export const usd = (row) => (row.key === 'outside' || row.cost_usd == null ? '—' : `≈ $${Math.round(row.cost_usd)}`);

/** "+4 pp" with its glyph (▲ over plan, ▼ under). */
export function delta(d) {
  if (d == null) return { text: '—', glyph: '' };
  const r = Math.round(d);
  return { text: `${r > 0 ? '+' : r < 0 ? '−' : ''}${Math.abs(r)} pp`, glyph: r > 0 ? '▲' : r < 0 ? '▼' : '' };
}

export const minutes = (m) =>
  m == null
    ? '—'
    : m >= 60
      ? `${Math.floor(m / 60)} h ${String(Math.round(m % 60)).padStart(2, '0')}`
      : `${Math.round(m)} min`;

/** Hours in each state, in a fixed order, with their share of the bar. */
export function stateBar(states) {
  const order = ['green', 'amber', 'red', 'catch_up'];
  const total = (states ?? []).reduce((a, s) => a + s.minutes, 0);
  return order
    .map((id) => {
      const m = (states ?? []).find((s) => s.state === id)?.minutes ?? 0;
      return { id, hours: Math.round(m / 60), share: total ? m / total : 0 };
    })
    .filter((s) => s.share > 0);
}

/** Governor action kinds in words. */
export const ACTION_LABEL = {
  queue: 'Queued at the live cap',
  live_cap: 'Live cap changed',
  defer_after_reset: 'Deferred (budget ask)',
  downgrade: 'Downgraded model',
  restore_model: 'Restored model',
  release: 'Released from the queue',
  catch_up: 'Catch-up release',
  pause_all: 'Paused all',
  resume_all: 'Resumed all',
  state_change: 'Pressure state changes',
  five_hour_guard: '5h guard',
  run_now: 'Run now (your override)',
  sprint: 'Sprint mode',
};

/** The four KPI tiles of the report header. */
export function kpis(r) {
  const red = (r.states ?? []).find((s) => s.state === 'red')?.minutes ?? 0;
  const acts = (r.actions ?? []).reduce((a, x) => a + x.count, 0);
  const undone = (r.actions ?? []).reduce((a, x) => a + x.undone, 0);
  const overrides = (r.actions ?? []).find((a) => a.kind === 'run_now')?.count ?? 0;
  return [
    {
      id: 'used',
      label: r.in_progress ? 'Used so far' : 'Used at reset',
      value: `${Math.round(r.totals.weekly_pct)}%`,
      note: `agents ${Math.round(r.totals.agents_pp)} pp · you ${Math.round(r.totals.interactive_pp)} pp · outside ${Math.round(r.totals.outside_pp)} pp`,
    },
    {
      id: 'limits',
      label: 'Limit hits',
      value: String((r.limit_hits ?? []).length),
      note: (r.limit_hits ?? []).length
        ? r.limit_hits.map((h) => `${h.window} ${clock(Date.parse(h.ts))}`).join(' · ')
        : 'no window hit 100%',
    },
    {
      id: 'red',
      label: 'Time in Red',
      value: `${Math.round(red / 60)} h`,
      note: `of ${Math.round((r.states ?? []).reduce((a, s) => a + s.minutes, 0) / 60)} h`,
    },
    {
      id: 'actions',
      label: 'Governor actions',
      value: String(acts),
      note: `${overrides} override${overrides === 1 ? '' : 's'} by you · ${undone} undone`,
    },
  ];
}

/** No usage at all this week (the "no data" state). */
export const noData = (r) =>
  !r ||
  (r.totals.weekly_pct === 0 && !(r.by_project ?? []).length && !(r.allocations ?? []).some((a) => a.used_pp > 0));

/** The report as Markdown (`e`). */
export function reportMarkdown(r) {
  const head = `# Week ${weekNo(Date.parse(r.week_start))}${r.in_progress ? ' (so far)' : ''}\n\n${dayTime(Date.parse(r.week_start))} → ${dayTime(Date.parse(r.reset_at))} · ${Math.round(r.totals.weekly_pct)} % ${r.in_progress ? 'so far' : 'at reset'}${r.provenance === 'estimated' ? ' · estimated' : ''}\n`;
  const alloc = [
    '## Plan vs actual\n',
    '| Allocation | Weight | Planned | Used | Delta | Queued p50 / max |',
    '|---|---|---|---|---|---|',
    ...(r.allocations ?? []).map(
      (a) =>
        `| ${a.target || a.id} | ${a.weight} | ${Math.round(a.planned_pp)} pp | ${Math.round(a.used_pp)} pp | ${delta(a.delta_pp).text} | ${minutes(a.queued_p50_min)} / ${minutes(a.queued_max_min)} |`,
    ),
  ].join('\n');
  const tabs = TABS.map((t) =>
    [
      `\n## By ${t.id}\n`,
      `| ${t.label} | Used | Share | ≈ API $ |`,
      '|---|---|---|---|',
      ...(r[t.rows] ?? []).map(
        (x) => `| ${x.label} | ${Math.round(x.pp)} pp | ${Math.round(x.share * 100)} % | ${usd(x)} |`,
      ),
    ].join('\n'),
  ).join('\n');
  return `${head}\n${alloc}\n${tabs}\n\n≈ API $: ${API_TIP}\n`;
}
