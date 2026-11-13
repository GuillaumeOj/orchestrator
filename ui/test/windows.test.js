// Floating windows (design-notes §3, §12.1): Settings, Project settings, Week
// planner and Keyboard shortcuts open in `app.window`, never as panes; one at a
// time; ⌘, / ⌘/ toggle; ⌘W closes the window first; the opener gets the focus
// back; saved window panes are dropped at restore; the Discard-changes guard.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as L from '../src/lib/layout.js';
import { fixture } from './fixture.js';

const calls = [];
const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    const r = replies[cmd];
    return r instanceof Error ? Promise.reject(r) : Promise.resolve(typeof r === 'function' ? r(args) : r);
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
}));

const A = await import('../src/lib/app.svelte.js');
const {
  app,
  closeWindow,
  guardWindow,
  openPlanner,
  openProjectSettings,
  openSettings,
  openShortcuts,
  reopenClosed,
  restoreLayout,
  routeOpen,
  run,
} = A;
const { default: ProjectSettingsWindow } = await import('../src/components/ProjectSettingsWindow.svelte');
const { SECTIONS: SETTINGS_SECTIONS } = await import('../src/components/SettingsWindow.svelte');

let unguard = () => {};
beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = { ...fixture(), setup: { done: true } };
  app.panes = { p1: { type: 'mission', title: 'Mission control' } };
  app.layout = L.leaf('p1');
  app.focused = 'p1';
  app.overlay = null;
  app.window = null;
  app.windowReturn = null;
  app.settingsSection = null;
  app.toasts = [];
});
afterEach(() => {
  unguard();
  unguard = () => {};
  app.window = null;
});

const paneTypes = () => Object.values(app.panes).map((p) => p.type);

describe('openers (design-notes §3.1)', () => {
  it('each opener sets app.window and adds no pane', () => {
    openSettings('mcp', 'figma');
    expect(app.window).toEqual({ kind: 'settings', section: 'mcp', server: 'figma' });
    openProjectSettings('shop-api', 'files');
    expect(app.window).toEqual({ kind: 'project-settings', projectId: 'shop-api', section: 'files', server: null });
    openPlanner();
    expect(app.window).toEqual({ kind: 'planner' });
    openShortcuts();
    expect(app.window).toEqual({ kind: 'shortcuts' });
    expect(paneTypes()).toEqual(['mission']);
  });

  it('Settings opens on Integrations, then on its last section while the app runs', () => {
    openSettings();
    expect(app.window.section).toBe('integrations');
    openSettings('builds');
    closeWindow();
    openSettings();
    expect(app.window.section).toBe('builds');
  });

  it('an open Settings window updates in place (section and server)', () => {
    openSettings('integrations');
    openSettings('mcp', 's');
    expect(app.window).toEqual({ kind: 'settings', section: 'mcp', server: 's' });
  });

  it('a project with section mcp is Project settings › MCP overrides with the server; other sections ignore it', () => {
    openSettings('mcp', 'figma', 'shop-api');
    expect(app.window).toEqual({ kind: 'project-settings', projectId: 'shop-api', section: 'mcp', server: 'figma' });
    openSettings('integrations', 'github', 'shop-api');
    expect(app.window).toEqual({ kind: 'settings', section: 'integrations', server: 'github' });
  });

  it('Project settings default to Tracker; one window for every project', () => {
    openProjectSettings('shop-api');
    expect(app.window.section).toBe('tracker');
    openProjectSettings('blog', 'notion');
    expect(app.window).toMatchObject({ kind: 'project-settings', projectId: 'blog', section: 'notion' });
  });

  it('no project: a toast, no window', () => {
    openProjectSettings(null);
    expect(app.window).toBeNull();
    expect(app.toasts.at(-1).text).toBe('No project in focus');
  });

  it('a decision card routes its MCP link by project (routeOpen)', () => {
    routeOpen({ open: 'settings', server: 'penpot' }, 'shop-api');
    expect(app.window).toMatchObject({
      kind: 'project-settings',
      projectId: 'shop-api',
      section: 'mcp',
      server: 'penpot',
    });
    routeOpen({ open: 'settings', server: 'penpot' });
    expect(app.window).toMatchObject({ kind: 'settings', section: 'mcp', server: 'penpot' });
  });

  it('palette entries open windows', () => {
    const cmd = (id) => A.commands().find((c) => c.id === id);
    cmd('planner').run();
    expect(app.window.kind).toBe('planner');
    cmd('shortcuts').run();
    expect(app.window.kind).toBe('shortcuts');
    expect(cmd('shortcuts').hint).toBe('⌘+/');
    cmd('project.settings.shop-api').run();
    expect(app.window).toMatchObject({ kind: 'project-settings', projectId: 'shop-api' });
  });

  it('Settings has 9 sections, Keyboard is gone', () => {
    expect(SETTINGS_SECTIONS.map((s) => s.id)).toEqual([
      'claude',
      'integrations',
      'overwatch',
      'permissions',
      'mcp',
      'terminal',
      'pacing',
      'memory',
      'builds',
    ]);
  });

  it('no source opens these screens as panes', () => {
    const files = [];
    const walk = (d) => {
      for (const f of readdirSync(d)) {
        const p = join(d, f);
        if (statSync(p).isDirectory()) walk(p);
        else if (/\.(js|svelte)$/.test(f)) files.push(p);
      }
    };
    walk(join(__dirname, '../src'));
    const bad = files.filter((f) =>
      /openSingleton\('planner'\)|type: '(settings|project-settings|planner)'/.test(readFileSync(f, 'utf8')),
    );
    expect(bad).toEqual([]);
  });
});

describe('keys (design-notes §3.3)', () => {
  it('⌘, toggles Settings; on another window it replaces it', () => {
    run('settings');
    expect(app.window.kind).toBe('settings');
    run('settings');
    expect(app.window).toBeNull();
    openPlanner();
    run('settings');
    expect(app.window.kind).toBe('settings');
  });

  it('⌘/ toggles the shortcuts sheet', () => {
    run('shortcuts');
    expect(app.window.kind).toBe('shortcuts');
    run('shortcuts');
    expect(app.window).toBeNull();
  });

  it('⌘W closes the window, not the tab', () => {
    openPlanner();
    run('pane.close');
    expect(app.window).toBeNull();
    expect(paneTypes()).toEqual(['mission']);
  });
});

describe('focus (design-notes §3.3)', () => {
  it('replacing keeps the first opener; closing gives it the focus back', async () => {
    const btn = document.createElement('button');
    document.body.append(btn);
    btn.focus();
    openSettings();
    expect(app.windowReturn).toBe(btn);
    const other = document.createElement('button');
    document.body.append(other);
    other.focus();
    openPlanner();
    expect(app.windowReturn).toBe(btn);
    closeWindow();
    expect(app.windowReturn).toBeNull();
    await tick();
    await tick();
    expect(document.activeElement).toBe(btn);
    btn.remove();
    other.remove();
  });

  it('an opener that is gone: the focused pane gets the focus back', async () => {
    const btn = document.createElement('button');
    document.body.append(btn);
    btn.focus();
    openShortcuts();
    btn.remove();
    closeWindow();
    await tick();
    await tick();
    expect(app.focused).toBe('p1');
    expect(document.activeElement).not.toBe(btn);
  });

  it('§9 bug 2: "Past weekly reports" opens the report on its list, "Weekly report" on this week', () => {
    A.commands()
      .find((c) => c.id === 'report.list')
      .run();
    const id = Object.keys(app.panes).find((k) => app.panes[k].type === 'report');
    expect(app.panes[id]).toMatchObject({ reportId: 'current', list: true });
    A.commands()
      .find((c) => c.id === 'report.current')
      .run();
    expect(app.panes[id]).toMatchObject({ reportId: 'current', list: false });
  });

  it('a pane that opens (the palette above the window) closes the window', () => {
    openSettings();
    A.openReport('current');
    expect(app.window).toBeNull();
    expect(paneTypes()).toContain('report');
  });
});

describe('restore and reopen', () => {
  it('saved settings / project-settings / planner panes are dropped; no window opens at launch', async () => {
    let tree = L.leaf('p1');
    for (const id of ['p2', 'p3', 'p4']) tree = L.addTab(tree, id);
    replies.settings_get = ({ key }) =>
      key === 'layout'
        ? {
            layout: tree,
            panes: {
              p1: { type: 'settings', section: 'mcp', title: 'Settings' },
              p2: { type: 'project-settings', projectId: 'shop-api', title: 'shop-api settings' },
              p3: { type: 'planner', title: 'Week planner' },
              p4: { type: 'mission', title: 'Mission control' },
            },
            focused: 'p1',
          }
        : null;
    replies.mission_filter = null;
    app.panes = {};
    app.layout = null;
    await restoreLayout();
    expect(paneTypes()).toEqual(['mission']);
    expect(app.window).toBeNull();
  });

  it('a closed window is not offered by Reopen closed tab', async () => {
    openSettings();
    closeWindow();
    openPlanner();
    run('pane.close');
    await reopenClosed();
    expect(app.window).toBeNull();
    expect(paneTypes()).toEqual(['mission']);
    expect(app.toasts.at(-1).text).toBe('No closed tab to reopen');
  });
});

describe('Discard-changes guard (design-notes §3.3)', () => {
  it('closing, ⌘W or replacing a dirty window asks; Keep editing keeps it; Discard goes on', () => {
    openProjectSettings('shop-api', 'notion');
    unguard = guardWindow(() => true);
    closeWindow();
    expect(app.window).not.toBeNull();
    expect(app.overlay).toMatchObject({
      kind: 'confirm',
      text: 'Discard changes?',
      yes: 'Discard',
      no: 'Keep editing',
    });
    app.overlay = null; // Keep editing
    run('pane.close');
    expect(app.overlay?.text).toBe('Discard changes?');
    app.overlay = null;
    openSettings('integrations');
    expect(app.window.kind).toBe('project-settings');
    app.overlay.onYes();
    expect(app.window).toMatchObject({ kind: 'settings', section: 'integrations' });
  });

  it('switching section asks; a clean window never asks; a server change in place never asks', () => {
    openProjectSettings('shop-api', 'tracker');
    unguard = guardWindow(() => true);
    A.windowSection('notion');
    expect(app.window.section).toBe('tracker');
    app.overlay.onYes();
    expect(app.window.section).toBe('notion');
    app.overlay = null;
    unguard();
    unguard = guardWindow(() => false);
    A.windowSection('files');
    expect(app.window.section).toBe('files');
    expect(app.overlay).toBeNull();
  });

  it('Project settings: a backdrop click on unsaved Notion flashes the footer; Esc and × ask', async () => {
    replies.project_settings = { settings: {}, backend: { type: 'local' }, track_orchestrator_dir: true };
    replies.board_view = null;
    replies.integrations_view = { items: [] };
    openProjectSettings('shop-api', 'notion');
    const { container } = render(ProjectSettingsWindow);
    const input = await waitFor(() => {
      const el = container.querySelector('input[placeholder="Notion page URL or id"]');
      if (!el) throw new Error('not loaded');
      return el;
    });
    await fireEvent.input(input, { target: { value: 'https://notion.so/x' } });
    await fireEvent.click(container.querySelector('.backdrop'));
    expect(app.window).not.toBeNull();
    expect(app.overlay).toBeNull();
    expect(container.querySelector('.foot').classList.contains('flash')).toBe(true);
    await fireEvent.click(container.querySelector('.x'));
    expect(app.overlay?.text).toBe('Discard changes?');
    app.overlay.onYes();
    expect(app.window).toBeNull();
  });

  it('Project settings: Notion saves only its page; Revert puts the saved page back (§9 bug 1)', async () => {
    const backend = { type: 'github' };
    replies.project_settings = { settings: { notion_parent_page: 'old' }, backend, track_orchestrator_dir: true };
    replies.project_settings_set = (a) => ({ ...a.form });
    replies.board_view = null;
    replies.integrations_view = { items: [] };
    openProjectSettings('shop-api', 'notion');
    const { container, getByText } = render(ProjectSettingsWindow);
    const input = await waitFor(() => {
      const el = /** @type {HTMLInputElement | null} */ (
        container.querySelector('input[placeholder="Notion page URL or id"]')
      );
      if (!el) throw new Error('not loaded');
      return el;
    });
    await fireEvent.input(input, { target: { value: 'typo' } });
    await fireEvent.click(getByText('Revert'));
    expect(input.value).toBe('old');
    expect(calls.filter(([c]) => c === 'project_settings').length).toBe(1); // no reload of the other section
    await fireEvent.input(input, { target: { value: 'https://notion.so/x' } });
    await fireEvent.click(getByText('Save'));
    await waitFor(() => expect(calls.some(([c]) => c === 'project_settings_set')).toBe(true));
    const [, args] = calls.find(([c]) => c === 'project_settings_set');
    expect(args.form.settings.notion_parent_page).toBe('https://notion.so/x');
    expect(args.form.backend).toEqual(backend); // the saved backend, as it is
  });

  it('Project settings: a clean section closes from the backdrop at once', async () => {
    replies.project_settings = { settings: {}, backend: { type: 'local' }, track_orchestrator_dir: true };
    replies.board_view = null;
    replies.integrations_view = { items: [] };
    openProjectSettings('shop-api', 'notion');
    const { container } = render(ProjectSettingsWindow);
    await waitFor(() => expect(container.querySelector('input[placeholder="Notion page URL or id"]')).toBeTruthy());
    await fireEvent.click(container.querySelector('.backdrop'));
    expect(app.window).toBeNull();
    expect(app.overlay).toBeNull();
  });
});
