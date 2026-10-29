// The PR line (ux-brief §1): CI dot and count, the job list, the position badge
// and the "checked" text, from `run.pr` (api-contract §2). Pure: tested in
// test/pr.test.js. The UI never recomputes `ci.overall`: it takes the core's.
import { ago, clock, dayHm } from './format.js';

const OK = 'var(--color-status-done)';
const ERR = 'var(--color-status-error)';
const RUN = 'var(--color-status-queued)';
const IDLE = 'var(--color-status-idle)';
const ATTN = 'var(--color-status-waiting-for-you)';
const MUTED = 'var(--color-text-muted)';

/** The CI dot's color from `ci.overall` (§1.2). */
export const ciColor = (overall) => ({ ok: OK, error: ERR, running: RUN })[overall] ?? IDLE;

// GitHub job state → [word, family]; the family sets the dot (§1.5).
const JOB = {
  failure: ['failed', 'error'],
  cancelled: ['cancelled', 'error'],
  timed_out: ['timed out', 'error'],
  action_required: ['action required', 'error'],
  startup_failure: ['startup failure', 'error'],
  stale: ['stale', 'error'],
  error: ['error', 'error'],
  in_progress: ['running', 'running'],
  queued: ['queued', 'waiting'],
  waiting: ['waiting', 'waiting'],
  requested: ['requested', 'waiting'],
  pending: ['pending', 'waiting'],
  expected: ['expected', 'waiting'],
  success: ['passed', 'ok'],
  neutral: ['neutral', 'ok'],
  skipped: ['skipped', 'ok'],
};

/** Family of a job state: error | running | waiting | ok (unknown values wait). */
export const jobFamily = (state) => JOB[state]?.[1] ?? 'waiting';

/** The word for a job state; anything else is the raw value, `_` → space. */
export const jobWord = (state) => JOB[state]?.[0] ?? String(state ?? '').replaceAll('_', ' ');

/** A job's dot color. */
export const jobColor = (state) => ciColor(jobFamily(state));

/** The state word's color: the dot's for error / running, muted otherwise. */
export const jobWordColor = (state) => (['error', 'running'].includes(jobFamily(state)) ? jobColor(state) : MUTED);

/** `4/6`, or `no checks` when the PR has no job. */
export const ciCount = (ci) => (!ci || ci.overall === 'none' || !ci.total ? 'no checks' : `${ci.done}/${ci.total}`);

const jobs = (ci) => ci?.jobs ?? [];
const countOf = (ci, family) => jobs(ci).filter((j) => jobFamily(j.state) === family).length;

/** The popover's header (§1.5): `CI · 4 of 6 finished · 1 failed · 1 running · 1 waiting`. */
export function ciHeader(ci) {
  if (!ci || ci.overall === 'none' || !ci.total) return 'No CI checks on this PR.';
  const parts = [`CI · ${ci.done} of ${ci.total} finished`];
  const failed = countOf(ci, 'error');
  const running = countOf(ci, 'running');
  const waiting = countOf(ci, 'waiting');
  if (failed) parts.push(`${failed} failed`);
  if (running) parts.push(`${running} running`);
  if (waiting) parts.push(`${waiting} waiting`);
  if (!failed && !running && !waiting && ci.done === ci.total) parts.push('all passed');
  return parts.join(' · ');
}

/** The CI button's accessible name: the state in words (§5). */
export function ciLabel(ci) {
  if (!ci || ci.overall === 'none' || !ci.total) return 'CI: no checks. Show jobs';
  const jobsLine = `${ci.done} of ${ci.total} jobs finished`;
  const failed = countOf(ci, 'error');
  const state =
    ci.overall === 'error'
      ? `${failed || 1} failed`
      : ({ ok: 'passed', running: 'running', waiting: 'waiting' }[ci.overall] ?? ci.overall);
  return `CI: ${state}, ${jobsLine}. Show jobs`;
}

/** The popover's footer: `+N more on GitHub ↗` when the core capped the list. */
export const ciFooter = (ci) =>
  ci && ci.total > jobs(ci).length ? `+${ci.total - jobs(ci).length} more on GitHub ↗` : 'Open checks on GitHub ↗';

/** A job row's text: `workflow / name`, the name alone without a workflow. */
export const jobName = (j) => (j.workflow ? `${j.workflow} / ${j.name}` : j.name);

/** A job row's accessible name: `test, workflow ci, failed`. */
export const jobLabel = (j) => `${j.name}${j.workflow ? `, workflow ${j.workflow}` : ''}, ${jobWord(j.state)}`;

/**
 * The position badge (§1.3): `{ text, short, color, tip }`. `short` is the form
 * under width pressure (the base name moves to the tooltip).
 */
export function positionBadge(position, base = 'main') {
  switch (position?.status) {
    case 'up_to_date':
      return { text: 'up to date', short: 'up to date', color: MUTED, tip: `Contains every commit of ${base}.` };
    case 'behind':
      return {
        text: `↓${position.behind} behind ${base}`,
        short: `↓${position.behind} behind`,
        color: ATTN,
        tip: `${base} has ${position.behind} commit${position.behind === 1 ? '' : 's'} the PR does not. The lead is told and rebases the feature onto ${base}.`,
      };
    case 'conflicts':
      return {
        text: 'conflicts',
        short: 'conflicts',
        color: ERR,
        tip: `The PR conflicts with ${base}. The lead is told and resolves them in a rebase.`,
      };
    default:
      return {
        text: '—',
        short: '—',
        color: MUTED,
        tip: `GitHub is still computing the PR's position against ${base}.`,
      };
  }
}

/** `↑2 to push`, short `↑2`; null when nothing is unpushed. */
export function toPush(n) {
  if (!(n > 0)) return null;
  return {
    text: `↑${n} to push`,
    short: `↑${n}`,
    tip: `${n} commit${n === 1 ? '' : 's'} on the integration branch not in the PR yet. The lead asks a push gate with request_pr.`,
  };
}

export const REBASED_TIP =
  "The branch was rebased onto main. The next push replaces the PR's history (force-with-lease).";

/**
 * `checked 2m ago` · `checked just now` · `checked Tue 14:58` (> 24 h); stale:
 * `last checked 12m ago`. `short` is `2m ago`; `clock` goes to the tooltip.
 */
export function checkedText(ms, { now = Date.now(), stale = false } = {}) {
  if (ms == null) return null;
  const old = now - ms >= 86400000;
  const rel = old ? dayHm(ms) : now - ms < 60000 ? 'just now' : `${ago(ms, now)} ago`;
  const prefix = stale ? 'last checked' : 'checked';
  return { text: `${prefix} ${rel}`, short: rel, clock: `checked ${clock(ms, now)}` };
}

/** The PR line's shape from the run: which of the brief's states (§1.4) it is in. */
export function lineState(run) {
  if (!run?.pr_number && !run?.pr_url) return 'none';
  const pr = run.pr;
  // Not watched yet (a PR open, first check coming) vs. never watched (a run done before this change).
  if (!pr)
    return (run.status === 'pr_open' || run.resume_status === 'pr_open') && run.pr_number ? 'checking' : 'legacy';
  // A first check that failed leaves a status with no state yet (`error` set): shown as open and stale.
  return pr.state === 'merged' || pr.state === 'closed' ? pr.state : 'open';
}

/** Toast when a refresh fails (the contract has no offline code: the core's text is shown). */
export const refreshError = (run, err) => `Couldn't check PR #${run?.pr_number ?? ''}: ${err}`;
