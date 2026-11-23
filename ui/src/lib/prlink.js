// A feature linked to someone else's PR (allow-launch-team-on-update-pr, notes §3–§5, api-contract §4–§6):
// the Feature pane's identity lines, the push gate's copy and outcomes, the delete dialog's copy. Pure; tested
// in test/prlink.test.js. Text from GitHub is untrusted: it is shown as plain text only.
import { authorName } from './prlaunch.js';

/** The feature's PR link (`pr_link`), null for an unlinked feature. */
export const linkOf = (feature) => feature?.pr_link ?? null;

/** The real integration branch: `feature.branch` (api-contract §4), `orch/<slug>/integration` for an older core. */
export const branchOf = (feature) => feature?.branch ?? (feature?.slug ? `orch/${feature.slug}/integration` : '');

const sha7 = (s) => (s ? String(s).slice(0, 7) : '');

/** `renovate/tokio-1.x → main` */
export const headBase = (link) => `${link.head_ref} → ${link.base_ref}`;

/** The Feature pane's linked line (§3.2): the pieces, the PR number and the author shown by the component. */
export function linkedLine(link) {
  return { number: link.number, url: link.url, branches: headBase(link), author: authorName({ login: link.author }) };
}

/** The foreign-head warning (§3.3, F4): `{ before, sha, after }`; the sha is a link when known. */
export function foreignHeadWarning(link, pr) {
  const n = link.number;
  const by =
    link.author && /\[bot\]$|^app\//.test(link.author) ? `, probably by ${authorName({ login: link.author })}` : '';
  const sha = pr?.foreign_head_sha ?? null;
  const tail = ' The lead was told and brings it in with a rebase before asking for the push.';
  return sha
    ? {
        before: `PR #${n}'s head moved: `,
        sha: sha7(sha),
        url: `${link.url}/commits/${sha}`,
        after: ` was pushed outside the app${by}.${tail}`,
      }
    : {
        before: `PR #${n}'s head moved: a commit was pushed outside the app${by}.${tail}`,
        sha: null,
        url: null,
        after: '',
      };
}

/** The pre-push line (§3.2), until `pr_open`; `gate` = a push gate is pending. */
export const prePushText = (n, gate) =>
  gate
    ? `The lead asked to push to PR #${n}.`
    : `Nothing pushed to PR #${n} yet. The lead asks you to approve the push when the work is done (Push gate).`;

/** The closed line (§3.4). */
export const closedText = (n) =>
  `PR #${n} was closed on GitHub. The team can't push to it, and the app never opens another pull request. Reopen it on GitHub, or delete this feature.`;

/**
 * Where the pane stands for a linked feature: `before` (no push yet) | `open` | `closed` | `merged`,
 * `none` without a run, or once the run ended without a PR outcome (no push is coming).
 */
export function linkedState(run) {
  if (!run) return 'none';
  const st = run.pr?.state;
  if (st === 'merged' || st === 'closed') return st;
  if (run.status === 'pr_open' || run.resume_status === 'pr_open') return 'open';
  return run.status === 'done' || run.status === 'failed' ? 'none' : 'before';
}

/** The spec gate's muted line for a linked feature (§3.5, D5). */
export const specGateNote = (link) =>
  `Works on PR #${link.number} (${link.head_ref}), opened by ${authorName({ login: link.author })}. Approving lets the team change that branch.`;

// ---------------------------------------------------------------- the push gate (§4)

/** Whether a decision is a push to a linked PR. */
export const isLinkedPush = (d) =>
  d?.kind === 'gate' && d.payload?.action === 'push_pr' && !!d.payload?.linked && typeof d.payload.linked === 'object';

/** The commits' count of a linked push. */
export const commitCount = (p) => (Array.isArray(p?.commits) ? p.commits.length : 0);

/** `Push 3 commits to PR #42` (§4; 1 commit singular). */
export const pushTitle = (p) => {
  const n = commitCount(p);
  return `Push ${n} commit${n === 1 ? '' : 's'} to PR #${p.linked.number}`;
};

export const PUSH_NOTE = "Goes to the existing branch of someone else's PR. No new pull request is opened.";

/** The card's lines for a linked push: `{ title, sub, note, lease, diffHead, moved }`. */
export function linkedGate(d, feature = null, run = null) {
  const p = d.payload;
  const link = linkOf(feature);
  const author = link?.author ? ` · opened by ${authorName({ login: link.author })}` : '';
  return {
    title: pushTitle(p),
    sub: `${p.branch}${p.base ? ` → ${p.base}` : ''}${author}`,
    note: PUSH_NOTE,
    lease: p.lease_sha
      ? `Pushes with a lease on ${sha7(p.lease_sha)}, the head the app last saw. If someone pushed since, nothing is overwritten.`
      : null,
    diffHead: `Your team's changes on top of PR #${p.linked.number}`,
    moved: run?.pr?.foreign_head
      ? `PR #${p.linked.number}'s head moved since this request. The push will be refused until the lead rebases on it.`
      : null,
  };
}

/** A refused push's text from `payload.outcome.code` (F6); null when the code is unknown (the core's message shows). */
export function pushRefusal(d) {
  const o = d?.payload?.outcome;
  const n = d?.payload?.linked?.number;
  if (!o?.code || n == null) return null;
  if (o.code === 'pr_moved')
    return `Not pushed: PR #${n} moved since the request. The lead was told and rebases on it, then asks again.`;
  if (o.code === 'pr_closed') return `Not pushed: PR #${n} was closed on GitHub.`;
  return o.message ? `Not pushed: ${o.message}` : null;
}

/** The settled line after a push: `Pushed 3 commits to PR #42`. */
export const pushedLine = (d) => {
  const n = commitCount(d?.payload);
  const pr = d?.payload?.linked?.number;
  return n ? `Pushed ${n} commit${n === 1 ? '' : 's'} to PR #${pr}` : `Pushed to PR #${pr}`;
};

// ---------------------------------------------------------------- delete (§5, F7)

/** The delete dialog's linked copy for a feature and its plan (`feature_delete_plan`). */
export function deleteCopy(feature, plan = null) {
  const link = linkOf(feature);
  const branch = plan?.branch ?? branchOf(feature);
  if (!link) return { linked: false, branch, remove: null, kept: null, discard: null, stays: null };
  const n = link.number;
  const un = plan?.unpushed;
  return {
    linked: true,
    branch,
    stays: `PR #${n} stays open on GitHub. Nothing is deleted there.`,
    remove: `Remove its worktrees and local branches. ${branch} is removed only if it is already pushed to PR #${n}; otherwise it is kept.`,
    discard: `Discard unmerged work in the worktrees (commits on ${branch} that PR #${n} does not have stay on the local branch)`,
    kept:
      un == null
        ? null
        : un > 0
          ? `${branch} has ${un} commit${un === 1 ? '' : 's'} that PR #${n} does not have: it will be kept.`
          : `${branch} is pushed: it will be removed locally.`,
  };
}

/** The delete toast's tail for the branches kept (`kept: [{branch, commits}]`, §5, F7); '' when none. */
export const keptText = (kept, feature) => {
  const n = linkOf(feature)?.number;
  const to = n ? ` to PR #${n}` : '';
  return (kept ?? [])
    .map((k) => {
      // `commits: null`: git could not count them, the branch is kept all the same.
      if (k.commits == null) return ` · kept ${k.branch}: can't tell whether its commits are pushed${to}`;
      return ` · kept ${k.branch}: ${k.commits} commit${k.commits === 1 ? '' : 's'} not pushed${to}`;
    })
    .join('');
};
