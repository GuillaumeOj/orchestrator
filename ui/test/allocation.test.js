// Project settings › Allocation: the pure helpers and the section's main
// states against a mocked core.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  addDraft,
  capBar,
  isDirty,
  keysFor,
  ownShare,
  pendingRemoval,
  savedToast,
  share,
  tableRows,
  toForm,
  validate,
} from '../src/lib/allocation.js';
import { app } from '../src/lib/app.svelte.js';
import { fixture } from './fixture.js';

const calls = [];
const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    const r = replies[cmd];
    return r instanceof Error ? Promise.reject(r) : Promise.resolve(typeof r === 'function' ? r(args) : r);
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
}));

const { default: ProjectSettingsPane } = await import('../src/panes/ProjectSettingsPane.svelte');

const alloc = (id, weight, over = {}) => ({
  id,
  target: id,
  weight,
  tier: 'normal',
  hard_cap_pct: null,
  used_pct: 1,
  ...over,
});
const view = (over = {}) => ({
  project: 'shop-api',
  name: 'shop-api',
  plan_confirmed: true,
  allocation: alloc('shop-api', 50, { tier: 'high', hard_cap_pct: 30, used_pct: 7.4 }),
  own_index: 0,
  others: [alloc('blog', 30), alloc('infra', 20, { tier: 'low' }), alloc('default', 1, { target: 'other sessions' })],
  proposed_weight: 33,
  ...over,
});
const out = (over = {}) => view({ allocation: null, own_index: null, ...over });

describe('allocation helpers', () => {
  it('share rounds per row and ignores bad weights', () => {
    expect(share(50, 101)).toBe(50);
    expect(share(1, 3)).toBe(33);
    expect(share(0, 10)).toBe(0);
    expect(share('x', 10)).toBe(0);
    expect(share(5, 0)).toBe(0);
  });

  it('the table puts the own row at own_index and follows the typed weight', () => {
    const v = view({ own_index: 1 });
    const rows = tableRows(v, { weight: '60', tier: 'high', cap: '30' }, 'shop-api');
    expect(rows.map((r) => r.label)).toEqual(['blog', 'shop-api', 'infra', 'other sessions']);
    expect(rows[1]).toMatchObject({ own: true, weight: 60, share: 54, tag: 'this project' });
    expect(rows[0].share).toBe(27);
  });

  it('pending removal drops the own row and hints on other sessions', () => {
    const rows = tableRows(view(), { weight: '0', tier: 'high', cap: '30' }, 'shop-api');
    expect(rows.some((r) => r.own)).toBe(false);
    expect(rows.at(-1)).toMatchObject({ label: 'other sessions', tag: '+ shop-api' });
    expect(ownShare(view(), { weight: '0', tier: 'high', cap: '' }, 'shop-api')).toBeNull();
  });

  it('not in the plan: the table is the plan, other sessions carries the hint; Add shows the share', () => {
    const v = out();
    expect(tableRows(v, null, 'docs').at(-1).tag).toBe('+ docs');
    expect(ownShare(v, addDraft(v), 'docs')).toBe(Math.round((33 / 84) * 100));
  });

  it('proposed default and fallbacks', () => {
    expect(addDraft(out())).toEqual({ weight: '33', tier: 'normal', cap: '' });
    expect(addDraft(out({ proposed_weight: 0 })).weight).toBe('1');
    expect(addDraft({}).weight).toBe('1');
  });

  it('validation: negative or empty weight, cap outside 0–100', () => {
    expect(validate({ weight: '-2', tier: 'high', cap: '120' })).toEqual({
      weight: 'Weight must be 0 or more',
      cap: 'Hard cap must be between 0 and 100',
    });
    expect(validate({ weight: '', tier: 'high', cap: '' }).weight).toBeTruthy();
    expect(validate({ weight: '0', tier: 'high', cap: '0' })).toEqual({});
  });

  it('form → payload: numbers, null cap, weight 0 = remove', () => {
    expect(toForm({ weight: '40', tier: 'low', cap: '' })).toEqual({ weight: 40, tier: 'low', hard_cap_pct: null });
    expect(toForm({ weight: '40', tier: 'low', cap: '25' }).hard_cap_pct).toBe(25);
    expect(toForm({ weight: '0', tier: 'low', cap: '25' })).toBeNull();
    expect(pendingRemoval({ weight: '0' })).toBe(true);
    expect(pendingRemoval({ weight: '0.5' })).toBe(false);
  });

  it('dirty compares the payload with the saved allocation', () => {
    const v = view();
    expect(isDirty(v, { weight: '50', tier: 'high', cap: '30' })).toBe(false);
    expect(isDirty(v, { weight: '51', tier: 'high', cap: '30' })).toBe(true);
    expect(isDirty(v, { weight: '0', tier: 'high', cap: '30' })).toBe(true); // removal
    expect(isDirty(out(), null)).toBe(false);
    expect(isDirty(out(), { weight: '33', tier: 'normal', cap: '' })).toBe(true);
    expect(isDirty(out(), { weight: '0', tier: 'normal', cap: '' })).toBe(false);
  });

  it('cap bar: green, amber from 80 % of the cap, red at the cap; the track spans cap / 0.8', () => {
    expect(capBar(7.4, 30, 'a')).toMatchObject({ state: 'green', words: '25% of the cap' });
    expect(capBar(24, 30, 'a').state).toBe('amber');
    expect(capBar(25.1, 30, 'a').words).toBe('▲ close to the cap · 25.1 of 30%');
    expect(capBar(30, 30, 'a').words).toMatch(/^■ cap reached · a's agents are stopped/);
    expect(capBar(30, 30, 'a').fill).toBeCloseTo(80);
    expect(capBar(0, 0, 'a').state).toBe('green');
    expect(capBar(0.1, 0, 'a').state).toBe('red');
    expect(capBar(1, null, 'a')).toBeNull();
  });

  it('toasts and the keys line', () => {
    expect(savedToast('shop-api', {})).toBe("Saved shop-api's allocation · this week's usage kept");
    expect(savedToast('shop-api', null)).toBe("Removed shop-api from the plan · this week's usage kept");
    expect(keysFor(out(), null, false, false)).toEqual([
      ['a', 'add'],
      ['g', 'week planner'],
    ]);
    expect(keysFor(view(), { weight: '50', tier: 'high', cap: '30' }, false, false).map((k) => k[0])).toEqual([
      'x',
      'g',
    ]);
    expect(keysFor(null, null, false, true)).toEqual([['r', 'retry']]);
  });
});

describe('Project settings › Allocation', () => {
  beforeEach(() => {
    cleanup();
    calls.length = 0;
    for (const k of Object.keys(replies)) delete replies[k];
    app.data = fixture();
    app.toasts = [];
    app.panes = {};
    app.overlay = null;
    replies.project_settings = {
      settings: {},
      backend: { type: 'local' },
      track_orchestrator_dir: true,
      default_branch: 'main',
    };
    replies.board_view = null;
    replies.integrations_view = { items: [] };
  });

  async function open(v) {
    replies.project_allocation = v;
    app.panes.p1 = {
      type: 'project-settings',
      projectId: 'shop-api',
      section: 'allocation',
      title: 'shop-api settings',
    };
    const r = render(ProjectSettingsPane, { paneId: 'p1' });
    await waitFor(() => {
      expect(r.container.querySelector('.alloc h2')).toBeTruthy();
      expect(r.container.querySelector('[aria-busy]')).toBeNull();
    });
    return { ...r, root: r.container.querySelector('[data-pane-focus]') };
  }
  const weight = (c) => /** @type {HTMLInputElement} */ (c.querySelector('[data-field="weight"]'));
  const btn = (c, re) => [...c.querySelectorAll('button')].find((b) => re.test(b.textContent));
  const type = async (el, v) => {
    el.value = v;
    await fireEvent.input(el);
    await tick();
  };

  it('in the plan: share follows the typed weight, Save sends the form and toasts', async () => {
    const { container, root } = await open(view());
    expect(container.textContent).toContain('50% of the plan');
    expect(container.textContent).toContain('used 7.4 pp this week');
    expect(btn(container, /^Save/).disabled).toBe(true);
    await type(weight(container), '60');
    expect(container.textContent).toContain('• changed');
    expect(container.textContent).toContain('54% of the plan');
    expect(btn(container, /^Save/).disabled).toBe(false);
    replies.project_allocation_set = (a) =>
      view({ allocation: alloc('shop-api', a.form.weight, { tier: 'high', hard_cap_pct: 30 }) });
    await fireEvent.keyDown(root, { key: 'Enter', metaKey: true });
    await waitFor(() => expect(app.toasts.at(-1)?.text).toBe("Saved shop-api's allocation · this week's usage kept"));
    expect(calls.find(([c]) => c === 'project_allocation_set')[1]).toEqual({
      project: 'shop-api',
      form: { weight: 60, tier: 'high', hard_cap_pct: 30 },
    });
    expect(btn(container, /^Save/).disabled).toBe(true);
  });

  it('single keys act only when no field has focus', async () => {
    const { container, root } = await open(view());
    await fireEvent.keyDown(weight(container), { key: 'x' });
    expect(container.textContent).not.toContain('From the save on');
    await fireEvent.keyDown(root, { key: 'x' });
    await tick();
    expect(container.textContent).toContain("From the save on, shop-api's agents are charged to other sessions.");
    expect(weight(container).disabled).toBe(false);
    expect(container.querySelector('#alloc-cap').disabled).toBe(true);
    // esc = Keep in the plan: the weight comes back.
    await fireEvent.keyDown(root, { key: 'Escape' });
    await tick();
    expect(weight(container).value).toBe('50');
    expect(container.textContent).not.toContain('From the save on');
  });

  it('remove then Save sends null and the removal toast', async () => {
    const { container, root } = await open(view());
    await fireEvent.keyDown(root, { key: 'x' });
    replies.project_allocation_set = () => out();
    await fireEvent.keyDown(root, { key: 'Enter', metaKey: true });
    await waitFor(() =>
      expect(app.toasts.at(-1)?.text).toBe("Removed shop-api from the plan · this week's usage kept"),
    );
    expect(calls.find(([c]) => c === 'project_allocation_set')[1].form).toBeNull();
    expect(container.textContent).toContain("shop-api isn't in this week's plan");
  });

  it('not in the plan: a adds with the proposal, x right after cancels the Add', async () => {
    const { container, root } = await open(out());
    expect(container.textContent).toContain(
      "shop-api isn't in this week's plan. Its agents are charged to other sessions.",
    );
    expect(btn(container, /Add to the weekly plan/)).toBeTruthy();
    await fireEvent.keyDown(root, { key: 'a' });
    await tick();
    expect(weight(container).value).toBe('33');
    expect(container.querySelector('.tr.own')?.textContent).toContain('shop-api');
    await fireEvent.keyDown(root, { key: 'x' });
    await tick();
    expect(weight(container)).toBeNull();
    expect(calls.some(([c]) => c === 'project_allocation_set')).toBe(false);
  });

  it('validation: inline error, Save disabled, nothing saved', async () => {
    const { container, root } = await open(view());
    await type(weight(container), '-2');
    expect(container.textContent).toContain('Weight must be 0 or more');
    expect(btn(container, /^Save/).disabled).toBe(true);
    await fireEvent.keyDown(root, { key: 'Enter', metaKey: true });
    expect(calls.some(([c]) => c === 'project_allocation_set')).toBe(false);
    await type(weight(container), '50');
    await type(container.querySelector('#alloc-cap'), '120');
    expect(container.textContent).toContain('Hard cap must be between 0 and 100');
  });

  it('no plan confirmed: read only split, no fields, a button to the planner', async () => {
    const v = view({
      plan_confirmed: false,
      allocation: alloc('shop-api', 1),
      others: [alloc('blog', 1), alloc('default', 1, { target: 'other sessions' })],
    });
    const { container, root } = await open(v);
    expect(container.textContent).toContain('No weekly plan yet');
    expect(weight(container)).toBeNull();
    expect(btn(container, /^Save/)).toBeUndefined();
    await fireEvent.keyDown(root, { key: 'g' });
    expect(app.overlay).toEqual({ kind: 'plan' });
  });

  it('save error keeps the edits and shows the core message', async () => {
    const { container, root } = await open(view());
    await type(weight(container), '60');
    replies.project_allocation_set = 'set up the weekly plan first';
    replies.project_allocation_set = new Error('set up the weekly plan first');
    await fireEvent.keyDown(root, { key: 'Enter', metaKey: true });
    await waitFor(() =>
      expect(container.textContent).toContain("Couldn't save: set up the weekly plan first. Nothing was changed."),
    );
    expect(weight(container).value).toBe('60');
  });

  it('load error: box with Retry, r reloads', async () => {
    replies.project_allocation = new Error("the core isn't reachable");
    app.panes.p1 = {
      type: 'project-settings',
      projectId: 'shop-api',
      section: 'allocation',
      title: 'shop-api settings',
    };
    const { container, getByText } = render(ProjectSettingsPane, { paneId: 'p1' });
    await waitFor(() => getByText(/Couldn't load shop-api's allocation: the core isn't reachable/));
    replies.project_allocation = view();
    await fireEvent.keyDown(container.querySelector('[data-pane-focus]'), { key: 'r' });
    await waitFor(() => getByText('Rest of the plan'));
  });
});
