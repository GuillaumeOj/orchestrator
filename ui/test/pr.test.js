import { describe, expect, it } from 'vitest';
import {
  checkedText,
  ciColor,
  ciCount,
  ciFooter,
  ciHeader,
  ciLabel,
  jobColor,
  jobWord,
  lineState,
  positionBadge,
  toPush,
} from '../src/lib/pr.js';
import { runFinished, runStateColor, runStateLabel, timelineFilter, timelineText } from '../src/lib/teams.js';

const job = (state, name = 'x', workflow = null) => ({ name, workflow, state, url: null });
const ci = (overall, done, total, jobs = []) => ({ overall, done, total, jobs });

describe('CI dot and count', () => {
  it('colors the dot from overall', () => {
    expect(ciColor('ok')).toContain('done');
    expect(ciColor('error')).toContain('error');
    expect(ciColor('running')).toContain('queued');
    expect(ciColor('waiting')).toContain('idle');
    expect(ciColor('none')).toContain('idle');
  });
  it('colors every job state', () => {
    for (const s of ['failure', 'cancelled', 'timed_out', 'action_required', 'startup_failure', 'stale', 'error'])
      expect(jobColor(s)).toContain('error');
    expect(jobColor('in_progress')).toContain('queued');
    for (const s of ['queued', 'waiting', 'requested', 'pending', 'expected', 'weird'])
      expect(jobColor(s)).toContain('idle');
    for (const s of ['success', 'neutral', 'skipped']) expect(jobColor(s)).toContain('done');
  });
  it('words', () => {
    expect([
      jobWord('failure'),
      jobWord('timed_out'),
      jobWord('in_progress'),
      jobWord('success'),
      jobWord('some_thing'),
    ]).toEqual(['failed', 'timed out', 'running', 'passed', 'some thing']);
  });
  it('counts', () => {
    expect(ciCount(ci('error', 4, 6))).toBe('4/6');
    expect(ciCount(ci('none', 0, 0))).toBe('no checks');
  });
  it('header and accessible names', () => {
    const c = ci('error', 4, 6, [
      job('failure'),
      job('in_progress'),
      job('queued'),
      job('success'),
      job('success'),
      job('skipped'),
    ]);
    expect(ciHeader(c)).toBe('CI · 4 of 6 finished · 1 failed · 1 running · 1 waiting');
    expect(ciLabel(c)).toBe('CI: 1 failed, 4 of 6 jobs finished. Show jobs');
    expect(ciHeader(ci('ok', 2, 2, [job('success'), job('skipped')]))).toBe('CI · 2 of 2 finished · all passed');
    expect(ciLabel(ci('running', 2, 6))).toBe('CI: running, 2 of 6 jobs finished. Show jobs');
    expect(ciLabel(ci('none', 0, 0))).toBe('CI: no checks. Show jobs');
    expect(ciHeader(ci('none', 0, 0))).toBe('No CI checks on this PR.');
  });
  it('footer when the core capped the list', () => {
    expect(ciFooter(ci('ok', 1, 1, [job('success')]))).toBe('Open checks on GitHub ↗');
    expect(ciFooter(ci('ok', 120, 120, Array(100).fill(job('success'))))).toBe('+20 more on GitHub ↗');
  });
});

describe('position, push, checked', () => {
  it('position text with the base name', () => {
    expect(positionBadge({ status: 'up_to_date' }, 'main').text).toBe('up to date');
    const b = positionBadge({ status: 'behind', behind: 3 }, 'develop');
    expect([b.text, b.short]).toEqual(['↓3 behind develop', '↓3 behind']);
    expect(b.color).toContain('waiting-for-you');
    expect(positionBadge({ status: 'conflicts' }).color).toContain('error');
    expect(positionBadge({ status: 'unknown' }).text).toBe('—');
  });
  it('to push', () => {
    expect(toPush(0)).toBeNull();
    expect(toPush(2)).toMatchObject({ text: '↑2 to push', short: '↑2' });
  });
  it('checked', () => {
    const now = Date.UTC(2026, 9, 29, 12, 0, 0);
    expect(checkedText(now - 10000, { now }).text).toBe('checked just now');
    expect(checkedText(now - 120000, { now }).text).toBe('checked 2m ago');
    expect(checkedText(now - 120000, { now }).short).toBe('2m ago');
    expect(checkedText(now - 720000, { now, stale: true }).text).toBe('last checked 12m ago');
    expect(checkedText(now - 2 * 86400000, { now }).text).toMatch(/^checked \w{3} \d\d:\d\d$/);
    expect(checkedText(null)).toBeNull();
  });
  it('line state', () => {
    expect(lineState({ pr_url: null })).toBe('none');
    expect(lineState({ pr_url: 'u', pr_number: 12, pr: null, status: 'pr_open' })).toBe('checking');
    expect(lineState({ pr_url: 'u', pr_number: 12, pr: null, status: 'done' })).toBe('legacy');
    expect(lineState({ pr_url: 'u', pr_number: 12, pr: { state: 'merged' } })).toBe('merged');
    expect(lineState({ pr_url: 'u', pr_number: 12, pr: { state: 'open' } })).toBe('open');
  });
});

describe('run state pr_open', () => {
  it('label and color; not finished', () => {
    expect(runStateLabel('pr_open')).toBe('PR open');
    expect(runStateColor('pr_open')).toContain('working');
    expect(runFinished('pr_open')).toBe(false);
  });
});

describe('timeline', () => {
  const t = (kind, detail) => timelineText({ kind, detail });
  it('opened, merged, closed, pushed', () => {
    expect(t('pr.opened', { number: 12 })).toBe('opened PR #12');
    expect(t('pr.merged', { number: 12 })).toBe('PR #12 merged');
    expect(t('pr.closed', { number: 12 })).toBe('PR #12 closed without merge');
    expect(t('pr.pushed', { number: 12, commits: 2, force: false })).toBe('pushed 2 commits to PR #12');
    expect(t('pr.pushed', { number: 12, commits: 4, force: true })).toBe('force-pushed PR #12 (rebased)');
  });
  it('status', () => {
    const failed = [
      { name: 'test', workflow: 'ci' },
      { name: 'e2e', workflow: 'ci' },
    ];
    expect(t('pr.status', { number: 12, ci: { from: 'running', to: 'error', failed_jobs: failed } })).toBe(
      'PR #12 CI failed (ci / test, ci / e2e)',
    );
    expect(t('pr.status', { number: 12, ci: { from: 'error', to: 'ok' } })).toBe('PR #12 CI ok');
    expect(t('pr.status', { number: 12, position: { to: 'behind', behind: 3 } })).toBe('PR #12 behind main by 3');
    expect(t('pr.status', { number: 12, position: { to: 'conflicts' } })).toBe('PR #12 conflicts with main');
    expect(t('pr.status', { number: 12, position: { to: 'up_to_date' } })).toBe('PR #12 up to date');
    expect(
      t('pr.status', {
        number: 12,
        ci: { to: 'error', failed_jobs: [{ name: 'test', workflow: 'ci' }] },
        position: { to: 'behind', behind: 3 },
      }),
    ).toBe('PR #12 CI failed (ci / test) · behind main by 3');
    const many = ['a', 'b', 'c', 'd', 'e'].map((name) => ({ name, workflow: null }));
    expect(t('pr.status', { number: 1, ci: { to: 'error', failed_jobs: many } })).toBe('PR #1 CI failed (a, b, c, +2)');
  });
  it('rebase', () => {
    expect(t('git.rebase', { onto: 'main', base_commits: 3, branches_moved: [] })).toBe(
      'rebased the feature onto main (3 new commits from main)',
    );
    expect(t('git.rebase', { onto: 'main', base_commits: 3, branches_moved: ['a', 'b'] })).toBe(
      'rebased the feature onto main (3 new commits from main) · moved 2 branches',
    );
  });
  it('pr.* and git.rebase are Git', () => {
    const git = timelineFilter('git');
    expect([git({ kind: 'pr.opened' }), git({ kind: 'git.rebase' }), git({ kind: 'task.done' })]).toEqual([
      true,
      true,
      false,
    ]);
  });
});
