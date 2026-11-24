import { describe, expect, it } from 'vitest';
import { decisionToast, effectText, marker, prGate, prGateTitle } from '../src/lib/inbox.js';
import {
  branchOf,
  closedText,
  deleteCopy,
  foreignHeadWarning,
  isLinkedPush,
  keptText,
  linkedGate,
  linkedState,
  prePushText,
  pushedLine,
  pushRefusal,
  specGateNote,
} from '../src/lib/prlink.js';

const link = {
  number: 42,
  url: 'https://github.com/acme/shop/pull/42',
  head_ref: 'renovate/tokio-1.x',
  base_ref: 'main',
  author: 'renovate[bot]',
};
const feature = { slug: 'pr-42-update', branch: 'renovate/tokio-1.x', pr_link: link };

const gate = (extra = {}, lifecycle = { state: 'open' }) => ({
  id: 'd1',
  kind: 'gate',
  gate: 'pr',
  lifecycle,
  subject: { feature: 'pr-42-update', agent: 'lead' },
  payload: {
    action: 'push_pr',
    branch: 'renovate/tokio-1.x',
    base: 'main',
    linked: { number: 42, url: link.url },
    lease_sha: '9f2c1abdeadbeef',
    commits: [{ sha: 'a' }, { sha: 'b' }, { sha: 'c' }],
    ...extra,
  },
});

describe('branch', () => {
  it('takes the feature branch, the integration name for an older core', () => {
    expect(branchOf(feature)).toBe('renovate/tokio-1.x');
    expect(branchOf({ slug: 'x' })).toBe('orch/x/integration');
  });
});

describe('pane copy', () => {
  it('says the PR state in the pre-push line', () => {
    expect(prePushText(42, false)).toMatch(/^Nothing pushed to PR #42 yet/);
    expect(prePushText(42, true)).toBe('The lead asked to push to PR #42.');
    expect(closedText(42)).toMatch(/was closed on GitHub.*never opens another pull request/);
  });

  it('reads the state from the run', () => {
    expect(linkedState({ status: 'executing' })).toBe('before');
    expect(linkedState({ status: 'pr_open', pr: { state: 'open' } })).toBe('open');
    expect(linkedState({ status: 'integrating', pr: { state: 'closed' } })).toBe('closed');
    expect(linkedState({ status: 'done', pr: { state: 'merged' } })).toBe('merged');
    expect(linkedState(null)).toBe('none');
    expect(linkedState({ status: 'failed' })).toBe('none');
  });

  it('warns about a moved head with its sha, or without', () => {
    const w = foreignHeadWarning(link, { foreign_head_sha: 'a41d7e0123' });
    expect(w).toMatchObject({ sha: 'a41d7e0', url: `${link.url}/commits/a41d7e0123` });
    expect(w.after).toContain('probably by renovate[bot]');
    const none = foreignHeadWarning(link, { foreign_head: true });
    expect(none.sha).toBeNull();
    expect(none.before).toContain('a commit was pushed outside the app, probably by renovate[bot]');
    expect(foreignHeadWarning({ ...link, author: 'mia' }, null).before).not.toContain('probably');
  });

  it('puts the spec gate note under a linked feature', () => {
    expect(specGateNote(link)).toBe(
      'Works on PR #42 (renovate/tokio-1.x), opened by renovate[bot]. Approving lets the team change that branch.',
    );
  });
});

describe('push gate', () => {
  it('recognises a linked push only', () => {
    expect(isLinkedPush(gate())).toBe(true);
    expect(isLinkedPush(gate({ linked: undefined }))).toBe(false);
  });

  it('titles with the commit count, singular at one', () => {
    expect(prGateTitle(gate())).toBe('Push 3 commits to PR #42');
    expect(prGateTitle(gate({ commits: [{}] }))).toBe('Push 1 commit to PR #42');
  });

  it('builds the card lines', () => {
    const g = prGate(gate(), feature, { pr: { foreign_head: true } });
    expect(g.title).toBe('Push 3 commits to PR #42');
    expect(g.sub).toBe('renovate/tokio-1.x → main · opened by renovate[bot]');
    expect(g.note).toBe("Goes to the existing branch of someone else's PR. No new pull request is opened.");
    expect(g.lease).toBe(
      'Pushes with a lease on 9f2c1ab, the head the app last saw. If someone pushed since, nothing is overwritten.',
    );
    expect(g.diffHead).toBe("Your team's changes on top of PR #42");
    expect(g.moved).toMatch(/head moved since this request/);
    expect(g.closes).toBeNull();
    expect(linkedGate(gate(), feature, null).moved).toBeNull();
  });

  it('leaves an unlinked gate alone', () => {
    expect(prGate(gate({ linked: undefined }), null)).toMatchObject({ title: null, sub: null });
  });

  it('settles as pushed, with a toast', () => {
    const d = gate({}, { state: 'effect', effect: 'pr_pushed', effect_detail: link.url, decided_by: 'user' });
    expect(effectText(d)).toBe('Pushed 3 commits to PR #42');
    expect(decisionToast(d)).toBe('Pushed to PR #42');
    expect(marker(d).url).toBe(link.url);
    expect(pushedLine(gate({ commits: [{}] }))).toBe('Pushed 1 commit to PR #42');
  });

  it('settles as failed with the refusal, per code (F6)', () => {
    const failed = (code, message = 'm') => gate({ outcome: { code, message } }, { state: 'failed', error: 'm' });
    expect(pushRefusal(failed('pr_moved'))).toBe(
      'Not pushed: PR #42 moved since the request. The lead was told and rebases on it, then asks again.',
    );
    expect(pushRefusal(failed('pr_closed'))).toBe('Not pushed: PR #42 was closed on GitHub.');
    expect(pushRefusal(failed('push_failed', 'network down'))).toBe('Not pushed: network down');
    expect(marker(failed('pr_moved')).text).toMatch(/^Not pushed: PR #42 moved/);
    expect(effectText(failed('pr_closed'))).toBe('Not pushed: PR #42 was closed on GitHub.');
    expect(marker(gate({}, { state: 'failed', error: 'boom' })).text).toBe('Failed · boom');
  });
});

describe('delete dialog copy', () => {
  it('is untouched for an unlinked feature', () => {
    expect(deleteCopy({ slug: 'x' }).linked).toBe(false);
  });

  it('names the branch, the rule and what is kept (D2, F7)', () => {
    const c = deleteCopy(feature, { branch: 'renovate/tokio-1.x', linked: true, unpushed: 2 });
    expect(c.stays).toBe('PR #42 stays open on GitHub. Nothing is deleted there.');
    expect(c.remove).toContain('renovate/tokio-1.x is removed only if it is already pushed to PR #42');
    expect(c.kept).toBe('renovate/tokio-1.x has 2 commits that PR #42 does not have: it will be kept.');
    expect(deleteCopy(feature, { unpushed: 0 }).kept).toBe('renovate/tokio-1.x is pushed: it will be removed locally.');
    expect(deleteCopy(feature, null).kept).toBeNull();
  });

  it('adds the kept branches to the toast', () => {
    expect(keptText([{ branch: 'renovate/tokio-1.x', commits: 2 }], feature)).toBe(
      ' · kept renovate/tokio-1.x: 2 commits not pushed to PR #42',
    );
    expect(keptText([{ branch: 'renovate/tokio-1.x', commits: null }], feature)).toBe(
      " · kept renovate/tokio-1.x: can't tell whether its commits are pushed to PR #42",
    );
    expect(keptText([], feature)).toBe('');
    expect(keptText(undefined, feature)).toBe('');
  });
});
