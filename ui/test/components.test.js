import { describe, expect, it, beforeEach } from 'vitest';
import { render, fireEvent } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { app } from '../src/lib/app.svelte.js';
import { fixture } from './fixture.js';
import InboxPane from '../src/panes/InboxPane.svelte';
import Sidebar from '../src/components/Sidebar.svelte';
import StatusBar from '../src/components/StatusBar.svelte';
import PacingIndicator from '../src/components/PacingIndicator.svelte';
import StatusBadge from '../src/components/StatusBadge.svelte';

beforeEach(() => {
  app.data = fixture();
  app.pending = [];
  app.toasts = [];
  app.panes = {};
  app.focused = null;
});

describe('components', () => {
  it('status badge carries a label, not only a color', () => {
    const { getByText } = render(StatusBadge, { state: 'waiting_for_you' });
    expect(getByText('Waiting for you')).toBeTruthy();
  });

  it('sidebar lists solo sessions under their project', () => {
    const { container, getByText } = render(Sidebar);
    expect(getByText('shop-api')).toBeTruthy();
    const rows = [...container.querySelectorAll('[data-row]')].map((r) => r.textContent.replace(/\s+/g, ' ').trim());
    expect(rows[0]).toMatch(/^▾ shop-api/);
    expect(rows[1]).toMatch(/back-1 62%/);
    expect(rows[3]).toMatch(/blog/);
  });

  it('title bar figures follow J8: Week total, agents, plan and the delta', () => {
    const { container } = render(PacingIndicator);
    const t = container.textContent.replace(/\s+/g, ' ');
    expect(t).toContain('Week 56%');
    expect(t).toContain('agents 47%');
    expect(t).toContain('plan 38% (+9)');
    expect(t).toContain('5h 61%');
  });

  it('status bar shows the 5h guard and memory', () => {
    const { container } = render(StatusBar);
    const t = container.textContent.replace(/\s+/g, ' ');
    expect(t).toContain('5h guard');
    expect(t).toContain('RAM 898 MB / 8.0 GB');
    expect(t).toContain('Red');
  });

  it('inbox triage: j moves, a decides with a 5 s undo window, u undoes', async () => {
    const { container, getAllByText } = render(InboxPane);
    const root = container.querySelector('[data-pane-focus]');
    expect(getAllByText('Allow once', { exact: false }).length).toBeGreaterThan(0);
    await fireEvent.keyDown(root, { key: 'j' });
    expect(container.querySelector('.item.sel').textContent).toContain('Defer');
    await fireEvent.keyDown(root, { key: 'k' });
    await fireEvent.keyDown(root, { key: 'a' });
    flushSync();
    expect(app.pending).toHaveLength(1);
    expect(app.pending[0]).toMatchObject({ item: 'perm-1', action: 'approve' });
    expect(app.toasts.at(-1).text).toBe('Allowed once · front-1 · pnpm add @stripe/stripe-js');
    expect(container.querySelector('.item.sel').textContent).toContain('Defer'); // selection moved on
    await fireEvent.keyDown(root, { key: 'u' });
    expect(app.pending).toHaveLength(0);
  });
});

describe('diff pane', () => {
  it('renders its file list and reports a failed load', async () => {
    const { default: DiffPane } = await import('../src/panes/DiffPane.svelte');
    app.panes = { p1: { type: 'diff', projectId: 'shop-api', path: '/w', title: 'diff' } };
    const { container } = render(DiffPane, { paneId: 'p1' });
    expect(container.querySelector('[aria-label="Changed files"]')).toBeTruthy();
    // Outside Tauri the load rejects: the pane says so instead of crashing.
    await new Promise((r) => setTimeout(r, 0));
    expect(container.textContent).toMatch(/not in Tauri/);
  });
});
