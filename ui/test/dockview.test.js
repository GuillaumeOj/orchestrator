// The Dock's tab strip and body (spec §3.1): agent and shell tabs, overflow,
// the zoomed and filling states.
import { fireEvent, render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fixture } from './fixture.js';

vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  errText: (e) => String(e),
  listen: () => Promise.resolve(() => {}),
  invoke: () => Promise.resolve(null),
}));
vi.mock('../src/panes/AgentPane.svelte', async () => ({
  default: (await import('./fixtures/PaneStub.svelte')).default,
}));
vi.mock('../src/panes/TermPaneView.svelte', async () => ({
  default: (await import('./fixtures/PaneStub.svelte')).default,
}));

const { app } = await import('../src/lib/app.svelte.js');
const { default: Dock } = await import('../src/components/Dock.svelte');

beforeEach(() => {
  document.body.innerHTML = '';
  const data = fixture();
  data.agents[0].team = { node: 'back' };
  app.data = data;
  app.panes = {
    p1: { type: 'project', projectId: 'shop-api', title: 'shop-api' },
    p2: { type: 'agent', agentId: 'back-1', title: 'back-1' },
    p3: { type: 'shell', termId: 1, title: 'zsh · shop-api' },
    p4: { type: 'agent', agentId: 'front-1', title: 'front-1' },
  };
  app.layout = { kind: 'leaf', id: 'l1', tabs: ['p1'], active: 'p1' };
  app.dock = { open: true, height: 0.45, tabs: ['p2', 'p3', 'p4'], active: 'p2', v: 2 };
  app.focused = 'p2';
  app.zoomed = null;
});

describe('Dock', () => {
  it('draws an agent tab (mark, team node or id, status icon) and a shell tab (its title)', () => {
    const { container } = render(Dock);
    const tabs = [...container.querySelectorAll('.tab')];
    expect(tabs.map((t) => t.querySelector('.name').textContent)).toEqual(['back', 'zsh · shop-api', 'front-1']);
    expect(tabs[0].querySelector('.mark')).not.toBeNull();
    expect(tabs[0].querySelector('.badge')).not.toBeNull();
    expect(tabs[1].querySelector('.mark')).toBeNull();
    expect(tabs[0].getAttribute('title')).toMatch(/^back-1/);
    expect(tabs[1].getAttribute('title')).toBe('zsh · shop-api');
    expect(tabs[0].querySelector('.x').getAttribute('title')).toBe('Close tab · the agent keeps running');
    expect(tabs[1].querySelector('.x').getAttribute('title')).toBe('Close shell');
    expect(tabs[0].classList.contains('on')).toBe(true);
    expect(container.querySelector('[data-stub="p2"]')).not.toBeNull();
  });

  it('shows the body of the active tab only, and closing an agent tab closes just that tab', async () => {
    const { container } = render(Dock);
    await fireEvent.click(container.querySelectorAll('.tab .label')[1]);
    expect(container.querySelector('[data-stub="p3"]')).not.toBeNull();
    expect(container.querySelector('[data-stub="p2"]')).toBeNull();
    await fireEvent.click(container.querySelectorAll('.tab .x')[0]);
    expect(app.dock.tabs).toEqual(['p3', 'p4']);
    expect(container.querySelectorAll('.tab')).toHaveLength(2);
  });

  it('keeps the handle and its height while the workspace shows, and drops them when it fills the column', async () => {
    const { container } = render(Dock);
    expect(container.querySelector('.handle')).not.toBeNull();
    expect(container.querySelector('section').style.flexBasis).toBe('45%');
    expect(container.querySelector('.zoomtag')).toBeNull();
    app.zoomed = 'p2';
    await vi.waitFor(() => expect(container.querySelector('.handle')).toBeNull());
    expect(container.querySelector('.zoomtag').textContent).toBe('ZOOMED');
    expect(container.textContent).toContain('restore');
    expect(container.querySelector('section').classList.contains('fill')).toBe(true);
  });

  it('fills the column with no workspace pane, without the ZOOMED tag', async () => {
    app.layout = null;
    const { container } = render(Dock);
    expect(container.querySelector('section').classList.contains('fill')).toBe(true);
    expect(container.querySelector('.handle')).toBeNull();
    expect(container.querySelector('.zoomtag')).toBeNull();
  });
});
