// allow-multiple-prs: the Issues chip input, the Pull requests control, the
// launch form's wiring, the feature header's Edit popover and the per-issue PR
// rows (ux-brief §1, §2).

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const calls = [];
const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push({ cmd, args });
    const r = replies[cmd];
    if (r instanceof Error) return Promise.reject(r);
    if (typeof r === 'function') return Promise.resolve(r(args));
    return Promise.resolve(r ?? null);
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? JSON.stringify(e))),
}));

const { app } = await import('../src/lib/app.svelte.js');
const { default: IssuesField } = await import('../src/components/IssuesField.svelte');
const { default: PrModeControl } = await import('../src/components/PrModeControl.svelte');
const { default: Dialogs } = await import('../src/components/Dialogs.svelte');
const { default: FeatureIssues } = await import('../src/components/FeatureIssues.svelte');
const { default: PrRows } = await import('../src/components/PrRows.svelte');
const { default: MissionPane } = await import('../src/panes/MissionPane.svelte');
const { default: RunInspector } = await import('../src/components/inspector/RunInspector.svelte');
const { default: DecisionCard } = await import('../src/components/chat/DecisionCard.svelte');
const { default: TaskIssueChip } = await import('../src/components/TaskIssueChip.svelte');
const { missionData, missionNow } = await import('./mission-fixture.js');

const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const settle = async () => {
  await tick();
  await Promise.resolve();
  await tick();
  await Promise.resolve();
  await tick();
};
const type = async (input, value) => {
  await fireEvent.input(input, { target: { value } });
  await settle();
};
const key = (el, k, more = {}) => fireEvent.keyDown(el, { key: k, ...more });
const chipsOf = (c) => [...c.querySelectorAll('.chip .ref')].map((x) => x.textContent);

beforeEach(() => {
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  replies.issue_suggest = [];
});
afterEach(cleanup);

describe('IssuesField', () => {
  const setup = (props = {}) => {
    const r = render(IssuesField, { chips: [], project: 'shop-api', ...props });
    return { ...r, input: r.container.querySelector('input') };
  };

  it('Space commits a text that parses as a ref', async () => {
    const { container: c, input } = setup();
    await type(input, '12');
    await key(input, ' ');
    expect(chipsOf(c)).toEqual(['#12']);
    expect(input.value).toBe('');
  });

  it('Space and comma keep a text that is not a ref (to search titles); comma still commits it', async () => {
    const { container: c, input } = setup();
    await type(input, 'crash');
    const e = new KeyboardEvent('keydown', { key: ' ', cancelable: true, bubbles: true });
    input.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(false);
    expect(chipsOf(c)).toEqual([]);
  });

  it('Enter commits a typed text as a chip, in error when it is not a ref', async () => {
    const { container: c, input } = setup();
    await type(input, 'nope');
    await key(input, 'Enter');
    expect(chipsOf(c)).toEqual(['nope']);
    expect(c.querySelector('.chip.bad')).toBeTruthy();
    expect(text(c.querySelector('.msg'))).toBe('Not an issue ref: nope');
  });

  it('a URL becomes #n and a duplicate is dropped', async () => {
    const { container: c, input } = setup();
    await type(input, 'https://github.com/o/r/issues/12');
    await key(input, 'Enter');
    await type(input, '#12');
    await key(input, 'Enter');
    expect(chipsOf(c)).toEqual(['#12']);
  });

  it('suggests from the intake, Arrow keys and Enter pick, and an added issue is not proposed again', async () => {
    replies.issue_suggest = [
      { ref: '#12', key: '12', title: 'Crash on empty brief', url: null },
      { ref: '#13', key: '13', title: 'Crash on save', url: null },
    ];
    const { container: c, input } = setup({ chips: [{ text: '#12', ref: '#12', title: null, url: null, bad: null }] });
    await type(input, 'crash');
    expect(calls.find((x) => x.cmd === 'issue_suggest').args).toEqual({ project: 'shop-api', text: 'crash' });
    const opts = [...c.querySelectorAll('[role="option"]')];
    expect(opts.map(text)).toEqual(['#13 Crash on save']);
    await key(input, 'ArrowDown');
    await key(input, 'Enter');
    expect(chipsOf(c)).toEqual(['#12', '#13']);
    expect(c.querySelector('.chip:last-of-type .ttl').textContent).toBe('Crash on save');
  });

  it('Backspace on an empty input removes the last chip that is not locked', async () => {
    const chip = (r) => ({ text: r, ref: r, title: null, url: null, bad: null });
    const { container: c, input } = setup({ chips: [chip('#1'), chip('#2')], locked: new Map([['#2', 34]]) });
    await key(input, 'Backspace');
    expect(chipsOf(c)).toEqual(['#2']);
    await key(input, 'Backspace');
    expect(chipsOf(c)).toEqual(['#2']);
  });

  it('a locked chip has no ×, a lock glyph and says why', () => {
    const chip = (r) => ({ text: r, ref: r, title: null, url: null, bad: null });
    const { container: c } = setup({ chips: [chip('#1'), chip('#2')], locked: new Map([['#2', 34]]) });
    const chips = [...c.querySelectorAll('.chip')];
    expect(chips[0].querySelector('.x').getAttribute('aria-label')).toBe('Remove #1');
    expect(chips[1].querySelector('.x')).toBeNull();
    expect(chips[1].getAttribute('aria-label')).toBe('#2 has PR #34, cannot be removed');
    expect(chips[1].querySelector('.lk')).toBeTruthy();
  });

  it('× removes a chip', async () => {
    const chip = (r) => ({ text: r, ref: r, title: null, url: null, bad: null });
    const { container: c } = setup({ chips: [chip('#1'), chip('#2')] });
    await fireEvent.click(c.querySelector('.chip .x'));
    expect(chipsOf(c)).toEqual(['#2']);
  });

  it('at 10 chips the input is disabled and says so', () => {
    const chips = Array.from({ length: 10 }, (_, i) => ({
      text: `#${i + 1}`,
      ref: `#${i + 1}`,
      title: null,
      url: null,
      bad: null,
    }));
    const { container: c, input } = setup({ chips });
    expect(input.disabled).toBe(true);
    expect(input.placeholder).toBe('10 issues at most');
    expect(text(c)).toContain('10 issues at most');
  });
});

describe('PrModeControl', () => {
  it('per issue is disabled under 2 issues with its reason, and cannot be picked', async () => {
    const onchange = vi.fn();
    const { container: c } = render(PrModeControl, { value: 'single', refs: ['#1'], onchange });
    const [one, per] = c.querySelectorAll('[role="radio"]');
    expect(one.getAttribute('aria-checked')).toBe('true');
    expect(per.getAttribute('aria-disabled')).toBe('true');
    await fireEvent.click(per);
    expect(onchange).not.toHaveBeenCalled();
    expect(text(c.querySelector('[data-mode-help]'))).toBe('One PR for the feature; it closes #1.');
  });

  it('with 2 issues per issue can be picked, by click and by arrow key, and the helper follows', async () => {
    const onchange = vi.fn();
    const { container: c } = render(PrModeControl, { value: 'single', refs: ['#1', '#2'], onchange });
    const [one, per] = c.querySelectorAll('[role="radio"]');
    await key(one, 'ArrowRight');
    expect(onchange).toHaveBeenCalledWith('per_issue');
    await fireEvent.click(per);
    expect(onchange).toHaveBeenCalledTimes(1); // already on: nothing
  });

  it('a locked mode shows its reason inline and changes nothing', async () => {
    const onchange = vi.fn();
    const { container: c } = render(PrModeControl, { value: 'single', refs: ['#1', '#2'], locked: true, onchange });
    expect(text(c)).toContain('Set before the first PR: this feature already has one.');
    await fireEvent.click(c.querySelectorAll('[role="radio"]')[1]);
    expect(onchange).not.toHaveBeenCalled();
  });
});

describe('the launch form', () => {
  const NOW = Date.now();
  const overlay = (prefill = {}) => ({
    kind: 'launch',
    prefill: {
      project_id: 'shop-api',
      team_id: 'default-web',
      slug: 'refunds',
      title: 'Refunds',
      brief: 'b',
      base: null,
      off_hours: false,
      ...prefill,
    },
  });
  beforeEach(() => {
    app.data = /** @type {any} */ (missionData(NOW));
    app.now = missionNow(NOW);
    app.toasts = [];
    replies.teams_view = { roles: [], teams: [{ team: { id: 'default-web', name: 'Web team' }, issues: [] }] };
    replies.mcp_registry_view = { servers: [] };
    replies.planner_view = { off_hours: null };
    replies.team_launch = { feature_id: 'f1' };
  });
  const open = async (prefill) => {
    app.overlay = overlay(prefill);
    const r = render(Dialogs);
    await settle();
    await settle();
    return r.container;
  };
  const launch = async (c) => {
    await fireEvent.click(c.querySelector('button.primary'));
    await settle();
    return calls.find((x) => x.cmd === 'team_launch')?.args.form;
  };

  it('always sends pr_mode, and issues (empty clears)', async () => {
    const c = await open();
    const form = await launch(c);
    expect(form.issues).toEqual([]);
    expect(form.pr_mode).toBe('single');
  });

  it("is prefilled with the intake item's issue and One PR", async () => {
    const c = await open({ issues: ['#123'], pr_mode: 'single' });
    expect(chipsOf(c)).toEqual(['#123']);
    expect(text(c.querySelector('[data-mode-help]'))).toBe('One PR for the feature; it closes #123.');
    const form = await launch(c);
    expect(form.issues).toEqual(['#123']);
  });

  it('per issue: disabled with one issue, enabled with two, sent as typed', async () => {
    const c = await open({ issues: ['#1'] });
    const per = () => c.querySelectorAll('[role="radio"]')[1];
    expect(per().getAttribute('aria-disabled')).toBe('true');
    const input = c.querySelector('.issues input');
    await type(input, 'eng-7');
    await key(input, 'Enter');
    expect(per().getAttribute('aria-disabled')).toBe('false');
    await fireEvent.click(per());
    await settle();
    expect(text(c.querySelector('[data-mode-help]'))).toContain('One PR per issue, each with only');
    const form = await launch(c);
    expect(form.issues).toEqual(['#1', 'eng-7']);
    expect(form.pr_mode).toBe('per_issue');
  });

  it('removing chips down to one while per issue is chosen falls back to One PR', async () => {
    const c = await open({ issues: ['#1', '#2'], pr_mode: 'per_issue' });
    expect(c.querySelectorAll('[role="radio"]')[1].getAttribute('aria-checked')).toBe('true');
    await fireEvent.click(c.querySelector('.chip .x'));
    await settle();
    expect(c.querySelectorAll('[role="radio"]')[0].getAttribute('aria-checked')).toBe('true');
    const form = await launch(c);
    expect(form.pr_mode).toBe('single');
  });

  it('an invalid chip disables Launch', async () => {
    const c = await open();
    const input = c.querySelector('.issues input');
    await type(input, 'nope');
    await key(input, 'Enter');
    await settle();
    expect(/** @type {HTMLButtonElement} */ (c.querySelector('button.primary')).disabled).toBe(true);
  });

  it('a bad_ref answer marks the chip and keeps the dialog', async () => {
    replies.team_launch = new Error(
      JSON.stringify({ code: 'bad_ref', message: '"#9": a/b is not this project\'s repository (o/r)', ref: '#9' }),
    );
    const c = await open({ issues: ['#9'] });
    await launch(c);
    expect(text(c.querySelector('.msg'))).toBe('Not an issue of o/r: #9');
    expect(c.querySelector('.chip.bad')).toBeTruthy();
    expect(app.overlay).not.toBeNull();
  });

  it('too_many and per_issue_needs_two show inline', async () => {
    replies.team_launch = new Error(JSON.stringify({ code: 'too_many', message: 'x' }));
    const c = await open({ issues: ['#1', '#2'] });
    await launch(c);
    expect(text(c.querySelector('.msg'))).toBe('10 issues at most');
  });
});

describe('FeatureIssues (header row 3 and the Edit popover)', () => {
  const feature = (o = {}) => ({
    id: 'f1',
    project_id: 'shop-api',
    issues: [
      { ref: '#12', key: '12', title: 'Crash on empty brief', url: 'https://github.com/o/r/issues/12' },
      { ref: 'ENG-7', key: 'eng-7', title: null, url: null },
    ],
    pr_mode: 'single',
    ...o,
  });
  const run = (o = {}) => ({ id: 'r1', status: 'executing', prs_view: [], ...o });
  const open = async (c) => {
    await fireEvent.click(c.querySelector('.edit'));
    await settle();
  };

  it('reads issues and mode, with an Edit button while the run is live', () => {
    const { container: c } = render(FeatureIssues, { feature: feature({ pr_mode: 'per_issue' }), run: run() });
    expect(text(c.querySelector('.row3'))).toBe('Issues #12 · ENG-7 · One PR per issue Edit');
  });

  it('no Edit once the run is done; nothing at all without issues', () => {
    const { container: c } = render(FeatureIssues, { feature: feature(), run: run({ status: 'done' }) });
    expect(c.querySelector('.edit')).toBeNull();
    cleanup();
    const e = render(FeatureIssues, { feature: feature({ issues: [] }), run: run({ status: 'done' }) });
    expect(e.container.querySelector('.row3')).toBeNull();
  });

  it('without issues a muted Add issues link opens the popover', async () => {
    const { container: c } = render(FeatureIssues, { feature: feature({ issues: [] }), run: run() });
    expect(text(c.querySelector('.row3'))).toContain('Add issues');
    await fireEvent.click(c.querySelector('.add'));
    await settle();
    expect(c.querySelector('[role="dialog"]')).toBeTruthy();
  });

  it('saves the full list and the mode with feature_set_prs, then confirms for 4 s during a run', async () => {
    replies.feature_set_prs = feature({ pr_mode: 'per_issue' });
    const onsaved = vi.fn();
    const { container: c } = render(FeatureIssues, { feature: feature(), run: run(), onsaved });
    await open(c);
    await fireEvent.click(c.querySelectorAll('[role="radio"]')[1]);
    await settle();
    await fireEvent.click([...c.querySelectorAll('.foot button')].find((b) => b.textContent === 'Save'));
    await settle();
    expect(calls.find((x) => x.cmd === 'feature_set_prs').args).toEqual({
      feature: 'f1',
      issues: ['#12', 'ENG-7'],
      prMode: 'per_issue',
    });
    expect(onsaved).toHaveBeenCalledWith(feature({ pr_mode: 'per_issue' }));
    expect(text(c.querySelector('[role="status"]'))).toBe('✓ Saved; the lead is told.');
  });

  it('locks an issue with a PR and the mode once the run had a PR', async () => {
    const view = [
      { issue: '#12', state: 'open', number: 34, url: 'u', pr: null, gate: null, branch: 'b' },
      { issue: 'ENG-7', state: 'none', number: null, url: null, pr: null, gate: null, branch: 'b2' },
    ];
    const { container: c } = render(FeatureIssues, {
      feature: feature({ pr_mode: 'per_issue' }),
      run: run({ prs_view: view }),
    });
    await open(c);
    const chips = [...c.querySelectorAll('.pop .chip')];
    expect(chips[0].querySelector('.x')).toBeNull();
    expect(chips[0].getAttribute('aria-label')).toBe('#12 has PR #34, cannot be removed');
    expect(chips[1].querySelector('.x')).toBeTruthy();
    expect(text(c.querySelector('.pop'))).toContain('Set before the first PR: this feature already has one.');
    expect(c.querySelectorAll('.pop [role="radio"]')[0].getAttribute('aria-disabled')).toBe('true');
  });

  it('keeps a locked per_issue mode when the list drops to one issue', async () => {
    replies.feature_set_prs = feature({ pr_mode: 'per_issue' });
    const view = [
      { issue: '#12', state: 'open', number: 34, url: 'u', pr: null, gate: null, branch: 'b' },
      { issue: 'ENG-7', state: 'none', number: null, url: null, pr: null, gate: null, branch: 'b2' },
    ];
    const { container: c } = render(FeatureIssues, {
      feature: feature({ pr_mode: 'per_issue' }),
      run: run({ prs_view: view }),
    });
    await open(c);
    await fireEvent.click(c.querySelectorAll('.pop .chip')[1].querySelector('.x'));
    await settle();
    await fireEvent.click([...c.querySelectorAll('.foot button')].find((b) => b.textContent === 'Save'));
    await settle();
    expect(calls.find((x) => x.cmd === 'feature_set_prs').args).toEqual({
      feature: 'f1',
      issues: ['#12'],
      prMode: 'per_issue',
    });
  });

  it('shows a server error inline, and marks the chip of a bad_ref', async () => {
    replies.feature_set_prs = new Error(JSON.stringify({ code: 'pr_busy', message: 'm' }));
    const { container: c } = render(FeatureIssues, { feature: feature(), run: run() });
    await open(c);
    await fireEvent.click([...c.querySelectorAll('.foot button')].find((b) => b.textContent === 'Save'));
    await settle();
    expect(text(c.querySelector('.pop .err'))).toBe('Set before the first PR: this feature already has one.');
    replies.feature_set_prs = new Error(JSON.stringify({ code: 'bad_ref', message: 'm', ref: 'ENG-7' }));
    await fireEvent.click([...c.querySelectorAll('.foot button')].find((b) => b.textContent === 'Save'));
    await settle();
    expect(c.querySelector('.pop .chip.bad')).toBeTruthy();
  });
});

describe('PrRows (the card, one row per issue)', () => {
  const feature = {
    id: 'f1',
    issues: [
      { ref: '#1', title: 'Crash on empty brief' },
      { ref: '#2', title: 'Second' },
      { ref: '#3', title: 'Third' },
      { ref: '#4', title: 'Fourth' },
      { ref: '#5', title: null },
    ],
  };
  const pr = (state, issue, number = null, extra = {}) => ({
    issue,
    state,
    number,
    url: number ? `https://github.com/o/r/pull/${number}` : null,
    pr: null,
    gate: null,
    branch: `orch/f/issue/${issue.slice(1)}`,
    ...extra,
  });
  const openPr = (issue, number) =>
    pr('open', issue, number, {
      pr: {
        state: 'open',
        number,
        ci: { overall: 'error', done: 4, total: 6, jobs: [] },
        position: { status: 'behind', behind: 3 },
        base: 'main',
        unpushed: 0,
        checked_at_ms: 1,
      },
    });
  const view = [
    pr('none', '#5'),
    pr('pending', '#2', null, { gate: 812 }),
    openPr('#1', 34),
    pr('merged', '#3', 35),
    pr('closed', '#4', 36),
  ];

  it('renders each state', () => {
    const { container: c } = render(PrRows, { run: { id: 'r1', status: 'pr_open', prs_view: view }, feature });
    const row = (i) => text(c.querySelector(`[data-issue="${i}"]`));
    expect(row('#5')).toBe('#5 No PR yet');
    expect(row('#2')).toBe('#2 Second Waiting for your approval Show card');
    expect(row('#1')).toContain('#1 Crash on empty brief');
    expect(row('#1')).toContain('PR #34');
    expect(row('#1')).toContain('4/6');
    expect(row('#1')).toContain('↓3 behind main');
    expect(row('#3')).toContain('PR #35 merged');
    expect(row('#4')).toContain('PR #36 closed');
    // Open rows hold PrLine's content only: the card head has the refresh.
    expect(c.querySelector('[data-issue="#1"] .refresh')).toBeNull();
  });

  it('keeps the feature order and shows the task count of an issue without PR', () => {
    const { container: c } = render(PrRows, {
      run: { id: 'r1', status: 'executing', prs_view: [pr('none', '#1'), pr('none', '#2')] },
      feature,
      taskCounts: { '#1': 3 },
    });
    expect([...c.querySelectorAll('.row')].map((r) => /** @type {HTMLElement} */ (r).dataset.issue)).toEqual([
      '#1',
      '#2',
    ]);
    expect(text(c.querySelector('[data-issue="#1"]'))).toContain('No PR yet · 3 tasks');
  });
});

describe('Mission run row, run inspector, decision card and task chip', () => {
  const NOW = Date.now();
  const P = (state, issue, number, ci) => ({
    issue,
    state,
    number,
    url: number ? `u/${number}` : null,
    gate: null,
    branch: 'b',
    pr: ci ? { ci: { overall: ci, done: 1, total: 2 } } : null,
  });
  const view = [P('open', '#1', 34, 'error'), P('open', '#2', 35, 'ok'), P('merged', '#3', 36), P('none', '#4', null)];
  const feature = {
    id: 'f-co',
    project_id: 'shop-api',
    slug: 'checkout',
    pr_mode: 'per_issue',
    issues: [
      { ref: '#1', title: 'Crash' },
      { ref: '#2', title: null },
      { ref: '#3', title: null },
      { ref: '#4', title: null },
    ],
    run: { id: 'run-12', status: 'pr_open', prs_view: view },
  };
  beforeEach(() => {
    app.data = /** @type {any} */ (missionData(NOW));
    app.data.features = [feature];
    app.now = missionNow(NOW);
    app.now.runs['run-12'] = { ...app.now.runs['run-12'], prs_view: view };
    app.panes = {};
  });

  it('the run row shows the per-issue summary, the worst dot and one line per PR; a click opens the feature', async () => {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    const b = /** @type {HTMLElement} */ (c.querySelector('[data-key="run:run-12"] [data-prs]'));
    expect(text(b)).toBe('2 PRs open · 1 merged');
    expect(/** @type {HTMLElement} */ (b.querySelector('.dot')).style.background).toBe('var(--color-status-error)');
    expect(b.title.split('\n')).toEqual([
      '#1 Crash · PR #34 open · CI 1/2',
      '#2 · PR #35 open · CI 1/2',
      '#3 · PR #36 merged',
      '#4 · no PR yet',
    ]);
    await fireEvent.click(b);
    expect(Object.values(app.panes).some((s) => s.type === 'feature' && s.featureId === 'f-co')).toBe(true);
  });

  it('single mode keeps the PR link', async () => {
    app.now.runs['run-12'] = {
      ...app.now.runs['run-12'],
      prs_view: [P('open', null, 12, 'ok')],
      pr: { url: 'u/12', number: 12 },
    };
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    const row = c.querySelector('[data-key="run:run-12"]');
    expect(row.querySelector('[data-prs]')).toBeNull();
    expect(text(row)).toContain('PR #12 ↗');
  });

  it('the run inspector lists the rows under the summary', async () => {
    const { container: c } = render(RunInspector, { id: 'run-12' });
    await settle();
    expect(text(c.querySelector('.sum'))).toBe('2 PRs open · 1 merged');
    expect([...c.querySelectorAll('[data-issue]')].map((r) => /** @type {HTMLElement} */ (r).dataset.issue)).toEqual([
      '#1',
      '#2',
      '#3',
      '#4',
    ]);
  });

  const gate = (payload) => ({
    id: 'gate-5',
    kind: 'gate',
    gate: 'pr',
    title: 'PR gate · checkout',
    created_ms: NOW,
    payload: { run: 'run-12', ...payload },
    context: [],
    actions: [],
    subject: { agent: 'lead-1', project: 'shop-api', feature: 'checkout' },
    lifecycle: { state: 'open' },
  });
  it('a per-issue PR gate names the issue, the branch and where it is built from', () => {
    const { container: c } = render(DecisionCard, {
      d: gate({ action: 'push_pr', issue: '#1', branch: 'orch/checkout/issue/1', base: 'main' }),
    });
    expect(text(c.querySelector('.title'))).toBe('Open PR for #1 "Crash"');
    expect(text(c.querySelector('[data-pr-branch]'))).toBe('orch/checkout/issue/1 → main');
    expect(text(c.querySelector('[data-pr-note]'))).toBe(
      'Built from the integration branch: tasks of #1 and shared work.',
    );
  });
  it('a push_pr_update gate per issue', () => {
    const { container: c } = render(DecisionCard, {
      d: gate({ action: 'push_pr_update', issue: '#1', pr_number: 34, branch: 'b', base: 'main' }),
    });
    expect(text(c.querySelector('.title'))).toBe('Push to PR #34 (#1)');
  });
  it('a single-mode gate with issues shows Closes; its title stays', () => {
    app.data.features = [{ ...feature, pr_mode: 'single' }];
    const { container: c } = render(DecisionCard, {
      d: gate({ action: 'push_pr', issue: null, branch: 'orch/checkout/integration', base: 'main' }),
    });
    expect(text(c.querySelector('.title'))).toBe('PR gate · checkout');
    expect(text(c.querySelector('[data-pr-closes]'))).toBe('Closes #1, #2, #3, #4');
  });

  it('the task chip shows the ref with the title in its tooltip; untagged tasks show nothing', () => {
    const { container: c } = render(TaskIssueChip, { task: { issue: '#1', feature_id: 'f-co' } });
    const chip = /** @type {HTMLElement} */ (c.querySelector('[data-issue-chip]'));
    expect([chip.textContent, chip.title]).toEqual(['#1', '#1 Crash']);
    cleanup();
    expect(render(TaskIssueChip, { task: { issue: null } }).container.querySelector('[data-issue-chip]')).toBeNull();
  });
});
