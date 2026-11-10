// R13 (design-notes §13.1, 04-feature-pr-running / -review-done): the PR line in
// the feature pane's team column: the link, the CI chip (dot + n/m), the CI
// word and the position, the review state when GitHub gave one, and the
// checks popover (every check with its state, "Open checks on GitHub").

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const calls = [];
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => (calls.push([cmd, args]), Promise.resolve(null)),
  listen: () => Promise.resolve(() => {}),
  errText: (e) => String(e?.message ?? e),
}));

// jsdom has no ResizeObserver (the line fits its items to the width).
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};

const { default: PrLine } = await import('../src/components/PrLine.svelte');

const job = (name, state, workflow = 'ci') => ({ name, workflow, state, url: `https://github.com/o/r/runs/${name}` });
const URL = 'https://github.com/o/r/pull/57';

/** A run with an open PR #57: `jobs` its checks, `over` replaces fields of `pr`. */
function run(jobs, over = {}) {
  const done = jobs.filter((j) => !['in_progress', 'queued', 'waiting', 'pending'].includes(j.state)).length;
  const failing = jobs.some((j) => j.state === 'failure');
  const overall = failing ? 'error' : done < jobs.length ? 'running' : 'ok';
  return {
    id: 'run-1',
    status: 'pr_open',
    pr_url: URL,
    pr_number: 57,
    pr: {
      state: 'open',
      base: 'main',
      ci: { overall, done, total: jobs.length, jobs },
      position: { status: 'up_to_date', behind: 0, ahead: 3 },
      unpushed: 0,
      rewritten: false,
      checked_at_ms: Date.now() - 60000,
      error: null,
      ...over,
    },
  };
}

const RUNNING = [
  job('e2e shard 1', 'in_progress'),
  job('ui tests', 'in_progress'),
  job('preview', 'queued', 'deploy'),
  job('lint', 'success'),
  job('build', 'success'),
];
const PASSED = RUNNING.map((j) => ({ ...j, state: 'success' }));
const text = (el) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
/** The line's visible items in order (flex gaps separate them, not spaces); icons left out. */
const items = (line) =>
  [...line.querySelectorAll(':scope > *, :scope > .ci > .cibtn')]
    .filter(
      (e) => !e.classList.contains('ci') && !e.classList.contains('grow') && e.getAttribute('aria-hidden') !== 'true',
    )
    .map((e) =>
      text(e)
        .replace(/[\u{F0000}-\u{FFFFF}]/gu, '')
        .trim(),
    )
    .filter(Boolean);

beforeEach(() => {
  cleanup();
  calls.length = 0;
});

describe('PrLine (R13)', () => {
  it('running: link, CI chip n/m, "running", then the position', () => {
    const { container } = render(PrLine, { run: run(RUNNING), feature: 'f1' });
    const line = container.querySelector('.prline');
    expect(text(line.querySelector('.pr'))).toMatch(/^PR #57/);
    const chip = line.querySelector('.cibtn');
    expect(text(chip)).toBe('2/5');
    expect(chip.getAttribute('aria-label')).toMatch(/^CI: running, 2 of 5 jobs finished/);
    expect(text(line.querySelector('[data-ci-word]'))).toBe('running');
    expect(items(line).slice(0, 4)).toEqual(['PR #57', '2/5', 'running', 'up to date']);
    expect(line.querySelector('[data-review]')).toBeNull();
  });

  it('review done: "passed", the position and "approved"', () => {
    const { container } = render(PrLine, { run: run(PASSED, { review_decision: 'APPROVED' }), feature: 'f1' });
    const line = container.querySelector('.prline');
    expect(text(line.querySelector('.cibtn'))).toBe('5/5');
    expect(text(line.querySelector('[data-ci-word]'))).toBe('passed');
    expect(text(line.querySelector('[data-review]'))).toBe('approved');
    expect(items(line).slice(0, 5)).toEqual(['PR #57', '5/5', 'passed', 'up to date', 'approved']);
  });

  it('a failing check says how many failed', () => {
    const jobs = [job('lint', 'failure'), job('build', 'success')];
    const { container } = render(PrLine, { run: run(jobs), feature: 'f1' });
    expect(text(container.querySelector('[data-ci-word]'))).toBe('1 failed');
  });

  it('the CI chip opens the checks popover: a row per check with its state, then Open checks on GitHub', async () => {
    const { container } = render(PrLine, { run: run(RUNNING), feature: 'f1' });
    const chip = container.querySelector('.cibtn');
    await fireEvent.click(chip);
    expect(chip.getAttribute('aria-expanded')).toBe('true');
    expect(chip.classList.contains('on')).toBe(true);
    const pop = container.querySelector('.pop');
    expect(text(pop.querySelector('.ph'))).toBe('CI · 2 of 5 finished · 2 running · 1 waiting');
    const rows = [...pop.querySelectorAll('.row')].map((r) => [
      text(r.querySelector('.jn')).replace(/\s*\/\s*/, ' / '),
      text(r.querySelector('.jw')),
    ]);
    expect(rows).toEqual([
      ['ci / e2e shard 1', 'running'],
      ['ci / ui tests', 'running'],
      ['deploy / preview', 'queued'],
      ['ci / lint', 'passed'],
      ['ci / build', 'passed'],
    ]);
    const foot = pop.querySelector('.pf');
    expect(text(foot)).toBe('Open checks on GitHub ↗');
    await fireEvent.click(foot);
    expect(calls).toContainEqual(['open_url', { url: `${URL}/checks` }]);
  });

  it('no native title on its controls (tooltips instead)', () => {
    const { container } = render(PrLine, { run: run(RUNNING), feature: 'f1' });
    for (const b of container.querySelectorAll('button')) expect(b.hasAttribute('title')).toBe(false);
  });
});
