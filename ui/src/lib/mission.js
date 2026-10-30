// Mission control (FR-1302, P4.2 spec §3.2, notes p4-2 §1–3): the lanes as a
// render of `now`. Pure: tested in test/mission.test.js. Nothing here reads
// the app state or the clock; the pane passes `now` and the time.

import { clock, dayHm, plural, startOfDay, waited } from './format.js';
import { agentOf, isOpen } from './inbox.js';
import { STATUS } from './status.js';

// ---------------------------------------------------------------- phase stepper

/** The six steps (notes §1.2), in order. */
const STEPS = [
  { id: 'draft', label: 'Draft' },
  { id: 'spec', label: 'Spec' },
  { id: 'planning', label: 'Planning' },
  { id: 'executing', label: 'Executing' },
  { id: 'integrating', label: 'Integrating' },
  { id: 'pr', label: 'PR' },
];

/** A run phase's step index; gates sit on their step. */
const STEP_OF = { draft: 0, spec: 1, spec_gate: 1, planning: 2, executing: 3, integrating: 4, pr_gate: 5, pr_open: 5 };
const HELD = new Set(['paused', 'deferred', 'failed']);

/** Run phase glyphs and colors (docs/design/README.md, `color.phase.*`). */
const PHASE = {
  draft: { icon: '\u{F0CB6}', color: 'var(--color-phase-draft)' },
  spec: { icon: '\u{F09EE}', color: 'var(--color-phase-spec)' },
  spec_gate: { icon: '\u{F0E29}', color: 'var(--color-phase-spec-gate)' },
  planning: { icon: '\u{F199C}', color: 'var(--color-phase-planning)' },
  executing: { icon: '\u{F1936}', color: 'var(--color-phase-executing)' },
  integrating: { icon: '\u{F062D}', color: 'var(--color-phase-integrating)' },
  pr_gate: { icon: '\u{F04C2}', color: 'var(--color-phase-pr-gate)' },
  pr_open: { icon: '\u{F04C2}', color: 'var(--color-phase-pr-gate)' },
  done: { icon: '\u{F023C}', color: 'var(--color-phase-done)' },
  paused: { icon: '\u{F03E6}', color: 'var(--color-phase-paused)' },
  deferred: { icon: '\u{F00F0}', color: 'var(--color-phase-deferred)' },
  failed: { icon: '\u{F0029}', color: 'var(--color-phase-failed)' },
};

/** Lower-case phase words for the text next to the stepper. */
const PHASE_WORD = {
  draft: 'draft',
  spec: 'spec',
  spec_gate: 'spec gate',
  planning: 'planning',
  executing: 'executing',
  integrating: 'integrating',
  pr_gate: 'PR gate',
  pr_open: 'PR',
  paused: 'paused',
  deferred: 'deferred',
  failed: 'failed',
  done: 'done',
};

/**
 * The stepper of a run record: each step done, current or todo; the
 * current step's glyph and color; its mark (at gate, paused, deferred,
 * failed); the text equivalent "phase 4 of 6: executing". A held run
 * without `held_in` has no current step.
 */
export function stepper(run) {
  const phase = run?.phase;
  const held = HELD.has(phase);
  const at = STEP_OF[held ? run?.held_in : phase] ?? -1;
  const mark = held ? phase : phase === 'spec_gate' || phase === 'pr_gate' ? 'at gate' : null;
  const look = PHASE[phase] ?? PHASE.draft;
  const steps = STEPS.map((s, i) => ({
    ...s,
    state: at < 0 ? 'todo' : i < at ? 'done' : i === at ? 'current' : 'todo',
    ...(i === at ? { icon: look.icon, color: look.color, mark } : {}),
  }));
  // The step in the phase's words ("PR gate", "PR"); a held run adds how it is held.
  const word = PHASE_WORD[held ? run?.held_in : phase] ?? phase;
  const aria =
    at < 0 ? `phase: ${PHASE_WORD[phase] ?? phase}` : `phase ${at + 1} of 6: ${held ? `${word}, ${phase}` : word}`;
  return { steps, current: at, mark, aria };
}

/** The row's phase text (notes §1.2, run-row frame): "executing · 2 h", "paused · 40 min", "deferred · until Mon 09:00". */
export function phaseText(run, now) {
  const p = run?.phase;
  if (p === 'deferred') return run.until_ms ? `deferred · until ${dayHm(run.until_ms)}` : 'deferred';
  return [PHASE_WORD[p] ?? p, waited(run?.phase_since_ms, now)].filter(Boolean).join(' · ');
}

/** The phase in words with the step a held run left: "executing", "paused · executing". */
export function phaseName(run) {
  const p = run?.phase;
  const word = PHASE_WORD[p] ?? p;
  return HELD.has(p) && run.held_in ? `${word} · ${PHASE_WORD[run.held_in] ?? run.held_in}` : word;
}

/** The full phase text (tooltip, aria, Inspector): "paused · executing · 40 min". */
export function phaseFull(run, now) {
  const p = run?.phase;
  if (!HELD.has(p) || !run.held_in) return phaseText(run, now);
  const step = PHASE_WORD[run.held_in] ?? run.held_in;
  if (p === 'deferred')
    return [`deferred · ${step}`, run.until_ms ? `until ${dayHm(run.until_ms)}` : null].filter(Boolean).join(' · ');
  return [p, step, waited(run.phase_since_ms, now)].filter(Boolean).join(' · ');
}

/** "3/7 tasks done", "no tasks yet" (notes §1.3). */
export const tasksText = (t) => (!t?.total ? 'no tasks yet' : `${t.done}/${t.total} tasks done`);

/** Inspector: "7 tasks · 3 done · 2 in progress · 1 in review · 1 blocked" (zero parts left out). */
export function tasksDetail(t) {
  if (!t?.total) return 'no tasks yet';
  const parts = [
    [t.done, 'done'],
    [t.in_progress, 'in progress'],
    [t.review, 'in review'],
    [t.blocked, 'blocked'],
  ].filter(([n]) => n);
  return [plural(t.total, 'task'), ...parts.map(([n, w]) => `${n} ${w}`)].join(' · ');
}

/** "PR #12 ↗" ("PR ↗" without a number). */
export const prText = (pr) => (pr ? `PR${pr.number != null ? ` #${pr.number}` : ''} ↗` : '');

// ---------------------------------------------------------------- records

/** When the record's wait (else its level) began. */
const sinceOf = (rec) => rec?.waiting_on?.since_ms ?? rec?.since_ms ?? 0;
/** When a run's wait began: its blocker's, else its phase's. */
const runSince = (run) => run?.blocker?.waiting_on?.since_ms ?? run?.phase_since_ms ?? 0;

/** The reason's first clause: "permission: Bash …" → "permission", "→ back-1 · API contract" → "→ back-1". */
const firstClause = (s) =>
  String(s ?? '')
    .split(' · ')[0]
    .split(':')[0]
    .trim();
const cut = (s, n = 32) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

/**
 * A member chip's short wait (notes §1.3, frames): "Edit checkout.ts",
 * "permission · 2 min", "→ back-1 · 12 min", "not started", "queued ≈14:20".
 */
export function shortWait(rec, now) {
  if (!rec) return '';
  const since = waited(rec.waiting_on?.since_ms ?? rec.since_ms, now);
  switch (rec.status) {
    case 'not_started':
      return 'not started';
    case 'queued':
      return rec.queue?.eta_ms ? `queued ≈${clock(rec.queue.eta_ms, now)}` : 'queued';
    case 'paused':
    case 'suspended':
    case 'done':
      return rec.status;
    case 'idle':
      // An idle member may still wait on a teammate or a task.
      if (!rec.waiting_on) return 'idle';
      break;
    case 'error':
      return `error · ${since}`;
    case 'blocked_by_limit':
      return cut(rec.reason ?? 'limit');
  }
  if (!rec.waiting_on) return cut(rec.activity || rec.reason || STATUS[rec.status]?.label.toLowerCase() || '');
  return cut([firstClause(rec.reason) || rec.waiting_on.kind, since].filter(Boolean).join(' · '));
}

/** A run row's default target (D6): its blocker's agent, else its lead, else its first member. */
export const defaultTarget = (run) => run?.blocker?.agent ?? run?.lead ?? run?.members?.[0] ?? null;

/** The agent a row's actions act on: the selected chip (an agent id, while still a member), else the
 *  default; a solo or queue row's own agent. Outcome and decision rows have none (only `o` / `d` act there). */
export function target(row, chip = null) {
  if (row?.kind === 'run') return (chip && row.run.members.includes(chip) ? chip : null) ?? defaultTarget(row.run);
  if (row?.kind === 'solo' || row?.kind === 'queue') return row.agent.agent;
  return null;
}

/** What the target's record allows (notes §3.1): interrupt a live turn, suspend an idle or waiting
 *  agent, resume a paused, suspended or failed one; pause and kill any. */
export function allowed(rec) {
  if (!rec) return {};
  const turn = rec.live && !['idle', 'suspended', 'done', 'paused', 'not_started', 'queued'].includes(rec.status);
  return {
    i: !!turn,
    p: true,
    s: !!rec.live && ['idle', 'waiting', 'needs_you'].includes(rec.level),
    r: ['paused', 'suspended', 'error'].includes(rec.status),
  };
}

/** The decision a row opens with `d`: the target's wait on the user, else the run's blocker's. */
export function decisionOf(row, rec) {
  if (row?.kind === 'decisions') return row.ids[0] ?? null;
  const user = (w) => (w?.kind === 'user' ? w.ref : null);
  return user(rec?.waiting_on) ?? (row?.kind === 'run' ? user(row.run.blocker?.waiting_on) : null);
}

// ---------------------------------------------------------------- queue

const pp1 = (v) => Number(v).toFixed(1);

/** "≈ +2.1 pp week" / "≈ +6 pp 5h" in the window that binds it (notes §1.5); '' without a figure. */
export function runNowCost(q) {
  const r = q?.run_now;
  if (!r) return '';
  if (r.binds === 'five_hour') return r.five_hour_pp == null ? '' : `≈ +${Math.round(r.five_hour_pp)} pp 5h`;
  return r.week_pp == null ? '' : `≈ +${pp1(r.week_pp)} pp week`;
}

/** The Run now / Start now confirmation (notes §1.12, FR-809). */
export function runNowDialog(rec) {
  const fresh = rec.status === 'not_started';
  const verb = fresh ? 'Start' : 'Run';
  const r = rec.queue?.run_now ?? {};
  const costs = [
    r.week_pp != null && `≈ +${pp1(r.week_pp)} pp of the week`,
    r.five_hour_pp != null && `≈ +${Math.round(r.five_hour_pp)} pp of the 5h window`,
  ].filter(Boolean);
  const lines = [`It is ${fresh ? 'not started' : 'queued'}${rec.reason ? `: ${rec.reason}` : ''}.`];
  if (costs.length) {
    lines.push(`${verb === 'Start' ? 'Starting' : 'Running'} it now costs ${costs.join(' and ')}.`);
    lines.push(
      r.binds === 'five_hour'
        ? 'The 5h window holds it: that is the window this is counted against.'
        : 'The week holds it: that is the window this is counted against.',
    );
  }
  // FR-809: held by the cap, it runs above it.
  if (/cap/.test(rec.reason ?? ''))
    lines.push(costs.length ? 'It runs above the live cap.' : 'Running it now uses a slot above the cap.');
  const cost = runNowCost(rec.queue);
  return { title: `${verb} ${rec.agent} now?`, detail: lines.join('\n'), yes: `${verb} now${cost ? ` ${cost}` : ''}` };
}

// ---------------------------------------------------------------- lanes

const UNIT = { run: 'run', solo: 'session', queue: 'agent', outcome: 'outcome' };

/** A lane header's count: its rows with their unit (D11): "1 run · 1 session", "2 agents". */
export function rowCount(rows) {
  const n = {};
  for (const r of rows) if (UNIT[r.kind]) n[r.kind] = (n[r.kind] ?? 0) + 1;
  return Object.keys(UNIT)
    .filter((k) => n[k])
    .map((k) => plural(n[k], UNIT[k]))
    .join(' · ');
}

/** The lanes, top to bottom (spec §3.2); `level` gives the header's icon. */
const LANES = [
  { id: 'needs_you', label: 'Needs you', level: 'needs_you' },
  { id: 'running', label: 'Running', level: 'running' },
  { id: 'waiting', label: 'Waiting on', level: 'waiting' },
  { id: 'queued', label: 'Queued', level: 'queued' },
  { id: 'done', label: 'Done today', level: 'done' },
];

const LANE_OF = {
  needs_you: 'needs_you',
  error: 'needs_you',
  running: 'running',
  waiting: 'waiting',
  queued: 'queued',
  idle: 'idle',
};

/** Queued rows: the Governor's order, then not started (no entry), then paused; oldest first within. */
function queueRank(rec) {
  if (rec.queue?.position != null) return [0, rec.queue.position];
  return [rec.status === 'paused' ? 2 : 1, sinceOf(rec)];
}

const byText = new Intl.Collator().compare;

// A row is kept while its record (and its run) is the same object: a `now`
// event re-renders only the rows it changed (NFR-1b).
const rowCache = new WeakMap();
function rowOf(rec, kind, make, run = null) {
  const hit = rowCache.get(rec)?.[kind];
  if (hit && (hit.run ?? null) === run) return hit;
  const row = make();
  const slot = rowCache.get(rec) ?? {};
  slot[kind] = row;
  rowCache.set(rec, slot);
  return row;
}

/**
 * Mission control's rows from `now` ({agents, runs, decisions, outcomes},
 * keyed as the store keeps them) and the project filter (null = all).
 * Returns { lanes: [{id, label, level, header, count, rows}], idle: {header,
 * count, rows} | null }. Empty lanes are left out except Needs you (D11).
 */
export function lanes(now, filter = null, at = Date.now()) {
  // A record with no project (a decision about the app) shows under any filter.
  const shown = (project) => !filter || project == null || filter.includes(project);
  const runs = now?.runs ?? {};
  const agents = Object.values(now?.agents ?? {});
  const rows = { needs_you: [], running: [], waiting: [], queued: [], idle: [] };
  const errorRows = [];

  // Runs that are rows; a member of any other run (done, or at level done) is a solo row.
  const rowRuns = new Set();
  for (const r of Object.values(runs)) {
    const lane = LANE_OF[r.level];
    if (r.phase === 'done' || !lane) continue;
    rowRuns.add(r.run);
    if (!shown(r.project)) continue;
    const row = rowOf(
      r,
      'run',
      () => ({ kind: 'run', key: `run:${r.run}`, run: r, project: r.project, since: runSince(r) }),
      r,
    );
    (r.level === 'error' ? errorRows : rows[lane]).push(row);
  }
  for (const a of agents) {
    if (!shown(a.project)) continue;
    // A member of a run with a record is in that run's row (D4).
    const solo = !a.run || !rowRuns.has(a.run);
    if (a.level === 'queued') {
      const run = solo ? null : runs[a.run];
      const make = () => ({
        kind: 'queue',
        key: `queue:${a.agent}`,
        agent: a,
        run,
        project: a.project,
        since: sinceOf(a),
      });
      rows.queued.push(rowOf(a, 'queue', make, run));
      continue;
    }
    if (!solo) continue;
    const lane = LANE_OF[a.level];
    if (!lane) continue;
    const row = rowOf(a, 'solo', () => ({
      kind: 'solo',
      key: `agent:${a.agent}`,
      agent: a,
      project: a.project,
      since: sinceOf(a),
    }));
    (a.level === 'error' ? errorRows : rows[lane]).push(row);
  }

  const oldest = (x, y) => x.since - y.since || byText(x.key, y.key);
  rows.needs_you.sort(oldest);
  errorRows.sort(oldest);
  rows.needs_you.push(...errorRows);
  rows.running.sort((x, y) => byText(String(x.project), String(y.project)) || oldest(x, y));
  rows.waiting.sort(oldest);
  const runRows = rows.queued.filter((r) => r.kind === 'run').sort(oldest);
  const queueRows = rows.queued
    .filter((r) => r.kind === 'queue')
    .sort((x, y) => {
      const [a, b] = [queueRank(x.agent), queueRank(y.agent)];
      return a[0] - b[0] || a[1] - b[1] || byText(x.key, y.key);
    });
  rows.queued = [...runRows, ...queueRows];
  rows.idle.sort(oldest);

  // Open decisions no Needs-you row stands for (no record, or its agent's row is elsewhere):
  // one line in Needs you (D3). A run row stands for every member's.
  const covered = new Set();
  for (const r of rows.needs_you) {
    if (r.kind === 'run') for (const m of r.run.members) covered.add(m);
    else covered.add(r.agent.agent);
  }
  const loose = Object.values(now?.decisions ?? {})
    .filter((d) => isOpen(d) && !covered.has(agentOf(d)) && shown(d.subject?.project ?? null))
    .sort((x, y) => (x.created_ms ?? 0) - (y.created_ms ?? 0));
  const needs = [...rows.needs_you];
  if (loose.length)
    needs.push({ kind: 'decisions', key: 'decisions', ids: loose.map((d) => d.id), count: loose.length });

  const day = startOfDay(at);
  const done = (now?.outcomes ?? [])
    .filter((o) => o.at_ms >= day && shown(o.project ?? null))
    .sort((x, y) => y.at_ms - x.at_ms)
    .map((o) =>
      rowOf(o, 'outcome', () => ({ kind: 'outcome', key: `outcome:${o.id}`, outcome: o, project: o.project })),
    );

  const byLane = { needs_you: needs, running: rows.running, waiting: rows.waiting, queued: rows.queued, done };
  const out = LANES.map((l) => {
    const count = rowCount(byLane[l.id]);
    return { ...l, rows: byLane[l.id], count, header: count ? `${l.label} · ${count}` : l.label };
  }).filter((l) => l.id === 'needs_you' || l.rows.length);
  const idleCount = rowCount(rows.idle);
  return {
    lanes: out,
    idle: rows.idle.length
      ? { id: 'idle', label: 'Idle', level: 'idle', rows: rows.idle, count: idleCount, header: `Idle · ${idleCount}` }
      : null,
  };
}

/** "2 decisions not tied to a session" (notes §1.1). */
export const looseText = (n) => `${plural(n, 'decision')} not tied to a session`;

/** Done today shows this many rows, then "Show all n". */
export const DONE_ROWS = 10;

/**
 * The j/k stops, in order (notes §3.1): rows of open lanes, one stop for
 * a collapsed lane, the Show all button, the Idle line (always) and its
 * rows when open. `view`: { collapsed: Set|{[id]: bool}, idleOpen, showAll }.
 */
export function stops(model, view = {}) {
  const collapsed = (id) => !!view.collapsed?.[id];
  const out = [];
  for (const l of model.lanes) {
    if (collapsed(l.id)) {
      out.push(`lane:${l.id}`);
      continue;
    }
    const rows = l.id === 'done' && !view.showAll ? l.rows.slice(0, DONE_ROWS) : l.rows;
    out.push(...rows.map((r) => r.key));
    if (l.id === 'done' && l.rows.length > DONE_ROWS) out.push('showall');
  }
  if (model.idle) {
    out.push('lane:idle');
    if (view.idleOpen) out.push(...model.idle.rows.map((r) => r.key));
  }
  return out;
}

/** Every row by key, the Idle line's included. */
export function rowIndex(model) {
  const m = new Map();
  for (const l of model.lanes) for (const r of l.rows) m.set(r.key, r);
  for (const r of model.idle?.rows ?? []) m.set(r.key, r);
  return m;
}

// ---------------------------------------------------------------- toolbar, footer

/** The filter button (notes §1.9): "All projects", "shop-api", "2 projects of 4". */
export function filterLabel(filter, projects) {
  if (!filter) return 'All projects';
  if (filter.length === 1) return projects.find((p) => p.id === filter[0])?.name ?? filter[0];
  return `${filter.length} projects of ${projects.length}`;
}

/** A filter after toggling `id` (notes §1.9): the last project cannot be unchecked; all checked = null. */
export function toggleProject(filter, id, all) {
  const cur = filter ?? all;
  const next = cur.includes(id) ? cur.filter((x) => x !== id) : [...cur, id];
  if (!next.length) return filter;
  return all.every((x) => next.includes(x)) ? null : all.filter((x) => next.includes(x));
}

export const COUNT_KEYS = ['needs_you', 'error', 'running', 'waiting', 'queued', 'idle', 'live', 'done_today'];

/** The core's per-project counts summed over the filter (D8); the global counts with no filter. */
function filteredCounts(counts, filter) {
  if (!filter) return { ...counts?.global };
  const sum = Object.fromEntries(COUNT_KEYS.map((k) => [k, 0]));
  for (const id of filter) for (const k of COUNT_KEYS) sum[k] += counts?.projects?.[id]?.[k] ?? 0;
  // The live cap is global: its figures stay global under a filter.
  return { ...sum, live: counts?.global?.live, live_cap: counts?.global?.live_cap };
}

/** "1 needs you · 2 running · 1 queued" from counts, zero parts left out (P4.1 §1.2). */
export function countParts(c, { running = false } = {}) {
  const parts = [];
  if (c.needs_you) parts.push(`${c.needs_you} need${c.needs_you === 1 ? 's' : ''} you`);
  if (c.error) parts.push(plural(c.error, 'error'));
  if (c.running || running) parts.push(`${c.running ?? 0} running`);
  if (c.waiting) parts.push(`${c.waiting} waiting`);
  if (c.queued) parts.push(`${c.queued} queued`);
  return parts;
}

/** The footer's left part (notes §1.8, frames): counts, then "live cap 2/2", then the filter. */
export function footerText(counts, filter, projects) {
  if (!projects.length) return 'No projects';
  const c = filteredCounts(counts, filter);
  const live = c.live_cap != null ? `live cap ${c.live ?? 0}/${c.live_cap}` : `${c.live ?? 0} live`;
  return [...countParts(c, { running: true }), live, filter && filter.length > 1 ? filterLabel(filter, projects) : null]
    .filter(Boolean)
    .join(' · ');
}

/** "Hiding blog, infra · 3 running, 1 queued" (notes §1.9) for the projects a filter leaves out. */
export function hiddenText(counts, filter, projects) {
  if (!filter) return '';
  const hidden = projects.filter((p) => !filter.includes(p.id));
  if (!hidden.length) return '';
  const c = filteredCounts(
    counts,
    hidden.map((p) => p.id),
  );
  return [`Hiding ${hidden.map((p) => p.name).join(', ')}`, ...countParts(c)].join(' · ');
}

/**
 * The toolbar's key hints for the row at hand (notes §1.9, lead's fix 3):
 * [[keys, label]], hidden ones dropped. `rec` is the target's record.
 */
export function hints(row, { target: t = null, rec = null, decision = null } = {}) {
  const h = [['j k', 'move']];
  if (!row) return [...h, ['f', 'filter']];
  if (row.kind === 'outcome') return [...h, ['o', 'open'], ['f', 'filter']];
  if (row.kind === 'decisions') return [...h, ['⏎', 'open in Inbox'], ['f', 'filter']];
  if (t) h.push(['⏎', `open ${t}`]);
  if (row.kind === 'queue') {
    const fresh = rec?.status === 'not_started';
    if (rec?.queue) h.push(['n', fresh ? 'start now' : 'run now'], ['⌫', 'cancel']);
    if (rec?.status === 'paused') h.push(['r', 'resume']);
  } else if (t && rec) {
    const ok = allowed(rec);
    const keys = ['i', 'p', 's', 'r'].filter((k) => ok[k]);
    h.push([[...keys, '⇧K'].join(' '), t]);
  }
  if (decision) h.push(['d', 'decision']);
  if (row.kind === 'run') {
    h.push(['h l', 'chips']);
    if (row.run.pr) h.push(['o', 'PR']);
  }
  h.push(['space', 'details'], ['f', 'filter']);
  return h;
}
