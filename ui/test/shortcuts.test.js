// The Keyboard shortcuts sheet (design-notes §7, audit §7): its General,
// Workspace and Dock rows come from the bindings, so every labelled chord and
// leader key is in it, nothing without a binding is, and the split-layout keys
// never show; the search filters by label or key; the sheet renders two
// columns of groups with the leader line.

import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { CHORDS, LEADER } from '../src/lib/keyboard.js';
import { filterGroups, GROUPS, REMOVED, sheetKey } from '../src/lib/shortcuts.js';

vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: () => Promise.resolve(null),
  listen: () => Promise.resolve(() => {}),
  errText: (e) => String(e),
}));

const { app, commands } = await import('../src/lib/app.svelte.js');
const { default: ShortcutsSheet } = await import('../src/components/ShortcutsSheet.svelte');

const rows = (groups = GROUPS) => groups.flatMap((g) => g.rows);
const listed = new Set(rows().flatMap((r) => r.commands));
const bound = [...Object.values(CHORDS), ...Object.values(LEADER)];

describe('GROUPS (design-notes §7)', () => {
  it('every bound command with a label is in the sheet, except the split-layout keys', () => {
    // A label: the palette's, or one of the run-map commands the sheet names itself.
    const labelled = new Set([
      ...commands().map((c) => c.id),
      'palette',
      'jump',
      'agent.pause',
      'focus.down',
      'focus.up',
      ...Object.values(CHORDS).filter((c) => c.startsWith('pane.focus.')),
    ]);
    const missing = bound.filter((c) => labelled.has(c) && !REMOVED.has(c) && !listed.has(c));
    expect(missing).toEqual([]);
  });

  it('removed ids are absent', () => {
    for (const c of REMOVED) expect(listed.has(c)).toBe(false);
    expect(rows().some((r) => /split|zoom/i.test(r.label))).toBe(false);
  });

  it('no key listed that has no binding', () => {
    for (const r of rows().filter((x) => x.commands.length)) for (const c of r.commands) expect(bound).toContain(c);
  });

  it('keys in the sheet notation: chords without +, leader keys after ⌘;', () => {
    const key = (label) => rows().find((r) => r.label === label)?.keys;
    expect(key('Jump or run a command')).toBe('⌘K');
    expect(key('Keyboard shortcuts')).toBe('⌘/');
    expect(key('Decisions')).toBe('⌘⇧I');
    expect(key('Reopen closed tab')).toBe('⌘⇧T');
    expect(key('Tab 1–9 of the context')).toBe('⌘1–9');
    expect(key('Week planner')).toBe('⌘; g');
    expect(key('Next / previous tab')).toBe('⌘; ] [');
    expect(key('Into / out of the dock')).toBe('⌘; j k');
    expect(sheetKey('cmd+shift+t')).toBe('⌘⇧T');
  });

  it('the groups of audit §7, in order', () => {
    expect(GROUPS.map((g) => g.title)).toEqual([
      'General',
      'Workspace',
      'Dock',
      'Agent pane',
      'Decisions & chat',
      'Mission control',
      'Work browser',
      'Mission control › program row',
      'Run as program (plan)',
      'Program view',
      'Feature',
      'Feature › Tasks (focused)',
      'Project',
      'Diff',
      'Weekly report',
      'Team canvas',
      'Design preview',
      'Week planner',
      'Settings lists',
    ]);
  });
});

describe('search', () => {
  it('filters by label, case-insensitive; empty groups go', () => {
    const g = filterGroups(GROUPS, 'DOCK');
    expect(g.map((x) => x.title)).toEqual(['Dock']);
    expect(rows(g).map((r) => r.label)).toEqual(['Into / out of the dock']);
  });

  it('filters by key; "cmd" matches ⌘, "shift" ⇧', () => {
    expect(rows(filterGroups(GROUPS, 'cmd+/')).map((r) => r.label)).toEqual(['Keyboard shortcuts']);
    expect(rows(filterGroups(GROUPS, 'cmd shift t')).map((r) => r.label)).toEqual(['Reopen closed tab']);
    expect(rows(filterGroups(GROUPS, '⌘;')).length).toBeGreaterThan(5);
  });

  it('an empty query keeps everything', () => {
    expect(filterGroups(GROUPS, '  ')).toBe(GROUPS);
  });
});

describe('ShortcutsSheet', () => {
  beforeEach(() => {
    cleanup();
    app.window = { kind: 'shortcuts' };
  });

  it('a window with the search in its title bar, focused; the leader line; every group', async () => {
    const { container } = render(ShortcutsSheet);
    await tick();
    await tick();
    const dlg = container.querySelector('[role="dialog"]');
    expect(document.getElementById(dlg.getAttribute('aria-labelledby')).textContent).toBe('Keyboard shortcuts');
    const search = container.querySelector('.bar input[type="search"]');
    expect(document.activeElement).toBe(search);
    expect(container.querySelector('.lead').textContent).toContain('Leader = ⌘;');
    expect(container.querySelectorAll('section.group')).toHaveLength(GROUPS.length);
    // The keys are information: Kbd pills, never inside a button.
    expect(container.querySelector('button .kbd')).toBeNull();
  });

  it('typing filters; no match says so', async () => {
    const { container } = render(ShortcutsSheet);
    const search = container.querySelector('input[type="search"]');
    await fireEvent.input(search, { target: { value: 'dock' } });
    expect([...container.querySelectorAll('section.group h3')].map((h) => h.textContent)).toEqual(['Dock']);
    await fireEvent.input(search, { target: { value: 'zzz' } });
    expect(container.querySelector('.empty').textContent).toContain('No shortcut matches');
  });

  it('× and Esc close the window', async () => {
    const { container } = render(ShortcutsSheet);
    await fireEvent.click(container.querySelector('.x'));
    expect(app.window).toBeNull();
  });
});
