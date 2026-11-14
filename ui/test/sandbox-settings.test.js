// Settings → Sandbox (allow-fine-tune-sandbox-permissions, ux-flows §3): states (loading, error, empty,
// parse error, warnings), built-in rows, the add form with its live check, Remove with Undo, the applies
// line, and the reload on the `sandbox` topic. Against a mocked core.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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

const { app } = await import('../src/lib/app.svelte.js');
const { applyNow } = await import('../src/lib/now.svelte.js');
const { appliesLine, groups } = await import('../src/lib/sandbox.js');
const { default: SandboxSettings } = await import('../src/components/SandboxSettings.svelte');

const text = (el) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const btn = (c, label) => [...c.querySelectorAll('button')].find((b) => text(b) === label);
const tick = (ms = 0) => new Promise((r) => setTimeout(r, ms));

const globalScope = (over = {}) => ({
  scope: 'global',
  file: 'global',
  path: '/Users/me/Library/Application Support/Orchestrator/policy.toml',
  exists: true,
  warnings: [],
  entries: [],
  defaults: [{ kind: 'network', value: 'registry.npmjs.org', source: 'core' }],
  ...over,
});
const projectScope = (over = {}) => ({
  scope: 'project',
  project: 'shop-api',
  file: 'project:shop-api',
  path: '/code/shop-api/.orchestrator/policy.toml',
  exists: true,
  warnings: [],
  entries: [],
  defaults: [],
  ...over,
});
const entry = (id, kind, value, extra = {}) => ({ id, kind, value, source: 'file', ...extra });

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  replies.sandbox_list = [globalScope(), projectScope()];
  replies.sandbox_kinds = ['network', 'write', 'local_ports', 'unix_socket'];
  replies.teams_view = { teams: [], roles: [{ id: 'back', name: 'Backend' }] };
});
afterEach(cleanup);

async function loaded() {
  const r = render(SandboxSettings);
  await waitFor(() => expect(r.container.querySelector('[data-sandbox-file]')).not.toBeNull());
  return r;
}
const projectBlock = (c) => c.querySelector('[data-sandbox-file="project:shop-api"]');

describe('states', () => {
  it('loading, then the blocks: All projects first, built-in rows muted and tagged, no Remove on them', async () => {
    const { container } = render(SandboxSettings);
    expect(text(container)).toContain('Loading…');
    await waitFor(() => expect(container.querySelectorAll('[data-sandbox-file]')).toHaveLength(2));
    const files = [...container.querySelectorAll('[data-sandbox-file]')].map((f) =>
      f.getAttribute('data-sandbox-file'),
    );
    expect(files).toEqual(['global', 'project:shop-api']);
    const t = text(container.querySelector('[data-sandbox-file="global"]'));
    expect(t).toContain('All projects');
    expect(t).toContain('Internet hosts');
    expect(t).toContain('registry.npmjs.org');
    expect(t).toContain('built in');
    expect(t).toContain('Every agent gets these.');
    expect(t).toContain('Nothing added for all projects.');
    expect(container.querySelector('[data-row="builtin"] button')).toBeNull();
  });

  it('a load error with Retry', async () => {
    replies.sandbox_list = new Error('boom');
    const { container } = render(SandboxSettings);
    await waitFor(() => expect(text(container)).toContain('Could not read the sandbox settings: boom'));
    replies.sandbox_list = [globalScope()];
    await fireEvent.click(btn(container, 'Retry'));
    await waitFor(() => expect(container.querySelector('[data-sandbox-file]')).not.toBeNull());
  });

  it('an empty project, and no project file yet', async () => {
    replies.sandbox_list = [globalScope(), projectScope({ exists: false })];
    const { container } = await loaded();
    const p = projectBlock(container);
    expect(text(p)).toContain('no policy file yet');
    expect(text(p)).toContain('Nothing extra: agents get the built-in hosts only.');
    expect(btn(p, 'Create and open')).toBeTruthy();
  });

  it('no projects: the line under All projects', async () => {
    replies.sandbox_list = [globalScope()];
    const { container } = await loaded();
    expect(text(container)).toContain('No projects yet. Add one to give it its own allowances.');
  });

  it('role hosts show in the project, muted, "built in · back", no Remove', async () => {
    replies.sandbox_list = [
      globalScope(),
      projectScope({ defaults: [{ kind: 'network', value: 'crates.io', source: 'role:back', role: 'back' }] }),
    ];
    const { container } = await loaded();
    expect(text(projectBlock(container))).toContain('built in · back');
  });

  it('a parse error: the last good rows, Add and Remove disabled with the line', async () => {
    replies.sandbox_list = [
      globalScope(),
      projectScope({ error: { line: 14, message: 'bad key' }, entries: [entry('s1', 'network', 'api.github.com')] }),
    ];
    const { container } = await loaded();
    const p = projectBlock(container);
    expect(text(p)).toContain('Parse error at line 14: bad key. Using the last good version.');
    expect(text(p)).toContain('api.github.com');
    expect(btn(p, 'Add').disabled).toBe(true);
    expect(btn(p, 'Add').title).toBe('Fix the file first: line 14.');
    expect(p.querySelector('[aria-label="Remove api.github.com"]').disabled).toBe(true);
    expect(btn(p, 'Open in editor')).toBeTruthy();
  });

  it('warnings: three, then "Show 1 more", and the edit line', async () => {
    const warnings = [1, 2, 3, 4].map((n) => ({
      id: `s${n}`,
      message: `~/.ssh${n} is a credential store: never allowed`,
    }));
    replies.sandbox_list = [globalScope(), projectScope({ warnings })];
    const { container } = await loaded();
    const p = projectBlock(container);
    expect(p.querySelectorAll('.warn')).toHaveLength(3);
    expect(text(p)).toContain('Allowance s1: ~/.ssh1 is a credential store: never allowed. Skipped.');
    expect(text(p)).toContain('Edit the file to fix it.');
    await fireEvent.click(btn(p, 'Show 1 more'));
    expect(p.querySelectorAll('.warn')).toHaveLength(4);
  });

  it('no kinds: the version note and no Add buttons', async () => {
    replies.sandbox_kinds = [];
    const { container } = await loaded();
    expect(text(container)).toContain("This version of Claude Code can't widen the sandbox.");
    expect(btn(container, 'Add')).toBeUndefined();
  });

  it('only the kinds the core returns are offered', async () => {
    replies.sandbox_kinds = ['network', 'write'];
    replies.sandbox_list = [globalScope({ entries: [entry('s2', 'local_ports', null)] })];
    const { container } = await loaded();
    expect(container.querySelector('[data-group="local_ports"]')).toBeNull();
    expect(groups({ entries: [entry('s2', 'local_ports', null)] }, ['network'])).toEqual([]);
  });

  it('rows: role chip, note, local ports without a value, socket', async () => {
    replies.sandbox_list = [
      globalScope(),
      projectScope({
        entries: [
          entry('s1', 'network', 'api.github.com', { role: 'back', note: 'release check' }),
          entry('s2', 'local_ports', null),
          entry('s3', 'unix_socket', '/tmp/x.sock'),
        ],
      }),
    ];
    const { container } = await loaded();
    const t = text(projectBlock(container));
    expect(t).toContain('back only');
    expect(t).toContain('release check');
    expect(t).toContain('Local ports (test servers)');
    expect(t).toContain('Sockets');
    expect(t).toContain('/tmp/x.sock');
  });

  it("the links: Open Permissions settings and What can't be allowed", async () => {
    const { container } = await loaded();
    expect(container.querySelector('[data-limits]')).toBeNull();
    await fireEvent.click(btn(container, "What can't be allowed"));
    expect(text(container.querySelector('[data-limits]'))).toContain('"All hosts" (*) can\'t be allowed.');
    await fireEvent.click(btn(container, 'Open Permissions settings'));
    expect(app.window).toMatchObject({ kind: 'settings', section: 'permissions' });
  });
});

describe('add form', () => {
  async function open(container) {
    await fireEvent.click(btn(projectBlock(container), 'Add'));
    return container.querySelector('form[aria-label="Add an allowance"]');
  }
  const type = (form, label, v) =>
    fireEvent.input(form.querySelector(`input[placeholder="${label}"]`), { target: { value: v } });

  it('checks nothing while the value is empty, and a local-ports allowance needs no value', async () => {
    replies.sandbox_check = (a) => ({ ok: true, value: a.value });
    const { container } = await loaded();
    const form = await open(container);
    await tick(400);
    expect(calls.some((c) => c[0] === 'sandbox_check')).toBe(false);
    expect(btn(form, 'Add').disabled).toBe(true);
  });

  it('a host: OK shows a check, a normalised value is announced, Add posts the scope, role and note', async () => {
    replies.sandbox_check = (a) => ({ ok: true, value: a.value.toLowerCase() });
    replies.sandbox_add = {
      entry: entry('s9', 'network', 'api.github.com'),
      file: 'project:shop-api',
      applies: 'next_start',
      running: 2,
      created: true,
    };
    const { container } = await loaded();
    const form = await open(container);
    await type(form, 'api.github.com', 'API.GitHub.com');
    await waitFor(() => expect(text(form)).toContain('Will be saved as api.github.com'), { timeout: 2000 });
    expect(text(form)).toContain(
      'Without your GitHub login: issues and pull requests still go through the orchestrator.',
    );
    expect(calls.filter((c) => c[0] === 'sandbox_check').at(-1)[1]).toEqual({
      kind: 'network',
      value: 'API.GitHub.com',
      scope: 'project',
      project: 'shop-api',
    });
    await fireEvent.input(form.querySelector('input[placeholder="Optional, for whoever reads the file"]'), {
      target: { value: 'release check' },
    });
    replies.sandbox_list = [globalScope(), projectScope({ entries: [entry('s9', 'network', 'api.github.com')] })];
    await fireEvent.click(btn(form, 'Add'));
    await waitFor(() => expect(container.querySelector('[data-row="s9"]')).not.toBeNull());
    expect(calls.find((c) => c[0] === 'sandbox_add')[1]).toEqual({
      scope: 'project',
      project: 'shop-api',
      kind: 'network',
      value: 'API.GitHub.com',
      role: null,
      note: 'release check',
    });
    expect(container.querySelector('form[aria-label="Add an allowance"]')).toBeNull();
    expect(container.querySelector('[data-row="s9"]').classList.contains('hl')).toBe(true);
    expect(text(container.querySelector('[data-applies]'))).toBe(
      'Applies when an agent next starts. 2 agents running.',
    );
  });

  it('the core refusing the value: its sentence under the field, Add disabled', async () => {
    replies.sandbox_check = { ok: false, reason: '`~/.ssh` is a credential store: never allowed' };
    const { container } = await loaded();
    const form = await open(container);
    await type(form, 'api.github.com', '*');
    await waitFor(() => expect(text(form)).toContain('`~/.ssh` is a credential store: never allowed'), {
      timeout: 2000,
    });
    expect(form.querySelector('input[aria-invalid="true"]')).not.toBeNull();
    expect(btn(form, 'Add').disabled).toBe(true);
  });

  it('an example chip fills the value', async () => {
    replies.sandbox_check = (a) => ({ ok: true, value: a.value });
    const { container } = await loaded();
    const form = await open(container);
    await fireEvent.click(btn(form, '*.githubusercontent.com'));
    expect(form.querySelector('input[placeholder="api.github.com"]').value).toBe('*.githubusercontent.com');
    await waitFor(() => expect(btn(form, 'Add').disabled).toBe(false), { timeout: 2000 });
  });

  it('a refused add (race): the form stays with the reason', async () => {
    replies.sandbox_check = (a) => ({ ok: true, value: a.value });
    replies.sandbox_add = new Error('the file changed');
    const { container } = await loaded();
    const form = await open(container);
    await type(form, 'api.github.com', 'docs.rs');
    await waitFor(() => expect(btn(form, 'Add').disabled).toBe(false), { timeout: 2000 });
    await fireEvent.click(btn(form, 'Add'));
    await waitFor(() => expect(text(form)).toContain('Could not add it: the file changed.'));
    expect(btn(form, 'Add').disabled).toBe(false);
  });

  it('a duplicate says so and no row is added', async () => {
    replies.sandbox_check = (a) => ({ ok: true, value: a.value });
    replies.sandbox_add = {
      entry: entry('s1', 'network', 'docs.rs'),
      applies: 'next_start',
      running: 0,
      created: false,
    };
    const { container } = await loaded();
    const form = await open(container);
    await type(form, 'api.github.com', 'docs.rs');
    await waitFor(() => expect(btn(form, 'Add').disabled).toBe(false), { timeout: 2000 });
    await fireEvent.click(btn(form, 'Add'));
    await waitFor(() => expect(text(projectBlock(container))).toContain('Already allowed here.'));
  });

  it('an empty host after typing: the local message', async () => {
    const { container } = await loaded();
    const form = await open(container);
    await type(form, 'api.github.com', 'x');
    await type(form, 'api.github.com', '');
    expect(text(form)).toContain('Enter a host.');
    expect(btn(form, 'Add').disabled).toBe(true);
  });

  it('Cancel closes the form; the form shows the role picker for a team project', async () => {
    const { container } = await loaded();
    const form = await open(container);
    await waitFor(() => expect(text(form)).toContain('Only for role'));
    await fireEvent.click(btn(form, 'Cancel'));
    expect(container.querySelector('form[aria-label="Add an allowance"]')).toBeNull();
  });

  it('a solo project has no role picker', async () => {
    replies.teams_view = { teams: [], roles: [] };
    const { container } = await loaded();
    const form = await open(container);
    await tick(20);
    expect(text(form)).not.toContain('Only for role');
  });
});

describe('remove', () => {
  const withRow = () => {
    replies.sandbox_list = [
      globalScope(),
      projectScope({ entries: [entry('s1', 'network', 'api.github.com', { note: 'n' })] }),
    ];
  };

  it('removes at once into an Undo line, with the applies line; Undo adds it back with the same fields', async () => {
    withRow();
    replies.sandbox_remove = {
      removed: entry('s1', 'network', 'api.github.com'),
      file: 'project:shop-api',
      applies: 'next_start',
      running: 0,
    };
    replies.sandbox_add = {
      entry: entry('s5', 'network', 'api.github.com'),
      applies: 'next_start',
      running: 0,
      created: true,
    };
    const { container } = await loaded();
    const p = projectBlock(container);
    replies.sandbox_list = [globalScope(), projectScope()];
    await fireEvent.click(p.querySelector('[aria-label="Remove api.github.com"]'));
    await waitFor(() => expect(text(p)).toContain('Removed api.github.com'));
    expect(calls).toContainEqual(['sandbox_remove', { id: 's1', file: 'project:shop-api' }]);
    expect(text(p)).toContain('Applies when an agent next starts.');
    await fireEvent.click(p.querySelector('[aria-label="Undo removing api.github.com"]'));
    await waitFor(() => expect(calls.find((c) => c[0] === 'sandbox_add')).toBeTruthy());
    expect(calls.find((c) => c[0] === 'sandbox_add')[1]).toEqual({
      scope: 'project',
      project: 'shop-api',
      kind: 'network',
      value: 'api.github.com',
      role: null,
      note: 'n',
    });
  });

  it('a failed remove says why and keeps the row', async () => {
    withRow();
    replies.sandbox_remove = new Error('locked');
    const { container } = await loaded();
    await fireEvent.click(container.querySelector('[aria-label="Remove api.github.com"]'));
    await waitFor(() => expect(text(container)).toContain('Could not remove it: locked.'));
    expect(container.querySelector('[aria-label="Remove api.github.com"]')).not.toBeNull();
  });
});

describe('refresh', () => {
  it('the `sandbox` topic reloads the list and highlights a row added elsewhere', async () => {
    const { container } = await loaded();
    replies.sandbox_list = [globalScope(), projectScope({ entries: [entry('s7', 'write', '/x/y')] })];
    applyNow({ rev: app.now.rev + 1, topics: ['sandbox'] });
    await waitFor(() => expect(container.querySelector('[data-row="s7"]')).not.toBeNull());
    expect(container.querySelector('[data-row="s7"]').classList.contains('hl')).toBe(true);
    expect(container.querySelector('[data-applies]')).toBeNull();
  });
});

describe('appliesLine', () => {
  it.each([
    [{ applies: 'live', running: 3 }, 'Applies now.'],
    [{ applies: 'next_start', running: 0 }, 'Applies when an agent next starts.'],
    [{ applies: 'next_start', running: 1 }, 'Applies when an agent next starts. 1 agent running.'],
    [{ applies: 'next_start', running: 2 }, 'Applies when an agent next starts. 2 agents running.'],
  ])('%j', (r, want) => expect(appliesLine(r)).toBe(want));
});
