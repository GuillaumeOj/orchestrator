// allow-multiple-prs: the issues field's pure helpers (ux-brief §1, §2.1) and
// the per-issue PR summaries (§2.2, §4), plus the timeline and decision copy.
import { describe, expect, it } from 'vitest';
import { prGate, prGateTitle } from '../src/lib/inbox.js';
import {
  addChip,
  chipError,
  chipOf,
  issueErrorText,
  issuesRead,
  issueTexts,
  lockedIssues,
  markBad,
  modeFor,
  modeHelp,
  modeLockedBy,
  parseCoded,
  parseRef,
  perIssueAllowed,
} from '../src/lib/issues.js';
import { isPerIssue, prRowState, prsCounts, prsDot, prsSummary, prsTooltip } from '../src/lib/pr.js';
import { timelineText } from '../src/lib/teams.js';

describe('parseRef', () => {
  it('reads the refs the field accepts', () => {
    expect(parseRef('12')).toBe('#12');
    expect(parseRef('#12')).toBe('#12');
    expect(parseRef('eng-7')).toBe('ENG-7');
    expect(parseRef('o/r#9')).toBe('#9');
    expect(parseRef('https://github.com/o/r/issues/12')).toBe('#12');
  });
  it('rejects text that is not a ref', () => {
    expect(parseRef('crash on empty')).toBeNull();
    expect(parseRef('')).toBeNull();
    expect(parseRef('https://github.com/o/r/pull/12')).toBeNull();
  });
});

describe('chips', () => {
  it('sends the text as typed and shows the ref', () => {
    const c = chipOf(' 12 ');
    expect([c.text, c.ref, c.bad]).toEqual(['12', '#12', null]);
    const u = chipOf('https://github.com/o/r/issues/3');
    expect([u.text, u.ref]).toEqual(['https://github.com/o/r/issues/3', '#3']);
  });
  it('a text that is no ref is a chip in error', () => {
    const c = chipOf('nope');
    expect(c.bad).toBe('Not an issue ref: nope');
    expect(chipError([chipOf('1'), c])).toBe('Not an issue ref: nope');
    expect(chipError([chipOf('1')])).toBeNull();
  });
  it('drops a duplicate and stops at 10', () => {
    expect(addChip([chipOf('12')], chipOf('#12')).length).toBe(1);
    const ten = Array.from({ length: 10 }, (_, i) => chipOf(String(i + 1)));
    expect(addChip(ten, chipOf('11'))).toBe(ten);
    expect(issueTexts([chipOf('12'), chipOf('eng-7')])).toEqual(['12', 'eng-7']);
  });
  it('marks the chip the server named', () => {
    const chips = markBad([chipOf('12'), chipOf('13')], '13', 'Not an issue of o/r: 13');
    expect(chips.map((c) => c.bad)).toEqual([null, 'Not an issue of o/r: 13']);
  });
});

describe('server errors', () => {
  it('reads coded text', () => {
    expect(parseCoded('{"code":"too_many","message":"x"}').code).toBe('too_many');
    expect(parseCoded('boom')).toEqual({ code: null, message: 'boom' });
  });
  it('words each code', () => {
    const e = (code, extra = {}) => issueErrorText({ code, message: 'm', ...extra });
    expect(e('bad_ref', { ref: 'x', message: 'no' })).toBe('Not an issue ref: x');
    expect(e('bad_ref', { ref: '#4', message: '"#4": a/b is not this project\'s repository (o/r)' })).toBe(
      'Not an issue of o/r: #4',
    );
    expect(e('too_many')).toBe('10 issues at most');
    expect(e('pr_busy')).toBe('Set before the first PR: this feature already has one.');
    expect(e('issue_has_pr', { ref: '#12' })).toBe('#12 has a PR: it cannot be removed');
    expect(e('other')).toBe('m');
  });
});

describe('PR mode', () => {
  it('per issue needs two issues and falls back to one PR below', () => {
    expect(perIssueAllowed(1)).toBe(false);
    expect(perIssueAllowed(2)).toBe(true);
    expect(modeFor('per_issue', 1)).toBe('single');
    expect(modeFor('per_issue', 2)).toBe('per_issue');
    expect(modeFor('single', 0)).toBe('single');
  });
  it('helper copy by choice', () => {
    expect(modeHelp('single', [])).toBe('One PR for the feature.');
    expect(modeHelp('single', ['#12', 'ENG-7'])).toBe('One PR for the feature; it closes #12, ENG-7.');
    expect(modeHelp('per_issue', ['#12', '#13'])).toBe(
      "One PR per issue, each with only that issue's tasks. Work not tied to an issue goes into every PR.",
    );
  });
  it('the mode is locked once the run had a PR in any state, or a PR request', () => {
    expect(modeLockedBy(null)).toBe(false);
    expect(modeLockedBy({ prs_view: [{ issue: '#1', state: 'none' }] })).toBe(false);
    expect(modeLockedBy({ prs_view: [{ issue: '#1', state: 'pending' }] })).toBe(true);
    expect(modeLockedBy({ prs_view: [{ issue: '#1', state: 'closed' }] })).toBe(true);
    expect(modeLockedBy({ prs_view: [], pr_url: 'u' })).toBe(true);
  });
  it('locks the issues with a pending, open or merged PR', () => {
    const view = [
      { issue: '#1', state: 'open', number: 34 },
      { issue: '#2', state: 'none' },
      { issue: '#3', state: 'closed', number: 5 },
      { issue: '#4', state: 'pending' },
    ];
    expect([...lockedIssues(view)]).toEqual([
      ['#1', 34],
      ['#4', null],
    ]);
  });
  it('the read line', () => {
    expect(issuesRead([])).toBeNull();
    expect(issuesRead([{ ref: '#12' }, { ref: 'ENG-7' }])).toBe('#12 · ENG-7');
  });
});

const P = (state, issue = '#1', over = {}) => ({
  issue,
  state,
  number: state === 'none' || state === 'pending' ? null : 30,
  url: 'u',
  pr: null,
  ...over,
});

describe('prsSummary (ux-brief §4 precedence)', () => {
  it('every issue merged', () => {
    expect(prsSummary([P('merged', '#1'), P('merged', '#2'), P('merged', '#3')])).toBe('3 PRs merged');
  });
  it('some open, then merged', () => {
    expect(prsSummary([P('open', '#1'), P('open', '#2'), P('merged', '#3')])).toBe('2 PRs open · 1 merged');
    expect(prsSummary([P('open', '#1'), P('none', '#2')])).toBe('1 PR open');
  });
  it('none open and some merged', () => {
    expect(prsSummary([P('merged', '#1'), P('none', '#2'), P('closed', '#3')])).toBe('1 of 3 merged');
  });
  it('otherwise no PR yet: pending and closed add nothing', () => {
    expect(prsSummary([P('none', '#1'), P('pending', '#2'), P('closed', '#3')])).toBe('no PR yet');
    expect(prsSummary([])).toBe('no PR yet');
  });
  it('counts for the card head', () => {
    expect(prsCounts([P('open', '#1'), P('open', '#2'), P('merged', '#3')])).toBe('2 open · 1 merged');
    expect(prsCounts([P('none', '#1')])).toBe('');
  });
  it('tells the two modes apart', () => {
    expect(isPerIssue([P('open')])).toBe(true);
    expect(isPerIssue([P('open', null)])).toBe(false);
    expect(isPerIssue([])).toBe(false);
  });
});

describe('prsDot: the worst open PR', () => {
  const open = (overall, issue) => P('open', issue, { pr: { ci: { overall } } });
  it('error > running > waiting > ok', () => {
    expect(prsDot([open('ok', '#1'), open('error', '#2'), open('running', '#3')])).toBe('var(--color-status-error)');
    expect(prsDot([open('ok', '#1'), open('running', '#2')])).toBe('var(--color-status-queued)');
    expect(prsDot([open('ok', '#1'), open('waiting', '#2')])).toBe('var(--color-status-idle)');
    expect(prsDot([open('ok', '#1')])).toBe('var(--color-status-done)');
  });
  it('none when no PR is open', () => {
    expect(prsDot([P('merged', '#1'), P('closed', '#2'), P('pending', '#3')])).toBeNull();
    expect(prsDot([])).toBeNull();
  });
  it('lists one line per PR in the tooltip', () => {
    const t = prsTooltip([open('error', '#1'), P('merged', '#2'), P('pending', '#3'), P('none', '#4')], (r) =>
      r === '#1' ? { title: 'Crash' } : null,
    );
    expect(t.split('\n')).toEqual([
      '#1 Crash · PR #30 open · CI no checks',
      '#2 · PR #30 merged',
      '#3 · waiting for your approval',
      '#4 · no PR yet',
    ]);
  });
});

describe('prRowState', () => {
  it('the rows that are not an open PR', () => {
    expect(prRowState(P('none'))).toEqual({ text: 'No PR yet', tone: 'muted' });
    expect(prRowState(P('none'), 3).text).toBe('No PR yet · 3 tasks');
    expect(prRowState(P('pending'))).toEqual({ text: 'Waiting for your approval', tone: 'waiting' });
    expect(prRowState(P('merged', '#1', { number: 34 }))).toEqual({ text: 'PR #34 merged', tone: 'done' });
    expect(prRowState(P('closed', '#1', { number: 34 }))).toEqual({ text: 'PR #34 closed', tone: 'muted' });
  });
});

describe('timeline copy (ux-brief §6)', () => {
  const ev = (kind, detail) => ({ kind, detail });
  it('feature.prs', () => {
    expect(timelineText(ev('feature.prs', { issues: ['#12', 'ENG-7'], pr_mode: 'per_issue', by: 'user' }))).toBe(
      'You set one PR per issue (#12, ENG-7)',
    );
    expect(timelineText(ev('feature.prs', { issues: [], pr_mode: 'single', by: 'user' }))).toBe(
      'You set one PR for the feature',
    );
    expect(timelineText(ev('feature.prs', { issues: ['#12', 'ENG-7'], pr_mode: 'single', by: 'user' }))).toBe(
      'You changed the issues: #12, ENG-7',
    );
  });
  it('pr.build failed names the conflict', () => {
    expect(
      timelineText(
        ev('pr.build', {
          issue: '#12',
          ok: false,
          commits: 0,
          shared: 0,
          skipped: 0,
          conflict: { task: 4, commit: 'abcdef123', files: ['a.rs', 'b.rs'] },
        }),
      ),
    ).toBe("PR for #12 can't be built: T-4 conflicts in a.rs, b.rs");
    expect(
      timelineText(ev('pr.build', { issue: '#12', ok: false, conflict: { commit: 'abcdef123', files: ['a.rs'] } })),
    ).toBe("PR for #12 can't be built: abcdef1 conflicts in a.rs");
  });
  it('pr.build ok says what it replayed', () => {
    expect(timelineText(ev('pr.build', { issue: '#12', ok: true, commits: 3, shared: 2, skipped: 1 }))).toBe(
      'built the PR branch for #12 (3 commits, 2 shared, 1 already there)',
    );
  });
  it('PR events carry the issue after the number; a missing key is single mode', () => {
    expect(timelineText(ev('pr.opened', { number: 34, url: 'u', issue: '#12' }))).toBe('opened PR #34 (#12)');
    expect(timelineText(ev('pr.opened', { number: 34, url: 'u' }))).toBe('opened PR #34');
    expect(timelineText(ev('pr.merged', { number: 34, issue: '#12' }))).toBe('PR #34 (#12) merged');
    expect(timelineText(ev('pr.closed', { number: 34, issue: '#12' }))).toBe('PR #34 (#12) closed without merge');
    expect(timelineText(ev('pr.pushed', { number: 34, commits: 2, force: false, issue: '#12' }))).toBe(
      'pushed 2 commits to PR #34 (#12)',
    );
    expect(timelineText(ev('pr.pushed', { number: 34, commits: 2, force: true, issue: '#12' }))).toBe(
      'force-pushed PR #34 (#12) (rebased)',
    );
    expect(timelineText(ev('pr.status', { number: 34, issue: '#12', ci: { to: 'ok' } }))).toBe('PR #34 (#12) CI ok');
  });
  it('run.state keeps the why', () => {
    expect(timelineText(ev('run.state', { from: 'pr_open', to: 'done', why: 'every issue PR merged' }))).toContain(
      '(every issue PR merged)',
    );
  });
});

describe('PR gate cards (ux-brief §3)', () => {
  const gate = (payload) => ({ kind: 'gate', gate: 'pr', title: 'PR gate · feat', payload });
  const feature = {
    issues: [
      { ref: '#12', title: 'Crash on empty brief' },
      { ref: 'ENG-7', title: null },
    ],
  };
  it('push_pr per issue: title, branch line, built-from note', () => {
    const g = prGate(gate({ action: 'push_pr', issue: '#12', branch: 'orch/f/issue/12', base: 'main' }), feature);
    expect(g.title).toBe('Open PR for #12 "Crash on empty brief"');
    expect(g.sub).toBe('orch/f/issue/12 → main');
    expect(g.note).toBe('Built from the integration branch: tasks of #12 and shared work.');
    expect(g.closes).toBeNull();
  });
  it('an issue without a title', () => {
    expect(prGateTitle(gate({ action: 'push_pr', issue: 'ENG-7' }))).toBe('Open PR for ENG-7');
  });
  it('push_pr_update per issue', () => {
    const g = prGate(
      gate({ action: 'push_pr_update', issue: '#12', pr_number: 34, branch: 'b', base: 'main' }),
      feature,
    );
    expect(g.title).toBe('Push to PR #34 (#12)');
    expect(g.note).toBeNull();
  });
  it('single mode with issues shows Closes; without, the card is unchanged', () => {
    expect(prGate(gate({ action: 'push_pr', issue: null }), feature)).toEqual({
      title: null,
      sub: null,
      note: null,
      closes: 'Closes #12, ENG-7',
    });
    expect(prGate(gate({ action: 'push_pr', issue: null }), { issues: [] }).closes).toBeNull();
    expect(prGate(gate({ action: 'push_pr_update', issue: null }), feature).closes).toBeNull();
  });
  it('other cards are not PR gates', () => {
    expect(prGate({ kind: 'permission', payload: {} })).toBeNull();
    expect(prGateTitle({ kind: 'gate', payload: { action: 'spec' } })).toBeNull();
  });
});
