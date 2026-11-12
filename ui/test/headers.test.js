// R4, R9 (design-notes §13.1, audit §10): one pane header pattern. Identity
// left, icon buttons right (each with a tooltip and an `aria-label`), danger
// icons in the danger colour, the `⋯` Menu last holding the rest.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { fixture } from './fixture.js';

const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    const r = replies[cmd];
    return Promise.resolve(typeof r === 'function' ? r(args) : (r ?? null));
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => String(e?.message ?? e),
}));

const { default: ProjectPane } = await import('../src/panes/ProjectPane.svelte');
const { default: FeaturePane } = await import('../src/panes/FeaturePane.svelte');

const view = (run) => ({
  feature: { id: 'f1', project_id: 'shop-api', slug: 'checkout', integration_worktree: '/w/co' },
  run,
  team: { name: 'Squad', version: 1, nodes: [{ id: 'lead', is_lead: true, role_id: 'lead' }] },
  project: { id: 'shop-api', name: 'shop-api', backend: 'local' },
  agents: [],
  tasks: [],
  docs: ['spec.md'],
  artifacts: [],
  timeline: [],
});

/** The header's buttons in order: `[aria-label, danger?]`. */
const buttons = (head) => [
  ...head.querySelectorAll('.head-acts > button, .head-acts button[aria-label="More actions"]'),
];
async function menuLabels(head) {
  await fireEvent.click(head.querySelector('button[aria-label="More actions"]'));
  const rows = [...document.body.querySelectorAll('[role="menu"] [role^="menuitem"]')].map(
    (r) => r.querySelector('.lab').textContent,
  );
  await fireEvent.keyDown(document.activeElement, { key: 'Escape' });
  return rows;
}

function expectPattern(head, want) {
  const bs = buttons(head);
  expect(bs.map((b) => b.getAttribute('aria-label'))).toEqual(want.map(([l]) => l));
  for (const [i, [, danger]] of want.entries()) {
    expect(bs[i].classList.contains('icon-btn')).toBe(true);
    expect(bs[i].classList.contains('danger')).toBe(!!danger);
    expect(bs[i].hasAttribute('title')).toBe(false); // a tooltip, not a native title
  }
  // `⋯` is last.
  expect(bs.at(-1).getAttribute('aria-label')).toBe('More actions');
  expect(head.querySelector('.kbd')).toBeNull();
}

beforeEach(() => {
  cleanup();
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.toasts = [];
});

describe('project pane header (R4)', () => {
  it.each([
    [false, ['Fetch', 'Shell', 'Diff main', 'Trust in a pane']],
    [true, ['Fetch', 'Shell', 'Diff main']],
  ])('trusted %s: gear, danger bin, `⋯` = %j', async (trusted, items) => {
    app.data.projects[0].default_branch = 'main';
    replies.project_detail = { trusted, worktrees: [], branches: [] };
    app.panes = { p1: { type: 'project', projectId: 'shop-api' } };
    const { container } = render(ProjectPane, { paneId: 'p1' });
    await waitFor(() =>
      expect(container.textContent).toContain(trusted ? 'Claude Code trusts this folder' : 'Not trusted yet'),
    );
    const head = container.querySelector('.head');
    expect(head.querySelector('h2').textContent).toBe('shop-api');
    expectPattern(head, [['Project settings'], ['Remove project…', true], ['More actions']]);
    expect(await menuLabels(head)).toEqual(items);
  });

  it('the gear opens Project settings', async () => {
    replies.project_detail = { trusted: true, worktrees: [], branches: [] };
    app.panes = { p1: { type: 'project', projectId: 'shop-api' } };
    app.window = null;
    const { container } = render(ProjectPane, { paneId: 'p1' });
    await fireEvent.click(container.querySelector('button[aria-label="Project settings"]'));
    expect(app.window).toMatchObject({ kind: 'project-settings', projectId: 'shop-api' });
    app.window = null;
  });
});

describe('feature pane header (R9)', () => {
  const live = { id: 'run-1', status: 'executing', team_id: 't1', started_at_ms: Date.now() - 3600000 };

  it('running: Pause, Stop, danger Delete, `⋯` = Board, Canvas, Publish to Notion, Open in Notion', async () => {
    replies.feature_view = view(live);
    replies.feature_doc = '# Spec';
    app.panes = { f1: { type: 'feature', featureId: 'f1' } };
    const { container } = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => expect(container.querySelector('.fh')).toBeTruthy());
    const head = container.querySelector('.fh');
    expectPattern(head, [['Pause team'], ['Stop the team…'], ['Delete feature…', true], ['More actions']]);
    expect(await menuLabels(head)).toEqual(['Board', 'Canvas', 'Publish to Notion', 'Open in Notion']);
  });

  it('two rows: row 1 = slug, pill, actions; row 2 = team, branch, started', async () => {
    replies.feature_view = view(live);
    app.panes = { f1: { type: 'feature', featureId: 'f1' } };
    const { container } = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => expect(container.querySelector('.fh .row1')).toBeTruthy());
    const [r1, r2] = container.querySelector('.fh').children;
    expect(r1.querySelector('.slug')).toBeTruthy();
    expect(r1.querySelector('.pill')).toBeTruthy();
    expect(r1.querySelector('.head-acts')).toBeTruthy();
    expect(r2.textContent).toMatch(/^Squad v1 · .*orch\/.+\/integration · started /);
  });

  it('no run: no pill and no started; row 2 keeps team · branch', async () => {
    replies.feature_view = view(null);
    app.panes = { f1: { type: 'feature', featureId: 'f1' } };
    const { container } = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => expect(container.querySelector('.fh .row1')).toBeTruthy());
    const head = container.querySelector('.fh');
    expect(head.querySelector('.pill')).toBeNull();
    const row2 = head.children[1].textContent;
    expect(row2).not.toContain('started');
    expect(row2).toMatch(/^Squad v1 · /);
  });

  it('paused: Resume team; a finished run has no Pause/Stop, Canvas still in `⋯`', async () => {
    replies.feature_view = view({ ...live, status: 'paused' });
    app.panes = { f1: { type: 'feature', featureId: 'f1' } };
    let r = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => expect(r.container.querySelector('.fh')).toBeTruthy());
    expect(buttons(r.container.querySelector('.fh'))[0].getAttribute('aria-label')).toBe('Resume team');
    cleanup();
    replies.feature_view = view({ ...live, status: 'done' });
    r = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => expect(r.container.querySelector('.fh')).toBeTruthy());
    expectPattern(r.container.querySelector('.fh'), [['Delete feature…', true], ['More actions']]);
  });

  it('the Tasks card has no Board button; the message box is To Select + input + Send icon', async () => {
    replies.feature_view = view(live);
    app.panes = { f1: { type: 'feature', featureId: 'f1' } };
    const { container } = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => expect(container.querySelector('.fh')).toBeTruthy());
    const tasks = [...container.querySelectorAll('.card')].find(
      (c) => c.querySelector('.cardh b')?.textContent === 'Tasks',
    );
    expect(tasks.querySelector('.cardh button')).toBeNull();
    const msg = container.querySelector('.msg');
    expect(msg.querySelector('input.field')).toBeTruthy();
    expect(msg.querySelector('button[aria-label="Send"]').classList.contains('icon-btn')).toBe(true);
  });
});
