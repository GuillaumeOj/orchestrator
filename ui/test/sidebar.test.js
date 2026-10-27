import { render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fixture } from './fixture.js';

let next = null; // the next app_state payload
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
  listen: () => Promise.resolve(() => {}),
  invoke: (cmd) => (cmd === 'app_state' ? Promise.resolve(next) : Promise.resolve(null)),
}));

const { app, closePane, refresh, run } = await import('../src/lib/app.svelte.js');
const { default: Sidebar } = await import('../src/components/Sidebar.svelte');

const feature = { id: 'shop-api-fx', project_id: 'shop-api', slug: 'fx', branch: 'orch/fx/integration', run: null };
const lead = (featureId) => ({
  ...fixture().agents[0],
  id: 'fx-lead-1',
  state: 'working',
  team: { run_id: 'run-1', node: 'lead', feature_id: featureId, project_id: 'shop-api' },
});

beforeEach(() => {
  app.data = { ...fixture(), features: [feature] };
  app.panes = { p1: { type: 'feature', featureId: feature.id, projectId: 'shop-api', title: 'fx' } };
  app.layout = { kind: 'leaf', id: 'l1', tabs: ['p1'], active: 'p1' };
  app.focused = 'p1';
  app.toasts = [];
});

describe('sidebar', () => {
  it('shows a team agent whose feature is not listed (deleted under a running team)', () => {
    app.data = { ...fixture(), features: [], agents: [...fixture().agents, lead('shop-api-gone')] };
    const { container } = render(Sidebar);
    const row = [...container.querySelectorAll('.row.feature')].find((r) => r.textContent.includes('gone'));
    expect(row?.textContent).toContain('deleted');
    expect(container.textContent).toContain('lead');
  });

  it('lists a team agent under its feature', () => {
    app.data = { ...fixture(), features: [feature], agents: [...fixture().agents, lead(feature.id)] };
    const { container } = render(Sidebar);
    expect(container.querySelector('.row.feature')?.textContent).toContain('fx');
    expect(container.querySelector('.row.agent.team')).not.toBeNull();
  });
});

describe('feature panes', () => {
  it('keeps the features and their panes when the core could not read them', async () => {
    next = { ...fixture(), features: null };
    await refresh();
    expect(app.data.features).toEqual([feature]);
    expect(app.panes.p1?.type).toBe('feature');
  });

  it('closes the pane of a feature really gone, without offering to reopen it', async () => {
    next = { ...fixture(), features: [] };
    await refresh();
    expect(app.panes.p1).toBeUndefined();
    run('pane.reopen');
    expect(Object.values(app.panes)).toHaveLength(0);
  });

  it('reopens a closed feature pane (⌘⇧T)', () => {
    closePane('p1');
    expect(Object.values(app.panes)).toHaveLength(0);
    run('pane.reopen');
    const panes = Object.values(app.panes);
    expect(panes).toHaveLength(1);
    expect(panes[0]).toMatchObject({ type: 'feature', featureId: feature.id });
  });
});
