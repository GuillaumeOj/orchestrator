import { fireEvent, render } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { beforeEach, describe, expect, it } from 'vitest';
import PacingIndicator from '../src/components/PacingIndicator.svelte';
import Sidebar from '../src/components/Sidebar.svelte';
import StatusBadge from '../src/components/StatusBadge.svelte';
import StatusBar from '../src/components/StatusBar.svelte';
import { app, decide, onOutcome } from '../src/lib/app.svelte.js';
import InboxPane from '../src/panes/InboxPane.svelte';
import { fixture } from './fixture.js';

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
    expect(app.toasts.at(-1).text).toBe('Allowing once · front-1 · pnpm add @stripe/stripe-js');
    expect(container.querySelector('.item.sel').textContent).toContain('Defer'); // selection moved on
    await fireEvent.keyDown(root, { key: 'u' });
    expect(app.pending).toHaveLength(0);
  });

  const withItems = (...extra) => {
    const d = fixture();
    d.inbox.items = [...extra, ...d.inbox.items];
    app.data = d;
  };
  const detail = (container) => container.querySelector('.detail');
  const buttons = (container) =>
    [...detail(container).querySelectorAll('.actions button')].map((b) => b.textContent.trim());

  it('permission card: tool, cwd name with the full path on hover, input collapsed past 6 lines', async () => {
    const perm = fixture().inbox.items[0];
    const command = Array.from({ length: 9 }, (_, i) => `echo ${i + 1}`).join('\n');
    withItems({ ...perm, id: 'inbox-9', payload: { ...perm.payload, tool_input: { command } } });
    const { container, getByText } = render(InboxPane);
    const cwd = getByText('front-1', { selector: '.call .mono' });
    expect(cwd.getAttribute('title')).toBe('/Users/me/dev/shop-web.worktrees/front-1');
    expect(detail(container).querySelector('.input').textContent).toBe(command.split('\n').slice(0, 6).join('\n'));
    await fireEvent.click(getByText('Show all · 3 more lines'));
    expect(detail(container).querySelector('.input').textContent).toBe(command);
    expect(buttons(container)).toEqual([
      'Allow once a',
      'Deny r',
      'Deny with message ⏎',
      'Always allow',
      'Answer in the pane',
      'Open o',
    ]);
    expect(container.querySelector('.parked')).toBeNull();
  });

  it('an outcome decided elsewhere drops our pending decision, a parked one keeps it', () => {
    const perm = fixture().inbox.items[0];
    decide(perm, 'approve');
    onOutcome({ item: perm.id, decided_by: 'expired', effect: 'parked', at_ms: 0 });
    expect(app.pending).toHaveLength(1);
    onOutcome({ item: perm.id, decided_by: 'pane', effect: 'in_pane', at_ms: 0 });
    expect(app.pending).toHaveLength(0);
    expect(app.toasts.at(-1).text).toBe('Answered in the pane · front-1');
  });

  it('"Answer in the pane" opens the pane and decides nothing', async () => {
    const { container } = render(InboxPane);
    const b = [...detail(container).querySelectorAll('.actions button')].find(
      (x) => x.textContent === 'Answer in the pane',
    );
    await fireEvent.click(b);
    expect(app.pending).toHaveLength(0);
    expect(Object.values(app.panes)).toMatchObject([{ type: 'agent', agentId: 'front-1' }]);
  });

  it('parked permission: the Parked marker, no "Answer in the pane", Always allow only when offered', () => {
    const perm = fixture().inbox.items[0];
    withItems({
      ...perm,
      id: 'inbox-9',
      status: 'parked',
      actions: ['a', 'r', 'enter', 'o'],
      payload: { ...perm.payload, ask_rule: 12, always: null },
    });
    const { container } = render(InboxPane);
    expect(detail(container).querySelector('.parked').textContent).toContain('Parked');
    expect(detail(container).textContent).toContain('the agent stopped and resumes on your answer');
    expect(container.querySelector('.item.sel').textContent).toContain('Permission · parked');
    expect(buttons(container)).toEqual(['Allow once a', 'Deny r', 'Deny with message ⏎', 'Open o']);
  });

  it('startup card: the PTY tail in monospace and its actions', async () => {
    const tail = Array.from({ length: 20 }, (_, i) => `line ${i + 1}`).join('\n');
    withItems({
      id: 'startup-4',
      level: 1,
      kind: 'startup',
      status: 'open',
      agent_id: 'back-1',
      title: 'back-1 could not resume',
      detail: 'Its resume start exited at once (status 1)',
      preview: 'line 20',
      created_ms: Date.now(),
      actions: ['o', 'restart_fresh', 'dismiss', 'delete'],
      payload: { waiting_on: 'startup', mode: 'resume', since_ms: 0, failed: 'resume', exit_status: 1, tail },
    });
    const { container } = render(InboxPane);
    expect(container.querySelector('.item.sel').textContent).toContain('Startup');
    expect(detail(container).querySelector('pre.mono').textContent).toBe(tail.split('\n').slice(-12).join('\n'));
    expect(buttons(container)).toEqual(['Restart fresh', 'Dismiss', 'Delete', 'Open pane o']);
    await fireEvent.click(
      [...detail(container).querySelectorAll('.actions button')].find((b) => b.textContent === 'Restart fresh'),
    );
    expect(app.pending[0]).toMatchObject({ item: 'startup-4', action: 'restart_fresh' });
    expect(app.toasts.at(-1).text).toBe('Restarting fresh · back-1');
  });

  it('restart gate: its text, Approve/Reject and the after-restart note, never "Spec gate"', () => {
    withItems({
      id: 'gate-3',
      level: 1,
      kind: 'gate',
      status: 'open',
      agent_id: 'back-1',
      title: 'back-1 waits for a decision',
      detail: 'Still waiting when the core restarted: run the migration. Approve to let it proceed',
      preview: 'run the migration',
      created_ms: Date.now(),
      actions: ['a', 'r', 'o'],
      payload: { action: 'restart_gate', agent: 'back-1', detail: 'run the migration', restarted: true },
    });
    const { container } = render(InboxPane);
    expect(container.textContent).not.toContain('Spec gate');
    expect(detail(container).querySelector('.rendered .md').textContent).toBe('run the migration');
    expect(detail(container).textContent).toContain('After restart');
    expect(buttons(container)).toEqual(['Approve a', 'Reject r', 'Open o']);
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
