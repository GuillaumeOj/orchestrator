// The workspace's project context (design-notes §5.1, §5.3, §12.2, §12.7,
// §12.8; acceptance F2, F3.4): which tabs a context shows, the active tab on a
// change, opening a pane of another project, ⌘1…9, the context Select (counts,
// waiting dot) and the empty states.
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { countsOf, fixture, nowOf, rec } from './fixture.js';

const saves = []; // `layout` values written by saveLayout
const calls = [];
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  errText: (e) => String(e),
  listen: () => Promise.resolve(() => {}),
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    if (cmd === 'settings_set' && args.key === 'layout') saves.push(args.value);
    return Promise.resolve(null);
  },
}));
const stub = async () => ({ default: (await import('./fixtures/PaneStub.svelte')).default });
for (const p of [
  'Agent',
  'Board',
  'Diff',
  'Feature',
  'Mission',
  'Project',
  'Report',
  'Setup',
  'TeamCanvas',
  'TermPaneView',
])
  vi.doMock(`../src/panes/${p === 'TermPaneView' ? p : `${p}Pane`}.svelte`, stub);

const A = await import('../src/lib/app.svelte.js');
const { app } = A;
const { default: Workspace } = await import('../src/components/Workspace.svelte');

const feature = { id: 'blog-fx', project_id: 'blog', slug: 'fx', branch: 'orch/fx/integration', run: null };

/** Tabs: Mission control (app-wide), shop-api project, blog feature, shop-api board, an editor without a project. */
function seed() {
  app.data = { ...fixture(), features: [feature] };
  app.now = nowOf(app.data, { decisions: [] });
  app.panes = {
    m: { type: 'mission', title: 'Mission control' },
    sp: { type: 'project', projectId: 'shop-api', title: 'shop-api' },
    bf: { type: 'feature', featureId: 'blog-fx', projectId: 'blog', title: 'fx' },
    sb: { type: 'board', projectId: 'shop-api', title: 'shop-api tasks' },
    ed: { type: 'editor', projectId: null, title: 'nvim · ' },
  };
  app.layout = { kind: 'leaf', id: 'l1', tabs: ['m', 'sp', 'bf', 'sb', 'ed'], active: 'sp' };
  app.focused = 'sp';
  app.dock = { open: false, height: 0.45, tabs: [], active: null, v: 2 };
  app.workspace = { project: 'all' };
  app.contextOpen = false;
  app.overlay = null;
  app.window = null;
}

beforeEach(() => {
  seed();
  saves.length = 0;
  calls.length = 0;
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe('paneProject (design-notes §5.3)', () => {
  it('project-bound panes give their project; app-wide ones give null', () => {
    const p = A.paneProject;
    expect(p({ type: 'project', projectId: 'a' })).toBe('a');
    expect(p({ type: 'board', projectId: 'a' })).toBe('a');
    expect(p({ type: 'diff', projectId: 'a' })).toBe('a');
    expect(p({ type: 'trust', projectId: 'a' })).toBe('a');
    // A feature: the listed feature's project wins over the spec's.
    expect(p({ type: 'feature', featureId: 'blog-fx', projectId: 'stale' })).toBe('blog');
    expect(p({ type: 'feature', featureId: 'unknown', projectId: 'shop-api' })).toBe('shop-api');
    expect(p({ type: 'canvas', teamId: 't', projectId: 'a' })).toBe('a');
    expect(p({ type: 'canvas', teamId: 't', projectId: null })).toBe(null);
    expect(p({ type: 'editor', projectId: 'a' })).toBe('a');
    expect(p({ type: 'editor' })).toBe(null);
    for (const type of ['mission', 'setup', 'report']) expect(p({ type, projectId: 'a' })).toBe(null);
    expect(p(undefined)).toBe(null);
  });
});

describe('visible tabs and the active tab (rules 1, 2, §12.8)', () => {
  it('All shows every tab; a project shows its own and the app-wide ones, hidden tabs stay open', () => {
    expect(A.visibleTabs()).toEqual(['m', 'sp', 'bf', 'sb', 'ed']);
    A.setContext('blog');
    expect(A.visibleTabs()).toEqual(['m', 'bf', 'ed']);
    expect(Object.keys(app.panes)).toHaveLength(5);
    expect(app.layout.tabs).toHaveLength(5);
    A.setContext('all');
    expect(A.visibleTabs()).toHaveLength(5);
  });

  it('a hidden active tab gives way to the first visible one; a visible one stays', () => {
    A.setContext('blog');
    expect(app.layout.active).toBe('m');
    expect(app.focused).toBe('m');
    A.focusPane('bf');
    A.setContext('all');
    expect(app.layout.active).toBe('bf');
    // An app-wide tab belongs to every context: Mission control stays active.
    A.focusPane('m');
    A.setContext('shop-api');
    expect(app.layout.active).toBe('m');
  });

  it('a context with no tab: the empty state (no active tab), the dock focus is kept', () => {
    app.layout = { kind: 'leaf', id: 'l1', tabs: ['sp'], active: 'sp' };
    app.dock = { open: true, height: 0.45, tabs: ['d1'], active: 'd1', v: 2 };
    app.panes.d1 = { type: 'shell', title: 'zsh' };
    A.setContext('blog');
    expect(A.activeTab()).toBe(null);
    expect(app.focused).toBe('d1');
    app.focused = 'sp';
    A.setContext('shop-api');
    A.setContext('blog');
    expect(app.focused).toBe('d1');
  });

  it('closing the active tab activates the next visible tab, not a hidden one', () => {
    A.setContext('shop-api');
    A.focusPane('sp');
    A.closePane('sp');
    // bf (blog) is next in the strip but hidden: sb is the next visible tab.
    expect(app.layout.active).toBe('sb');
    expect(app.focused).toBe('sb');
    A.closePane('sb');
    expect(app.layout.active).toBe('ed');
    A.closePane('ed');
    expect(app.layout.active).toBe('m');
  });

  it('setContext is saved; an unknown project falls back to All', () => {
    vi.useFakeTimers();
    A.setContext('blog');
    vi.advanceTimersByTime(500);
    expect(saves.at(-1).workspace).toEqual({ project: 'blog' });
    A.setContext('gone');
    expect(app.workspace.project).toBe('all');
  });

  it('a project removed while it is the context: back to All', async () => {
    A.setContext('blog');
    const data = { ...fixture(), features: [], projects: fixture().projects.filter((p) => p.id !== 'blog') };
    calls.length = 0;
    const ipc = await import('../src/lib/ipc.js');
    const spy = vi.spyOn(ipc, 'invoke').mockImplementation((cmd) => Promise.resolve(cmd === 'app_state' ? data : null));
    await A.refresh();
    spy.mockRestore();
    expect(app.workspace.project).toBe('all');
  });
});

describe('opening a pane of another project (rule 3)', () => {
  it('switches the context to it, but All stays All; app-wide panes never change it', () => {
    A.openBoard('blog');
    expect(app.workspace.project).toBe('all');
    A.setContext('shop-api');
    A.openFeature('blog-fx');
    expect(app.workspace.project).toBe('blog');
    expect(app.focused).toBe('bf');
    A.openProject('shop-api');
    expect(app.workspace.project).toBe('shop-api');
    A.openMission();
    A.openReport();
    expect(app.workspace.project).toBe('shop-api');
  });

  it('a canvas opened in All projects is app-wide; in a project it is bound to it', () => {
    A.openCanvas('t1');
    const id = A.findPane((s) => s.type === 'canvas');
    expect(app.panes[id].projectId).toBe(null);
    A.setContext('blog');
    expect(A.visibleTabs()).toContain(id);
    A.openCanvas('t2');
    expect(app.panes[A.findPane((s) => s.teamId === 't2')].projectId).toBe('blog');
  });
});

describe('⌘1…9 (design-notes §4.1, F3.4)', () => {
  it('picks the n-th visible tab; beyond the count does nothing, no toast', () => {
    A.setContext('blog');
    app.toasts = [];
    A.run('pane.focus.2');
    expect(app.focused).toBe('bf');
    A.run('pane.focus.3');
    expect(app.focused).toBe('ed');
    A.run('pane.focus.4');
    expect(app.focused).toBe('ed');
    expect(app.toasts).toEqual([]);
  });
});

describe('the context Select (rule 7, §12.7, 03-context-select-open)', () => {
  it('options: All projects (every tab) + each project (its own tabs), a dot per project that needs you', () => {
    app.now.counts.projects = { blog: countsOf({ needs_you: 1 }) };
    expect(A.contextOptions()).toEqual([
      { value: 'all', label: 'All projects', count: 5, sep: true },
      { value: 'shop-api', label: 'shop-api', count: 2, dot: false },
      { value: 'blog', label: 'blog', count: 1, dot: true },
    ]);
  });

  it('the closed dot: an agent of another project waits; never in All; not for the context own agents', () => {
    app.now.agents = {};
    app.now.agents['rss-feed'] = rec('rss-feed', 'waiting_for_you', { project: 'blog' });
    expect(A.contextWaiting()).toBe(null);
    A.setContext('blog');
    expect(A.contextWaiting()).toBe(null);
    A.setContext('shop-api');
    expect(A.contextWaiting()).toBe('rss-feed (blog) is waiting for you');
    // The dock's half of the rule (T6) plugs in through dockShowsWaiting; today the dock shows no dot.
    expect(A.dockShowsWaiting('rss-feed')).toBe(false);
  });

  it('renders compact with the context, the count-free closed value and the waiting dot tooltip', async () => {
    app.now.agents = {};
    app.now.agents['rss-feed'] = rec('rss-feed', 'waiting_for_you', { project: 'blog' });
    A.setContext('shop-api');
    const { container } = render(Workspace);
    await tick();
    const box = container.querySelector('[aria-label="Project context"]');
    expect(box.textContent).toContain('shop-api');
    expect(box.querySelector('.cnt')).toBeNull();
    expect(box.querySelector('.dot').getAttribute('aria-label')).toBe('rss-feed (blog) is waiting for you');
    await fireEvent.click(box);
    const opts = [...document.querySelectorAll('[role="option"]')].map((o) =>
      o.textContent.replace(/\s+/g, ' ').trim(),
    );
    expect(opts[0]).toMatch(/^All projects\s?5/);
    // A separator after All projects, with no group header (03-context-select-open).
    const list = document.querySelector('[role="listbox"]');
    expect(list.children[1].getAttribute('role')).toBe('separator');
    expect(list.querySelector('.gh')).toBeNull();
    // Picking a project sets the context.
    await fireEvent.click(
      [...document.querySelectorAll('[role="option"]')].find((o) => o.textContent.includes('blog')),
    );
    expect(app.workspace.project).toBe('blog');
  });

  it('the palette "Set the project context…" opens it (no Mission filter, no throw)', async () => {
    const { container } = render(Workspace);
    await tick();
    const cmd = A.commands().find((c) => c.id === 'mission.filter');
    expect(cmd.label).toBe('Set the project context…');
    cmd.run();
    await tick();
    await tick();
    expect(app.contextOpen).toBe(false);
    expect(container.querySelector('[aria-label="Project context"]').getAttribute('aria-expanded')).toBe('true');
    expect('mission' in app).toBe(false);
    expect('missionFilterOpen' in app).toBe(false);
  });
});

describe('Workspace (design-notes §4.3, 03-app-*, 03-empty-workspace-*)', () => {
  it('one strip of the visible tabs, the active pane only, no pane header or number', async () => {
    A.setContext('blog');
    const { container } = render(Workspace);
    await tick();
    expect([...container.querySelectorAll('[role="tab"] .name')].map((t) => t.textContent)).toEqual([
      'Mission control',
      'fx',
      'nvim · ',
    ]);
    expect(container.querySelectorAll('[data-stub]')).toHaveLength(1);
    expect(container.querySelector('[role="tabpanel"]').getAttribute('aria-labelledby')).toBe('ws-tab-m');
    expect(container.querySelector('.ph, .num, .handle, .zoomtag')).toBeNull();
    A.setContext('shop-api');
    await tick();
    expect(container.querySelector('[role="tab"][aria-selected="true"] .name').textContent).toBe('Mission control');
    A.focusPane('sp');
    await tick();
    expect(container.querySelector('[role="tab"][aria-selected="true"] .name').textContent).toBe('shop-api · project');
  });

  it('empty in a project: ⌘K line, Open <project> and New session', async () => {
    app.layout = { kind: 'leaf', id: 'l1', tabs: ['sp'], active: 'sp' };
    A.setContext('blog');
    const { container } = render(Workspace);
    await tick();
    expect(container.querySelector('.empty').textContent.replace(/\s+/g, ' ').trim()).toBe(
      '⌘K to open anything Open blog New session',
    );
    await fireEvent.click([...container.querySelectorAll('.empty button')][0]);
    expect(Object.values(app.panes).some((s) => s.type === 'project' && s.projectId === 'blog')).toBe(true);
  });

  it('empty in All projects: Mission control and New session', async () => {
    app.layout = null;
    app.panes = {};
    const { container } = render(Workspace);
    await tick();
    const [mc, ns] = container.querySelectorAll('.empty button');
    expect(mc.textContent).toBe('Mission control');
    await fireEvent.click(ns);
    expect(app.overlay).toEqual({ kind: 'new-session' });
    await fireEvent.click(mc);
    expect(Object.values(app.panes).map((s) => s.type)).toEqual(['mission']);
  });

  it('a tab click focuses it; its × closes it', async () => {
    const { container } = render(Workspace);
    await tick();
    const tab = (name) => [...container.querySelectorAll('[role="tab"]')].find((t) => t.textContent.includes(name));
    await fireEvent.click(tab('fx'));
    expect(app.focused).toBe('bf');
    await fireEvent.click(tab('fx').querySelector('.x'));
    expect(app.panes.bf).toBeUndefined();
  });
});
