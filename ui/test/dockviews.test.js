// The Dock (spec §3.1, design-notes §5.2, §12.3, §12.7): Agents / Shells views on
// one compact strip, the project filter, the waiting-dot rule, opening rules,
// empty views, closing the last tab of a view, cycling, ⌘J, the Hide dock icon.
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fixture } from './fixture.js';

let nextTerm = 1;
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  errText: (e) => String(e),
  listen: () => Promise.resolve(() => {}),
  invoke: (cmd, args) => {
    if (cmd === 'term_spawn')
      return Promise.resolve({ id: nextTerm++, cwd: args.req.cwd ?? '/dev/shop-api', shell: 'zsh' });
    return Promise.resolve(null);
  },
}));
vi.mock('../src/panes/AgentPane.svelte', async () => ({
  default: (await import('./fixtures/PaneStub.svelte')).default,
}));
vi.mock('../src/panes/TermPaneView.svelte', async () => ({
  default: (await import('./fixtures/PaneStub.svelte')).default,
}));

const A = await import('../src/lib/app.svelte.js');
const { app } = A;
const { default: Dock } = await import('../src/components/Dock.svelte');

/** Waits as the core says: the record's level `needs_you`. */
const waiting = (agentId, project) => {
  app.now.agents[agentId] = { agent: agentId, status: 'waiting_for_you', level: 'needs_you', project };
};

beforeEach(() => {
  document.body.innerHTML = '';
  const data = fixture();
  data.agents[0].team = { node: 'back' };
  app.data = data;
  app.now.agents = {};
  app.panes = {
    p1: { type: 'project', projectId: 'shop-api', title: 'shop-api' },
    p2: { type: 'agent', agentId: 'back-1', projectId: 'shop-api', title: 'back-1' },
    p3: { type: 'shell', termId: 1, title: 'zsh · ~/shop-api', cwd: '/dev/shop-api', live: true, shell: 'zsh' },
    p4: { type: 'agent', agentId: 'front-1', projectId: 'shop-api', title: 'front-1' },
    p5: { type: 'agent', agentId: 'rss-feed', projectId: 'blog', title: 'rss-feed' },
    p6: { type: 'shell', termId: 2, title: 'gh auth login' },
  };
  app.layout = { kind: 'leaf', id: 'l1', tabs: ['p1'], active: 'p1' };
  app.dock = {
    open: true,
    height: 0.45,
    tabs: ['p2', 'p3', 'p4', 'p5', 'p6'],
    active: 'p2',
    v: 2,
    view: 'agents',
    agentProject: 'all',
    activeAgent: 'p2',
    activeShell: 'p3',
  };
  app.workspace = { project: 'all' };
  app.focused = 'p2';
});

describe('dock derivations (§5.2)', () => {
  it('dockAgents / dockShells / dockTabs per view and filter', () => {
    expect(A.dockAgents()).toEqual(['p2', 'p4', 'p5']);
    expect(A.dockShells()).toEqual(['p3', 'p6']);
    expect(A.dockTabs()).toEqual(['p2', 'p4', 'p5']);
    app.dock.agentProject = 'blog';
    expect(A.dockTabs()).toEqual(['p5']);
    app.dock.view = 'shells';
    expect(A.dockTabs()).toEqual(['p3', 'p6']);
  });

  it('counts are the open tabs of the kind, whatever the filter', () => {
    app.dock.agentProject = 'blog';
    expect(A.dockViewOptions().map((o) => [o.label, o.count])).toEqual([
      ['Agents', 3],
      ['Shells', 2],
    ]);
  });

  it('the project options list only projects with an open agent tab, the filter kept at 0', () => {
    expect(A.dockProjectOptions().map((o) => [o.label, o.count])).toEqual([
      ['All projects', 3],
      ['shop-api', 2],
      ['blog', 1],
    ]);
    app.dock.tabs = ['p2', 'p3'];
    expect(A.dockProjectOptions().map((o) => o.label)).toEqual(['All projects', 'shop-api']);
    app.dock.agentProject = 'blog';
    expect(A.dockProjectOptions().map((o) => [o.label, o.count])).toEqual([
      ['All projects', 1],
      ['shop-api', 1],
      ['blog', 0],
    ]);
  });
});

describe('waiting dot rule (§12.7)', () => {
  it('Agents view: no dot on the view Select; the project Select only for another project than the filter', () => {
    waiting('rss-feed', 'blog');
    expect(A.dockViewWaiting()).toBe(null);
    expect(A.dockProjectWaiting()).toBe(null); // All projects: never
    app.dock.agentProject = 'shop-api';
    expect(A.dockProjectWaiting()).toBe('rss-feed (blog) is waiting for you');
    expect(A.dockViewWaiting()).toBe(null);
    app.dock.agentProject = 'blog';
    expect(A.dockProjectWaiting()).toBe(null); // the waiting agent is the filter's own: visible
  });

  it('Shells view: the dot moves to the view Select, the project Select is gone', () => {
    waiting('rss-feed', 'blog');
    app.dock.agentProject = 'shop-api';
    app.dock.view = 'shells';
    expect(A.dockViewWaiting()).toBe('rss-feed (blog) is waiting for you');
    app.now.agents = {};
    expect(A.dockViewWaiting()).toBe(null);
  });

  it('options carry their own dot', () => {
    waiting('rss-feed', 'blog');
    expect(A.dockProjectOptions().map((o) => !!(/** @type {any} */ (o).dot))).toEqual([false, false, true]);
  });

  it('one waiting agent never lights two closed Selects (dock + context)', () => {
    waiting('rss-feed', 'blog');
    app.workspace = { project: 'shop-api' };
    app.dock.agentProject = 'shop-api';
    // The dock is open with that agent's tab: its project Select carries the dot, the context's does not.
    expect(A.dockProjectWaiting()).not.toBe(null);
    expect(A.contextWaiting()).toBe(null);
    // Dock hidden: the context Select carries it.
    app.dock.open = false;
    expect(A.contextWaiting()).toBe('rss-feed (blog) is waiting for you');
    // Tab closed: the same.
    app.dock.open = true;
    app.dock.tabs = app.dock.tabs.filter((t) => t !== 'p5');
    expect(A.contextWaiting()).toBe('rss-feed (blog) is waiting for you');
  });
});

describe('opening, switching, closing', () => {
  it('opening an agent switches to Agents; opening a shell to Shells', async () => {
    app.dock.view = 'shells';
    app.dock.active = 'p3';
    A.openAgent('rss-feed');
    expect(app.dock.view).toBe('agents');
    expect(app.dock.active).toBe('p5');
    await A.openShell();
    expect(app.dock.view).toBe('shells');
    expect(app.dock.active).toBe(app.dock.activeShell);
  });

  it('the filter stays All, or follows an agent that the filter hides', () => {
    A.focusPane('p5');
    expect(app.dock.agentProject).toBe('all');
    app.dock.agentProject = 'shop-api';
    app.dock.active = 'p2';
    A.focusPane('p4'); // inside the filter: unchanged
    expect(app.dock.agentProject).toBe('shop-api');
    A.focusPane('p5'); // hidden by it: the filter moves to its project
    expect(app.dock.agentProject).toBe('blog');
    expect(A.dockTabs()).toEqual(['p5']);
  });

  it("switching view restores that view's active tab and moves the dock focus", () => {
    A.focusPane('p4');
    A.setDockView('shells');
    expect(app.dock.active).toBe('p3');
    expect(app.focused).toBe('p3');
    A.focusPane('p6');
    A.setDockView('agents');
    expect(app.dock.active).toBe('p4');
    expect(app.focused).toBe('p4');
    // A workspace focus stays where it is.
    A.focusPane('p1');
    A.setDockView('shells');
    expect(app.focused).toBe('p1');
    expect(app.dock.active).toBe('p6');
  });

  it('the project filter picks its first agent when the active one is hidden', () => {
    A.setDockProject('blog');
    expect(app.dock.active).toBe('p5');
    expect(app.dock.agentProject).toBe('blog');
  });

  it('an empty filtered view keeps the filter: no active tab', () => {
    A.setDockProject('blog');
    A.closePane('p5');
    expect(app.dock.agentProject).toBe('blog');
    expect(app.dock.active).toBe(null);
    expect(app.dock.view).toBe('agents');
    expect(app.dock.open).toBe(true);
  });

  it('closing the last tab of a view switches to the other view, else closes the dock', () => {
    A.setDockView('shells');
    A.closePane('p3');
    expect(app.dock.view).toBe('shells');
    A.closePane('p6');
    expect(app.dock.view).toBe('agents');
    expect(app.dock.active).toBe('p2');
    expect(app.dock.open).toBe(true);
    for (const id of ['p2', 'p4', 'p5']) A.closePane(id);
    expect(app.dock.open).toBe(false);
    expect(app.dock.tabs).toEqual([]);
  });

  it('closing the active tab activates its neighbour in the view', () => {
    A.focusPane('p4');
    A.closePane('p4');
    expect(app.dock.active).toBe('p5');
    expect(app.dock.activeAgent).toBe('p5');
  });

  it('dock.next / dock.prev cycle within the view and the filter', () => {
    A.cycleDock(1);
    expect(app.dock.active).toBe('p4');
    A.cycleDock(1);
    A.cycleDock(1);
    expect(app.dock.active).toBe('p2');
    A.cycleDock(-1);
    expect(app.dock.active).toBe('p5');
    A.setDockProject('shop-api');
    A.cycleDock(1);
    expect(app.dock.active).toBe('p4');
    A.cycleDock(1);
    expect(app.dock.active).toBe('p2');
    A.setDockView('shells');
    A.cycleDock(1);
    expect(app.dock.active).toBe('p6');
  });

  it('⌘J with no tab opens a shell; with tabs it shows and hides', async () => {
    app.dock = { ...app.dock, open: false, tabs: [], active: null, activeAgent: null, activeShell: null };
    app.panes = { p1: app.panes.p1 };
    A.toggleDock();
    await vi.waitFor(() => expect(app.dock.tabs).toHaveLength(1));
    expect(app.dock.open).toBe(true);
    expect(app.dock.view).toBe('shells');
    A.toggleDock();
    expect(app.dock.open).toBe(false);
  });
});

describe('Dock', () => {
  it('draws the Agents view: view Select with the count, project Select, agent tabs, Hide dock, no +', () => {
    const { container } = render(Dock);
    const view = container.querySelector('[aria-label="Dock view"]');
    expect(view.textContent.replace(/\s+/g, ' ')).toContain('Agents 3');
    expect(container.querySelector('[aria-label="Project filter"]').textContent).toContain('All projects');
    const tabs = [...container.querySelectorAll('[role="tab"]')];
    expect(tabs.map((t) => t.querySelector('.name').textContent)).toEqual(['back', 'front-1', 'rss-feed']);
    expect(tabs[0].querySelector('.mark')).not.toBeNull();
    expect(tabs[0].querySelector('.badge')).not.toBeNull();
    expect(tabs[0].getAttribute('aria-selected')).toBe('true');
    expect(container.querySelector('[aria-label="New shell"]')).toBeNull();
    expect(container.querySelector('[aria-label="Hide dock"]')).not.toBeNull();
    expect(container.querySelector('.kbd')).toBeNull();
    expect(container.querySelector('[data-stub="p2"]')).not.toBeNull();
  });

  it('draws the Shells view: shell titles, + New shell, no project Select', () => {
    app.dock.view = 'shells';
    app.dock.active = 'p3';
    const { container } = render(Dock);
    expect(container.querySelector('[aria-label="Project filter"]')).toBeNull();
    const tabs = [...container.querySelectorAll('[role="tab"]')];
    expect(tabs.map((t) => t.querySelector('.name').textContent)).toEqual(['zsh · ~/shop-api', 'gh auth login']);
    expect(tabs[0].querySelector('.mark')).toBeNull();
    expect(container.querySelector('[aria-label="New shell"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="Dock view"]').textContent.replace(/\s+/g, ' ')).toContain('Shells 2');
  });

  it('shows the waiting dot on the project Select, then on the view Select in Shells', async () => {
    waiting('rss-feed', 'blog');
    app.dock.agentProject = 'shop-api';
    app.dock.active = 'p2';
    const { container } = render(Dock);
    await tick();
    const project = container.querySelector('[aria-label="Project filter"]');
    expect(project.querySelector('.dot').getAttribute('aria-label')).toBe('rss-feed (blog) is waiting for you');
    expect(container.querySelector('[aria-label="Dock view"] .dot')).toBeNull();
    A.setDockView('shells');
    await tick();
    expect(container.querySelector('[aria-label="Project filter"]')).toBeNull();
    expect(container.querySelector('[aria-label="Dock view"] .dot').getAttribute('aria-label')).toBe(
      'rss-feed (blog) is waiting for you',
    );
  });

  it('picking in the Selects switches the view and narrows the tabs', async () => {
    const { container } = render(Dock);
    await fireEvent.click(container.querySelector('[aria-label="Project filter"]'));
    await fireEvent.click(
      [...document.querySelectorAll('[role="option"]')].find((o) => o.textContent.includes('blog')),
    );
    expect(app.dock.agentProject).toBe('blog');
    await tick();
    expect([...container.querySelectorAll('[role="tab"]')]).toHaveLength(1);
    await fireEvent.click(container.querySelector('[aria-label="Dock view"]'));
    await fireEvent.click(
      [...document.querySelectorAll('[role="option"]')].find((o) => o.textContent.includes('Shells')),
    );
    expect(app.dock.view).toBe('shells');
  });

  it('says what an empty view is: no shell (with New shell and no +), no agent in the project', async () => {
    app.dock.tabs = ['p2'];
    app.dock.view = 'shells';
    app.dock.active = null;
    const { container } = render(Dock);
    await tick();
    expect(container.textContent).toContain('No shell open');
    expect(container.querySelector('[aria-label="New shell"]')).toBeNull();
    expect([...container.querySelectorAll('button')].some((b) => b.textContent === 'New shell')).toBe(true);
    A.setDockView('agents');
    A.setDockProject('blog');
    await tick();
    expect(container.textContent).toContain('No agent running in blog');
  });

  it('the Hide dock button hides the dock', async () => {
    const { container } = render(Dock);
    await fireEvent.click(container.querySelector('[aria-label="Hide dock"]'));
    expect(app.dock.open).toBe(false);
  });

  it('shows the body of the active tab only, and closing a tab closes just that tab', async () => {
    const { container } = render(Dock);
    await fireEvent.click(container.querySelectorAll('[role="tab"]')[1]);
    expect(container.querySelector('[data-stub="p4"]')).not.toBeNull();
    expect(container.querySelector('[data-stub="p2"]')).toBeNull();
    await fireEvent.click(container.querySelectorAll('[role="tab"] .x')[0]);
    expect(app.dock.tabs).toEqual(['p3', 'p4', 'p5', 'p6']);
    await tick();
    expect(container.querySelectorAll('[role="tab"]')).toHaveLength(2);
  });

  it('keeps the handle and its height while the workspace shows, and drops them when it fills the column', async () => {
    const { container } = render(Dock);
    expect(container.querySelector('.handle')).not.toBeNull();
    expect(container.querySelector('section').style.flexBasis).toBe('45%');
    expect(container.textContent).not.toContain('ZOOMED');
    app.layout = null;
    await vi.waitFor(() => expect(container.querySelector('.handle')).toBeNull());
    expect(container.querySelector('section').classList.contains('fill')).toBe(true);
  });
});
