// The one "now" (P4.1, api-contract §1–3): attention records, decisions,
// outcomes and counts as the core computed them, loaded from `now_snapshot`
// and kept current by the `now` event's deltas. Surfaces read these and
// derive nothing. Pane views are invalidated by the event's topics: a pane
// refetches its view when its topic arrives, on mount and on window focus.

import { app, decisionPushed } from './app.svelte.js';
import { waited } from './format.js';
import { invoke, listen } from './ipc.js';
import { STATUS } from './status.js';

const ZERO = {
  needs_you: 0,
  decisions: 0,
  running: 0,
  waiting: 0,
  queued: 0,
  idle: 0,
  error: 0,
  live: 0,
  done_today: 0,
};

// A closed decision stays (with its final lifecycle) as long as the core's
// snapshot keeps it: until the next snapshot, at most 10 min.
const CLOSED_MS = 10 * 60000;
const closedAt = new Map(); // decision id → when an event closed it

const byKey = (list, key) => Object.fromEntries((list ?? []).map((r) => [r[key], r]));

/** Replaces the copy with a snapshot (closed decisions not in it are dropped). */
function setSnapshot(s) {
  closedAt.clear();
  app.now = {
    rev: s?.rev ?? 0,
    agents: byKey(s?.agents, 'agent'),
    runs: byKey(s?.runs, 'run'),
    decisions: byKey(s?.decisions, 'id'),
    outcomes: s?.outcomes ?? [],
    counts: s?.counts ?? { global: {}, projects: {} },
  };
}

let loading = null;
// The newest rev an event carried that the copy could not apply (a gap).
let missed = 0;

/** One snapshot read at a time; a call made meanwhile waits for that one.
 *  A gap event newer than the snapshot it got (sent while it was read) reads once more. */
export function loadNow() {
  loading ??= invoke('now_snapshot')
    .then(setSnapshot)
    .catch(() => {})
    .finally(() => {
      loading = null;
      const again = missed > app.now.rev;
      missed = 0;
      if (again) loadNow();
    });
  return loading;
}

/** A `now` event: its deltas, or a resync when a rev was missed. Then its
 *  topics (an old rev's too: what a pane shows is not in the snapshot). */
export function applyNow(ev) {
  const n = app.now;
  if (ev.rev === n.rev + 1) {
    const at = ev.attention ?? {};
    for (const r of at.agents ?? []) n.agents[r.agent] = r;
    for (const r of at.runs ?? []) n.runs[r.run] = r;
    for (const id of at.removed?.agents ?? []) delete n.agents[id];
    for (const id of at.removed?.runs ?? []) delete n.runs[id];
    for (const d of ev.decisions?.upsert ?? []) {
      const prev = n.decisions[d.id];
      n.decisions[d.id] = d;
      closedAt.delete(d.id);
      decisionPushed(prev, d);
    }
    // A closed decision keeps its last upsert (the final lifecycle) for a while.
    const nowMs = Date.now();
    for (const id of ev.decisions?.closed ?? []) closedAt.set(id, nowMs);
    for (const [id, t] of closedAt)
      if (nowMs - t > CLOSED_MS) {
        delete n.decisions[id];
        closedAt.delete(id);
      }
    // Outcomes: today's unseen ones only (a long session would keep every day's).
    if (ev.outcomes?.length) n.outcomes = [...digest(), ...ev.outcomes];
    if (ev.counts) n.counts = ev.counts;
    n.rev = ev.rev;
  } else if (ev.rev > n.rev) {
    missed = Math.max(missed, ev.rev);
    loadNow();
  }
  dispatch(ev.topics ?? []);
}

// Showing the window fires both `visibilitychange` and `focus`: one refetch.
const REFOCUS_MS = 1000;
let refocusedAt = 0;

/** The window is back: a fresh snapshot (pushes may have been missed) and every view refetched. */
function refocus() {
  if (document.hidden || Date.now() - refocusedAt < REFOCUS_MS) return;
  refocusedAt = Date.now();
  loadNow();
  for (const s of subs) if (s.focus) s.run();
}

/** Subscribes to the `now` event and the window coming back, and reads the snapshot. Returns the cleanup. */
export async function connectNow() {
  document.addEventListener('visibilitychange', refocus);
  window.addEventListener('focus', refocus);
  const unlisten = await listen('now', (ev) => applyNow(ev.payload));
  await loadNow();
  return () => {
    document.removeEventListener('visibilitychange', refocus);
    window.removeEventListener('focus', refocus);
    unlisten();
  };
}

// ---------------------------------------------------------------- accessors

export const attention = (agentId) => app.now.agents[agentId] ?? null;
/** The agent's status (§9.5 badge key); `idle` for an agent the core has no record of. */
export const statusOf = (agentId) => attention(agentId)?.status ?? 'idle';
/** The Governor's reason for a queued agent, else '—'. */
export const governorReason = (rec) => (rec?.waiting_on?.kind === 'governor' ? rec.reason : '—');
export const runAttention = (runId) => app.now.runs[runId] ?? null;
/** The run record of a feature's current run. */
export const featureRun = (f) => (f.run ? runAttention(f.run.id) : null);
/** A run's phase: its record's, else (a finished run has no record) the run's own status. */
export const runPhase = (run) => (run ? (runAttention(run.id)?.phase ?? run.status ?? null) : null);

/** Counts: global (no scope), `{ project }` or `{ run }`; zeros when the core has none. */
export function counts(scope = null) {
  const c = scope?.project
    ? app.now.counts.projects?.[scope.project]
    : scope?.run
      ? runAttention(scope.run)?.counts
      : app.now.counts.global;
  return { ...ZERO, ...c };
}

/** "1 needs you", "2 need you" (notes §1.2: every number says what it is). */
export const needYouText = (n) => `${n} need${n === 1 ? 's' : ''} you`;

const SINCE_LEVELS = new Set(['needs_you', 'waiting', 'queued', 'error']);

/** What the record says (notes §1.1): a running agent's activity, else the core's reason. */
export const waitWhat = (rec) =>
  rec?.level === 'running' && !rec.waiting_on ? rec.activity || rec.reason : rec?.reason;
/** How long it has held, for the levels that show since; else ''. */
export const waitSince = (rec, now = Date.now()) => (SINCE_LEVELS.has(rec?.level) ? waited(since(rec), now) : '');

/** The waiting-on line without a badge (tooltips, notes F1): "<status> · <reason> · <since>". */
export function waitLine(rec, now = Date.now()) {
  if (!rec) return '';
  return [STATUS[rec.status]?.label, waitWhat(rec), waitSince(rec, now)].filter(Boolean).join(' · ');
}

/** "5 live / cap 4" (ux-brief §1: every number says what it is). */
export const liveText = (c) => `${c.live} live${c.live_cap != null ? ` / cap ${c.live_cap}` : ''}`;

/** When the record's wait (else its level) began. */
export const since = (rec) => rec?.waiting_on?.since_ms ?? rec?.since_ms ?? null;

/** The agents that need you, longest waiting first. */
export const needsYou = () =>
  Object.values(app.now.agents)
    .filter((r) => r.level === 'needs_you')
    .sort((x, y) => since(x) - since(y));

const startOfDay = (now = Date.now()) => new Date(now).setHours(0, 0, 0, 0);

/** Today's outcomes not yet cleared, newest first (the Inbox digest, level 4). */
export const digest = (now = Date.now()) =>
  app.now.outcomes.filter((o) => !o.seen && o.at_ms >= startOfDay(now)).sort((x, y) => y.at_ms - x.at_ms);

const evNo = (id) => Number(String(id).replace(/^ev-/, '')) || 0;

/** Clear (c): today's outcomes up to the newest shown are seen (`digest_seen`);
 *  they still count in done_today. */
export async function clearDigest(list = digest()) {
  if (!list.length) return;
  const top = list.reduce((m, o) => (evNo(o.id) > evNo(m.id) ? o : m));
  await invoke('digest_seen', { upTo: top.id });
  const upTo = evNo(top.id);
  app.now.outcomes = app.now.outcomes.map((o) => (evNo(o.id) <= upTo ? { ...o, seen: true } : o));
}

/** Decisions still waiting on the user (open or parked), in no order. */
export const openDecisions = () =>
  Object.values(app.now.decisions).filter((d) => d.lifecycle?.state === 'open' || d.lifecycle?.state === 'parked');

// ---------------------------------------------------------------- topics

const subs = new Set();

function dispatch(topics) {
  if (!topics.length) return;
  for (const s of subs) if (topics.some(s.match)) s.run();
}

/**
 * Keeps a view current without polling: `load` runs now (unless `now` is
 * false: the caller loads itself), when a topic it matches arrives (a list of
 * topics, or a predicate) and when the window comes back (unless `focus` is
 * false). Coalesced: at most one load in flight, and one more after it when a
 * topic arrived meanwhile; with `every`, at most one load per `every` ms.
 * Nothing loads after the cleanup. Returns the cleanup.
 */
export function onTopics(match, load, { now = true, focus = true, every = 0 } = {}) {
  const test = typeof match === 'function' ? match : (t) => match.includes(t);
  let inFlight = null;
  let again = false;
  let stopped = false;
  let startedAt = -Infinity;
  let timer = null;
  const run = () => {
    if (stopped) return Promise.resolve();
    if (inFlight || timer) {
      again ||= !timer;
      return inFlight ?? Promise.resolve();
    }
    // `every`: at most one load per `every` ms; a topic sooner loads at the end of it.
    const wait = startedAt + every - Date.now();
    if (wait > 0) {
      timer = setTimeout(() => {
        timer = null;
        run();
      }, wait);
      return Promise.resolve();
    }
    startedAt = Date.now();
    inFlight = Promise.resolve()
      .then(load)
      .catch(() => {})
      .finally(() => {
        inFlight = null;
        if (again) {
          again = false;
          run();
        }
      });
    return inFlight;
  };
  const sub = { match: test, run, focus };
  subs.add(sub);
  if (now) run();
  return () => {
    stopped = true;
    clearTimeout(timer);
    subs.delete(sub);
  };
}
