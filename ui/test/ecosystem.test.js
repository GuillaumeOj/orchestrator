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
const { default: FeaturePane } = await import('../src/panes/FeaturePane.svelte');
const { default: TeamCanvasPane } = await import('../src/panes/TeamCanvasPane.svelte');

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

describe('role inspector, Design section and presets (T-7)', () => {
  beforeEach(() => {
    cleanup();
    calls.length = 0;
    for (const k of Object.keys(replies)) delete replies[k];
    app.data = fixture();
    app.toasts = [];
    app.overlay = null;
  });

  const art = (id, uri, over = {}) => ({
    id,
    feature_id: 'shop-onb',
    team_run_id: 'run-1',
    kind: 'design',
    uri,
    from_node: 'ui',
    to_node: 'front',
    note: '',
    created_at_ms: new Date(2026, 9, 21, 13, 12).getTime() + id,
    meta: null,
    ...over,
  });
  const file = (id, name, over = {}) =>
    art(id, `.orchestrator/features/onboarding/design/${name}`, {
      meta: {
        tool: 'file',
        path: `.orchestrator/features/onboarding/design/${name}`,
        previewable: true,
        on_integration: true,
      },
      ...over,
    });

  const featureView = (artifacts) => ({
    feature: { id: 'shop-onb', project_id: 'shop', slug: 'onboarding', integration_worktree: '/w/onb' },
    run: { id: 'run-1', team_id: 'full-stack', status: 'executing', started_at_ms: Date.now() - 60000, pr_url: null },
    team: {
      name: 'Full-stack',
      version: 3,
      nodes: [
        { id: 'lead', role_id: 'lead', is_lead: true },
        { id: 'ui', role_id: 'ui', is_lead: false },
        { id: 'front', role_id: 'frontend', is_lead: false },
      ],
    },
    project: { id: 'shop', name: 'shop-api', backend: 'local' },
    agents: [
      { id: 'ui-1', node: 'ui', state: 'idle' },
      { id: 'front-2', node: 'front', state: 'idle' },
    ],
    tasks: [
      { id: 2, title: 'Welcome frames in Penpot', status: 'in_progress', node: 'ui', depends_on: [], backend: 'local' },
    ],
    docs: [],
    artifacts,
    timeline: [],
  });

  it('Design section: no designs yet names who is drawing', async () => {
    replies.feature_view = featureView([]);
    app.panes.f1 = { type: 'feature', featureId: 'shop-onb', title: 'onboarding' };
    const { getByText } = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() =>
      getByText(
        "No designs yet. ui-1 is on 'Welcome frames in Penpot'. Exported frames show here when they're handed off.",
      ),
    );
  });

  it('Design section: previews, not merged, too large, link only; keys open, preview and ask to export', async () => {
    replies.feature_view = featureView([
      file(1, 'welcome-1.png', { note: 'Step 1' }),
      file(2, 'welcome-2.svg', {
        meta: { tool: 'file', path: 'x/welcome-2.svg', previewable: true, on_integration: false },
      }),
      file(3, 'welcome-3.png'),
      art(4, 'https://design.penpot.app/#/workspace?file-id=f&page-id=p', {
        meta: { tool: 'penpot', title: 'Onboarding / Welcome', previewable: false },
      }),
    ]);
    replies.artifact_preview = (a) =>
      a.artifact === 3
        ? Promise.reject('too large (7.4 MB)')
        : {
            mime: a.artifact === 2 ? 'image/svg+xml' : 'image/png',
            data_b64: 'AAAA',
            bytes: 219000,
            source: a.artifact === 2 ? 'branch' : 'integration',
          };
    replies.artifact_open = null;
    replies.message_send = { ids: [1] };
    app.panes.f1 = { type: 'feature', featureId: 'shop-onb', title: 'onboarding' };
    const { container, getByText } = render(FeaturePane, { paneId: 'f1' });
    await waitFor(() => expect(container.querySelectorAll('.tile img')).toHaveLength(2));
    const sec = container.querySelector('[aria-label="Design"]');
    expect(sec.textContent).toContain("On ui-1's branch · not merged");
    expect(sec.textContent).toContain('Too large to preview (7.4 MB · max 5 MB)');
    expect(sec.textContent).toContain("Link only: front-2 can't open it.");
    expect(getByText('Onboarding / Welcome')).toBeTruthy();
    // The SVG is an <img> data URL, never inline markup.
    expect(container.querySelector('svg image, .tile svg')).toBeNull();
    expect(container.querySelector('.tile img[alt="welcome-2.svg"]').getAttribute('src')).toMatch(
      /^data:image\/svg\+xml;base64,/,
    );
    // d focuses the section; ⏎ opens the first, Space previews it.
    await fireEvent.keyDown(container.querySelector('.feature'), { key: 'd' });
    expect(document.activeElement).toBe(sec);
    await fireEvent.keyDown(sec, { key: 'Enter' });
    await waitFor(() => expect(calls.some(([c, a]) => c === 'artifact_open' && a.artifact === 1)).toBe(true));
    await fireEvent.keyDown(sec, { key: ' ' });
    await tick();
    expect(container.querySelector('[aria-label="Preview"]').textContent).toContain(
      'welcome-1.png · 214 KB · integration branch',
    );
    await fireEvent.keyDown(sec, { key: 'ArrowRight' });
    await tick();
    expect(container.querySelector('[aria-label="Preview"]').textContent).toContain('SVG shown as an image');
    await fireEvent.keyDown(sec, { key: 'Escape' });
    await tick();
    expect(container.querySelector('[aria-label="Preview"]')).toBeNull();
    // l l l to the Penpot link, x asks ui to export it.
    for (let i = 0; i < 3; i++) await fireEvent.keyDown(sec, { key: 'l' });
    await fireEvent.keyDown(sec, { key: 'x' });
    await waitFor(() =>
      expect(
        calls.some(([c, a]) => c === 'message_send' && a.to === 'ui' && /export "Onboarding \/ Welcome"/.test(a.body)),
      ).toBe(true),
    );
  });

  const role = (id, name, over = {}) => ({
    id,
    name,
    scope: { kind: 'global' },
    color: null,
    model: 'opus',
    effort: null,
    tools: { edit: true, bash: true, web: false },
    subagents_allowed: false,
    permission_profile: 'autonomous',
    budget_priority: 'normal',
    prompt: '',
    version: 1,
    mcp_refs: [],
    strict_mcp: true,
    downgrade: null,
    issues: [],
    ...over,
  });
  const canvasTeam = {
    id: 'full-stack',
    name: 'Full-stack',
    scope: { kind: 'global' },
    version: 3,
    nodes: [
      { id: 'lead', role_id: 'lead', overrides: {}, position: [0, 0], max_concurrency: 1, is_lead: true },
      { id: 'ui', role_id: 'ui', overrides: {}, position: [0, 200], max_concurrency: 1, is_lead: false },
    ],
    edges: [{ from: 'ui', to: 'lead', kind: 'reports-to' }],
  };

  function canvasReplies(roles) {
    replies.teams_view = () => ({
      teams: [
        {
          team: canvasTeam,
          issues: [],
          warnings: [],
          rules: [],
          preset: true,
          starter: 'full-stack',
          update_available: true,
        },
        {
          team: { ...canvasTeam, id: 'mine', name: 'Mine', version: 1 },
          issues: [],
          rules: [],
          preset: false,
          starter: null,
          update_available: false,
        },
      ],
      roles,
    });
    replies.mcp_registry_view = {
      servers: [
        server('penpot', { name: 'Penpot', kind: 'connector', design_tool: 'penpot', status: 'unknown' }),
        server('figma', { name: 'Figma', kind: 'http', status: 'needs_auth' }),
      ],
      project_overrides: {},
    };
    // The core keeps what was saved (the next teams_view shows it).
    replies.role_save = (a) => {
      roles.splice(
        roles.findIndex((r) => r.id === a.role.id),
        1,
        a.role,
      );
      return a.role;
    };
  }

  it('role inspector: MCP picker with statuses, connector + strict warning, downgrade field', async () => {
    const ui = role('ui', 'UI', { mcp_refs: ['penpot', 'gone'] });
    canvasReplies([role('lead', 'Lead'), ui]);
    app.panes.c1 = { type: 'canvas', teamId: 'full-stack', title: 'Team' };
    const { container, getByText } = render(TeamCanvasPane, { paneId: 'c1' });
    await waitFor(() => getByText('Lead', { selector: '.muted', exact: false }));
    const pane = container.querySelector('.canvas-pane');
    await fireEvent.keyDown(pane, { key: 'j' }); // lead → ui
    await waitFor(() => getByText(/is a claude.ai connector, so this role can't use strict MCP/));
    const insp = container.querySelector('[aria-label="Role inspector"]');
    expect(insp.textContent).toContain('Not seen yet');
    expect(insp.textContent).toContain('Needs auth');
    expect(insp.textContent).toContain('connector · needs your claude.ai login');
    expect(insp.textContent).toContain('gone is not in the MCP registry.');
    // Tick Figma: saved by registry order.
    const figma = [...insp.querySelectorAll('.srv')]
      .find((l) => l.textContent.includes('Figma'))
      .querySelector('input');
    await fireEvent.click(figma);
    await waitFor(() =>
      expect(
        calls.some(
          ([c, a]) => c === 'role_save' && a.role.id === 'ui' && a.role.mcp_refs.join() === 'penpot,figma,gone',
        ),
      ).toBe(true),
    );
    // Downgrade on: opus → sonnet when Red, saved; picking opus (the role's own) is refused.
    const dg = [...insp.querySelectorAll('input[type=checkbox]')].find((i) =>
      i.parentElement.textContent.includes('cheaper model'),
    );
    await fireEvent.click(dg);
    await waitFor(() =>
      expect(
        calls.some(
          ([c, a]) => c === 'role_save' && a.role.downgrade?.to === 'sonnet' && a.role.downgrade.when === 'red',
        ),
      ).toBe(true),
    );
    expect(insp.textContent).toContain('Switches at the next task, never mid-task.');
    const to = /** @type {HTMLSelectElement} */ (insp.querySelector('select[aria-label="Downgrade to"]'));
    const n = calls.length;
    to.value = 'opus';
    await fireEvent.change(to);
    expect(insp.textContent).toContain("Pick a model other than the role's (opus).");
    expect(calls.slice(n).some(([c]) => c === 'role_save')).toBe(false);
    expect(insp.textContent).toContain('opus window in Red');
  });

  it('role inspector: a downgrade on the lead warns', async () => {
    canvasReplies([role('lead', 'Lead', { downgrade: { to: 'sonnet', when: 'amber' } }), role('ui', 'UI')]);
    app.panes.c1 = { type: 'canvas', teamId: 'full-stack', title: 'Team' };
    const { getByText } = render(TeamCanvasPane, { paneId: 'c1' });
    await waitFor(() => getByText(/This role is the team's lead. A downgraded lead plans and reviews/));
    expect(getByText("No MCP servers. The role only has Claude Code's built-in tools.")).toBeTruthy();
  });

  it('presets: the library marks starters and updates, D 2 duplicates into the project, Launch opens the dialog', async () => {
    canvasReplies([role('lead', 'Lead'), role('ui', 'UI')]);
    replies.team_duplicate = (a) => ({
      team: { ...canvasTeam, id: 'full-stack-copy', name: 'Full-stack (copy)', scope: a.to },
      roles_copied: [],
    });
    replies.team_set_preset = (a) => ({ ...canvasTeam, id: a.team, preset: a.preset });
    app.panes.c1 = { type: 'canvas', teamId: 'full-stack', projectId: 'shop-api', title: 'Team' };
    const { container, getByText } = render(TeamCanvasPane, { paneId: 'c1' });
    await waitFor(() => getByText('Lead', { selector: '.muted', exact: false }));
    const pane = container.querySelector('.canvas-pane');
    await fireEvent.keyDown(pane, { key: 'Escape' }); // nothing selected: D is about the team
    await fireEvent.keyDown(pane, { key: 'D', shiftKey: true });
    await tick();
    expect(container.querySelector('[aria-label="Duplicate"]').textContent).toContain('to shop-api');
    await fireEvent.keyDown(pane, { key: '2' });
    await waitFor(() =>
      expect(
        calls.some(
          ([c, a]) =>
            c === 'team_duplicate' && a.team === 'full-stack' && a.to.kind === 'project' && a.to.id === 'shop-api',
        ),
      ).toBe(true),
    );
    await waitFor(() => expect(app.toasts.at(-1)?.text).toBe('Duplicated Full-stack to shop-api'));
    await fireEvent.keyDown(pane, { key: 'R', shiftKey: true });
    await tick();
    const lib = container.querySelector('[aria-label="Library"]');
    expect(lib.textContent).toContain('Starter');
    expect(lib.textContent).toContain('Update available');
    expect(lib.textContent).toContain("Your copy was edited, so it wasn't updated.");
    await fireEvent.click(getByText('Save as preset'));
    await waitFor(() =>
      expect(calls.some(([c, a]) => c === 'team_set_preset' && a.team === 'mine' && a.preset)).toBe(true),
    );
    await fireEvent.click(lib.querySelectorAll('.pacts .link')[0]);
    expect(app.overlay).toEqual({ kind: 'launch', teamId: 'full-stack', projectId: 'shop-api' });
    app.overlay = null;
  });
});
