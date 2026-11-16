// The issues of a feature and its PR mode (allow-multiple-prs ux-brief §1, §2.1):
// ref parsing for the chip input, the helper copy, the locks and the server
// errors. Pure: tested in test/issues.test.js.

export const MAX_ISSUES = 10;

/** The two modes, as the segmented control lists them. */
export const PR_MODES = [
  { value: 'single', label: 'One PR' },
  { value: 'per_issue', label: 'One PR per issue' },
];

/** What the chip input sends and shows for a text that parses as a ref, else null. */
export function parseRef(text) {
  const t = String(text ?? '').trim();
  if (!t) return null;
  let m = t.match(/^#?(\d+)$/);
  if (m) return `#${Number(m[1])}`;
  m = t.match(/^[A-Za-z][A-Za-z0-9]*-\d+$/);
  if (m) return t.toUpperCase();
  m = t.match(/^[\w.-]+\/[\w.-]+#(\d+)$/);
  if (m) return `#${Number(m[1])}`;
  m = t.match(/^https?:\/\/github\.com\/[\w.-]+\/[\w.-]+\/issues\/(\d+)(?:[/?#].*)?$/);
  if (m) return `#${Number(m[1])}`;
  return null;
}

/**
 * A chip for what was typed: `text` is what goes to the server (as typed, it
 * validates), `ref` what the chip shows. A text that is not a ref is a chip
 * in error from the start.
 */
export function chipOf(text, known = null) {
  const t = String(text).trim();
  const ref = parseRef(t);
  return {
    text: t,
    ref: ref ?? t,
    title: known?.title ?? null,
    url: known?.url ?? null,
    bad: ref ? null : `Not an issue ref: ${t}`,
  };
}

/** A feature's issue (`feature.issues[i]`) as a chip. */
export const chipOfIssue = (i) => ({ text: i.ref, ref: i.ref, title: i.title ?? null, url: i.url ?? null, bad: null });

/** The chips after adding `chip`: a duplicate (same canonical ref) is dropped; none past the cap. */
export function addChip(chips, chip) {
  if (chips.length >= MAX_ISSUES) return chips;
  if (chips.some((c) => c.ref.toLowerCase() === chip.ref.toLowerCase())) return chips;
  return [...chips, chip];
}

/** The refs the form sends, `text` as typed, in order. */
export const issueTexts = (chips) => chips.map((c) => c.text);

/** The message under the field while a chip is invalid (ux-brief §1.1), else null. */
export function chipError(chips) {
  const bad = chips.find((c) => c.bad);
  return bad ? bad.bad : null;
}

/** The chips with `bad` set on the one the server named (code `bad_ref`, its `ref`), `message` as the text. */
export function markBad(chips, ref, message) {
  return chips.map((c) => (c.text === ref || c.ref === ref ? { ...c, bad: message } : c));
}

/** The server's `{code, message, ref?}` text as an object; `{ message: text }` when it is not coded. */
export function parseCoded(text) {
  try {
    const v = JSON.parse(text);
    if (v && typeof v === 'object' && typeof v.code === 'string') return v;
  } catch {}
  return { code: null, message: String(text ?? '') };
}

/** The inline message of a server error (api-contract §3). */
export function issueErrorText(err) {
  const e = typeof err === 'string' ? parseCoded(err) : err;
  switch (e.code) {
    case 'bad_ref': {
      const ref = e.ref ?? '';
      // The server names the project's repository when the ref is another one's.
      const mine = /is not this project's repository \(([^)]+)\)/.exec(e.message)?.[1];
      return mine ? `Not an issue of ${mine}: ${ref}` : `Not an issue ref: ${ref}`;
    }
    case 'too_many':
      return `${MAX_ISSUES} issues at most`;
    case 'per_issue_needs_two':
      return 'Add at least 2 issues for one PR per issue';
    case 'issue_has_pr':
      return e.ref ? `${e.ref} has a PR: it cannot be removed` : e.message;
    case 'pr_busy':
      return MODE_LOCKED;
    case 'finished':
      return 'The run is done: issues and mode can no longer change.';
    default:
      return e.message;
  }
}

export const MODE_LOCKED = 'Set before the first PR: this feature already has one.';
export const NEEDS_TWO = 'Add at least 2 issues';

/** Whether "One PR per issue" can be picked with `n` issues (ux-brief §1.2). */
export const perIssueAllowed = (n) => n >= 2;

/** The mode once the chips changed: per issue falls back to One PR under 2 issues. */
export const modeFor = (mode, n) => (mode === 'per_issue' && !perIssueAllowed(n) ? 'single' : mode);

/** The helper line under the Pull requests control (ux-brief §1.2). */
export function modeHelp(mode, refs) {
  if (mode === 'per_issue')
    return "One PR per issue, each with only that issue's tasks. Work not tied to an issue goes into every PR.";
  return refs.length ? `One PR for the feature; it closes ${refs.join(', ')}.` : 'One PR for the feature.';
}

/** The mode's label in the read view. */
export const modeLabel = (mode) => (mode === 'per_issue' ? 'One PR per issue' : 'One PR');

/** The issues the user can still edit in the feature's popover: which have a PR (pending, open or merged). */
export function lockedIssues(prsView) {
  const out = new Map();
  for (const p of prsView ?? []) {
    if (p.issue && ['pending', 'open', 'merged'].includes(p.state)) out.set(p.issue, p.number ?? null);
  }
  return out;
}

/** The tooltip and aria-label of a locked chip. */
export const lockedText = (ref, number) =>
  number != null ? `Has PR #${number}` : `${ref} has a pending PR, cannot be removed`;

export const lockedLabel = (ref, number) =>
  number != null ? `${ref} has PR #${number}, cannot be removed` : `${ref} has a pending PR, cannot be removed`;

/**
 * Whether the feature's mode can still change: not once its latest run has had
 * a PR or a PR request, in any state (api-contract `pr_busy`). The core is the
 * judge; this only avoids offering what it refuses.
 */
export function modeLockedBy(run) {
  return (
    (run?.prs_view ?? []).some((p) => p.state !== 'none') ||
    !!run?.pr_url ||
    !!run?.pr_number ||
    (run?.prs ?? []).length > 0
  );
}

/** The feature header's read line: `Issues #12 · ENG-7` (the refs), or null without issue. */
export const issuesRead = (issues) => (issues?.length ? issues.map((i) => i.ref).join(' · ') : null);

/** An issue's label: `#12 Crash on empty brief`. */
export const issueLabel = (ref, title) => (title ? `${ref} ${title}` : ref);

/** The issue of a feature by ref (for its title and url). */
export const issueOf = (feature, ref) => (feature?.issues ?? []).find((i) => i.ref === ref) ?? null;
