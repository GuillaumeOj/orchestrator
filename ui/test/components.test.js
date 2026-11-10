import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import Banner from '../src/components/Banner.svelte';
import AgentInspector from '../src/components/inspector/AgentInspector.svelte';
import PacingIndicator from '../src/components/PacingIndicator.svelte';
import Sidebar from '../src/components/Sidebar.svelte';
import StatusBadge from '../src/components/StatusBadge.svelte';
import StatusBar from '../src/components/StatusBar.svelte';
import TaskModelChip from '../src/components/TaskModelChip.svelte';
import { app, decide, decisionPushed } from '../src/lib/app.svelte.js';
import AgentPane from '../src/panes/AgentPane.svelte';
import { fixture, nowOf, permDecision, rec } from './fixture.js';

// The agent pane's terminal is not under test here.
vi.mock('../src/lib/term.js', () => ({
  TermPane: class {
    alive = false;
    mount() {}
    unmount() {}
    focus() {}
    scheduleFit() {}
    start() {
      return Promise.resolve();
    }
  },
}));

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

// ---------------------------------------------------------------- agent pane (design-notes §1.3.1, §8, 04-agent-*)

const txt = (el) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const labels = (els) => [...els].map((b) => b.getAttribute('aria-label') ?? txt(b));

/** Renders the agent pane of `id` with its record (`status`, `more`) and `live`; returns the parts. */
function agentPane(id, status, { live = false, more = {}, decisions = [] } = {}) {
  const a = app.data.agents.find((x) => x.id === id);
  a.live = live;
  app.now = nowOf(app.data, { decisions });
  app.now.agents[id] = rec(id, status, { project: a.project_id, live, ...more });
  app.panes = { p1: { type: 'agent', agentId: id } };
  const { container } = render(AgentPane, { paneId: 'p1' });
  const strip = /** @type {HTMLElement | null} */ (container.querySelector('.strip'));
  return {
    container,
    header: container.querySelector('.ah'),
    headerButtons: labels(container.querySelectorAll('.ah button')),
    strip,
    kind: strip?.dataset.strip ?? null,
    stripButtons: strip ? labels(strip.querySelectorAll('button.btn')) : [],
  };
}

const permFor = (id) =>
  permDecision({ id: `perm-${id}`, subject: { agent: id, project: 'shop-api' }, title: 'Bash · pnpm add zod' });
const LIVE_HEADER = ['Interrupt', 'Suspend', 'More actions'];

describe('agent pane: header and strips (§1.3.1)', () => {
  it.each([
    [
      'live · permission',
      'front-1',
      'waiting_for_you',
      true,
      'perm',
      ['Allow once', 'Deny', 'Always allow', 'Answer in the pane'],
    ],
    [
      'not live · permission',
      'front-1',
      'waiting_for_you',
      false,
      'perm',
      ['Allow once', 'Deny', 'Always allow', 'Resume'],
    ],
    ['live · needs you', 'back-1', 'waiting_for_you', true, 'needs', []],
    ['not live · needs you', 'back-1', 'waiting_for_you', false, 'needs', ['Resume']],
    ['error', 'rss-feed', 'error', false, 'error', ['Resume']],
    ['suspended', 'rss-feed', 'suspended', false, 'suspended', ['Resume']],
  ])('%s', (_, id, status, live, kind, buttons) => {
    const perm = kind === 'perm';
    const p = agentPane(id, status, {
      live,
      decisions: perm ? [permFor(id)] : [],
      more:
        kind === 'needs'
          ? { waiting_on: { kind: 'user', ref: 'q-1', since_ms: Date.now() - 60000 }, reason: 'asked which provider' }
          : {},
    });
    expect(p.kind).toBe(kind);
    expect(p.stripButtons).toEqual(buttons);
    // The header: Interrupt + Suspend only while live, `⋯` last in every state; never Resume nor hints.
    expect(p.headerButtons).toEqual(live ? LIVE_HEADER : ['More actions']);
    expect(p.container.querySelector('.kbd')).toBeNull();
    // Resume has one home: the strip.
    expect(labels(p.container.querySelectorAll('button')).filter((l) => l === 'Resume').length).toBe(
      buttons.includes('Resume') ? 1 : 0,
    );
    if (perm) expect(txt(p.strip)).toContain('also in Decisions');
    expect(txt(p.strip)).not.toContain('⌘⇧I');
  });

  it('queued: Run now with its cost and Cancel', () => {
    const queue = {
      entry: 'q-1',
      position: 1,
      tier: 'normal',
      run_now: { week_pp: 2.1, five_hour_pp: 4, binds: 'week' },
    };
    const p = agentPane('rss-feed', 'queued', { more: { queue, reason: 'live cap 2/2' } });
    expect(p.kind).toBe('queued');
    expect(p.stripButtons[0]).toMatch(/^Run now/);
    expect(p.stripButtons[1]).toBe('Cancel');
    expect(p.headerButtons).toEqual(['More actions']);
  });

  it('blocked by limit: Resume disabled, the reset in its reason', () => {
    const p = agentPane('rss-feed', 'blocked_by_limit');
    expect(p.kind).toBe('blocked');
    const b = p.strip.querySelector('button');
    expect(txt(b)).toBe('Resume');
    expect(b.disabled).toBe(true);
    expect(txt(p.strip)).toMatch(/Blocked by the 5-hour limit · resets \d\d:\d\d/);
  });

  it('the WaitingChip is in the strip while one shows, else in the header', () => {
    const waiting = {
      waiting_on: { kind: 'agent', ref: 'lead', since_ms: Date.now() - 60000 },
      reason: '→ lead · review',
    };
    const shown = agentPane('back-1', 'waiting_for_you', { live: true, more: waiting });
    expect(txt(shown.header)).not.toContain('→ lead');
    expect(txt(shown.strip)).toContain('→ lead');
    const none = agentPane('back-1', 'working', { live: true, more: { ...waiting, level: 'running' } });
    expect(none.strip).toBeNull();
    expect(txt(none.header)).toContain('→ lead');
  });

  it('the `⋯` Menu lists the agent actions; Run off-hours needs a queued turn', async () => {
    const p = agentPane('rss-feed', 'suspended');
    const trigger = p.header.querySelector('button[aria-label="More actions"]');
    await fireEvent.click(trigger);
    expect(trigger.classList.contains('pressed')).toBe(true); // R7: drawn pressed while open
    const rows = [...document.body.querySelectorAll('[role="menu"] [role^="menuitem"]')];
    expect(rows.map((r) => r.querySelector('.lab').textContent)).toEqual([
      'Shell in its worktree',
      'Diff against base',
      'Open in kitty',
      'Run off-hours',
      'Restart fresh…',
      'Kill…',
      'Delete…',
    ]);
    const by = (l) => rows.find((r) => r.querySelector('.lab').textContent === l);
    expect(by('Run off-hours').getAttribute('role')).toBe('menuitemcheckbox');
    expect(by('Run off-hours').getAttribute('aria-disabled')).toBe('true');
    expect(by('Kill…').classList.contains('danger')).toBe(true);
    expect(by('Delete…').classList.contains('danger')).toBe(true);
    await fireEvent.click(by('Delete…'));
    expect(app.overlay).toEqual({ kind: 'delete-agent', agentId: 'rss-feed' });
  });

  it('watch bar: Take over and Back to Mission control, keys in their tooltips', () => {
    app.panes = { mc: { type: 'mission' }, p1: { type: 'agent', agentId: 'back-1', watch: true, over: 'mc' } };
    app.now = nowOf(app.data);
    const { container } = render(AgentPane, { paneId: 'p1' });
    const bar = container.querySelector('.session.watch');
    expect(labels(bar.querySelectorAll('button'))).toEqual(['Take over', 'Back to Mission control']);
    expect(bar.querySelector('.kbd')).toBeNull();
  });
});

describe('agent inspector (§12.5, 04-agent-inspector)', () => {
  it.each([true, false])('live %s: Open session and the same `⋯`; no Resume, Interrupt, Suspend, Kill', (live) => {
    app.data.agents[2].live = live;
    app.now = nowOf(app.data);
    const { container } = render(AgentInspector, { id: 'rss-feed' });
    expect(labels(container.querySelectorAll('.acts button'))).toEqual(['Open session', 'More actions']);
    expect(container.querySelector('.kbd')).toBeNull();
  });
});

// ---------------------------------------------------------------- chrome (audit §3, §8; 03-title-status-banner)

describe('banner kinds and their buttons (audit §3)', () => {
  const buttons = (c) => labels(c.querySelectorAll('.banner button'));
  const base = () => {
    app.data.usage.snapshot.state = 'green';
    app.data.usage.five_hour.used_pct = 61;
  };

  it.each([
    ['core', (d) => (d.core = { ok: false, error: 'port 47820 busy' }), ['Dismiss']],
    [
      'week',
      (d) => (d.usage.weekly = { used_pct: 100, resets_at: Math.floor(Date.now() / 1000) + 3600 }),
      ['Week planner', 'Dismiss'],
    ],
    ['5h', (d) => (d.usage.five_hour.used_pct = 100), ['Week planner', 'Dismiss']],
    ['red', (d) => (d.usage.snapshot.state = 'red'), ['Week planner', 'Pause all', 'Dismiss']],
    ['ac', (d) => (d.banners = [{ kind: 'not_on_ac' }]), ['Week planner', 'Dismiss']],
    ['w:', (d) => (d.warnings = ['The GitHub token expires in 3 days.']), ['Dismiss']],
  ])('%s', (_, set, want) => {
    base();
    set(app.data);
    const { container } = render(Banner);
    expect(buttons(container)).toEqual(want);
    expect(container.querySelector('.kbd')).toBeNull();
    expect(txt(container)).not.toContain('Later');
  });

  it('× dismisses; Week planner opens the planner window', async () => {
    base();
    app.data.usage.five_hour.used_pct = 100;
    app.window = null;
    const { container } = render(Banner);
    await fireEvent.click(container.querySelector('.banner button'));
    expect(app.window?.kind).toBe('planner');
    await fireEvent.click(container.querySelector('button[aria-label="Dismiss"]'));
    expect(container.querySelector('.banner')).toBeNull();
    app.window = null;
  });
});

describe('status bar: information only (§8)', () => {
  it('pressure and off-hours are text, not buttons; LEADER only while the leader is held', async () => {
    app.data.power = { off_hours_pending: 1, on_ac: true };
    app.leader = false;
    const { container } = render(StatusBar);
    expect(container.querySelectorAll('button')).toHaveLength(0);
    expect(container.querySelector('[data-leader]')).toBeNull();
    expect(txt(container)).not.toContain('leader');
    app.leader = true;
    await tick();
    expect(txt(container.querySelector('[data-leader]'))).toContain('LEADER');
    app.leader = false;
  });
});

describe('pacing indicator: the Week planner entry', () => {
  it('its popover has a Week planner button that opens the window', async () => {
    app.window = null;
    const { container } = render(PacingIndicator);
    await fireEvent.click(container.querySelector('.ind'));
    const b = [...container.querySelectorAll('.pop button')].find((x) => txt(x) === 'Week planner');
    expect(b.querySelector('.kbd')).toBeNull();
    await fireEvent.click(b);
    expect(app.window?.kind).toBe('planner');
    app.window = null;
  });
});
