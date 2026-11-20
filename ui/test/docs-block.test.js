// The `docs` block of the spec gate card: a repo path opens in nvim, an absolute path
// (a doc kept outside the repository) opens in the Feature pane.

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const calls = [];
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    return Promise.resolve(null);
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
}));

const { app } = await import('../src/lib/app.svelte.js');
const { default: DocsBlock } = await import('../src/components/blocks/DocsBlock.svelte');

const decision = { subject: { project: 'p1', feature: 'checkout', agent: 'lead-1' } };

beforeEach(() => {
  calls.length = 0;
  app.data = {
    projects: [{ id: 'p1', name: 'Shop' }],
    features: [{ id: 'f1', project_id: 'p1', slug: 'checkout', integration_worktree: '/wt/int' }],
  };
  app.panes = {};
  app.focused = null;
  app.layout = null;
});
afterEach(cleanup);

const show = (path) => render(DocsBlock, { block: { type: 'docs', files: [{ path, lines: 3 }] }, decision });
const editorCalls = () => calls.filter(([c]) => c === 'editor_open');

describe('DocsBlock', () => {
  it('opens a repo-relative doc in the editor under the integration worktree', async () => {
    const { getByText } = show('.orchestrator/features/checkout/spec.md');
    await fireEvent.click(getByText('spec.md'));
    expect(editorCalls()).toHaveLength(1);
    expect(editorCalls()[0][1].file).toBe('/wt/int/.orchestrator/features/checkout/spec.md');
    expect(Object.values(app.panes).some((p) => p.type === 'feature')).toBe(false);
  });

  it('opens an absolute doc in the Feature pane, not the editor', async () => {
    const { getByText } = show('/Users/me/Library/Application Support/Orchestrator/features/checkout/spec.md');
    await fireEvent.click(getByText('spec.md'));
    expect(editorCalls()).toHaveLength(0);
    const pane = Object.values(app.panes).find((p) => p.type === 'feature');
    expect(pane?.featureId).toBe('f1');
    expect(pane?.doc).toBe('spec.md');
  });
});
