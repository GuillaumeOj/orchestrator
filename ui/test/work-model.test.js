// improve-intake F3 (notes §2–§5, §11): the Work browser's pure rules — glyphs, chips, roll-ups, row slots,
// visible rows (indent, depth tags, folds, more / loading / error lines), selection, filters, prefs, copy.

import { describe, expect, it } from 'vitest';
import {
  chipOf,
  cleanFilter,
  closedMark,
  crumbs,
  defaultFilter,
  depMark,
  emptyText,
  factsOf,
  filterCount,
  GLYPH,
  glyphOf,
  headerStatus,
  hintsFor,
  historyText,
  NOT_SIBLING,
  narrowed,
  noMatchText,
  prefsKey,
  prefsOf,
  progressText,
  readPrefs,
  resultsText,
  rightOf,
  rollupText,
  scopeTitle,
  selectionText,
  statusLine,
  tailOf,
  titleOf,
  toggleSelect,
  upstreamText,
  visibleRows,
} from '../src/lib/workmodel.js';
import { github, V2, wn } from './work-fixture.js';

describe('kind glyphs (§2.3)', () => {
  it('GitHub issues carry their state in the shape', () => {
    const { n13, n16, n20, m } = github();
    expect(glyphOf(n13)).toEqual({ g: GLYPH.issue_open, label: 'Open issue' });
    expect(glyphOf(n16)).toEqual({ g: GLYPH.issue_done, label: 'Closed issue, completed' });
    expect(glyphOf(n20)).toEqual({ g: GLYPH.issue_not_planned, label: 'Closed issue, not planned' });
    expect(glyphOf(m)).toEqual({ g: GLYPH.milestone, label: 'Milestone v2' });
  });

  it('Linear issues by state type, named by their state', () => {
    for (const [type, glyph] of [
      ['backlog', GLYPH.lin_backlog],
      ['unstarted', GLYPH.lin_unstarted],
      ['started', GLYPH.lin_started],
      ['completed', GLYPH.lin_completed],
      ['canceled', GLYPH.lin_canceled],
    ])
      expect(glyphOf(wn('lin:ENG-1', { upstream: { state: 'Todo', state_type: type } }))).toEqual({
        g: glyph,
        label: 'Linear issue, Todo',
      });
    expect(glyphOf(wn('lin-project:p', { kind: 'linear_project' })).label).toBe('Linear project');
    expect(glyphOf(wn('lin-milestone:m', { kind: 'linear_milestone' })).label).toBe('Project milestone');
    expect(glyphOf(wn('local:a.md', { kind: 'local' })).label).toBe('Local item');
  });
});

describe('row slots (§2.2)', () => {
  it('title, progress, upstream state, dependency mark', () => {
    const { m, n13, n14 } = github();
    expect(titleOf(m)).toBe('v2 · due 1 Dec');
    expect(progressText(m)).toBe('0 of 2');
    expect(progressText(n13)).toBe('1/2');
    expect(progressText(wn('gh:x#1'))).toBe('');
    expect(depMark(n14)).toEqual({ text: 'blocked by #13', tip: 'Blocked by #13 Address step' });
    const two = wn('gh:x#2', {
      upstream: {
        blocked_by: [
          { ref: '#1', open: true },
          { ref: '#3', open: true },
          { ref: '#4', open: false },
        ],
      },
    });
    expect(depMark(two).text).toBe('blocked by 2');
    expect(depMark(wn('gh:x#5', { upstream: { blocked_by: [{ ref: '#1', open: false }] } }))).toBeNull();
    expect(upstreamText(wn('lin:ENG-1', { upstream: { state: 'Todo' } }))).toBe('Todo');
    expect(upstreamText(wn('gh:x#1', { upstream: { board_status: 'Todo' } }))).toBe('Todo');
    expect(upstreamText(wn('gh:x#1'))).toBe('');
    expect(
      upstreamText(
        wn('lin-project:p', { kind: 'linear_project', upstream: { state: 'In progress', target_date: '2026-12-01' } }),
      ),
    ).toBe('In progress · target 1 Dec 2026');
  });

  it('tails: deleted upstream, write-back failed, closed parent with open children', () => {
    expect(tailOf(wn('gh:x#1', { upstream: { deleted: true } }))).toEqual({ warn: true, text: 'Deleted in GitHub' });
    expect(tailOf(wn('gh:x#1', { writeback: { status: 'failed' } }))).toEqual({
      warn: true,
      text: 'write-back failed',
    });
    expect(
      tailOf(wn('gh:x#30', { upstream: { state: 'closed' }, children: { total: 3, open: 2, done: 1 } })).text,
    ).toBe('closed · 2 open sub-issues');
  });
});

describe('app-state chip (§2.5)', () => {
  const { n13, n14, n15, n21, n16, n12, n9 } = github();
  it.each([
    [n13, 'working', 'running · checkout-address · executing', 'running · exec.'],
    [n14, 'waiting', 'waiting · Checkout v2', 'waiting'],
    [n15, 'queued', 'queued · Checkout v2', 'queued'],
    [n21, 'pr', 'PR #61 open', 'PR #61'],
    [n16, 'done', 'merged', 'merged'],
    [n12, 'program', 'program · 0 of 3 merged', 'program 0/3'],
    [n9, 'ready', 'ready', 'ready'],
  ])('%#: tone and long / short text', (node, tone, long, short) => {
    expect(chipOf(node)).toMatchObject({ tone, text: long });
    expect(chipOf(node, { short: true }).text).toBe(short);
  });

  it('tooltips name the program, the phase, the Ready filter', () => {
    expect(chipOf(n14).tip).toBe('In program Checkout v2: waiting for checkout-address to merge');
    expect(chipOf(n13).tip).toBe('Team run checkout-address, phase executing. g opens the feature.');
    expect(chipOf(n9, { readyTip: 'label ready' }).tip).toBe('Matches the Ready filter (label ready)');
    expect(chipOf(wn('gh:x#1', { app: { state: 'solo', agent: 'a1', name: 'fix-feed' } }))).toMatchObject({
      text: 'solo · fix-feed',
    });
  });

  it('no chip: app none and not ready, or hidden (it loses the ready chip)', () => {
    expect(chipOf(wn('gh:x#1'))).toBeNull();
    expect(chipOf({ ...n9, hidden: true })).toBeNull();
  });

  it('running and closed upstream: chip plus the closed mark', () => {
    const n = { ...n15, upstream: { ...n15.upstream, state: 'closed' } };
    expect(closedMark(n)).toEqual({
      text: 'closed upstream',
      tip: 'Closed in GitHub while the feature runs. It is not stopped.',
    });
    expect(closedMark(n15)).toBeNull();
  });
});

describe('roll-up (§2.6)', () => {
  it('one word list and order, zero counts omitted, > 3 parts shorten', () => {
    expect(rollupText({ running: 1, waiting: 1, merged: 1 })).toBe('1 running · 1 waiting · 1 merged');
    expect(rollupText({ running: 1, waiting: 1, queued: 1, merged: 1 })).toBe('1 running · 3 more');
    expect(rollupText({ pr: 2, failed: 1 })).toBe('2 PR open · 1 failed');
    expect(rollupText({})).toBe('');
  });

  it('a parent with no state of its own shows the roll-up; with one, its chip', () => {
    const { m, n12 } = github();
    expect(rightOf(m)).toEqual({ rollup: '1 running · 3 more', tip: '1 running, 1 waiting, 1 queued, 1 merged' });
    expect(rightOf(n12).chip.text).toBe('program · 0 of 3 merged');
  });
});

describe('visible rows', () => {
  const base = (over = {}) => {
    const g = github();
    const nodes = new Map(Object.values(g).map((n) => [n.id, n]));
    return {
      scope: [{ id: 'shop-api', name: 'shop-api' }],
      multi: false,
      nodes,
      kids: new Map([
        ['roots', [V2, g.n21.id, g.n20.id]],
        [V2, [g.n12.id, g.n9.id]],
        [g.n12.id, [g.n13.id, g.n14.id, g.n15.id]],
        [g.n13.id, [g.n16.id, g.n17.id]],
      ]),
      more: {},
      expanded: new Set([V2, g.n12.id, g.n13.id]),
      collapsedProjects: new Set(),
      loading: new Set(),
      errors: new Map(),
      folds: new Set(),
      tasks: new Map(),
      focusRoot: null,
      statusLines: null,
      ...over,
    };
  };
  const keys = (rows) => rows.map((r) => r.key);

  it('one project: no project row, roots indent 0, the Team tasks fold last under its item', () => {
    const rows = visibleRows(base());
    expect(keys(rows)).toEqual([
      V2,
      'gh:acme/shop#12',
      'gh:acme/shop#13',
      'gh:acme/shop#16',
      'gh:acme/shop#17',
      'fold:gh:acme/shop#13',
      'gh:acme/shop#14',
      'gh:acme/shop#15',
      'gh:acme/shop#9',
      'gh:acme/shop#21',
      'gh:acme/shop#20',
    ]);
    expect(rows.map((r) => r.indent)).toEqual([0, 1, 2, 3, 3, 3, 2, 2, 1, 0, 0]);
    expect(rows.find((r) => r.kind === 'fold')).toMatchObject({ count: 4, open: false });
  });

  it('several projects: a level-0 project row each, its status line under it', () => {
    const t = base({
      scope: [
        { id: 'shop-api', name: 'shop-api' },
        { id: 'mobile', name: 'mobile' },
      ],
      multi: true,
      expanded: new Set(),
      collapsedProjects: new Set(['mobile']),
      statusLines: new Map([['mobile', { tone: 'error', text: 'mobile · Linear: no API key', fix: 'integrations' }]]),
    });
    const rows = visibleRows(t);
    expect(keys(rows)).toEqual([
      'project:shop-api',
      V2,
      'gh:acme/shop#21',
      'gh:acme/shop#20',
      'project:mobile',
      'srcline:mobile',
    ]);
    expect(rows.map((r) => r.indent)).toEqual([0, 1, 1, 1, 0, 1]);
  });

  it('loading, error, more and the GitHub 100 note', () => {
    const g = github();
    const big = { ...g.n12, children: { total: 100, open: 100, done: 0 } };
    const t = base({
      loading: new Set([g.n13.id]),
      kids: new Map([
        ['roots', [V2]],
        [V2, [big.id]],
        [big.id, ['gh:acme/shop#14']],
      ]),
    });
    t.nodes.set(big.id, big);
    t.more = { [big.id]: { left: 50, cursor: 'c' } };
    t.expanded = new Set([V2, big.id, g.n14.id]);
    t.errors = new Map([[g.n14.id, 'GitHub timed out']]);
    const rows = visibleRows(t);
    expect(rows.map((r) => r.kind)).toEqual(['node', 'node', 'node', 'error', 'more', 'limit']);
    expect(rows.find((r) => r.kind === 'more')).toMatchObject({ left: 50, shown: 1 });
    t.expanded = new Set([V2, big.id, g.n13.id]);
    t.kids.set(big.id, [g.n13.id]);
    t.kids.delete(g.n13.id);
    expect(visibleRows(t).some((r) => r.kind === 'loading')).toBe(true);
  });

  it('depth beyond 4: indent stops at level 4, focus mode re-roots', () => {
    const chain = [40, 41, 42, 43, 44, 45].map((n, i) =>
      wn(`gh:acme/shop#${n}`, {
        parent: i ? `gh:acme/shop#${n - 1}` : null,
        depth: i + 1,
        children: { total: i < 5 ? 1 : 0, open: 1, done: 0 },
      }),
    );
    const nodes = new Map(chain.map((n) => [n.id, n]));
    const kids = new Map(
      /** @type {[string, string[]][]} */ ([
        ['roots', [chain[0].id]],
        ...chain.slice(0, 5).map((n, i) => [n.id, [chain[i + 1].id]]),
      ]),
    );
    const t = base({ nodes, kids, expanded: new Set(chain.map((n) => n.id)) });
    const rows = visibleRows(t);
    expect(rows.map((r) => [r.level, r.indent])).toEqual([
      [1, 0],
      [2, 1],
      [3, 2],
      [4, 3],
      [5, 4],
      [6, 4],
    ]);
    const focused = visibleRows({ ...t, focusRoot: chain[3].id });
    expect(focused.map((r) => [r.key, r.indent])).toEqual([
      ['gh:acme/shop#43', 0],
      ['gh:acme/shop#44', 1],
      ['gh:acme/shop#45', 2],
    ]);
  });
});

describe('selection (§5.4)', () => {
  const { n13, n14, n15, n9, n20 } = github();
  it('siblings only; non-siblings and closed items refused', () => {
    let r = toggleSelect(null, n13);
    expect(r.sel).toEqual({ parent: 'gh:acme/shop#12', ids: [n13.id] });
    r = toggleSelect(r.sel, n14);
    r = toggleSelect(r.sel, n15);
    expect(r.sel.ids).toHaveLength(3);
    expect(toggleSelect(r.sel, n9)).toEqual({ refused: NOT_SIBLING });
    expect(toggleSelect(r.sel, n20)).toEqual({ refused: '#20 is closed.' });
    expect(toggleSelect(null, { ...n13, team_task: true })).toEqual({ refused: null });
    expect(toggleSelect({ parent: n13.parent, ids: [n13.id] }, n13)).toEqual({ sel: null });
  });

  it('"3 selected under #12 Checkout v2"', () => {
    const { n12 } = github();
    expect(selectionText({ ids: [1, 2, 3] }, n12)).toBe('3 selected under #12 Checkout v2');
    expect(selectionText({ ids: [1] }, null, 'shop-api')).toBe('1 selected in shop-api');
  });
});

describe('hint row (§12.1, P4.2 §1.9): only the keys the focused row allows', () => {
  it('a ready, startable leaf', () => {
    const { n9 } = github();
    const keys = hintsFor({ kind: 'node', node: n9 }).map(([k]) => k);
    expect(keys).toEqual(['⏎', 't', 'a', 'o', '⌫', 'x', '/']);
  });
  it('a running item with children: g, z; no start keys', () => {
    const { n13 } = github();
    const keys = hintsFor({ kind: 'node', node: n13 }).map(([k]) => k);
    expect(keys).toEqual(['⏎', 'g', 'o', '⌫', 'x', 'z', '/']);
  });
  it('a program root: p opens the program', () => {
    const { n12 } = github();
    expect(hintsFor({ kind: 'node', node: n12 })).toContainEqual(['p', 'open program']);
  });
  it('with a selection: the selection keys only', () => {
    expect(hintsFor(null, { selection: 3 }).map(([k]) => k)).toEqual(['x', '⇧J ⇧K', 'p', 'Esc']);
  });
  it('focus mode adds ⇧Z and Esc', () => {
    const { n13 } = github();
    expect(hintsFor({ kind: 'node', node: n13 }, { focusMode: true }).map(([k]) => k)).toContain('⇧Z');
  });
});

describe('filters and prefs (§4.2, F3, F4)', () => {
  it('the count leaves out Hide closed and Show hidden', () => {
    expect(filterCount(defaultFilter())).toBe(0);
    expect(
      filterCount({ ...defaultFilter(), only_ready: true, milestone: 'v2', show_hidden: true, state: 'all' }),
    ).toBe(2);
    expect(narrowed(defaultFilter())).toBe(false);
    expect(narrowed({ ...defaultFilter(), state: 'closed' })).toBe(true);
  });

  it('a saved filter is cleaned', () => {
    expect(cleanFilter({ state: 'bogus', labels: 'x', cycle: 'later', extra: 1, only_ready: 1 })).toEqual({
      ...defaultFilter(),
      only_ready: true,
    });
  });

  it('the key is the sorted project ids; ≤ 500 expanded ids', () => {
    expect(prefsKey(['mobile', 'blog', 'shop-api'])).toBe('work.view.blog,mobile,shop-api');
    const expanded = new Set(Array.from({ length: 600 }, (_, i) => `n${i}`));
    const saved = prefsOf({ filter: defaultFilter(), expanded, focused: 'n1', details_open: false, focus_root: null });
    expect(saved.expanded).toHaveLength(500);
    expect(readPrefs(saved)).toMatchObject({ focused: 'n1', details_open: false, filter: defaultFilter() });
    expect(readPrefs(null)).toMatchObject({ expanded: [], details_open: true, focus_root: null });
  });

  it('tab titles (§1.2)', () => {
    expect(scopeTitle(['shop-api'], ['shop-api'])).toBe('Work · shop-api');
    expect(scopeTitle(['a', 'b', 'c'], [])).toBe('Work · 3 projects');
    expect(scopeTitle(null, [])).toBe('Work · all projects');
  });
});

describe('states copy (§11)', () => {
  const now = Date.parse('2026-11-13T12:00:00');
  it('header: first load, failed sources, refreshing, offline, huge tree, updated', () => {
    expect(
      headerStatus({ 'shop-api': { status: 'refreshing' } }, { firstLoad: true, names: { 'shop-api': 'acme/shop' } })
        .text,
    ).toBe('Loading acme/shop…');
    expect(headerStatus({ a: { status: 'error', error: { code: 'gh_auth' } } }, { now })).toMatchObject({
      text: '1 source failed',
      tone: 'error',
    });
    expect(headerStatus({ a: { status: 'refreshing' } }, { now }).text).toBe('refreshing…');
    expect(headerStatus({ a: { status: 'ok', offline: true, refreshed_at_ms: now - 2 * 3600000 } }, { now }).text).toBe(
      'Offline · updated 2 h ago',
    );
    expect(headerStatus({ a: { status: 'ok', refreshed_at_ms: now - 4 * 60000 } }, { now, rootsTotal: 512 }).text).toBe(
      '512 at the top · updated 4 min ago',
    );
    const limited = headerStatus(
      {
        a: { status: 'error', error: { code: 'rate_limited', retry_at_ms: now + 60000 }, refreshed_at_ms: now - 60000 },
      },
      { now },
    );
    expect(limited).toMatchObject({ text: 'updated 1 min ago', refreshDisabled: true });
  });

  it('status lines: offline, error with Fix, complexity, gh_sub_issues, gh_scope', () => {
    const p = { id: 'shop-api', name: 'shop-api', label: 'acme/shop', kind: 'github' };
    expect(statusLine(p, { status: 'ok', offline: true, refreshed_at_ms: now - 3600000 }, now).text).toMatch(
      /^Showing what was cached at \d\d:\d\d\. Starting still works; write-back waits\.$/,
    );
    expect(
      statusLine(p, { status: 'error', error: { code: 'gh_auth' }, refreshed_at_ms: now - 3 * 3600000 }, now),
    ).toMatchObject({
      tone: 'error',
      text: 'shop-api · GitHub: not logged in · items kept from 3 h ago',
      fix: 'integrations',
      fixLabel: 'Fix',
    });
    expect(
      statusLine({ ...p, kind: 'linear' }, { status: 'error', error: { code: 'linear_complexity' } }, now).text,
    ).toBe('Linear said the request was too large. Work shows what is cached. Some items may be missing.');
    expect(statusLine(p, { status: 'ok', warning: 'gh_sub_issues' }, now)).toMatchObject({
      text: "GitHub didn't return sub-issues for acme/shop. Work shows issues without hierarchy.",
      fixLabel: 'Retry',
    });
    expect(statusLine(p, { status: 'error', error: { code: 'gh_scope' } }, now)).toMatchObject({
      text: 'shop-api · GitHub needs the read:project scope to read the board',
      fixLabel: 'Copy command',
      command: 'gh auth refresh -s read:project',
    });
    expect(statusLine(p, { status: 'ok' }, now)).toBeNull();
  });

  it('empty tracker per kind (S7), no match (S8), search header', () => {
    expect(emptyText({ label: 'acme/shop', tracker: { kind: 'github' } })).toBe(
      'No open issues in acme/shop. Create them in GitHub; Work picks them up at the next refresh (r).',
    );
    expect(emptyText({ tracker: { kind: 'linear', teams: ['ENG'] } })).toBe('No issues in ENG.');
    expect(emptyText({ tracker: { kind: 'local', backlog_glob: '.orchestrator/backlog/*.md' } })).toBe(
      'No backlog files in .orchestrator/backlog/. Add a .md file there.',
    );
    expect(noMatchText(3, ['shop-api'])).toBe('Nothing matches 3 filters in shop-api');
    expect(resultsText({ total: 12 }, 'address', [])).toBe('12 results for "address"');
    expect(resultsText({ total: 140 }, 'a', [])).toBe('100+ results: refine the search');
    expect(resultsText({ total: 0 }, 'adress', ['shop-api', 'mobile'])).toBe(
      'No results for "adress" in shop-api, mobile',
    );
  });
});

describe('details copy (§3)', () => {
  it('facts lines per tracker', () => {
    const { n13 } = github();
    expect(factsOf(n13)).toBe('Feature · open · checkout, ready · assignee gu · milestone v2');
    const lin = wn('lin:ENG-101', { upstream: { state: 'Todo', priority: 2, estimate: 3, cycle: 'Cycle 14' } });
    expect(factsOf(lin)).toBe('Todo · High · estimate 3 · cycle 14');
  });

  it('history lines (F6) and the breadcrumb rule', () => {
    const n = github().n13;
    expect(historyText({ what: 'started', feature: 'checkout-address', program: { title: 'Checkout v2' } }, n)).toBe(
      'Started in program Checkout v2 · feature checkout-address',
    );
    expect(historyText({ what: 'spec_approved' }, n)).toBe('Spec approved');
    expect(historyText({ what: 'writeback_failed' }, n)).toBe('Could not update #13 in GitHub');
    const c = crumbs(
      [
        { id: 'a', ref: 'shop-api', title: 'shop-api' },
        { id: 'b', ref: '#40', title: 'Platform migration' },
        { id: 'c', ref: '#41', title: 'Config' },
        { id: 'd', ref: '#43', title: 'Split env files' },
      ],
      3,
    );
    expect(c.map((x) => x.text)).toEqual(['shop-api', '…', '#41 Config', '#43 Split env files']);
  });
});
