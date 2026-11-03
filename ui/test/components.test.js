import { fireEvent, render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it } from 'vitest';
import PacingIndicator from '../src/components/PacingIndicator.svelte';
import Sidebar from '../src/components/Sidebar.svelte';
import StatusBadge from '../src/components/StatusBadge.svelte';
import StatusBar from '../src/components/StatusBar.svelte';
import TaskModelChip from '../src/components/TaskModelChip.svelte';
import { app, decide, decisionPushed } from '../src/lib/app.svelte.js';
import { fixture, nowOf, permDecision } from './fixture.js';

beforeEach(() => {
  app.data = fixture();
  app.now = nowOf(app.data);
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

  it('week bars stack outside / you / agents up to the total, tick at the plan', async () => {
    const { container } = render(PacingIndicator);
    await fireEvent.click(container.querySelector('.ind'));
    const bars = [...container.querySelectorAll('[role="img"]')];
    expect(bars).toHaveLength(2); // the compact strip and the popover bar
    for (const bar of bars) {
      const widths = [...bar.querySelectorAll('span[data-seg]')].map((/** @type {HTMLElement} */ e) => [
        e.dataset.seg,
        e.style.width,
      ]);
      expect(widths).toEqual([
        ['outside', '4%'],
        ['you', '5%'],
        ['agents', '47%'],
      ]);
      expect(bar.querySelector('em').style.left).toBe('min(47%, 100% - 2px)');
      expect(bar.getAttribute('aria-label')).toBe('Week 56%: agents 47%, you 5%, outside app 4%; plan 47%');
    }
    const legend = container.querySelector('.pop').textContent.replace(/\s+/g, ' ');
    expect(legend).toContain('agents 47% · you 5% · outside app 4% · projected 98% at the reset');
  });

  it('week bar "you" is the segment, not interactive_used (critical work stays with the agents)', async () => {
    const d = fixture();
    d.usage.snapshot.interactive_used = 8; // 5 interactive + 3 critical, already in governed 47
    app.data = d;
    const { container } = render(PacingIndicator);
    await fireEvent.click(container.querySelector('.ind'));
    const bar = container.querySelector('.pop [role="img"]');
    expect(/** @type {HTMLElement} */ (bar.querySelector('span[data-seg="you"]')).style.width).toBe('5%');
    expect(bar.getAttribute('aria-label')).toBe('Week 56%: agents 47%, you 5%, outside app 4%; plan 47%');
    expect(container.querySelector('.pop').textContent).toContain('you 5% ·');
  });

  it('week bar without you or outside usage: agents only', () => {
    const d = fixture();
    Object.assign(d.usage.snapshot, { used: 47, interactive_used: 0, unattributed_so_far: 0, plan: 38 });
    app.data = d;
    const { container } = render(PacingIndicator);
    const bar = container.querySelector('[role="img"]');
    expect([...bar.querySelectorAll('span[data-seg]')].map((e) => e.getAttribute('data-seg'))).toEqual(['agents']);
    expect(bar.querySelector('em').style.left).toBe('min(38%, 100% - 2px)');
  });

  it('status bar shows the 5h guard and memory', () => {
    const { container } = render(StatusBar);
    const t = container.textContent.replace(/\s+/g, ' ');
    expect(t).toContain('5h guard');
    expect(t).toContain('RAM 898 MB / 8.0 GB');
    expect(t).toContain('Red');
  });

  it('a decision taken elsewhere drops ours still in its undo window; a parked one keeps it', () => {
    const perm = permDecision();
    decide(perm, 'approve');
    const parked = { ...perm, lifecycle: { state: 'parked', decided_by: 'expired' } };
    decisionPushed(perm, parked);
    expect(app.pending).toHaveLength(1);
    expect(app.toasts.at(-1).text).toBe('Parked · front-1 stopped and resumes on your answer');
    decisionPushed(parked, { ...perm, lifecycle: { state: 'effect', decided_by: 'pane', effect: 'allowed' } });
    expect(app.pending).toHaveLength(0);
    expect(app.toasts.at(-1).text).toBe('Answered in the pane · front-1');
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

describe('Markdown', () => {
  it('renders a table after a quote, not inside it, when its rows lack `>`', async () => {
    const { default: Markdown } = await import('../src/components/Markdown.svelte');
    const { container } = render(Markdown, { text: '> The table below.\n| a | b |\n|---|---|\n| 1 | 2 |' });
    const quote = container.querySelector('blockquote');
    const table = container.querySelector('table');
    expect(quote.textContent).not.toContain('|');
    expect(quote.querySelector('table')).toBeNull();
    expect(quote.compareDocumentPosition(table) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});

describe('task model chip', () => {
  it('shows the model with a tooltip, and nothing when unset', () => {
    const on = render(TaskModelChip, { task: { flow: { model: 'opus' } } });
    const chip = on.getByText('opus');
    expect(chip.getAttribute('title')).toBe('Runs on opus (set by the lead)');
    on.unmount();
    const off = render(TaskModelChip, { task: { flow: {} } });
    expect(off.container.querySelector('.chip')).toBeNull();
  });
});
