// Mission control (P4.2 spec §1, §3.2–3.5, §5): the exit criterion from one
// `now` fixture, the keys, sessions over the board in watch mode and the
// §9.7 keyboard contract around it, the project filter, the Overview's
// layout restored as Mission control, the Inspector for a run and a
// decision, and the render time for 50 agents and 10 runs.

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { flushSync, tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const calls = [];
const store = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === 'settings_get') return Promise.resolve(store[args.key] ?? null);
    if (cmd === 'settings_set') {
      store[args.key] = args.value;
      return Promise.resolve(null);
    }
    return Promise.resolve(null);
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => String(e?.message ?? e),
}));

// xterm.js is not under test: the pane records what reaches it.
const terms = [];
vi.mock('../src/lib/term.js', () => ({
  TermPane: class {
    alive = false;
    focused = 0;
    written = [];
    constructor(source, kitty, hooks) {
      this.source = source;
      this.hooks = hooks;
      terms.push(this);
    }
    mount() {}
    unmount() {}
    focus() {
      this.focused++;
    }
    write(d) {
      this.written.push(d);
    }
    scheduleFit() {}
    start() {
      return Promise.resolve();
    }
  },
}));

const A = await import('../src/lib/app.svelte.js');
const { app } = A;
const L = await import('../src/lib/layout.js');
const { classify } = await import('../src/lib/keyboard.js');
const { default: MissionPane } = await import('../src/panes/MissionPane.svelte');
const { default: AgentPane } = await import('../src/panes/AgentPane.svelte');
const { default: Inspector } = await import('../src/components/Inspector.svelte');
const { MIN, missionData, missionNow, counts } = await import('./mission-fixture.js');
const { rec } = await import('./fixture.js');

const text = (el) => el.textContent.replace(/\s+/g, ' ');
const row = (c, key) => c.querySelector(`[data-key="${key}"]`);
const key = (el, k, more = {}) => fireEvent.keyDown(el, { key: k, ...more });
const settle = async () => {
  await tick();
  await Promise.resolve();
  await tick();
};

function home() {
  app.panes = { mc: { type: 'mission', title: 'Mission control' } };
  app.layout = L.leaf('mc');
  app.focused = 'mc';
}

afterEach(cleanup);

beforeEach(() => {
  const now = Date.now();
  app.data = /** @type {any} */ (missionData(now));
  app.now = missionNow(now);
  app.selection = null;
  app.mission = { projects: null };
  app.missionFilterOpen = false;
  app.overlay = null;
  app.toasts = [];
  app.zoomed = null;
  home();
  calls.length = 0;
  terms.length = 0;
  for (const k of Object.keys(store)) delete store[k];
});

describe('exit criterion (spec §1): one screen answers what runs, waits on what, is queued why, was done today', () => {
  it('renders every row from one `now`, calls no view command and sets no timer but the 30 s clock', async () => {
    const intervals = vi.spyOn(globalThis, 'setInterval');
    const timeouts = vi.spyOn(globalThis, 'setTimeout');
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();

    // Running: every running row with its activity.
    expect(text(row(c, 'agent:fix-feed'))).toMatch(/blog › fix-feed\s*.*Working.*Edit src\/feed\.ts/);
    const run = text(row(c, 'run:run-12'));
    expect(run).toContain('lead-1');
    expect(run).toContain('Edit checkout.ts');
    // Waiting on what: every wait with its reason and since.
    expect(run).toContain('blocker: back-1 · permission: Bash pnpm add zod · 2 min');
    expect(run).toContain('front-1 → back-1 · 12 min');
    expect(run).toContain('executing · 2 h');
    expect(run).toContain('3/7 tasks done');
    // Queued why: reason, ETA and the Run now cost, from the record's `queue` (D7).
    const infra = text(row(c, 'queue:infra-1'));
    expect(infra).toContain('Queued');
    expect(infra).toContain('live cap 2/2 · next · ≈14:20');
    expect(infra).toContain('Run now ≈ +2.1 pp week');
    const ui = text(row(c, 'queue:ui-1'));
    expect(ui).toContain('ui-1 · shop-api › checkout');
    expect(ui).toContain('Not started yet');
    expect(ui).toContain('live cap 2/2 · ≈14:40');
    expect(ui).toContain('Start now ≈ +1.0 pp week');
    // Done today: every outcome, seen ones included.
    for (const t of [
      'Merged back-1 into integration',
      'T-3 done · checkout API',
      'PR opened · blog › fix-feed #12',
      'checkout: planning → executing',
    ])
      expect(text(c)).toContain(t);
    // Lane headers count rows (D11); the footer has the core's agent counts.
    expect([...c.querySelectorAll('.lh')].map((h) => text(h).trim())).toEqual([
      '⌄ ● Needs you · 1 run',
      '⌄ ▲ Running · 1 session',
      '⌄ □ Queued · 2 agents',
      '⌄ ■ Done today · 4 outcomes',
    ]);
    expect(text(c.querySelector('.foot'))).toContain('1 needs you · 2 running · 1 waiting · 2 queued · live cap 3/2');

    // A render of `now`: nothing fetched, no timer but now30's.
    expect(calls).toEqual([]);
    expect(intervals.mock.calls.every(([, ms]) => ms === 30000)).toBe(true);
    expect(timeouts).not.toHaveBeenCalled();
    intervals.mockRestore();
    timeouts.mockRestore();
  });

  it('a pushed change moves the run to Running without a refresh (acceptance §6.6)', async () => {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    app.now.runs['run-12'] = { ...app.now.runs['run-12'], level: 'running', blocker: undefined };
    flushSync();
    expect(text(c.querySelector('section[aria-label="Running"]'))).toContain('checkout');
    expect(text(c.querySelector('section[aria-label="Needs you"]'))).toContain('Nothing needs you');
  });
});

describe('keys (spec §3.4)', () => {
  it('j/k move across lanes; h/l move between chips; the Inspector follows (D10)', async () => {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    const mc = c.querySelector('.mc');
    expect(app.selection).toEqual({ kind: 'run', id: 'run-12' });
    expect(row(c, 'run:run-12').getAttribute('aria-selected')).toBe('true');
    expect(text(c.querySelector('.toolbar'))).toContain('open back-1');
    key(mc, 'j');
    await tick();
    expect(app.selection).toEqual({ kind: 'agent', id: 'fix-feed' });
    key(mc, 'j');
    key(mc, 'j');
    await tick();
    expect(app.selection).toEqual({ kind: 'agent', id: 'ui-1' });
    key(mc, 'k');
    key(mc, 'k');
    key(mc, 'k');
    await tick();
    expect(app.selection).toEqual({ kind: 'run', id: 'run-12' });
    // The default target is the blocker (back-1); l goes to front-1, h back to the run.
    key(mc, 'l');
    await tick();
    expect(app.selection).toEqual({ kind: 'agent', id: 'front-1' });
    expect(c.querySelector('.mchip.selected').textContent).toContain('front-1');
    key(mc, 'h');
    await tick();
    expect(app.selection).toEqual({ kind: 'run', id: 'run-12' });
    expect(c.querySelector('.mchip.selected').textContent).toContain('back-1');
  });

  it('space shows the row details: ctx, usage, Governor, memory per agent', async () => {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    key(c.querySelector('.mc'), ' ');
    await tick();
    const rows = [...c.querySelectorAll('.details tbody tr')].map((r) =>
      [...r.querySelectorAll('td')].map((x) => x.textContent),
    );
    const cells = (id) => rows.find((r) => r[0] === id);
    expect(cells('lead-1')).toEqual([
      'lead-1',
      'Edit checkout.ts',
      '1 h 30 min',
      '62 % ctx',
      '—',
      'Green · live',
      '412 MB',
    ]);
    expect(cells('ui-1')).toEqual(['ui-1', 'not started', '—', '—', '—', 'Not started · cap', '—']);
  });

  it('⇧K asks before a kill; a caps-lock K moves up', async () => {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    const mc = c.querySelector('.mc');
    key(mc, 'j');
    key(mc, 'K');
    await tick();
    expect(app.selection).toEqual({ kind: 'run', id: 'run-12' });
    expect(app.overlay).toBeNull();
    key(mc, 'K', { shiftKey: true });
    expect(app.overlay).toMatchObject({ kind: 'confirm', text: 'Kill back-1?' });
  });

  it('n on a queue row confirms with the cost in both windows (FR-809)', async () => {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    const mc = c.querySelector('.mc');
    for (let i = 0; i < 2; i++) key(mc, 'j');
    key(mc, 'n');
    expect(app.overlay).toMatchObject({
      kind: 'confirm',
      text: 'Run infra-1 now?',
      yes: 'Run now ≈ +2.1 pp week',
      no: 'Keep queued',
    });
    expect(app.overlay.detail).toContain('≈ +6 pp of the 5h window');
    app.overlay.onYes();
    await settle();
    expect(calls).toContainEqual({ cmd: 'governor_act', args: { action: 'run_now', arg: 314 } });
  });

  it('d opens the Inbox on the row decision; o on an outcome with a link opens it', async () => {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    key(c.querySelector('.mc'), 'd');
    await tick();
    expect(app.inboxFocus).toBe('inbox-1');
    expect(Object.values(app.panes).some((s) => s.type === 'inbox')).toBe(true);
  });
});

describe('sessions over the board (spec §3.3, D5)', () => {
  async function openBack1() {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    key(c.querySelector('.mc'), 'Enter');
    const id = A.findPane((s) => s.type === 'agent' && s.agentId === 'back-1');
    return { c, id };
  }

  it('⏎ opens the target as a tab in Mission control’s group, in watch mode', async () => {
    const { id } = await openBack1();
    expect(app.panes[id]).toMatchObject({ type: 'agent', agentId: 'back-1', watch: true, over: 'mc' });
    expect(L.leafOf(app.layout, 'mc').tabs).toEqual(['mc', id]);
    expect(app.focused).toBe(id);
    const pane = render(AgentPane, { paneId: id }).container;
    await settle();
    expect(text(pane.querySelector('.session.watch'))).toMatch(/Watching back-1 · ⏎ take over · esc back/i);
    // The terminal never got the keyboard.
    expect(terms.at(-1).focused).toBe(0);
  });

  it('Esc returns to Mission control and the tab stays; ⏎ takes over; leader o returns', async () => {
    const { id } = await openBack1();
    const pane = render(AgentPane, { paneId: id }).container;
    await settle();
    key(pane.querySelector('.session.watch'), 'Escape');
    expect(app.focused).toBe('mc');
    expect(L.leafOf(app.layout, 'mc').tabs).toContain(id);
    // ⏎ again on the row focuses the same tab, still watching.
    A.openSession('back-1');
    await settle();
    expect(app.focused).toBe(id);
    key(pane.querySelector('.session.watch'), 'Enter');
    await settle();
    expect(app.panes[id].watch).toBe(false);
    expect(terms.at(-1).focused).toBeGreaterThan(0);
    expect(text(pane.querySelector('.session'))).toContain('Typing in back-1');
    A.run('mission');
    expect(app.focused).toBe('mc');
  });

  it('a click in the terminal takes over', async () => {
    const { id } = await openBack1();
    render(AgentPane, { paneId: id });
    await settle();
    terms.at(-1).hooks.onFocus();
    expect(app.panes[id].watch).toBe(false);
  });

  it('a watched tab restores taken over (watch is not saved)', async () => {
    vi.useFakeTimers();
    try {
      const { id } = await openBack1();
      vi.advanceTimersByTime(500);
      expect(store.layout.panes[id].watch).toBeUndefined();
      expect(store.layout.panes[id].over).toBe('mc');
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('keyboard contract §9.7 with watch mode', () => {
  /** A keydown target inside a pane's xterm. */
  const inXterm = () => {
    const x = document.createElement('div');
    x.className = 'xterm';
    const t = document.createElement('textarea');
    x.appendChild(t);
    return t;
  };
  const ev = (k, more = {}) => new KeyboardEvent('keydown', { key: k, ...more });
  const hooksOf = (paneId) => {
    A.mountTerm(paneId, document.createElement('div'), {});
    return A.terms.get(paneId).hooks;
  };
  const NON_CMD = ['a', 'Escape', 'Enter', 'j', 'k', 'o', 'ArrowUp', 'Tab'];

  it('a shell pane and a taken-over agent pane get every non-⌘ key, Esc included', () => {
    app.panes.sh = { type: 'shell', termId: 't1', title: 'zsh' };
    app.panes.ag = { type: 'agent', agentId: 'back-1', title: 'back-1', over: 'mc', watch: false };
    for (const id of ['sh', 'ag']) {
      app.focused = id;
      const hooks = hooksOf(id);
      const ctx = A.keyContext(inXterm());
      expect(ctx.terminal).toBe(true);
      for (const k of NON_CMD) {
        // The app lets the key through to xterm.js, and xterm.js takes it.
        expect(['pass', 'pty']).toContain(classify(ev(k), ctx).kind);
        expect(hooks.onKey(ev(k))).toBe(true);
      }
      expect(classify(ev('Enter', { shiftKey: true }), ctx).kind).toBe(id === 'ag' ? 'pty' : 'pass');
      // ⌘ never reaches the PTY.
      expect(hooks.onKey(ev('k', { metaKey: true }))).toBe(false);
    }
  });

  it('in watch mode no key reaches the PTY', () => {
    app.panes.ag = { type: 'agent', agentId: 'back-1', title: 'back-1', over: 'mc', watch: true };
    app.focused = 'ag';
    const hooks = hooksOf('ag');
    const ctx = A.keyContext(inXterm());
    expect(ctx.terminal).toBe(false);
    for (const k of NON_CMD) expect(hooks.onKey(ev(k))).toBe(false);
    expect(classify(ev('Enter', { shiftKey: true }), ctx).kind).toBe('pass');
    expect(A.terms.get('ag').written).toEqual([]);
    // The leader still works: ⌘; then o opens Mission control.
    expect(classify(ev(';', { metaKey: true, code: 'Semicolon' }), ctx).kind).toBe('leader');
    expect(classify(ev('o', { code: 'KeyO' }), { ...ctx, leader: true })).toEqual({
      kind: 'command',
      command: 'mission',
    });
  });
});

describe('project filter (D8)', () => {
  it('f opens it; space toggles, ⏎ applies; rows follow; remembered in settings `mission`', async () => {
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    key(c.querySelector('.mc'), 'f');
    await tick();
    const pop = c.querySelector('.pop');
    expect(text(pop)).toContain('shop-api 1 needs you · 1 running · 1 waiting · 1 queued');
    key(pop, 'j');
    key(pop, ' ');
    key(pop, 'Enter');
    await settle();
    expect(store.mission).toEqual({ projects: ['blog', 'infra'] });
    expect(row(c, 'run:run-12')).toBeNull();
    expect(row(c, 'queue:infra-1')).not.toBeNull();
    expect(text(c.querySelector('.fbtn'))).toContain('2 projects of 3');
  });

  it('a filter that hides every row says so', async () => {
    app.now.agents = { 'fix-feed': app.now.agents['fix-feed'] };
    app.now.runs = {};
    app.now.outcomes = [];
    app.now.decisions = {};
    app.mission = { projects: ['infra'] };
    const { container: c } = render(MissionPane, { paneId: 'mc' });
    await settle();
    expect(text(c)).toContain('Nothing in the infra selected');
    expect(text(c.querySelector('.foot'))).toContain('Hiding shop-api, blog');
  });

  it('restored at start, with the layout', async () => {
    store.mission = { projects: ['blog'] };
    app.layout = null;
    app.panes = {};
    await A.restoreLayout();
    expect(app.mission.projects).toEqual(['blog']);
    expect(Object.values(app.panes).map((s) => s.type)).toEqual(['mission']);
  });
});

describe('layout (D9)', () => {
  it('a saved Overview pane restores as Mission control; leader o focuses it', async () => {
    store.layout = {
      layout: { kind: 'leaf', id: 'l1', tabs: ['p40', 'p41'], active: 'p41' },
      panes: { p40: { type: 'overview', title: 'All projects' }, p41: { type: 'inbox', title: 'Inbox' } },
      focused: 'p41',
    };
    await A.restoreLayout();
    expect(app.panes.p40).toMatchObject({ type: 'mission', title: 'Mission control' });
    A.run('mission');
    expect(app.focused).toBe('p40');
  });
});

describe('Inspector follows the selection (spec §3.5)', () => {
  it('a run: stepper, blocker, members, tasks, started, actions', () => {
    app.selection = { kind: 'run', id: 'run-12' };
    const t = text(render(Inspector).container);
    expect(t).toContain('Inspector · run');
    expect(t).toContain('checkout shop-api');
    expect(t).toMatch(/executing · since \d\d:\d\d \(2 h\)/);
    expect(t).toContain('back-1 · permission: Bash pnpm add zod');
    expect(t).toContain('front-1 → back-1 · 12 min');
    expect(t).toContain('ui-1 not started');
    expect(t).toContain('7 tasks · 3 done · 2 in progress · 1 in review · 1 blocked');
    expect(t).toContain('3 h 18 min ago');
    expect(t).toContain('Open feature');
  });

  it('a decision: kind, title, subject, marker, Open in Inbox; never Allow or Deny', () => {
    app.selection = { kind: 'decision', id: 'inbox-1' };
    const c = render(Inspector).container;
    const t = text(c);
    expect(t).toContain('Inspector · decision');
    expect(t).toContain('Permission');
    expect(t).toContain('Bash · pnpm add zod');
    expect(t).toContain('back-1 · shop-api › checkout');
    expect(t).toContain('Open in Inbox');
    expect([...c.querySelectorAll('button')].map((b) => b.textContent)).not.toContain('Allow once');
  });

  it('the focused agent pane still wins', () => {
    app.selection = { kind: 'run', id: 'run-12' };
    app.panes.ag = { type: 'agent', agentId: 'front-1', title: 'front-1' };
    app.focused = 'ag';
    expect(text(render(Inspector).container)).toContain('Inspector · agent');
  });
});

describe('performance (NFR-1b): 50 agents + 10 runs', () => {
  it('renders a `now` event in < 16 ms', async () => {
    const now = Date.now();
    const agents = {};
    const runs = {};
    const statuses = ['working', 'waiting_for_you', 'queued', 'idle', 'error'];
    for (let r = 0; r < 10; r++) {
      const members = [0, 1, 2].map((i) => `r${r}-${i}`);
      for (const m of members)
        agents[m] = rec(m, statuses[(r + m.length) % 5], {
          project: `p${r % 3}`,
          run: `run-${r}`,
          activity: 'Edit x.ts',
        });
      runs[`run-${r}`] = {
        run: `run-${r}`,
        feature: `f${r}`,
        project: `p${r % 3}`,
        phase: 'executing',
        phase_since_ms: now - r * MIN,
        level: ['needs_you', 'running', 'waiting', 'queued'][r % 4],
        members,
        lead: members[0],
        tasks: { total: 5, done: r % 5 },
        counts: counts(),
      };
    }
    for (let i = 0; i < 20; i++)
      agents[`s${i}`] = rec(`s${i}`, statuses[i % 5], {
        project: `p${i % 3}`,
        activity: 'Bash cargo test',
        reason: 'live cap 2/2 · next',
      });
    app.now = {
      rev: 1,
      agents,
      runs,
      decisions: {},
      outcomes: [],
      counts: { global: counts({ live_cap: 4 }), projects: {} },
    };
    render(MissionPane, { paneId: 'mc' });
    await settle();
    const times = [];
    for (let i = 0; i < 15; i++) {
      const t0 = performance.now();
      app.now.agents.s0 = { ...app.now.agents.s0, reason: `r${i}`, activity: `Edit ${i}.ts` };
      app.now.runs['run-1'] = { ...app.now.runs['run-1'], phase_since_ms: now - i * MIN };
      flushSync();
      times.push(performance.now() - t0);
    }
    times.sort((a, b) => a - b);
    expect(times[Math.floor(times.length / 2)]).toBeLessThan(16);
  });
});
