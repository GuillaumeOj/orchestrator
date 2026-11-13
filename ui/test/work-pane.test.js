// improve-intake F3 (notes §2–§5, §11, §12.1): the Work pane against a mocked core — rows and chips, keys,
// selection, filters and view prefs, search and reveal, details, focus here, states, entry points.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as A from '../src/lib/app.svelte.js';
import { app } from '../src/lib/app.svelte.js';
import { LEADER } from '../src/lib/keyboard.js';
import { fixture, nowOf } from './fixture.js';
import { github, STATUS_OK, treeReply, V2, wn } from './work-fixture.js';

const calls = [];
const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    const r = replies[cmd];
    return r instanceof Error ? Promise.reject(r) : Promise.resolve(typeof r === 'function' ? r(args) : (r ?? null));
  },
  listen: (ev, fn) => {
    listeners[ev] = fn;
    return Promise.resolve(() => {});
  },
  errText: (e) => String(e?.message ?? e),
}));
const listeners = {};

const { default: WorkPane } = await import('../src/panes/WorkPane.svelte');
const { openWork } = await import('../src/lib/work.js');

const called = (cmd) => calls.filter(([c]) => c === cmd).map(([, a]) => a);
const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const row = (c, key) => c.querySelector(`[data-key="${key}"]`);
const press = (el, key, opts = {}) => fireEvent.keyDown(el, { key, ...opts });

const TRACKER = {
  settings: {
    tracker: { kind: 'github', repo: 'acme/shop', project_board: null },
    ready: { kind: 'github', labels: ['ready'], milestone: null, board_status: null, issue_type: null },
    write_back: { enabled: true },
    refresh_secs: 600,
    group_by_milestone: true,
  },
  migration_notice: null,
};

let g;
/** @param {{ prefs?: any, nodes?: any[] | null, status?: any, more?: any }} [o] */
function setup({ prefs = null, nodes = null, status = undefined, more = undefined } = {}) {
  g = github();
  replies.settings_get = (a) => (a.key.startsWith('work.view.') ? prefs : null);
  replies.tracker_settings = TRACKER;
  replies.work_tree = treeReply(nodes ?? Object.values(g), { status, more });
  replies.work_node = (a) => ({
    ...g.n13,
    id: a.id,
    breadcrumb: [{ id: V2, ref: 'v2', title: 'v2' }],
    body: 'Customers enter a delivery address.',
    history: [],
  });
}

/** @param {any} [o] */
async function mount(projects = ['shop-api'], o = {}) {
  setup(o);
  app.panes = { w1: { type: 'work', projects } };
  const r = render(WorkPane, { paneId: 'w1' });
  await waitFor(() => expect(r.container.querySelector('[role="tree"] [data-key]')).toBeTruthy());
  return { ...r, pane: r.container.querySelector('.work') };
}

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.now = nowOf(app.data);
  app.panes = {};
  app.layout = null;
  app.toasts = [];
  app.overlay = null;
  app.width = 1600;
});
afterEach(() => vi.useRealTimers());

describe('rows, chips, roll-ups (§2.2–§2.6)', () => {
  it('one project: the roots without a project row; the milestone shows its roll-up', async () => {
    const { container } = await mount();
    expect(container.querySelector('.row.project')).toBeNull();
    const m = row(container, V2);
    expect(text(m)).toContain('v2 · due 1 Dec');
    expect(text(m)).toContain('0 of 2');
    expect(m.querySelector('.rollup').textContent).toBe('1 running · 3 more');
    expect(text(row(container, 'gh:acme/shop#21'))).toContain('PR #61 open');
    expect(row(container, 'gh:acme/shop#21').querySelector('[data-chip="pr"]')).toBeTruthy();
    expect(text(container.querySelector('h2'))).toBe('Work · shop-api');
    expect(text(container.querySelector('[data-status]'))).toBe('updated 4 min ago');
  });

  it('saved expanded nodes come back in one call (F37); rows show type, labels ≤ 2, dependency, progress', async () => {
    const { container } = await mount(['shop-api'], { prefs: { expanded: [V2, 'gh:acme/shop#12'] } });
    await waitFor(() => expect(row(container, 'gh:acme/shop#14')).toBeTruthy());
    expect(called('work_tree').at(-1).parents).toEqual([V2, 'gh:acme/shop#12']);
    const r14 = row(container, 'gh:acme/shop#14');
    expect(text(r14)).toContain('#14 Feature Payment step');
    expect(text(r14)).toContain('blocked by #13');
    expect(text(r14)).toContain('0/1');
    expect(text(r14)).toContain('waiting · Checkout v2');
    expect(text(row(container, 'gh:acme/shop#13'))).toContain('running · checkout-address · executing');
    expect(text(row(container, 'gh:acme/shop#12'))).toContain('program · 0 of 3 merged');
  });

  it('1280: short chips, labels dropped, the dependency mark as a glyph', async () => {
    app.width = 1280;
    const { container } = await mount(['shop-api'], { prefs: { expanded: [V2, 'gh:acme/shop#12'] } });
    await waitFor(() => expect(row(container, 'gh:acme/shop#14')).toBeTruthy());
    expect(row(container, 'gh:acme/shop#13').querySelector('[data-chip]').textContent).toBe('running · exec.');
    expect(row(container, 'gh:acme/shop#14').querySelector('.label')).toBeNull();
    expect(row(container, 'gh:acme/shop#14').querySelector('.dep .sr').textContent).toBe('blocked by #13');
  });

  it('several projects: project rows with the tracker and the ready count', async () => {
    app.data.projects.push({ id: 'mobile', name: 'mobile', path: '/m', git: {} });
    const { container } = await mount(['shop-api', 'mobile']);
    await waitFor(() => expect(text(row(container, 'project:shop-api'))).toContain('shop-api · GitHub acme/shop'));
    expect(text(container.querySelector('h2'))).toBe('Work · 2 projects');
  });
});

describe('keys (§12.1)', () => {
  it('j / k move, l expands (children load), h collapses then goes to the parent', async () => {
    const { container, pane } = await mount();
    await waitFor(() => expect(row(container, V2).classList.contains('focused')).toBe(true));
    await press(pane, 'l');
    await waitFor(() => expect(row(container, 'gh:acme/shop#12')).toBeTruthy());
    expect(called('work_tree').at(-1).parents).toEqual([V2]);
    await press(pane, 'l'); // expanded: first child
    expect(row(container, 'gh:acme/shop#12').classList.contains('focused')).toBe(true);
    await press(pane, 'j');
    expect(row(container, 'gh:acme/shop#9').classList.contains('focused')).toBe(true);
    await press(pane, 'h'); // a leaf: to its parent
    expect(row(container, V2).classList.contains('focused')).toBe(true);
    await press(pane, 'h'); // collapse
    expect(row(container, 'gh:acme/shop#12')).toBeNull();
  });

  it('t / a open the prefilled dialog through work_prefill; nothing starts on a key', async () => {
    replies.work_prefill = { session: { project_id: 'shop-api', name: 'flaky', message: 'm' } };
    const { container, pane } = await mount(['shop-api'], { prefs: { expanded: [V2], focused: 'gh:acme/shop#9' } });
    await waitFor(() => expect(row(container, 'gh:acme/shop#9')?.classList.contains('focused')).toBe(true));
    await press(pane, 'a');
    await waitFor(() => expect(app.overlay?.kind).toBe('new-session'));
    expect(called('work_prefill')).toEqual([{ id: 'gh:acme/shop#9', mode: 'solo' }]);
    expect(app.overlay.intake).toEqual({ source: 'gh:acme/shop#9' });
    expect(calls.some(([c]) => c === 'agent_start' || c === 'feature_launch')).toBe(false);
  });

  it('a disabled start says why; p on a program root opens the program (stub until F5)', async () => {
    const { container, pane } = await mount(['shop-api'], { prefs: { focused: 'gh:acme/shop#20' } });
    await waitFor(() => expect(row(container, 'gh:acme/shop#20')?.classList.contains('focused')).toBe(true));
    await press(pane, 't');
    expect(app.toasts.at(-1).text).toBe('Closed in GitHub. Reopen it there to start it.');
    expect(called('work_prefill')).toEqual([]);
    await press(pane, 'k');
    await press(pane, 'k');
    await press(pane, 'p'); // the milestone: Run as program
    expect(app.toasts.at(-1).text).toContain('Run as program · v2');
  });

  it('⌫ hides from Ready with Undo; u undoes within 5 s', async () => {
    replies.work_hide = null;
    replies.work_unhide = null;
    const { container, pane } = await mount(['shop-api'], { prefs: { focused: 'gh:acme/shop#21' } });
    await waitFor(() => expect(row(container, 'gh:acme/shop#21')?.classList.contains('focused')).toBe(true));
    await press(pane, 'Backspace');
    await waitFor(() =>
      expect(app.toasts.at(-1)?.text).toBe('#21 hidden from Ready. It comes back if it changes in GitHub.'),
    );
    expect(app.toasts.at(-1).action.label).toBe('Undo u');
    expect(called('work_hide')).toEqual([{ id: 'gh:acme/shop#21' }]);
    await press(pane, 'u');
    await waitFor(() => expect(called('work_unhide')).toEqual([{ id: 'gh:acme/shop#21' }]));
  });

  it('o opens the item upstream; g shows the feature; r refreshes the scope', async () => {
    replies.work_refresh = null;
    const { container, pane } = await mount(['shop-api'], { prefs: { focused: 'gh:acme/shop#21' } });
    await waitFor(() => expect(row(container, 'gh:acme/shop#21')?.classList.contains('focused')).toBe(true));
    await press(pane, 'o');
    expect(called('open_url')).toEqual([{ url: 'https://github.com/acme/shop/issues/21' }]);
    const open = vi.spyOn(A, 'openFeature');
    app.data.features = [{ id: 'f-pr', project_id: 'shop-api', slug: 'partial-refunds' }];
    await press(pane, 'g');
    await press(pane, 'r');
    expect(called('work_refresh')).toEqual([{ project: 'shop-api' }]);
    expect(text(container.querySelector('[data-status]'))).toBe('refreshing…');
    open.mockRestore();
  });

  it('Esc: clear search → clear selection → close details → leave focus mode', async () => {
    replies.work_search = { nodes: [], total: 0, hidden_by_filters: 0 };
    const { container, pane } = await mount(['shop-api'], {
      prefs: { expanded: [V2, 'gh:acme/shop#12'], focused: 'gh:acme/shop#13' },
    });
    await waitFor(() => expect(row(container, 'gh:acme/shop#13')?.classList.contains('focused')).toBe(true));
    await press(pane, 'z');
    await waitFor(() => expect(container.querySelector('.fcrumbs')).toBeTruthy());
    expect(text(container.querySelector('h2'))).toBe('Work · shop-api › #13');
    await press(pane, 'x');
    expect(container.querySelector('[data-selbar]')).toBeTruthy();
    await press(pane, 'Escape');
    expect(container.querySelector('[data-selbar]')).toBeNull();
    expect(container.querySelector('.details')).toBeTruthy();
    await press(pane, 'Escape');
    expect(container.querySelector('.details')).toBeNull();
    await press(pane, 'Escape');
    expect(container.querySelector('.fcrumbs')).toBeNull();
    expect(text(container.querySelector('h2'))).toBe('Work · shop-api');
    await press(pane, ' ');
    expect(container.querySelector('.details')).toBeTruthy();
  });

  it('⌘; b is the Work browser; the shortcut sheet lists it', () => {
    expect(LEADER.b).toBe('work.browser');
    expect(A.commands().find((c) => c.id === 'work.browser')?.hint).toBe('⌘+; b');
    expect(A.commands().map((c) => c.label)).toEqual(
      expect.arrayContaining(['Open Work (all projects)', 'Find work item…', 'Refresh Work', 'Work: shop-api']),
    );
  });
});

describe('focus here (§2.4)', () => {
  it('z re-roots with a breadcrumb bar; ⇧Z goes up one level; the first segment leaves', async () => {
    const { container, pane } = await mount(['shop-api'], {
      prefs: { expanded: [V2, 'gh:acme/shop#12'], focused: 'gh:acme/shop#13' },
    });
    await waitFor(() => expect(row(container, 'gh:acme/shop#13')?.classList.contains('focused')).toBe(true));
    await press(pane, 'z');
    await waitFor(() => expect(row(container, 'gh:acme/shop#16')).toBeTruthy());
    expect(row(container, V2)).toBeNull();
    expect(text(container.querySelector('.fcrumbs')).replace(/ ?› ?/g, ' › ')).toContain(
      'shop-api › v2 › #12 Checkout v2 › #13 Address step',
    );
    await press(pane, 'Z', { shiftKey: true });
    expect(text(container.querySelector('h2'))).toBe('Work · shop-api › #12');
    await fireEvent.click(container.querySelector('.fcrumbs button'));
    expect(container.querySelector('.fcrumbs')).toBeNull();
  });

  it('level ≥ 5 rows keep the level-4 indent and show L5', async () => {
    const chain = [40, 41, 42, 43, 44].map((n, i) =>
      wn(`gh:acme/shop#${n}`, {
        parent: i ? `gh:acme/shop#${n - 1}` : null,
        depth: i + 1,
        children: { total: i < 4 ? 1 : 0, open: 1, done: 0 },
      }),
    );
    const { container } = await mount(['shop-api'], { nodes: chain, prefs: { expanded: chain.map((n) => n.id) } });
    await waitFor(() => expect(row(container, 'gh:acme/shop#44')).toBeTruthy());
    expect(row(container, 'gh:acme/shop#44').querySelector('.ltag').textContent).toBe('L5');
    expect(row(container, 'gh:acme/shop#44').querySelectorAll('.guide')).toHaveLength(4);
  });
});

describe('selection (§5.4, S19)', () => {
  it('x on siblings; ⇧J extends; a non-sibling is refused with a toast; p runs selected', async () => {
    const { container, pane } = await mount(['shop-api'], {
      prefs: { expanded: [V2, 'gh:acme/shop#12'], focused: 'gh:acme/shop#13' },
    });
    await waitFor(() => expect(row(container, 'gh:acme/shop#13')?.classList.contains('focused')).toBe(true));
    await press(pane, 'x');
    expect(text(container.querySelector('[data-selbar]'))).toContain('1 selected under #12 Checkout v2');
    expect(/** @type {HTMLButtonElement} */ (container.querySelector('[data-selbar] .btn.primary')).disabled).toBe(
      true,
    );
    await press(pane, 'J', { shiftKey: true }); // #16 (child of #13, collapsed? no: #13 not expanded) → #14
    await press(pane, 'J', { shiftKey: true });
    expect(text(container.querySelector('[data-selbar]'))).toContain('3 selected under #12 Checkout v2');
    expect(text(container.querySelector('.details'))).toContain('3 selected under #12 Checkout v2');
    await press(pane, 'j'); // #9, under v2
    await press(pane, 'x');
    expect(app.toasts.at(-1).text).toBe('Select items under the same parent to run them as one program.');
    await press(pane, 'p');
    expect(app.toasts.at(-1).text).toContain('3 selected');
  });
});

describe('filters and view prefs (§4.2, F4)', () => {
  it('f opens the popover; Only ready applies live; prefs saved per scope', async () => {
    const { container, pane } = await mount();
    await press(pane, 'f');
    const pop = container.querySelector('.pop');
    expect(pop).toBeTruthy();
    expect(text(pop)).toContain('GitHub · shop-api');
    await fireEvent.click([...pop.querySelectorAll('label.cb input')][0]);
    await waitFor(() => expect(called('work_tree').at(-1).filter.only_ready).toBe(true));
    expect(text(container.querySelector('.fwrap > .btn'))).toBe('Filters · 1');
    expect(container.querySelector('.btn.toggle[aria-pressed="true"]')).toBeTruthy();
    await waitFor(
      () =>
        expect(called('settings_set').some((a) => a.key === 'work.view.shop-api' && a.value.filter.only_ready)).toBe(
          true,
        ),
      {
        timeout: 2000,
      },
    );
  });

  it('Hide closed mirrors the State filter', async () => {
    const { container } = await mount();
    const hide = [...container.querySelectorAll('.btn.toggle')].find((b) => b.textContent.startsWith('Hide closed'));
    expect(hide.getAttribute('aria-pressed')).toBe('true');
    await fireEvent.click(hide);
    await waitFor(() => expect(called('work_tree').at(-1).filter.state).toBe('all'));
  });
});

describe('search (§4.1)', () => {
  it('results replace the tree with breadcrumbs; ⏎ reveals in the tree', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    replies.work_search = {
      nodes: [
        {
          ...github().n17,
          breadcrumb: [
            { id: V2, ref: 'v2', title: 'v2' },
            { id: 'gh:acme/shop#12', ref: '#12', title: 'Checkout v2' },
          ],
        },
      ],
      total: 1,
      hidden_by_filters: 2,
    };
    replies.work_reveal = () => ({
      chain: [V2, 'gh:acme/shop#12', 'gh:acme/shop#13', 'gh:acme/shop#17'],
      nodes: Object.values(g),
    });
    const { container, pane } = await mount();
    await press(pane, '/');
    const input = /** @type {HTMLInputElement} */ (container.querySelector('input[type="search"]'));
    expect(document.activeElement).toBe(input);
    await fireEvent.input(input, { target: { value: 'address' } });
    await vi.advanceTimersByTimeAsync(260);
    await waitFor(() => expect(container.querySelector('.result')).toBeTruthy());
    expect(called('work_search')[0]).toMatchObject({ query: 'address', scope: { projects: ['shop-api'] } });
    expect(text(container.querySelector('[data-results]'))).toContain('1 result for "address"');
    expect(text(container.querySelector('[data-results]'))).toContain('2 more hidden by filters');
    expect(text(container.querySelector('.result'))).toContain('acme/shop › v2 › #12 Checkout v2');
    await fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(row(container, 'gh:acme/shop#17')?.classList.contains('focused')).toBe(true));
    expect(input.value).toBe('');
  });

  it('no results: copy and Clear search', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    replies.work_search = { nodes: [], total: 0, hidden_by_filters: 0 };
    const { container } = await mount();
    const input = container.querySelector('input[type="search"]');
    await fireEvent.input(input, { target: { value: 'adress' } });
    await vi.advanceTimersByTimeAsync(260);
    await waitFor(() =>
      expect(text(container.querySelector('[data-results]'))).toContain('No results for "adress" in shop-api'),
    );
  });
});

describe('details (§3, F36)', () => {
  it('first paint from the tree, body after 300 ms, cached by id + updated_at_ms', async () => {
    const { container, pane } = await mount(['shop-api'], {
      prefs: { expanded: [V2, 'gh:acme/shop#12'], focused: 'gh:acme/shop#13' },
    });
    await waitFor(() => expect(text(container.querySelector('.details'))).toContain('Address step'));
    expect(text(container.querySelector('.details'))).toContain(
      'Feature · open · checkout, ready · assignee gu · milestone v2',
    );
    await waitFor(
      () => expect(text(container.querySelector('.details'))).toContain('Customers enter a delivery address.'),
      { timeout: 2000 },
    );
    expect(called('work_node')).toEqual([{ id: 'gh:acme/shop#13', opts: { children: true } }]);
    expect(text(container.querySelector('.details'))).toContain('Blocking #14 Payment step');
    expect(text(container.querySelector('.details'))).toContain('#58 Postcode lookup · merged');
    expect(text(container.querySelector('.details'))).toContain('Nothing yet. Start it here.');
    expect(container.querySelector('.details [data-act="feature"]').textContent).toBe('Show feature');
    await press(pane, 'j');
    await press(pane, 'k');
    await new Promise((r) => setTimeout(r, 400));
    expect(called('work_node')).toHaveLength(1);
  });
});

describe('states (§11)', () => {
  it('S1 first load: Loading header and skeleton', async () => {
    setup({ nodes: [], status: { 'shop-api': { status: 'refreshing', refreshed_at_ms: null } } });
    app.panes = { w1: { type: 'work', projects: ['shop-api'] } };
    const { container } = render(WorkPane, { paneId: 'w1' });
    await waitFor(() => expect(text(container.querySelector('[data-status]'))).toBe('Loading acme/shop…'));
    expect(container.querySelector('.skel')).toBeTruthy();
  });

  it('S4 source error: the line with Fix; S3 offline', async () => {
    setup({
      status: {
        'shop-api': { status: 'error', error: { code: 'gh_auth' }, refreshed_at_ms: Date.now() - 3 * 3600000 },
      },
    });
    app.panes = { w1: { type: 'work', projects: ['shop-api'] } };
    const { container } = render(WorkPane, { paneId: 'w1' });
    await waitFor(() => expect(container.querySelector('[data-line="error"]')).toBeTruthy());
    expect(text(container.querySelector('[data-line="error"]'))).toContain(
      'shop-api · GitHub: not logged in · items kept from 3 h ago',
    );
    expect(text(container.querySelector('[data-status]'))).toBe('1 source failed');
    cleanup();
    setup({ status: { 'shop-api': { ...STATUS_OK, offline: true } } });
    const r2 = render(WorkPane, { paneId: 'w1' });
    await waitFor(() => expect(r2.container.querySelector('[data-line="info"]')).toBeTruthy());
    expect(text(r2.container.querySelector('[data-status]'))).toBe('Offline · updated 4 min ago');
  });

  it('S7 empty tracker and S8 no match for filters', async () => {
    setup({ nodes: [] });
    app.panes = { w1: { type: 'work', projects: ['shop-api'] } };
    const { container } = render(WorkPane, { paneId: 'w1' });
    await waitFor(() => expect(container.querySelector('[data-empty]')).toBeTruthy());
    expect(text(container.querySelector('[data-empty]'))).toBe(
      'No open issues in acme/shop. Create them in GitHub; Work picks them up at the next refresh (r).',
    );
    cleanup();
    setup({ nodes: [], prefs: { filter: { only_ready: true } } });
    const r2 = render(WorkPane, { paneId: 'w1' });
    await waitFor(() => expect(r2.container.querySelector('[data-empty]')).toBeTruthy());
    expect(text(r2.container.querySelector('[data-empty]'))).toBe(
      'Nothing matches 1 filter in shop-api · Reset filters',
    );
  });

  it('S10 huge tree: Load 100 more continues from the cursor', async () => {
    const { container } = await mount(['shop-api'], { more: { roots: { left: 412, cursor: 'c1' } } });
    expect(text(container.querySelector('[data-status]'))).toBe('415 at the top · updated 4 min ago');
    const more = row(container, 'more:roots');
    expect(text(more)).toContain('Showing 3 of 415 · or search with /');
    replies.work_tree = () => ({ nodes: [wn('gh:acme/shop#300')], more: {}, status: {} });
    await fireEvent.click(more.querySelector('button'));
    await waitFor(() => expect(row(container, 'gh:acme/shop#300')).toBeTruthy());
    expect(called('work_tree').at(-1)).toMatchObject({ parents: null, after: 'c1' });
  });

  it('S16 loading children, failure line with Retry', async () => {
    const { container, pane } = await mount();
    replies.work_tree = () => Promise.reject(new Error('GitHub timed out'));
    await press(pane, 'l');
    await waitFor(() => expect(row(container, `error:${V2}`)).toBeTruthy());
    expect(text(row(container, `error:${V2}`))).toContain('Could not load: GitHub timed out');
  });

  it('work.changed in scope reloads; out of scope is ignored', async () => {
    await mount();
    const n = called('work_tree').length;
    listeners['work.changed']({ payload: { project: 'blog' } });
    await tick();
    expect(called('work_tree')).toHaveLength(n);
    listeners['work.changed']({ payload: { project: 'shop-api' } });
    await waitFor(() => expect(called('work_tree').length).toBeGreaterThan(n));
  });
});

describe('entry points (§1.2): one tab per scope', () => {
  it('same scope focuses its tab and lands the reveal; another scope opens a new tab', () => {
    app.panes = {};
    app.layout = null;
    openWork({ projects: ['shop-api'] });
    openWork({ projects: ['shop-api'], reveal: 'gh:acme/shop#13' });
    const works = Object.values(app.panes).filter((s) => s.type === 'work');
    expect(works).toHaveLength(1);
    expect(works[0]).toMatchObject({
      projects: ['shop-api'],
      title: 'Work · shop-api',
      land: { reveal: 'gh:acme/shop#13' },
    });
    openWork({ projects: null });
    openWork({ projects: ['blog', 'shop-api'] });
    expect(Object.values(app.panes).map((s) => s.title)).toEqual([
      'Work · shop-api',
      'Work · all projects',
      'Work · 2 projects',
    ]);
    expect(A.paneProject(works[0])).toBe('shop-api');
  });
});
