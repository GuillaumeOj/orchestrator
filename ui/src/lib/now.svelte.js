// The one "now" (P4.1, api-contract §1–3): attention records, decisions,
// outcomes and counts as the core computed them, loaded from `now_snapshot`
// and kept current by the `now` event's deltas. Surfaces read these and
// derive nothing. Pane views are invalidated by the event's topics: a pane
// refetches its view when its topic arrives, on mount and on window focus.

import { app } from './app.svelte.js';
import { invoke, listen } from './ipc.js';

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
export function setSnapshot(s) {
  closedAt.clear();
  app.now = {
    rev: s?.rev ?? 0,
    agents: byKey(s?.agents, 'agent'),
    runs: byKey(s?.runs, 'run'),
    decisions: byKey(s?.decisions, 'id'),
    outcomes: s?.outcomes ?? [],
    counts: s?.counts ?? { global: { ...ZERO }, projects: {} },
  };
}

let loading = null;

/** One snapshot read at a time; a call made meanwhile waits for that one. */
export function loadNow() {
  loading ??= invoke('now_snapshot')
    .then(setSnapshot)
    .catch(() => {})
    .finally(() => (loading = null));
  return loading;
}

/** A `now` event: its deltas, or a resync when a rev was missed. Then its topics. */
export function applyNow(ev) {
  const n = app.now;
  if (ev.rev <= n.rev) return;
  if (ev.rev !== n.rev + 1) loadNow();
  else {
    const at = ev.attention ?? {};
    for (const r of at.agents ?? []) n.agents[r.agent] = r;
    for (const r of at.runs ?? []) n.runs[r.run] = r;
    for (const id of at.removed?.agents ?? []) delete n.agents[id];
    for (const id of at.removed?.runs ?? []) delete n.runs[id];
    for (const d of ev.decisions?.upsert ?? []) {
      n.decisions[d.id] = d;
      closedAt.delete(d.id);
    }
    // A closed decision keeps its last upsert (the final lifecycle) for a while.
    const nowMs = Date.now();
    for (const id of ev.decisions?.closed ?? []) closedAt.set(id, nowMs);
    for (const [id, t] of closedAt)
      if (nowMs - t > CLOSED_MS) {
        delete n.decisions[id];
        closedAt.delete(id);
      }
    if (ev.outcomes?.length) n.outcomes = [...n.outcomes, ...ev.outcomes];
    if (ev.counts) n.counts = ev.counts;
    n.rev = ev.rev;
  }
  dispatch(ev.topics ?? []);
}

/**
 * Subscribes to the `now` event and reads the snapshot. A window shown again
 * reads a fresh snapshot (pushes may have been missed) and refetches every
 * subscribed view. Returns the cleanup.
 */
export async function connectNow() {
  const onVisible = () => {
    if (document.hidden) return;
    loadNow();
    for (const s of subs) s.run();
  };
  document.addEventListener('visibilitychange', onVisible);
  const unlisten = await listen('now', (ev) => applyNow(ev.payload));
  await loadNow();
  return () => {
    document.removeEventListener('visibilitychange', onVisible);
    unlisten();
  };
}

// ---------------------------------------------------------------- accessors

export const attention = (agentId) => app.now.agents[agentId] ?? null;
export const runAttention = (runId) => app.now.runs[runId] ?? null;
/** The run record of a feature: by its run id, else by slug in its project. */
export const featureRun = (f) =>
  Object.values(app.now.runs).find((r) =>
    f.run?.id != null ? r.run === f.run.id : r.feature === f.slug && r.project === f.project_id,
  ) ?? null;

/** Counts: global (no scope), `{ project }` or `{ run }`; zeros when the core has none. */
export function counts(scope = null) {
  const c = scope?.project
    ? app.now.counts.projects?.[scope.project]
    : scope?.run
      ? runAttention(scope.run)?.counts
      : app.now.counts.global;
  return { ...ZERO, ...c };
}

/** When the record's wait (else its level) began. */
export const since = (rec) => rec?.waiting_on?.since_ms ?? rec?.since_ms ?? null;

/** The agents that need you, longest waiting first. */
export const needsYou = () =>
  Object.values(app.now.agents)
    .filter((r) => r.level === 'needs_you')
    .sort((x, y) => since(x) - since(y));

/** Decisions still waiting on the user (open or parked). */
export const openDecisions = () =>
  Object.values(app.now.decisions)
    .filter((d) => d.lifecycle?.state === 'open' || d.lifecycle?.state === 'parked')
    .sort((x, y) => x.level - y.level || x.created_ms - y.created_ms);

// ---------------------------------------------------------------- topics

const subs = new Set();

function dispatch(topics) {
  if (!topics.length) return;
  for (const s of subs) if (topics.some(s.match)) s.run();
}

/**
 * Keeps a view current without polling: `load` runs now (unless `now` is
 * false: the caller loads itself), when a topic it matches arrives (a list of
 * topics, or a predicate) and on window focus. Coalesced: at most one load in
 * flight, and one more after it when a topic arrived meanwhile. Returns the
 * cleanup.
 */
export function onTopics(match, load, { now = true } = {}) {
  const test = typeof match === 'function' ? match : (t) => match.includes(t);
  let inFlight = null;
  let again = false;
  const run = () => {
    if (inFlight) {
      again = true;
      return inFlight;
    }
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
  const sub = { match: test, run };
  subs.add(sub);
  const onFocus = () => run();
  if (typeof window !== 'undefined') window.addEventListener('focus', onFocus);
  if (now) run();
  return () => {
    subs.delete(sub);
    if (typeof window !== 'undefined') window.removeEventListener('focus', onFocus);
  };
}
