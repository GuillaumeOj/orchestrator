// P4.1 exit criterion (spec §1 a, §6.1): every surface renders the core's
// attention records and counts from one fixture, and they all agree. With 1
// permission prompt, 1 question, 1 queued agent, 1 not-started agent and 2
// running: needs you 2, decisions 2, queued 2, live 5 (with a waiting agent).
// app_state's raw states and live flags contradict the records on purpose: a
// surface that derives its own reads them and fails.

import { render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { act, countsOf, decision, fixture, permDecision, rec } from './fixture.js';

vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd) =>
    cmd === 'feature_view'
      ? Promise.resolve({
          feature: { id: 'f-co', project_id: 'shop', slug: 'checkout', integration_worktree: '/w/co' },
          run: { id: 'run-1', status: 'spec', started_at_ms: Date.now() - 3600000 },
          team: null,
          project: { id: 'shop', name: 'shop', backend: 'local' },
          agents: [{ id: 'perm-1' }, { id: 'ask-1' }, { id: 'run-1a' }, { id: 'rev-1' }],
          tasks: [],
          docs: [],
          artifacts: [],
          timeline: [],
        })
      : Promise.reject(new Error(`not in Tauri: ${cmd}`)),
  listen: () => Promise.resolve(() => {}),
  errText: (e) => String(e?.message ?? e),
}));

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

const { default: Sidebar } = await import('../src/components/Sidebar.svelte');
const { default: IconRail } = await import('../src/components/IconRail.svelte');
const { default: StatusBar } = await import('../src/components/StatusBar.svelte');
const { default: Banner } = await import('../src/components/Banner.svelte');
const { default: TitleBar } = await import('../src/components/TitleBar.svelte');
const { default: Inspector } = await import('../src/components/Inspector.svelte');
const { default: OverviewPane } = await import('../src/panes/OverviewPane.svelte');
const { default: InboxPane } = await import('../src/panes/InboxPane.svelte');
const { default: AgentPane } = await import('../src/panes/AgentPane.svelte');
const { default: FeaturePane } = await import('../src/panes/FeaturePane.svelte');

const MIN = 60000;
const team = (node) => ({ project_id: 'shop', feature_id: 'f-co', node });
// Raw app_state agents: every one idle and not live (the records say otherwise).
const raw = (id, project, more = {}) => ({ id, state: 'idle', live: false, project_id: project, ...more });

function world() {
  const now = Date.now();
  const d = /** @type {any} */ (fixture());
  d.projects = [
    { id: 'shop', name: 'shop', path: '/dev/shop', git: { branch: 'main', ahead: 0, behind: 0, dirty: 0 } },
    { id: 'blog', name: 'blog', path: '/dev/blog', git: { branch: 'main', ahead: 0, behind: 0, dirty: 0 } },
  ];
  d.features = [
    { id: 'f-co', project_id: 'shop', slug: 'checkout', branch: 'orch/checkout', run: { id: 'run-1', status: 'spec' } },
  ];
  const entry = (id) => ({ id, tier: 'normal', reason: { kind: 'live_cap', cap: 4 }, estimated_cost_pct: 1 });
  d.agents = [
    raw('perm-1', 'shop', { team: team('perm-1') }),
    raw('ask-1', 'shop', { team: team('ask-1') }),
    raw('run-1a', 'shop', { team: team('run-1a') }),
    raw('rev-1', 'shop', { team: team('rev-1') }),
    raw('queued-1', 'blog', { queue_entry: entry('q-1') }),
    raw('fresh-1', 'blog', { queue_entry: entry('q-2') }),
    raw('run-2', 'blog'),
  ];
  d.queue = []; // the raw queue is empty: queued comes from the counts
  const perm = permDecision({
    id: 'inbox-1',
    subject: { agent: 'perm-1', project: 'shop' },
    created_ms: now - 4 * MIN,
  });
  const ask = decision({
    id: 'inbox-2',
    kind: 'question',
    subject: { agent: 'ask-1', project: 'shop' },
    title: 'Which auth provider?',
    created_ms: now - 2 * MIN,
    context: [{ type: 'markdown', text: 'Which auth provider?' }],
    actions: [act('answer', 'Answer', 'enter'), act('reject', 'Dismiss', 'r'), act('open', 'Open pane', 'o')],
  });
  const user = (ref, since) => ({ waiting_on: { kind: 'user', ref, since_ms: since } });
  const gov = { waiting_on: { kind: 'governor', ref: 'x', since_ms: now - 5 * MIN } };
  const agents = [
    rec('perm-1', 'waiting_for_you', {
      project: 'shop',
      run: 'run-1',
      reason: 'permission: Bash pnpm add zod',
      ...user('inbox-1', now - 4 * MIN),
    }),
    rec('ask-1', 'waiting_for_you', {
      project: 'shop',
      run: 'run-1',
      reason: 'question: which auth provider?',
      ...user('inbox-2', now - 2 * MIN),
    }),
    rec('run-1a', 'working', { project: 'shop', run: 'run-1', activity: 'Bash cargo test' }),
    rec('rev-1', 'idle', {
      project: 'shop',
      run: 'run-1',
      level: 'waiting',
      reason: '→ lead · review of T-4',
      waiting_on: { kind: 'agent', ref: 'lead-1', since_ms: now - 3 * MIN },
    }),
    rec('queued-1', 'queued', { project: 'blog', live: false, reason: 'live cap 4/4 · 2nd in line', ...gov }),
    rec('fresh-1', 'not_started', { project: 'blog', live: false, reason: 'live cap 4/4 · 1st in line', ...gov }),
    rec('run-2', 'working', { project: 'blog', activity: 'Edit src/feed.ts' }),
  ];
  const C = countsOf;
  const shop = C({ needs_you: 2, decisions: 2, running: 1, waiting: 1, live: 4 });
  app.data = d;
  app.now = {
    rev: 1,
    agents: Object.fromEntries(agents.map((r) => [r.agent, r])),
    runs: {
      'run-1': {
        run: 'run-1',
        feature: 'checkout',
        project: 'shop',
        phase: 'executing',
        phase_since_ms: now - 10 * MIN,
        level: 'needs_you',
        members: ['perm-1', 'ask-1', 'run-1a', 'rev-1'],
        counts: shop,
      },
    },
    decisions: Object.fromEntries([perm, ask].map((x) => [x.id, x])),
    outcomes: [],
    counts: {
      global: C({ needs_you: 2, decisions: 2, running: 2, waiting: 1, queued: 2, live: 5, live_cap: 4 }),
      projects: { shop, blog: C({ running: 1, queued: 2, live: 1 }) },
    },
  };
}

const text = (c) => c.textContent.replace(/\s+/g, ' ');
const openPane = (spec) => {
  app.panes = { p1: { ...spec, title: 'x' } };
  app.focused = 'p1';
};

beforeEach(() => {
  world();
  app.pending = [];
  app.toasts = [];
  app.panes = {};
  app.focused = null;
  app.selectedAgent = null;
});

describe('every surface shows the same numbers (spec §6.1)', () => {
  it('needs you 2', () => {
    // Sidebar: the feature's badge is its run's needs_you.
    const side = render(Sidebar).container;
    expect(side.querySelector('.feature .badge').getAttribute('aria-label')).toBe('2 need you');
    // Icon rail: the project's needs_you, nothing for the other.
    const rail = render(IconRail).container;
    expect([...rail.querySelectorAll('.count')].map((x) => x.textContent)).toEqual(['2']);
    // Overview: the chip and the footer.
    const ov = render(OverviewPane, { paneId: 'ov' }).container;
    expect(text(ov)).toContain('Need you 2');
    expect(ov.querySelector('.totals').textContent).toContain('2 need you');
  });

  it('decisions 2', () => {
    const title = render(TitleBar).container;
    expect(title.querySelector('.inbox .count').textContent).toBe('2');
    const inbox = render(InboxPane).container;
    expect(inbox.querySelectorAll('.item')).toHaveLength(2);
    expect(text(inbox)).toContain('All 2');
  });

  it('queued 2 and live 5 / cap 4', () => {
    const bar = render(StatusBar).container;
    expect(bar.querySelector('.live').textContent).toContain('5 live / cap 4');
    expect(bar.querySelector('.queued').textContent).toContain('queued 2');
    const ov = render(OverviewPane, { paneId: 'ov' }).container;
    expect(text(ov)).toContain('Live 5');
    expect(text(ov)).toContain('Queued 2');
    expect(ov.querySelector('.totals').textContent).toContain('5 live / cap 4 · queued 2');
    // The Red banner (the fixture's Governor is in Red).
    const banner = render(Banner).container;
    expect(text(banner)).toContain('queued 2');
  });
});

describe('surfaces render the record: status, reason, since', () => {
  it('a wait on another agent reads the same in the Overview, the agent pane and the Inspector', () => {
    const want = /→ lead · review of T-4 (· )?3 min/;
    const ov = render(OverviewPane, { paneId: 'ov' }).container;
    expect(text(ov)).toMatch(want);
    openPane({ type: 'agent', agentId: 'rev-1' });
    const pane = render(AgentPane, { paneId: 'p1' }).container;
    expect(text(pane.querySelector('.ah'))).toMatch(want);
    const insp = render(Inspector).container;
    expect(text(insp)).toMatch(want);
    expect(text(insp)).toContain('Waiting on agent');
  });

  it('not started: "Not started yet" and Start now', () => {
    openPane({ type: 'agent', agentId: 'fresh-1' });
    const pane = render(AgentPane, { paneId: 'p1' }).container;
    expect(text(pane.querySelector('.ah'))).toContain('Not started yet');
    expect(text(pane.querySelector('.strip'))).toMatch(/Not started yet\s*\S*\s*live cap 4\/4 · 1st in line · 5 min/);
    expect(text(pane.querySelector('.strip'))).toContain('Start now');
    const ov = render(OverviewPane, { paneId: 'ov' }).container;
    const row = [...ov.querySelectorAll('tbody tr')].find((r) => r.textContent.includes('fresh-1'));
    expect(text(row)).toContain('Not started yet');
    expect(text(row)).toContain('Start now');
    // The governor column is the record's reason, no index arithmetic.
    expect(row.querySelector('.gov').textContent).toBe('live cap 4/4 · 1st in line');
  });

  it('badges come from the records, not the raw state', () => {
    const side = render(Sidebar).container;
    const badge = (id) =>
      [...side.querySelectorAll('.agent')].find((r) => r.textContent.includes(id)).querySelector('.badge');
    expect(badge('perm-1').getAttribute('title')).toBe('Waiting for you');
    expect(badge('run-2').getAttribute('title')).toBe('Working');
    expect(badge('queued-1').getAttribute('title')).toBe('Queued');
    // The feature's label is the run record's phase when nothing needs you.
    app.now.runs['run-1'].counts = { ...app.now.runs['run-1'].counts, needs_you: 0 };
    const side2 = render(Sidebar).container;
    expect(side2.querySelector('.feature .meta').textContent).toBe('Executing');
  });

  it('feature pane: members from the records, phase from the run record', async () => {
    openPane({ type: 'feature', featureId: 'f-co' });
    const { container } = render(FeaturePane, { paneId: 'p1' });
    await vi.waitFor(() => expect(container.querySelectorAll('.arow')).toHaveLength(4));
    const rows = [...container.querySelectorAll('.arow')].map((r) => [
      r.textContent.match(/perm-1|ask-1|run-1a|rev-1/)?.[0],
      r.querySelector('.badge').getAttribute('title'),
    ]);
    expect(rows).toEqual([
      ['perm-1', 'Waiting for you'],
      ['ask-1', 'Waiting for you'],
      ['run-1a', 'Working'],
      ['rev-1', 'Idle'],
    ]);
    expect(container.querySelector('.fh .pill').textContent).toBe('Executing');
  });

  it('inbox list badges use the decision-kind palette', () => {
    const inbox = render(InboxPane).container;
    const kinds = [...inbox.querySelectorAll('.item .kind')].map((k) => k.getAttribute('title'));
    expect(kinds).toEqual(['Permission', 'Question']);
    expect(/** @type {HTMLElement} */ (inbox.querySelector('.item .kind')).style.color).toBe(
      'var(--color-decision-permission)',
    );
  });
});
