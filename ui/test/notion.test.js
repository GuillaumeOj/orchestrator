// T-13 (C): Notion publish on the Feature pane and the Notion comment in
// the Inbox (FR-505), against a mocked core.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { act, decision, fixture, nowOf } from './fixture.js';

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

const { default: FeaturePane } = await import('../src/panes/FeaturePane.svelte');
const { default: InboxPane } = await import('../src/panes/InboxPane.svelte');

const view = {
  feature: { id: 'shop-onb', project_id: 'shop-api', slug: 'onboarding', integration_worktree: '/w/onb' },
  run: null,
  team: null,
  project: { id: 'shop-api', name: 'shop-api', backend: 'local' },
  agents: [],
  tasks: [],
  docs: ['spec.md', 'ux-brief.md', 'api-contract.md', 'decisions.md'],
  artifacts: [],
  timeline: [],
};
const notionInt = (has) => ({
  items: [{ id: 'notion', name: 'Notion', status: has ? 'connected' : 'not_configured', has_secret: has }],
});
const settings = (page) => ({
  settings: page ? { notion_parent_page: page } : {},
  backend: { type: 'local' },
  track_orchestrator_dir: true,
});

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.toasts = [];
  app.panes = { f1: { type: 'feature', featureId: 'shop-onb', title: 'onboarding' } };
  replies.feature_view = view;
  replies.feature_doc = '# Spec';
});

describe('Notion publish', () => {
  it('not configured: the button is disabled and says what is missing', async () => {
    replies.notion_status = { page_url: null, published_at_ms: null, comments_seen: 0, error: null };
    replies.project_settings = settings(null);
    replies.integrations_view = notionInt(true);
    const { container, getByText } = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => getByText('Publish to Notion', { exact: false }));
    const b = [...container.querySelectorAll('button')].find((x) => x.textContent.includes('Publish to Notion'));
    expect(b.disabled).toBe(true);
    expect(container.textContent).toContain(
      "Set a Notion parent page in shop-api's settings, and connect Notion (⌘+,).",
    );
    await fireEvent.keyDown(container.querySelector('.feature'), { key: 'N' });
    expect(calls.some(([c]) => c === 'notion_publish')).toBe(false);
  });

  it('N publishes: Publishing…, then the published line, the toast with Open, and Refresh', async () => {
    replies.notion_status = { page_url: null, published_at_ms: null, comments_seen: 0, error: null };
    replies.project_settings = settings('abc123');
    replies.integrations_view = notionInt(true);
    let done = () => {};
    replies.notion_publish = () =>
      new Promise((r) => {
        done = () => r({ page_url: 'https://notion.so/onb', published_at_ms: Date.now(), docs: view.docs });
      });
    replies.open_url = null;
    const { container, getByText } = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => getByText('Publish to Notion', { exact: false }));
    await fireEvent.keyDown(container.querySelector('.feature'), { key: 'N' });
    await waitFor(() => getByText('◷ Publishing 4 docs…'));
    done();
    await waitFor(() => getByText('Refresh in Notion', { exact: false }));
    expect(container.textContent).toContain('Open in Notion ↗');
    const t = app.toasts.at(-1);
    expect(t.text).toBe('Published 4 docs to Notion');
    t.action.run();
    expect(calls.some(([c, a]) => c === 'open_url' && a.url === 'https://notion.so/onb')).toBe(true);
  });

  it('a failed publish says why', async () => {
    replies.notion_status = { page_url: null, published_at_ms: null, comments_seen: 0, error: null };
    replies.project_settings = settings('abc123');
    replies.integrations_view = notionInt(true);
    replies.notion_publish = () =>
      Promise.reject(
        'Notion says the integration has no access to the parent page. Share the page with the integration in Notion.',
      );
    const { container, getByText } = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => getByText('Publish to Notion', { exact: false }));
    await fireEvent.keyDown(container.querySelector('.feature'), { key: 'N' });
    await waitFor(() =>
      getByText(
        '✕ Publish failed: Notion says the integration has no access to the parent page. Share the page with the integration in Notion.',
      ),
    );
  });

  it('a published feature shows its comments count', async () => {
    replies.notion_status = {
      page_url: 'https://notion.so/onb',
      published_at_ms: Date.now(),
      comments_seen: 2,
      error: null,
    };
    replies.project_settings = settings('abc123');
    replies.integrations_view = notionInt(true);
    const { getByText } = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => getByText(/· 2 comments/));
  });
});

describe('Notion comment in the Inbox', () => {
  it('reads Notion comment; o opens the page through the core (no reply in the app)', async () => {
    const d = decision({
      id: 'alert-9',
      kind: 'notion',
      level: 2,
      subject: { project: 'shop-api', feature: 'onboarding' },
      title: 'Comment on onboarding · spec.md',
      summary: 'Camille: "Should the third welcome screen be skippable?"',
      created_ms: Date.now(),
      context: [
        { type: 'markdown', text: 'Should the third welcome screen be skippable?', source: 'https://notion.so/onb#c1' },
      ],
      actions: [act('dismiss', 'Dismiss', 'r'), act('open', 'Open in Notion ↗', 'o')],
    });
    app.data = fixture();
    app.now = nowOf(app.data, { decisions: [d] });
    replies.inbox_act = { open: 'url', url: 'https://notion.so/onb#c1' };
    replies.open_url = null;
    const { container, getAllByText } = render(InboxPane);
    expect(getAllByText('Notion comment', { exact: false }).length).toBeGreaterThan(0);
    expect(container.textContent).toContain('Open in Notion ↗');
    expect(container.textContent).toContain('From notion.so/onb#c1 ↗');
    expect(container.textContent).not.toContain('Reply');
    await fireEvent.keyDown(container.querySelector('[data-pane-focus]'), { key: 'o' });
    await waitFor(() =>
      expect(calls.some(([c, a]) => c === 'open_url' && a.url === 'https://notion.so/onb#c1')).toBe(true),
    );
    expect(calls.find(([c]) => c === 'inbox_act')[1]).toEqual({ item: 'alert-9', action: 'open', message: null });
    // r dismisses, as on the other alerts (through the undo window).
    await fireEvent.keyDown(container.querySelector('[data-pane-focus]'), { key: 'r' });
    expect(app.pending.at(-1)).toMatchObject({ item: 'alert-9', action: 'dismiss' });
    expect(app.toasts.at(-1).text).toBe('Dismissing · Comment on onboarding · spec.md');
    app.pending = [];
  });

  it('a task sync conflict is a digest outcome: o opens the Board on the task', async () => {
    app.data = fixture();
    app.now = nowOf(app.data, {
      decisions: [],
      outcomes: [
        {
          id: 'ev-3',
          kind: 'sync_conflict',
          at_ms: Date.now(),
          project: 'shop-api',
          task: 40,
          text: "ENG-140 changed here and in Linear: kept Linear's",
          seen: false,
        },
      ],
    });
    app.layout = null;
    replies.board_view = {
      project: { id: 'shop-api', name: 'shop-api' },
      backend: 'linear',
      tasks: [],
      features: [],
      runs: [],
    };
    const { container } = render(InboxPane);
    await fireEvent.click(container.querySelector('[role="option"][title^="ENG-140"]'));
    await fireEvent.keyDown(container.querySelector('[data-pane-focus]'), { key: 'o' });
    await waitFor(() =>
      expect(
        Object.values(app.panes).some((p) => p.type === 'board' && p.projectId === 'shop-api' && p.taskId === 40),
      ).toBe(true),
    );
  });
});
