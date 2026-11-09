import { fireEvent, render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fixture } from './fixture.js';

let next = null; // the next app_state payload
const saves = []; // `layout` values written by saveLayout
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
  listen: () => Promise.resolve(() => {}),
  invoke: (cmd, args) => {
    if (cmd === 'settings_set' && args.key === 'layout') saves.push(args.value);
    return cmd === 'app_state' ? Promise.resolve(next) : Promise.resolve(null);
  },
}));

const { app, closePane, refresh, reopenClosed, run } = await import('../src/lib/app.svelte.js');
const { default: Sidebar } = await import('../src/components/Sidebar.svelte');
const { default: IconRail } = await import('../src/components/IconRail.svelte');

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
  app.workspace = { project: 'all' };
  app.sidebarTree = {};
  saves.length = 0;
});

describe('sidebar', () => {
  it('flags a deleted feature the core still lists for its team, and opens it', async () => {
    const gone = { ...feature, id: 'shop-api-gone', slug: 'gone', archived: true };
    app.data = { ...fixture(), features: [gone], agents: [...fixture().agents, lead(gone.id)] };
    app.panes = {};
    app.layout = null;
    const { container } = render(Sidebar);
    const row = [...container.querySelectorAll('.row.feature')].find((r) => r.textContent.includes('gone'));
    expect(row?.textContent).toContain('deleted');
    expect(container.querySelector('.row.agent.team')).not.toBeNull();
    await fireEvent.click(row);
    expect(Object.values(app.panes)).toMatchObject([{ type: 'feature', featureId: gone.id }]);
  });

  it('shows a team agent whose feature is not listed with the solo sessions', () => {
    for (const features of [[], null]) {
      app.data = { ...fixture(), features, agents: [...fixture().agents, lead('shop-api-gone')] };
      const { container, unmount } = render(Sidebar);
      expect(container.querySelector('.row.feature')).toBeNull();
      expect([...container.querySelectorAll('.row.agent')].some((r) => r.textContent.includes('fx-lead-1'))).toBe(true);
      unmount();
    }
  });

  it('lists a team agent under its feature', () => {
    app.data = { ...fixture(), features: [feature], agents: [...fixture().agents, lead(feature.id)] };
    const { container } = render(Sidebar);
    expect(container.querySelector('.row.feature')?.textContent).toContain('fx');
    expect(container.querySelector('.row.agent.team')).not.toBeNull();
  });
});

describe('sidebar tree and project context (design-notes §5.1 rule 4, §5.4)', () => {
  const projectRow = (c, name) => [...c.querySelectorAll('.row.project')].find((r) => r.textContent.includes(name));

  it('carets toggle app.sidebarTree (expanded booleans, absent = expanded) and save it; the row does not open', async () => {
    vi.useFakeTimers();
    try {
      const { container } = render(Sidebar);
      expect(container.querySelector('.row.feature')).not.toBeNull();
      await fireEvent.click(projectRow(container, 'shop-api').querySelector('.caret'));
      expect(app.sidebarTree).toEqual({ 'shop-api': false });
      expect(container.querySelector('.row.feature')).toBeNull();
      expect(app.workspace.project).toBe('all');
      await fireEvent.click(projectRow(container, 'shop-api').querySelector('.caret'));
      await fireEvent.click(container.querySelector('.row.feature .caret'));
      expect(app.sidebarTree).toEqual({ 'shop-api': true, 'f:shop-api-fx': false });
      expect(container.querySelector('.row.feature').closest('li').getAttribute('aria-expanded')).toBe('false');
      vi.advanceTimersByTime(500);
      expect(saves.at(-1).sidebarTree).toEqual({ 'shop-api': true, 'f:shop-api-fx': false });
    } finally {
      vi.useRealTimers();
    }
  });

  it('a project click sets the context and highlights only that row; none in All projects', async () => {
    const { container } = render(Sidebar);
    expect(container.querySelectorAll('.row.project.selected')).toHaveLength(0);
    await fireEvent.click(projectRow(container, 'blog'));
    expect(app.workspace.project).toBe('blog');
    expect([...container.querySelectorAll('.row.project.selected')].map((r) => r.textContent)).toEqual([
      expect.stringContaining('blog'),
    ]);
    expect(Object.values(app.panes).some((s) => s.type === 'project' && s.projectId === 'blog')).toBe(true);
  });

  it('a feature click sets its project context and opens it', async () => {
    app.workspace = { project: 'blog' };
    app.panes = {};
    app.layout = null;
    const { container } = render(Sidebar);
    await fireEvent.click(container.querySelector('.row.feature'));
    expect(app.workspace.project).toBe('shop-api');
    expect(Object.values(app.panes)).toMatchObject([{ type: 'feature', featureId: feature.id }]);
  });

  it('the icon rail marks the context project only, and a click sets it', async () => {
    const { container } = render(IconRail);
    expect(container.querySelectorAll('.slot.on')).toHaveLength(0);
    await fireEvent.click(container.querySelector('[aria-label="blog"]'));
    expect(app.workspace.project).toBe('blog');
    expect(container.querySelector('.slot.on [aria-label="blog"]')).not.toBeNull();
    expect(container.querySelectorAll('.slot.on')).toHaveLength(1);
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
    await reopenClosed();
    expect(Object.values(app.panes)).toHaveLength(0);
  });

  it('does not offer back a pane that closed itself', async () => {
    closePane('p1');
    await reopenClosed();
    expect(Object.values(app.panes)).toHaveLength(0);
  });

  it('reopens a closed feature pane (⌘⇧T)', async () => {
    run('pane.close');
    expect(Object.values(app.panes)).toHaveLength(0);
    await reopenClosed();
    const panes = Object.values(app.panes);
    expect(panes).toHaveLength(1);
    expect(panes[0]).toMatchObject({ type: 'feature', featureId: feature.id });
  });
});
