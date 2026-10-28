// P3 ecosystem: the pure helpers, and Settings › Integrations and MCP
// servers rendered against a mocked core.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import {
  checkedAt,
  connStatus,
  effectiveServers,
  envToText,
  integrationDetail,
  keychainAccount,
  secretEnv,
  serverForSave,
  serverProblem,
  sortIntegrations,
  textToEnv,
  usedByRoles,
  usedByText,
} from '../src/lib/ecosystem.js';
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

const { default: SettingsPane } = await import('../src/panes/SettingsPane.svelte');

const integ = (id, name, over = {}) => ({
  id,
  name,
  status: 'not_configured',
  detail: null,
  has_secret: false,
  needs_secret: id === 'linear' || id === 'notion',
  checked_at_ms: null,
  account: null,
  ...over,
});

const server = (id, over = {}) => ({
  id,
  name: id,
  kind: 'stdio',
  command: 'npx',
  args: [],
  env: {},
  allow_secret_env: false,
  design_tool: null,
  status: 'unknown',
  detail: null,
  checked_at_ms: null,
  used_by: [],
  ...over,
});

describe('ecosystem helpers', () => {
  it('statuses read as glyph + label; a connector never seen says so', () => {
    expect(connStatus('needs_auth').label).toBe('Needs auth');
    expect(connStatus('unknown', true).label).toBe('Not seen yet');
    expect(connStatus('unknown').label).toBe('Unknown');
    expect(connStatus('bogus').label).toBe('Unknown');
  });

  it('checked times are 24 h with a relative hint, a weekday after a day', () => {
    const now = new Date(2026, 9, 21, 15, 0).getTime();
    expect(checkedAt(now - 2 * 60000, now)).toBe('checked 14:58 · 2m ago');
    expect(checkedAt(now - 26 * 3600000, now)).toBe('checked Tue 13:00');
    expect(checkedAt(null, now)).toBe('');
  });

  it('integration details and order', () => {
    expect(integrationDetail(integ('github', 'GitHub', { status: 'connected', account: 'GuillaumeOj' }))).toBe(
      'via gh · GuillaumeOj',
    );
    expect(integrationDetail(integ('linear', 'Linear'))).toBe('Add a personal API key to use Linear as a task backend');
    expect(
      integrationDetail(integ('linear', 'Linear', { status: 'connected', account: 'Acme', detail: 'logged in as G' })),
    ).toBe('Acme · logged in as G');
    const rows = sortIntegrations([integ('github', 'GitHub'), integ('notion', 'Notion', { status: 'needs_auth' })]);
    expect(rows.map((r) => r.id)).toEqual(['notion', 'github']);
  });

  it('env lines, Keychain refs and the saved shape', () => {
    const env = textToEnv('FIGMA_TOKEN=keychain:figma\n\nDEBUG = 1\nbad line');
    expect(env).toEqual({ FIGMA_TOKEN: 'keychain:figma', DEBUG: '1' });
    expect(envToText(env)).toBe('FIGMA_TOKEN=keychain:figma\nDEBUG=1');
    expect(keychainAccount('keychain:figma')).toBe('figma');
    expect(keychainAccount('plain')).toBe(null);
    expect(secretEnv(env)).toEqual(['FIGMA_TOKEN']);
    const saved = serverForSave({
      ...server('figma'),
      kind: 'http',
      url: ' https://x/mcp ',
      command: 'ignored',
      status: 'connected',
    });
    expect(saved).toEqual({
      id: 'figma',
      name: 'figma',
      kind: 'http',
      env: {},
      allow_secret_env: false,
      design_tool: null,
      url: 'https://x/mcp',
    });
    expect(serverProblem({ ...server('Bad Id') })).toMatch(/a–z/);
    expect(serverProblem({ ...server('orch') })).toMatch(/own server/);
    expect(serverProblem({ ...server('x'), kind: 'connector', connector_name: '' })).toMatch(/connector name/);
    expect(serverProblem(server('ok'))).toBe(null);
  });

  it('used-by copy and project overrides', () => {
    const used = [
      { team: 'Full-stack', node: 'ux', role: 'ux' },
      { team: 'Full-stack', node: 'ui', role: 'ui' },
      { team: 'Design sprint', node: 'ui', role: 'ui' },
    ];
    expect(usedByText(used)).toBe('ux, ui in 2 teams');
    expect(usedByRoles(used)).toBe('ux, ui (Full-stack, Design sprint)');
    expect(usedByText([])).toBe('—');
    const list = effectiveServers(
      {
        servers: [server('penpot'), server('figma')],
        project_overrides: { penpot: server('penpot', { kind: 'http' }) },
      },
      'shop-api',
    );
    expect(list.map((s) => [s.id, s.kind, s.scope])).toEqual([
      ['penpot', 'http', 'shop-api'],
      ['figma', 'stdio', null],
    ]);
  });
});

describe('Settings pane', () => {
  beforeEach(() => {
    cleanup();
    calls.length = 0;
    for (const k of Object.keys(replies)) delete replies[k];
    app.data = fixture();
    app.toasts = [];
    app.overlay = null;
  });

  const items = () => [
    integ('github', 'GitHub', { status: 'connected', account: 'GuillaumeOj', checked_at_ms: Date.now() - 120000 }),
    integ('linear', 'Linear'),
    integ('notion', 'Notion', { has_secret: true, status: 'needs_auth', detail: '401 from api.notion.com' }),
    integ('penpot', 'Penpot', { status: 'unknown', detail: 'MCP: Penpot' }),
    integ('figma', 'Figma'),
  ];

  it('integrations: a revoked token needs auth and sorts first; a pasted key leaves the UI at once', async () => {
    replies.integrations_view = { items: items() };
    let release = () => {};
    replies.integration_secret_set = (a) =>
      new Promise((r) => {
        release = () => r(integ('linear', 'Linear', { has_secret: true, status: 'connected', account: 'Acme' }));
        expect(a).toEqual({ id: 'linear', secret: 'lin_api_SECRET1234' });
      });
    app.panes.s1 = { type: 'settings', section: 'integrations', title: 'Settings' };
    const { container, getByText } = render(SettingsPane, { paneId: 's1' });
    await waitFor(() => getByText('via gh · GuillaumeOj'));
    const rows = [...container.querySelectorAll('[data-integration]')].map((r) => r.getAttribute('data-integration'));
    expect(rows[0]).toBe('notion');
    expect(getByText('Needs auth')).toBeTruthy();
    expect(container.textContent).toContain(
      'Notion rejected the token (401 from api.notion.com). Paste a new token with Replace.',
    );
    expect(container.textContent).toContain('Not seen yet');
    expect(container.querySelector('nav').textContent).toContain('1');
    // Paste a Linear key, ⌘⏎: the field clears before the core answers.
    const input = /** @type {HTMLInputElement} */ (
      container.querySelector('input[placeholder="Paste a Linear personal API key"]')
    );
    expect(input.type).toBe('password');
    await fireEvent.input(input, { target: { value: 'lin_api_SECRET1234' } });
    expect(container.textContent).toContain('1234'); // the last 4, only while typing
    await fireEvent.keyDown(input, { key: 'Enter', metaKey: true });
    await tick();
    expect(input.value ?? '').toBe('');
    expect(container.innerHTML).not.toContain('SECRET');
    release();
    await waitFor(() => expect(app.toasts.at(-1)?.text).toBe('Linear token stored in Keychain'));
    await waitFor(() =>
      expect(container.querySelector('[data-integration="linear"]').textContent).toContain('Stored in Keychain'),
    );
    expect(container.innerHTML).not.toContain('lin_api');
  });

  it('integrations keys: j moves, t tests, x asks before removing the token', async () => {
    replies.integrations_view = { items: items() };
    replies.integration_test = (a) => integ(a.id, 'Notion', { has_secret: true, status: 'connected' });
    replies.integration_secret_remove = (a) => integ(a.id, 'Notion');
    app.panes.s1 = { type: 'settings', section: 'integrations', title: 'Settings' };
    const { container, getByText } = render(SettingsPane, { paneId: 's1' });
    await waitFor(() => getByText('Needs auth'));
    const sec = container.querySelector('[aria-label="Integrations"]');
    // x on Notion (first: it needs you) asks first.
    await fireEvent.keyDown(sec, { key: 'x' });
    expect(app.overlay).toMatchObject({ kind: 'confirm', text: 'Remove the Notion token from the Keychain?' });
    app.overlay = null;
    await fireEvent.keyDown(sec, { key: 't' });
    await waitFor(() => expect(calls.some(([c, a]) => c === 'integration_test' && a.id === 'notion')).toBe(true));
    await waitFor(() => expect(app.toasts.at(-1)?.text).toBe('Notion connected'));
    // j then x on a row without a stored token: nothing to remove.
    await fireEvent.keyDown(sec, { key: 'j' });
    await fireEvent.keyDown(sec, { key: 'x' });
    expect(app.overlay).toBe(null);
  });

  it('MCP servers: a Keychain env value needs the opt-in; a used server is not deleted', async () => {
    replies.mcp_registry_view = {
      servers: [
        server('penpot', {
          name: 'Penpot',
          kind: 'connector',
          connector_name: 'claude.ai Penpot',
          design_tool: 'penpot',
          used_by: [{ team: 'full-stack', node: 'ui', role: 'ui' }],
        }),
        server('figma-local', {
          name: 'Figma local',
          env: { FIGMA_TOKEN: 'keychain:figma' },
          status: 'needs_auth',
          detail: 'exit 1',
        }),
      ],
      project_overrides: {},
    };
    replies.integrations_view = { items: items() };
    replies.mcp_server_save = (a) => ({ ...a.server, status: 'unknown', used_by: [] });
    replies.mcp_server_delete = { deleted: false, used_by: [{ team: 'full-stack', node: 'ui', role: 'ui' }] };
    app.panes.s1 = { type: 'settings', section: 'mcp', server: 'figma-local', title: 'Settings' };
    const { container, getByText } = render(SettingsPane, { paneId: 's1' });
    await waitFor(() => getByText('Pass this secret to the agent.'));
    expect(container.textContent).toContain('FIGMA_TOKEN from Keychain: figma');
    expect(container.textContent).toContain('Figma local exited or refused its credentials: exit 1.');
    expect(container.textContent).toContain('ui in 1 team');
    const opt = /** @type {HTMLInputElement} */ ([...container.querySelectorAll('.drawer input[type=checkbox]')][0]);
    expect(opt.checked).toBe(false);
    await fireEvent.click(opt);
    const sec = container.querySelector('[aria-label="MCP servers"]');
    await fireEvent.keyDown(sec, { key: 'Enter', metaKey: true });
    await waitFor(() =>
      expect(
        calls.some(
          ([c, a]) =>
            c === 'mcp_server_save' &&
            a.server.allow_secret_env === true &&
            a.server.env.FIGMA_TOKEN === 'keychain:figma' &&
            a.scope.kind === 'global',
        ),
      ).toBe(true),
    );
    // The connector row: the hint explains strict MCP; delete is refused with who uses it.
    await fireEvent.click(container.querySelector('tbody tr'));
    await tick();
    expect(container.textContent).toContain("can't be passed in --mcp-config");
    await fireEvent.keyDown(sec, { key: 'x' });
    expect(app.overlay?.kind).toBe('confirm');
    expect(app.overlay.text).toBe('Delete Penpot from the MCP registry?');
    await app.overlay.onYes();
    await waitFor(() => getByText('Penpot is used by ui (full-stack).'));
  });

  it('MCP servers: empty registry invites to add one; n opens a new entry', async () => {
    replies.mcp_registry_view = { servers: [], project_overrides: {} };
    replies.integrations_view = { items: [] };
    app.panes.s1 = { type: 'settings', section: 'mcp', title: 'Settings' };
    const { container, getByText } = render(SettingsPane, { paneId: 's1' });
    await waitFor(() => getByText(/No MCP servers yet/));
    await fireEvent.keyDown(container.querySelector('[aria-label="MCP servers"]'), { key: 'n' });
    await tick();
    expect(getByText('New MCP server')).toBeTruthy();
  });

  it('sections not designed yet say where to edit', async () => {
    replies.integrations_view = { items: [] };
    app.panes.s1 = { type: 'settings', section: 'integrations', title: 'Settings' };
    const { container, getByText } = render(SettingsPane, { paneId: 's1' });
    const nav = container.querySelector('nav');
    await fireEvent.keyDown(nav, { key: 'j' });
    await fireEvent.keyDown(nav, { key: 'j' });
    await tick();
    expect(app.panes.s1.section).toBe('terminal');
    expect(getByText(/Not designed yet/)).toBeTruthy();
  });
});
