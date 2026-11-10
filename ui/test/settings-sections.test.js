// Both settings windows list their sections (9 and 7), each renders without the "Not designed
// yet" text, no Kbd in the nav, a deep link opens on its section (design-notes §13.4,
// acceptance F9 / F10). Plus the Overwatch Notifications switches and the MCP overrides control.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app, openProjectSettings, openSettings } from '../src/lib/app.svelte.js';
import { fixture } from './fixture.js';

const calls = [];
const replies = {};
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  invoke: (cmd, args) => {
    calls.push([cmd, args]);
    const r = replies[cmd];
    return r instanceof Error ? Promise.reject(r) : Promise.resolve(typeof r === 'function' ? r(args) : (r ?? null));
  },
  listen: () => Promise.resolve(() => {}),
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
}));

const { default: SettingsWindow, SECTIONS } = await import('../src/components/SettingsWindow.svelte');
const { default: ProjectSettingsWindow, SECTIONS: PSECTIONS } = await import(
  '../src/components/ProjectSettingsWindow.svelte'
);
const { default: OverwatchSettings } = await import('../src/components/OverwatchSettings.svelte');
const { default: McpSettings } = await import('../src/components/McpSettings.svelte');
const text = (el) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.data.projects[0].default_branch = 'main';
  app.kitty = { sources: [], options: {}, padding: [0, 0, 0, 0], opacity: 1, font_family: 'Menlo', unmapped: [] };
  app.window = null;
  app.toasts = [];
  replies.integrations_view = { items: [] };
  replies.mcp_registry_view = { servers: [], project_overrides: {} };
  replies.project_settings = { settings: {}, backend: { type: 'local' } };
  replies.project_branches = ['main'];
  replies.project_detail = { trusted: true, worktrees: [] };
  replies.settings_get = null;
});

describe('Settings window: 9 sections', () => {
  it('the nav lists them in order, with no Kbd and no Keyboard section', () => {
    expect(SECTIONS.map((s) => s.label)).toEqual([
      'Claude Code',
      'Integrations',
      'Overwatch',
      'Permissions',
      'MCP servers',
      'Terminal & editor',
      'Pacing policy',
      'Memory',
      'Builds',
    ]);
    app.window = { kind: 'settings', section: 'claude' };
    const { container } = render(SettingsWindow);
    expect(container.querySelectorAll('[role="tab"]')).toHaveLength(9);
    expect(container.querySelector('[role="tablist"] .kbd')).toBeNull();
  });

  it.each(SECTIONS.map((s) => [s.id, s.label]))('%s renders, not as a placeholder', async (id) => {
    app.window = { kind: 'settings', section: id };
    const { container } = render(SettingsWindow);
    await new Promise((r) => setTimeout(r, 0));
    expect(container.querySelector('[role="tab"][aria-selected="true"]').textContent).toContain(
      SECTIONS.find((s) => s.id === id).label,
    );
    expect(text(container)).not.toContain('Not designed yet');
    expect(container.querySelector('.fw-body, [class*="body"]')?.children.length ?? 1).toBeGreaterThan(0);
  });

  it('a deep link opens on its section', () => {
    openSettings('memory');
    expect(app.window).toMatchObject({ kind: 'settings', section: 'memory' });
    const { container } = render(SettingsWindow);
    expect(text(container.querySelector('[role="tab"][aria-selected="true"]'))).toBe('Memory');
    expect(container.querySelector('[aria-label="Memory settings"]')).toBeTruthy();
    app.window = null;
  });
});

describe('Project settings window: 7 sections', () => {
  it('the nav lists them, General first, with no Kbd', () => {
    expect(PSECTIONS.map((s) => s.label)).toEqual([
      'General',
      'Files',
      'Task backend',
      'Intake',
      'Allocation',
      'Notion',
      'MCP overrides',
    ]);
    app.window = { kind: 'project-settings', projectId: 'shop-api', section: 'backend' };
    const { container } = render(ProjectSettingsWindow);
    expect(container.querySelectorAll('[role="tab"]')).toHaveLength(7);
    expect(container.querySelector('[role="tablist"] .kbd')).toBeNull();
  });

  it.each(PSECTIONS.map((s) => [s.id, s.label]))('%s renders, not as a placeholder', async (id) => {
    replies.project_files = { items: [] };
    app.window = { kind: 'project-settings', projectId: 'shop-api', section: id };
    const { container } = render(ProjectSettingsWindow);
    await new Promise((r) => setTimeout(r, 0));
    expect(text(container.querySelector('[role="tab"][aria-selected="true"]'))).toBe(
      PSECTIONS.find((s) => s.id === id).label,
    );
    expect(text(container)).not.toContain('Not designed yet');
  });

  it('opens on Task backend by default (design-notes §3.1); a deep link wins', () => {
    openProjectSettings('shop-api');
    const { container } = render(ProjectSettingsWindow);
    expect(text(container.querySelector('[role="tab"][aria-selected="true"]'))).toBe('Task backend');
    app.window = null;
    openProjectSettings('shop-api', 'general');
    const r = render(ProjectSettingsWindow);
    expect(text(r.container.querySelectorAll('[role="tab"][aria-selected="true"]')[0])).toBe('General');
    app.window = null;
  });
});

describe('Overwatch › Notifications', () => {
  beforeEach(() => {
    replies.overwatch_settings = {
      enabled: true,
      model: 'sonnet',
      batch_secs: 30,
      permission_secs: 60,
      idle_secs: 300,
      rule_threshold: 3,
    };
  });

  it('two switches on the `notifications` setting, both on by default', async () => {
    const { container, getByText } = render(OverwatchSettings);
    await waitFor(() => expect(getByText('Notifications')).toBeTruthy());
    const sw = (k) => container.querySelector(`#ow-n-${k}`);
    expect(/** @type {any} */ (sw('desktop')).checked).toBe(true);
    expect(/** @type {any} */ (sw('sound')).checked).toBe(true);
  });

  it('reads the stored value and writes both keys back on a change', async () => {
    replies.settings_get = (a) => (a.key === 'notifications' ? { desktop: true, sound: false } : null);
    const { container } = render(OverwatchSettings);
    await waitFor(() => expect(/** @type {any} */ (container.querySelector('#ow-n-sound'))?.checked).toBe(false));
    await fireEvent.click(container.querySelector('#ow-n-desktop'));
    await waitFor(() =>
      expect(calls).toContainEqual(['settings_set', { key: 'notifications', value: { desktop: false, sound: false } }]),
    );
    await waitFor(() => expect(text(container)).toContain('✓ Saved'));
  });
});

describe('MCP overrides: Inherit / Override', () => {
  const global = {
    id: 'penpot',
    name: 'Penpot',
    kind: 'stdio',
    command: 'penpot',
    args: [],
    env: { A: '1' },
    status: 'connected',
    used_by: [],
  };
  const open = async (over = {}) => {
    replies.mcp_registry_view = { servers: [global], project_overrides: over };
    replies.mcp_server_save = (a) => ({ ...a.server, id: a.server.id, name: a.server.name });
    replies.mcp_server_delete = { deleted: true };
    const r = render(McpSettings, { project: 'shop-api', selected: 'penpot' });
    await waitFor(() => expect(r.container.querySelector('[data-override]')).toBeTruthy());
    return r;
  };
  const seg = (c, label) =>
    [...c.querySelectorAll('[data-override] [role="radio"]')].find((b) => b.textContent === label);

  it('two options, no Off; Inherit selected when the project has no copy', async () => {
    const { container } = await open();
    expect([...container.querySelectorAll('[data-override] [role="radio"]')].map((b) => b.textContent)).toEqual([
      'Inherit',
      'Override',
    ]);
    expect(seg(container, 'Inherit').getAttribute('aria-checked')).toBe('true');
  });

  it('the list row shows the state: Inherit without a project copy', async () => {
    const { container } = await open();
    expect(text(container.querySelector('[data-scope]'))).toBe('Inherit');
  });

  it('Override saves a project-scoped copy', async () => {
    const { container } = await open();
    await fireEvent.click(seg(container, 'Override'));
    await waitFor(() => expect(calls.some(([c]) => c === 'mcp_server_save')).toBe(true));
    const [, a] = calls.find(([c]) => c === 'mcp_server_save');
    expect(a.scope).toEqual({ kind: 'project', id: 'shop-api' });
    expect(a.server.id).toBe('penpot');
  });

  it('Inherit deletes the project copy at once when its env equals the global one', async () => {
    const { container } = await open({ penpot: { ...global } });
    expect(seg(container, 'Override').getAttribute('aria-checked')).toBe('true');
    await fireEvent.click(seg(container, 'Inherit'));
    await waitFor(() =>
      expect(calls).toContainEqual(['mcp_server_delete', { id: 'penpot', scope: { kind: 'project', id: 'shop-api' } }]),
    );
    expect(app.overlay).toBeNull();
  });

  it('Inherit asks first when the project copy has its own env', async () => {
    const { container } = await open({ penpot: { ...global, env: { A: '2' } } });
    await fireEvent.click(seg(container, 'Inherit'));
    expect(app.overlay?.text).toBe('Use the global Penpot in shop-api?');
    expect(calls.some(([c]) => c === 'mcp_server_delete')).toBe(false);
    app.overlay.onYes();
    await waitFor(() => expect(calls.some(([c]) => c === 'mcp_server_delete')).toBe(true));
    app.overlay = null;
  });
});
