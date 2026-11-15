// Settings → Permissions, the rules part (allow-fine-tune-sandbox-permissions, ux-flows §4): the rules of
// each file with their chips, Remove with Undo, the Add rule form (the core builds the pattern), Test a
// call and its results, the hint to Sandbox. Against a mocked core.

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
const { testResult } = await import('../src/lib/policy.js');
const { default: PermissionsSettings } = await import('../src/components/PermissionsSettings.svelte');

const text = (el) => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const btn = (c, label) => [...c.querySelectorAll('button')].find((b) => text(b) === label);

const files = [
  { scope: 'global', file: 'global', path: '/app/policy.toml', exists: true, rules: 1, sandbox: 0, warnings: [] },
  {
    scope: 'project',
    project: 'shop-api',
    file: 'project:shop-api',
    path: '/code/shop-api/.orchestrator/policy.toml',
    exists: true,
    rules: 1,
    sandbox: 0,
    warnings: [],
  },
];
const rule = (id, file, extra = {}) => ({
  id,
  match: 'Bash(cargo test:*)',
  effect: 'allow',
  summary: 'Bash commands starting with `cargo test`',
  file,
  path: '/x',
  ...extra,
});

beforeEach(() => {
  cleanup();
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.window = null;
  replies.policy_files = files;
  replies.policy_defaults = { team: 'overwatch', solo: 'user' };
  replies.settings_get = 300;
  replies.policy_rules = [
    rule('r12', 'project:shop-api', { role: 'back', note: 'tests' }),
    rule('r1', 'global', { effect: 'deny', match: 'Bash(rm:*)', summary: 'Bash commands starting with `rm`' }),
  ];
  replies.teams_view = { teams: [], roles: [{ id: 'back', name: 'Backend' }] };
});
afterEach(cleanup);

async function loaded() {
  const r = render(PermissionsSettings);
  await waitFor(() => expect(r.container.querySelector('[data-rule]')).not.toBeNull());
  return r;
}
const block = (c, file) => c.querySelector(`[data-rules-file="${file}"]`);

describe('the page', () => {
  it('explains permissions vs sandbox and links to Sandbox settings', async () => {
    const { container } = await loaded();
    const e = container.querySelector('[data-explainer]');
    expect(text(e)).toContain(
      "Permissions decide which calls need approval. The sandbox decides what an agent's shell commands can reach. A rule can't widen the sandbox.",
    );
    expect(text(e)).toContain(
      'A command fails with "Operation not permitted", or a host is blocked? That\'s the sandbox, not a permission.',
    );
    await fireEvent.click(btn(e, 'Open Sandbox settings'));
    expect(app.window).toMatchObject({ kind: 'settings', section: 'sandbox' });
    expect(text(container)).toContain(
      'Rules are also added from cards and removed from chat lines. The files are plain text.',
    );
  });
});

describe('rules list', () => {
  it('shows each rule under its file: sentence, effect chip, role chip, note, pattern, id', async () => {
    const { container } = await loaded();
    const p = text(block(container, 'project:shop-api'));
    expect(p).toContain('Bash commands starting with `cargo test`');
    expect(p).toContain('Allow');
    expect(p).toContain('back only');
    expect(p).toContain('tests');
    expect(p).toContain('Bash(cargo test:*)');
    expect(p).toContain('r12');
    expect(text(block(container, 'global'))).toContain('Deny');
  });

  it('effect chips read as the core says: Ask me, The Overwatch decides', async () => {
    replies.policy_rules = [rule('r2', 'global', { effect: 'user' }), rule('r3', 'global', { effect: 'overwatch' })];
    const { container } = await loaded();
    const t = text(block(container, 'global'));
    expect(t).toContain('Ask me');
    expect(t).toContain('The Overwatch decides');
  });

  it('loading, error with Retry, and an empty file', async () => {
    replies.policy_rules = new Error('boom');
    const { container } = render(PermissionsSettings);
    await waitFor(() => expect(text(container)).toContain('Could not read the rules: boom'));
    replies.policy_rules = [];
    await fireEvent.click(btn(container, 'Retry'));
    await waitFor(() => expect(container.querySelectorAll('[data-no-rules]')).toHaveLength(2));
    expect(text(container)).toContain('No rules here. Anything not matched follows the defaults below.');
  });

  it('a parse error: Add rule and Remove are disabled with the line', async () => {
    replies.policy_files = [files[0], { ...files[1], error: { line: 7, message: 'bad' } }];
    const { container } = await loaded();
    const p = block(container, 'project:shop-api');
    expect(btn(p, 'Add rule').disabled).toBe(true);
    expect(btn(p, 'Add rule').title).toBe('Fix the file first: line 7.');
    expect(p.querySelector('[aria-label^="Remove"]').disabled).toBe(true);
  });

  it('Remove goes at once and leaves an Undo line; Undo adds the same rule back', async () => {
    replies.revoke_rule = { revoked: true };
    replies.add_rule = { rule: rule('r13', 'project:shop-api'), file: 'project:shop-api', path: '/x' };
    const { container } = await loaded();
    const p = block(container, 'project:shop-api');
    replies.policy_rules = [rule('r1', 'global')];
    await fireEvent.click(p.querySelector('[aria-label="Remove Bash commands starting with `cargo test`"]'));
    await waitFor(() => expect(text(p)).toContain('Removed Bash commands starting with `cargo test`'));
    expect(calls).toContainEqual(['revoke_rule', { id: 'r12', file: 'project:shop-api' }]);
    await fireEvent.click(p.querySelector('[aria-label^="Undo removing"]'));
    await waitFor(() => expect(calls.find((c) => c[0] === 'add_rule')).toBeTruthy());
    expect(calls.find((c) => c[0] === 'add_rule')[1]).toEqual({
      scope: 'project',
      scope_id: 'shop-api',
      pattern: 'Bash(cargo test:*)',
      effect: 'allow',
      role: 'back',
      note: 'tests',
    });
  });

  it('a failed Remove says why', async () => {
    replies.revoke_rule = new Error('locked');
    const { container } = await loaded();
    await fireEvent.click(container.querySelector('[aria-label="Remove Bash commands starting with `rm`"]'));
    await waitFor(() => expect(text(container)).toContain('Could not remove it: locked.'));
  });
});

describe('add rule', () => {
  async function open(container, file = 'project:shop-api') {
    await fireEvent.click(btn(block(container, file), 'Add rule'));
    return container.querySelector('form[aria-label="Add a rule"]');
  }

  it('the preview comes from the core, then Add rule posts the pattern with scope, role and note', async () => {
    replies.rule_preview = { pattern: 'Bash(cargo test:*)', summary: 'Bash commands starting with `cargo test`' };
    replies.add_rule = { rule: rule('r20', 'project:shop-api'), file: 'project:shop-api', path: '/x' };
    const { container } = await loaded();
    const form = await open(container);
    expect(btn(form, 'Add rule').disabled).toBe(true);
    await fireEvent.input(form.querySelector('input[placeholder="cargo test"]'), { target: { value: 'cargo test' } });
    await waitFor(() => expect(form.querySelector('[data-preview]')).not.toBeNull(), { timeout: 2000 });
    expect(text(form.querySelector('[data-preview]'))).toBe(
      'This rule matches Bash(cargo test:*) Bash commands starting with `cargo test`',
    );
    expect(calls.filter((c) => c[0] === 'rule_preview').at(-1)[1]).toEqual({ tool: 'bash', value: 'cargo test' });
    await fireEvent.input(form.querySelector('input[placeholder="Optional"]'), { target: { value: 'tests' } });
    await fireEvent.click(btn(form, 'Add rule'));
    await waitFor(() => expect(calls.find((c) => c[0] === 'add_rule')).toBeTruthy());
    expect(calls.find((c) => c[0] === 'add_rule')[1]).toEqual({
      scope: 'project',
      scope_id: 'shop-api',
      pattern: 'Bash(cargo test:*)',
      effect: 'allow',
      role: null,
      note: 'tests',
    });
    await waitFor(() => expect(container.querySelector('form[aria-label="Add a rule"]')).toBeNull());
  });

  it('a preview error shows the core message and keeps Add rule disabled', async () => {
    replies.rule_preview = new Error('A path must be absolute.');
    const { container } = await loaded();
    const form = await open(container);
    await fireEvent.input(form.querySelector('input[placeholder="cargo test"]'), { target: { value: 'x' } });
    await waitFor(() => expect(text(form)).toContain('A path must be absolute.'), { timeout: 2000 });
    expect(btn(form, 'Add rule').disabled).toBe(true);
  });

  it('a refused add stays open with the reason', async () => {
    replies.rule_preview = { pattern: 'Bash(git push:*)', summary: 's' };
    replies.add_rule = new Error("git push is the core's");
    const { container } = await loaded();
    const form = await open(container);
    await fireEvent.input(form.querySelector('input[placeholder="cargo test"]'), { target: { value: 'git push' } });
    await waitFor(() => expect(btn(form, 'Add rule').disabled).toBe(false), { timeout: 2000 });
    await fireEvent.click(btn(form, 'Add rule'));
    await waitFor(() => expect(text(form)).toContain("Could not add the rule: git push is the core's."));
  });

  it('the global file adds with the global scope', async () => {
    replies.rule_preview = { pattern: 'WebFetch(domain:docs.rs)', summary: 's' };
    replies.add_rule = { rule: rule('r30', 'global'), file: 'global', path: '/x' };
    const { container } = await loaded();
    const form = await open(container, 'global');
    await fireEvent.click(form.querySelector('[aria-label="When an agent wants to"]'));
    await fireEvent.click(
      [...document.querySelectorAll('[role="option"]')].find((o) => text(o) === 'fetch a web page'),
    );
    await fireEvent.input(form.querySelector('input[placeholder="docs.rs"]'), { target: { value: 'docs.rs' } });
    await waitFor(() => expect(btn(form, 'Add rule').disabled).toBe(false), { timeout: 2000 });
    await fireEvent.click(btn(form, 'Add rule'));
    await waitFor(() =>
      expect(calls.find((c) => c[0] === 'add_rule')[1]).toMatchObject({
        scope: 'global',
        scope_id: null,
        pattern: 'WebFetch(domain:docs.rs)',
      }),
    );
  });

  it('"do anything" + Allow warns and needs "I understand"', async () => {
    replies.rule_preview = { pattern: '*', summary: 'Any tool call' };
    const { container } = await loaded();
    const form = await open(container);
    await fireEvent.click(form.querySelector('[aria-label="When an agent wants to"]'));
    await fireEvent.click([...document.querySelectorAll('[role="option"]')].find((o) => text(o) === 'do anything'));
    await waitFor(() => expect(form.querySelector('[data-preview]')).not.toBeNull(), { timeout: 2000 });
    expect(text(form)).toContain('This allows every call.');
    expect(btn(form, 'Add rule').disabled).toBe(true);
    await fireEvent.click(form.querySelector('input[type="checkbox"]'));
    expect(btn(form, 'Add rule').disabled).toBe(false);
  });
});

describe('test a call', () => {
  async function run(container, value = 'cargo test --workspace') {
    await fireEvent.input(container.querySelector('input[placeholder="cargo test --workspace"]'), {
      target: { value },
    });
    await fireEvent.click(btn(container, 'Test'));
  }

  it('Test is disabled while the input is empty', async () => {
    const { container } = await loaded();
    expect(btn(container, 'Test').disabled).toBe(true);
  });

  it('a matching rule: effect, rule id and file, its summary; the id jumps to the row', async () => {
    replies.policy_test = {
      tool: 'Bash',
      input: {},
      role: 'back',
      evaluation: { effect: 'allow', source: { kind: 'policy', id: 'r12', file: 'project:shop-api' } },
      hard: null,
    };
    const { container } = await loaded();
    await run(container);
    await waitFor(() => expect(container.querySelector('[data-test-result]')).not.toBeNull());
    expect(text(container.querySelector('[data-test-result]'))).toContain(
      "Allow · rule r12 (shop-api's file): Bash commands starting with `cargo test`",
    );
    expect(calls.find((c) => c[0] === 'policy_test')[1]).toEqual({
      project: 'shop-api',
      role: null,
      solo: false,
      tool: 'bash',
      value: 'cargo test --workspace',
    });
    await fireEvent.click(btn(container, 'Show rule r12'));
    expect(container.querySelector('[data-rule="project:shop-api:r12"]').classList.contains('hl')).toBe(true);
  });

  it('no rule: the default for team roles', async () => {
    replies.policy_test = { evaluation: { effect: 'overwatch', source: { kind: 'default' } }, hard: null };
    const { container } = await loaded();
    await run(container);
    await waitFor(() =>
      expect(text(container.querySelector('[data-test-result]'))).toBe(
        'The Overwatch decides · no rule matches; the default for team roles applies.',
      ),
    );
  });

  it('decided by the core: only you can allow this', async () => {
    replies.policy_test = {
      evaluation: { effect: 'user', source: { kind: 'hard', why: 'git push: the core pushes' } },
      hard: { kind: 'user_only', why: 'git push: the core pushes' },
    };
    const { container } = await loaded();
    await run(container, 'git push');
    await waitFor(() =>
      expect(text(container.querySelector('[data-test-result]'))).toBe(
        'Only you can allow this · git push: the core pushes.',
      ),
    );
  });

  it('the sandbox hint shows for a shell command with a URL only', async () => {
    replies.policy_test = { evaluation: { effect: 'user', source: { kind: 'default' } }, hard: null };
    const { container } = await loaded();
    await run(container, 'curl https://example.com');
    await waitFor(() =>
      expect(text(container.querySelector('[data-test-result]'))).toContain(
        "Passing this test doesn't mean the command can reach it. Check the sandbox: Open Sandbox settings.",
      ),
    );
    cleanup();
    const again = await loaded();
    await run(again.container, 'cargo test');
    await waitFor(() => expect(again.container.querySelector('[data-test-result]')).not.toBeNull());
    expect(text(again.container.querySelector('[data-test-result]'))).not.toContain('Passing this test');
  });

  it('an error: could not test the call', async () => {
    replies.policy_test = new Error("Test a real host, like api.docs.rs: a wildcard isn't a call.");
    const { container } = await loaded();
    await run(container);
    await waitFor(() =>
      expect(text(container)).toContain(
        "Could not test the call: Test a real host, like api.docs.rs: a wildcard isn't a call.",
      ),
    );
  });
});

describe('testResult', () => {
  it('solo sessions read "solo sessions"', () => {
    expect(testResult({ evaluation: { effect: 'user', source: { kind: 'default' } } }, { solo: true }).rest).toBe(
      'no rule matches; the default for solo sessions applies.',
    );
  });
});
