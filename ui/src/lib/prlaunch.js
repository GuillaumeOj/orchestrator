// Launching a team on a PR (allow-launch-team-on-update-pr, notes §1–§2, api-contract §1–§8): the PR row's
// copy and state, the dialog's error texts. Pure; tested in test/prlaunch.test.js. Text from GitHub is
// untrusted: it is shown as plain text only.
import { errText } from './ipc.js';
import { parseCoded } from './issues.js';
import { reviewWord } from './pr.js';

/** The author as shown: `app/` stripped (F13), `unknown author` for a null one (F9). */
export const authorName = (a) => (a?.login ? a.login.replace(/^app\//, '') : 'unknown author');

/** The row's state: launch | active | fork | none (an older core without `launch`). */
export function rowState(pr) {
  if (!pr?.launch) return 'none';
  if (pr.launch.ok) return 'launch';
  return pr.launch.code === 'active_feature' ? 'active' : pr.launch.code === 'fork' ? 'fork' : 'none';
}

export const FORK_REASON = (owner) =>
  owner ? `Fork PR (from ${owner}): the app pushes to origin only.` : 'Fork PR: the app pushes to origin only.';

/** The fork reason: the core's when it gave one, else the default with `fork_owner` (F13). */
export const forkReason = (pr) => FORK_REASON(pr?.fork_owner);

/** Line 2's pieces after the branches: CI, the review word (no "no review" piece, §9). */
export const reviewText = (pr) => reviewWord(pr.review_decision)?.text ?? '';
/** The PR row's CI text from `checks` (counts), '' without checks. */
export const ciText = (c) =>
  !c ? '' : c.failing ? `${c.failing} failing` : c.pending ? `${c.pending} pending` : c.passing ? 'passing' : '';

/** `Earlier team: slug · done` / `Team on this PR: slug · executing` (without a status, the slug alone). */
export const featureLine = (pr, kind) => {
  const f = pr.feature;
  if (!f) return '';
  const head = kind === 'active' ? 'Team on this PR' : 'Earlier team';
  return `${head}: ${f.slug}${f.run_status ? ` · ${f.run_status}` : ''}`;
};

/** The row patched from a `team_launch` result (F8): now an active team. */
export function patchRow(pr, feature) {
  if (!feature) return pr;
  return { ...pr, feature, launch: { ok: false, code: 'active_feature', reason: 'This PR already has a team' } };
}

/** A long branch cut in the middle (> 48 chars). */
export const midCut = (s, max = 48) => (s && s.length > max ? `${s.slice(0, 22)}…${s.slice(-(max - 23))}` : (s ?? ''));

/** Prefill error → { text, again, open } (§2.6). `pr` is { number }. */
export function prefillError(e, number) {
  const code = e?.code ?? null;
  const msg = e?.message ?? '';
  switch (code) {
    case 'pr_closed':
      return { text: `PR #${number} was closed on GitHub. A team can't work on it.` };
    case 'pr_merged':
      return { text: `PR #${number} was merged on GitHub. There is nothing left to fix.` };
    case 'fork':
      return {
        text: msg || `PR #${number} comes from a fork. The app pushes to origin only, so it can't launch a team on it.`,
      };
    case 'active_feature': {
      const f = e.feature;
      return {
        text: f ? `PR #${number} already has a team: ${f.slug}${f.run_status ? ` (${f.run_status})` : ''}.` : msg,
        open: f ?? null,
      };
    }
    case 'gh_missing':
      return {
        text: "The GitHub CLI (gh) isn't installed. Install it, run gh auth login, then try again.",
        again: true,
        cmd: true,
      };
    case 'gh_auth':
      return {
        text: "gh isn't signed in to GitHub. Run gh auth login in a terminal, then try again.",
        again: true,
        cmd: true,
      };
    case 'offline':
      return { text: "Couldn't reach GitHub. Check your connection and try again.", again: true };
    default:
      return { text: `Couldn't read PR #${number}: ${msg}`, again: true };
  }
}

/**
 * A launch error → { text, keep, open }: `keep` leaves Launch enabled (the fixable ones), `open` is the
 * feature to offer (§2.7, F1, F10, F11). `home` shortens paths; `pr` is the dialog's PR.
 */
export function launchError(e, pr, shorten = (p) => p) {
  const code = e?.code ?? null;
  const msg = e?.message ?? '';
  const d = e?.details ?? {};
  const head = pr?.head_ref ?? '';
  const n = pr?.number;
  switch (code) {
    case 'branch_checked_out':
      return {
        keep: true,
        text: d.path
          ? `${head} is checked out in ${shorten(d.path)}${d.feature ? ` (feature ${d.feature.slug ?? d.feature})` : ''}: ${d.feature ? 'delete that feature first' : 'switch that checkout to another branch'}, then launch again.`
          : msg,
      };
    case 'local_ahead':
      return {
        keep: true,
        text:
          d.commits != null
            ? `Local ${head} has ${d.commits} commit${d.commits === 1 ? '' : 's'} PR #${n} does not have. Rename or delete that local branch first.`
            : msg,
      };
    case 'fetch_failed':
      return {
        keep: true,
        text: /^could not/i.test(msg)
          ? `${msg}. Check the connection and try again.`
          : `Couldn't fetch ${head} from origin: ${msg}. Check the connection and try again.`,
      };
    case 'pr_closed':
      return { text: `PR #${n} was closed since this dialog opened. A team can't work on it.` };
    case 'pr_merged':
      return { text: `PR #${n} was merged since this dialog opened. There is nothing left to fix.` };
    case 'fork':
      return { text: msg || `PR #${n} comes from a fork. The app pushes to origin only.` };
    case 'active_feature': {
      const f = e.feature;
      return {
        text: f ? `PR #${n} already has a team: ${f.slug}${f.run_status ? ` (${f.run_status})` : ''}.` : msg,
        open: f ?? null,
      };
    }
    default:
      return { keep: true, text: msg };
  }
}

/** A pre-flight warning's text (F3): the core's message, or the §9 copy built from the details. */
export function warningText(w, pr) {
  if (w.code === 'local_ahead' && w.details?.commits != null)
    return `Local ${pr?.head_ref} has ${w.details.commits} commit${w.details.commits === 1 ? '' : 's'} PR #${pr?.number} does not have. Rename or delete that local branch first.`;
  return w.message ?? '';
}

/** A command error as `{code, message, details?, feature?}`: the object itself, or its JSON text (F10, F11). */
export const codedOf = (e) => (e && typeof e === 'object' && typeof e.code === 'string' ? e : parseCoded(errText(e)));
