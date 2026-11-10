// Design-notes §7 (spec §5.2): no `Kbd` inside or next to a control. Keys live
// in tooltips, the palette, Menu items (right-aligned), the Keyboard shortcuts
// sheet, the leader overlay and text placeholders. Two checks: the sources
// (every `<Kbd` / hand-written `.kbd` is in an allowed home) and the rendered
// chrome (no `.kbd` inside or beside a button or link, outside those homes).

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { cleanup, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { fixture, nowOf, permDecision, rec } from './fixture.js';

vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: () => Promise.resolve(null),
  listen: () => Promise.resolve(() => {}),
  errText: (e) => String(e?.message ?? e),
}));
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

const { default: TitleBar } = await import('../src/components/TitleBar.svelte');
const { default: Banner } = await import('../src/components/Banner.svelte');
const { default: StatusBar } = await import('../src/components/StatusBar.svelte');
const { default: Sidebar } = await import('../src/components/Sidebar.svelte');
const { default: Dock } = await import('../src/components/Dock.svelte');
const { default: Toasts } = await import('../src/components/Toasts.svelte');
const { default: Inspector } = await import('../src/components/Inspector.svelte');
const { default: AgentPane } = await import('../src/panes/AgentPane.svelte');

const SRC = join(import.meta.dirname, '../src');
const files = (dir) =>
  readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    return statSync(p).isDirectory() ? files(p) : p.endsWith('.svelte') ? [p] : [];
  });

// Where a `Kbd` may be written (§7): its own component, the palette, the Menu row, the
// sheet, the leader overlay, and the team canvas's connect / add rows (menu items).
const ALLOWED = {
  'components/Kbd.svelte': null,
  'components/Palette.svelte': null,
  'components/Menu.svelte': /<span class="key"><Kbd/,
  'components/ShortcutsSheet.svelte': /<dd><Kbd/,
  'components/StatusBar.svelte': /\{#if app\.leader\}<span class="leader"[\s\S]*<Kbd/,
  'panes/TeamCanvasPane.svelte': /role="menuitem"[^\n]*<span class="key"><Kbd/,
};

describe('sources', () => {
  it('`<Kbd` and hand-written `.kbd` only in the allowed homes', () => {
    const bad = [];
    for (const f of files(SRC)) {
      const rel = relative(SRC, f);
      const lines = readFileSync(f, 'utf8').split('\n');
      for (const [i, line] of lines.entries()) {
        if (!/<Kbd\b|class="kbd"/.test(line)) continue;
        const rule = ALLOWED[rel];
        if (!(rel in ALLOWED) || (rule && !rule.test(line))) bad.push(`${rel}:${i + 1}`);
      }
    }
    expect(bad).toEqual([]);
  });
});

/** Every `.kbd` that sits inside or beside a button or link, outside the allowed homes. */
function offenders(root) {
  const allowed = (k) => k.closest('[role^="menuitem"], .palette, .leader, .empty, .tooltip');
  return [...root.querySelectorAll('.kbd')].filter((k) => {
    if (allowed(k)) return false;
    if (k.closest('button, a, [role="button"]')) return true;
    const sibs = [...(k.parentElement?.children ?? [])];
    return sibs.some((s) => s !== k && s.matches('button, a, [role="button"]'));
  });
}

beforeEach(() => {
  cleanup();
  app.data = fixture();
  app.now = nowOf(app.data);
  app.panes = {};
  app.toasts = [];
  app.leader = false;
});

describe('rendered chrome', () => {
  it('title bar, status bar, sidebar, toasts, inspector: no pill', () => {
    app.toasts = [{ id: 1, text: 'Hid T-4', undo: true, item: {} }];
    for (const { container } of [render(TitleBar), render(StatusBar), render(Sidebar), render(Toasts)]) {
      expect(offenders(container)).toEqual([]);
      expect(container.querySelector('.kbd')).toBeNull();
    }
    app.inspector = true;
    expect(render(Inspector, { overlay: true }).container.querySelector('.kbd')).toBeNull();
    app.inspector = false;
  });

  it('every banner kind: no pill', () => {
    const d = app.data;
    d.core = { ok: false, error: 'port busy' };
    d.usage.five_hour.used_pct = 100;
    d.banners = [{ kind: 'not_on_ac' }];
    d.warnings = ['Check this'];
    const { container } = render(Banner);
    expect(container.querySelector('.kbd')).toBeNull();
  });

  it('the dock strip: no pill', () => {
    app.panes = { s1: { type: 'term', title: 'zsh' } };
    app.dock = { ...app.dock, open: true, tabs: ['s1'], active: 's1', view: 'shells' };
    const { container } = render(Dock);
    expect(offenders(container)).toEqual([]);
  });

  it('the leader overlay shows only while the leader is held', async () => {
    const { container } = render(StatusBar);
    expect(container.querySelector('.kbd')).toBeNull();
    app.leader = true;
    await tick();
    const k = container.querySelector('.kbd');
    expect(k.closest('.leader')).toBeTruthy();
    expect(offenders(container)).toEqual([]);
    app.leader = false;
  });

  it.each([
    ['permission', 'front-1', 'waiting_for_you', true],
    ['queued', 'rss-feed', 'queued', false],
    ['error', 'rss-feed', 'error', false],
    ['suspended', 'rss-feed', 'suspended', false],
  ])('agent strip %s: no pill', (kind, id, status, live) => {
    const a = app.data.agents.find((x) => x.id === id);
    a.live = live;
    app.now = nowOf(app.data, { decisions: kind === 'permission' ? [permDecision({ subject: { agent: id } })] : [] });
    app.now.agents[id] = rec(id, status, {
      live,
      queue: status === 'queued' ? { entry: 'q', position: 1, run_now: { week_pp: 1 } } : undefined,
    });
    app.panes = { p1: { type: 'agent', agentId: id } };
    const { container } = render(AgentPane, { paneId: 'p1' });
    expect(container.querySelector('.strip')).toBeTruthy();
    expect(container.querySelector('.kbd')).toBeNull();
  });
});
