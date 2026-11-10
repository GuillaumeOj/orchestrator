// R10 (design-notes §13.1): under the doc tabs, a muted line with the shown
// doc's repo-relative path and an Open in nvim icon button at its end, only
// when the doc is in the repo.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { docRepoPath } from '../src/lib/teams.js';
import { fixture } from './fixture.js';

const calls = [];
const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    const r = replies[cmd];
    return Promise.resolve(typeof r === 'function' ? r(args) : (r ?? null));
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => String(e?.message ?? e),
}));

const { default: FeaturePane } = await import('../src/panes/FeaturePane.svelte');

const feature = (wt = '/w/co') => ({ id: 'f1', project_id: 'shop-api', slug: 'checkout', integration_worktree: wt });
const view = (wt) => ({
  feature: feature(wt),
  run: null,
  team: null,
  project: { id: 'shop-api', name: 'shop-api', backend: 'local' },
  agents: [],
  tasks: [],
  docs: ['spec.md', 'api-contract.md'],
  artifacts: [],
  timeline: [],
});

beforeEach(() => {
  cleanup();
  calls.length = 0;
  app.data = fixture();
  app.panes = { f1: { type: 'feature', featureId: 'f1' } };
  replies.feature_doc = '# Doc';
});

describe('docRepoPath', () => {
  it('the feature folder path, for a plain doc of a feature with an integration worktree', () => {
    expect(docRepoPath(feature(), 'spec.md')).toBe('.orchestrator/features/checkout/spec.md');
    expect(docRepoPath(feature(), 'design/notes.md')).toBe('.orchestrator/features/checkout/design/notes.md');
  });
  it('null when the doc is not in the repo', () => {
    expect(docRepoPath(feature(null), 'spec.md')).toBeNull();
    expect(docRepoPath(feature(), null)).toBeNull();
    expect(docRepoPath(feature(), '/etc/passwd')).toBeNull();
    expect(docRepoPath(feature(), '../other/spec.md')).toBeNull();
    expect(docRepoPath(feature(), 'https://notion.so/x')).toBeNull();
  });
});

describe('the path line (R10)', () => {
  it('shows the path of the shown doc with the Open in nvim icon; follows the tab', async () => {
    replies.feature_view = view('/w/co');
    const { container, getByText } = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => expect(container.querySelector('[data-docpath]')).toBeTruthy());
    const line = () => container.querySelector('[data-docpath]');
    expect(line().textContent).toContain('.orchestrator/features/checkout/spec.md');
    const open = line().querySelector('button[aria-label="Open in nvim"]');
    expect(open.classList.contains('icon-btn')).toBe(true);
    // The old text button is gone.
    expect([...container.querySelectorAll('button')].some((b) => b.textContent.includes('Open in nvim'))).toBe(false);
    await fireEvent.click(getByText('api-contract.md'));
    await waitFor(() => expect(line().textContent).toContain('.orchestrator/features/checkout/api-contract.md'));
  });

  it('no line when the doc is not in the repo (no integration worktree)', async () => {
    replies.feature_view = view(null);
    const { container } = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => expect(container.querySelector('.rendered')).toBeTruthy());
    expect(container.querySelector('[data-docpath]')).toBeNull();
  });
});
