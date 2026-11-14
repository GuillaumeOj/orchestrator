// The `sandbox` Inbox card (allow-fine-tune-sandbox-permissions, ux-flows §2): title and subject by kind,
// the scope choice, invalid / error / stale / decided states, Undo of an Allow, and the decision's scope.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { decision, fixture, nowOf } from './fixture.js';

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

const { app, decide, flushDecisions } = await import('../src/lib/app.svelte.js');
const { keyIntent, UNDO_MS } = await import('../src/lib/inbox.js');
const { cardTitle, decidedText } = await import('../src/lib/sandbox.js');
const { default: DecisionCard } = await import('../src/components/chat/DecisionCard.svelte');
const { default: ChatLine } = await import('../src/components/chat/ChatLine.svelte');

const text = (el) => el.textContent.replace(/\s+/g, ' ').trim();
const buttons = (c) => [...c.querySelectorAll('button')].map((b) => text(b));
const press = (c, label) => fireEvent.click([...c.querySelectorAll('button')].find((b) => text(b) === label));

const card = (over = {}, payload = {}) =>
  decision({
    id: 'sandbox-7',
    kind: 'sandbox',
    title: 'Sandbox · network api.github.com',
    subject: { agent: 'back-1', project: 'shop-api', role: 'Backend' },
    context: [
      { type: 'markdown', text: 'The release check needs the GitHub API.' },
      { type: 'sandbox', kind: 'network', value: 'api.github.com', project: 'shop-api', role: 'back', invalid: null },
    ],
    actions: [
      { id: 'approve', label: 'Allow', key: 'a' },
      { id: 'reject', label: 'Deny', key: 'r' },
      { id: 'reply', label: 'Deny with message', key: 'enter' },
    ],
    payload: {
      kind: 'network',
      value: 'api.github.com',
      why: 'The release check needs the GitHub API.',
      project: 'shop-api',
      role: 'back',
      agent: 'back-1',
      ...payload,
    },
    ...over,
  });

beforeEach(() => {
  calls.length = 0;
  for (const k of Object.keys(replies)) delete replies[k];
  app.data = fixture();
  app.now = nowOf(app.data);
  app.pending = [];
  app.toasts = [];
  app.decideErrors = {};
});
afterEach(cleanup);

describe('titles by kind', () => {
  it.each([
    [{ kind: 'network', value: 'api.docs.rs' }, 'back-1 wants to reach api.docs.rs'],
    [{ kind: 'write', value: '~/.cache/ms-playwright' }, 'back-1 wants to write ~/.cache/ms-playwright'],
    [{ kind: 'local_ports', value: null }, 'back-1 wants to open local ports (test servers)'],
    [{ kind: 'unix_socket', value: '/tmp/x.sock' }, 'back-1 wants to use the socket /tmp/x.sock'],
  ])('%j', (payload, title) => {
    expect(cardTitle(card({}, payload))).toBe(title);
  });
});

describe('open card', () => {
  it('shows the kind, title, subject, reason, explainer, actions and the restart line', () => {
    const { container } = render(DecisionCard, { d: card() });
    const t = text(container);
    expect(t).toContain('Sandbox');
    expect(t).toContain('back-1 wants to reach api.github.com');
    expect(t).toContain('shop-api · back');
    expect(t).toContain('The release check needs the GitHub API.');
    expect(t).toContain("Widens what back-1's shell commands can reach. Permissions rules can't do this.");
    expect(buttons(container)).toEqual(expect.arrayContaining(['Allow for this project', 'Reject', 'Reply']));
    expect(t).toContain('back-1 restarts and retries.');
    expect(container.querySelector('[data-block="sandbox"]')).toBeNull();
  });

  it('a solo session reads "solo" and has no role item in the scope menu', async () => {
    const d = card({}, { role: null });
    const { container } = render(DecisionCard, { d });
    expect(text(container)).toContain('shop-api · solo');
    await fireEvent.click(container.querySelector('button[aria-label="More ways to allow"]'));
    const items = [...document.querySelectorAll('[role="menuitem"]')].map(text);
    expect(items).toEqual(['Allow everywhere']);
  });

  it('the caret menu offers everywhere and the role, and passes the scope', async () => {
    const onscope = vi.fn();
    const { container } = render(DecisionCard, { d: card(), onscope });
    await fireEvent.click(container.querySelector('button[aria-label="More ways to allow"]'));
    const items = [...document.querySelectorAll('[role="menuitem"]')];
    expect(items.map(text)).toEqual(['Allow everywhere', 'Allow for back only']);
    await fireEvent.click(items[1]);
    expect(onscope).toHaveBeenCalledWith('role');
  });

  it('the primary, Reject and Reply press their keys', async () => {
    const onkey = vi.fn();
    const { container } = render(DecisionCard, { d: card(), onkey });
    await press(container, 'Allow for this project');
    await press(container, 'Reject');
    await press(container, 'Reply');
    expect(onkey.mock.calls.map((c) => c[0])).toEqual(['a', 'r', 'y']);
  });

  it('a GitHub host adds the login line; another host does not', () => {
    const gh = render(DecisionCard, { d: card() });
    expect(text(gh.container)).toContain(
      'Without your GitHub login: issues and pull requests still go through the orchestrator.',
    );
    cleanup();
    const other = render(DecisionCard, { d: card({}, { value: 'docs.rs' }) });
    expect(text(other.container)).not.toContain('GitHub login');
  });

  it('parked: the kind line says so', () => {
    const { container } = render(DecisionCard, { d: card({ lifecycle: { state: 'parked' } }) });
    expect(text(container)).toContain('Sandbox · parked');
  });
});

describe('invalid, error, stale', () => {
  it('invalid: no Allow, the reason, Reject and Reply only', () => {
    const { container } = render(DecisionCard, {
      d: card({}, { invalid: '`~/.ssh` is a credential store: never allowed' }),
    });
    expect(buttons(container)).not.toContain('Allow for this project');
    expect(buttons(container)).toEqual(expect.arrayContaining(['Reject', 'Reply']));
    expect(container.querySelector('[data-invalid]').textContent).toContain(
      "Can't be allowed: `~/.ssh` is a credential store: never allowed",
    );
    expect(text(container)).toContain('The agent is told no. Reply to suggest something narrower.');
  });

  it('a refused Allow keeps the card open with the reason', () => {
    app.decideErrors = { 'sandbox-7': 'Could not allow it: disk full. Nothing was changed.' };
    const { container } = render(DecisionCard, { d: card() });
    expect(container.querySelector('[role="alert"]').textContent).toBe(
      'Could not allow it: disk full. Nothing was changed.',
    );
    expect(buttons(container)).toContain('Allow for this project');
  });

  it('stale: only Dismiss, and the line says nothing is retried', async () => {
    const onact = vi.fn();
    const { container } = render(DecisionCard, { d: card({ lifecycle: { state: 'stale' } }), onact });
    expect(text(container)).toContain('back-1 is no longer running. Nothing to retry.');
    expect(buttons(container)).toContain('Dismiss');
    expect(buttons(container)).not.toContain('Allow for this project');
    await press(container, 'Dismiss');
    expect(onact).toHaveBeenCalledWith('dismiss');
  });
});

describe('decided', () => {
  const settled = (scope, created = true, extra = {}) =>
    card(
      { lifecycle: { state: 'effect', effect: 'allowed', decided_by: 'user' } },
      {
        outcome: {
          entry: { id: 's4', kind: 'network', value: 'api.github.com', file: 'project:shop-api', ...extra },
          created,
          scope,
        },
      },
    );
  const line = { verb: 'allowed', agent: 'back-1', what: 'x', by: 'you', state: 'effect', at_ms: Date.now() };

  it.each([
    ['project', 'Allowed for shop-api by you'],
    ['global', 'Allowed for all projects by you'],
    ['role', 'Allowed for back in shop-api by you'],
  ])('%s', (scope, want) => {
    expect(decidedText(settled(scope), (id) => id)).toBe(want);
    const { container } = render(DecisionCard, { d: settled(scope), line });
    expect(text(container.querySelector('[data-sandbox-decided]'))).toBe(`${want} · Undo`);
  });

  it('an allowance that already existed has no Undo', () => {
    const { container } = render(DecisionCard, { d: settled('project', false), line });
    expect(text(container.querySelector('[data-sandbox-decided]'))).toBe('Already allowed for shop-api');
  });

  it('Undo removes the entry it wrote and says when the agent loses it', async () => {
    replies.sandbox_remove = { removed: {}, applies: 'next_start', running: 1 };
    const { container } = render(DecisionCard, { d: settled('project'), line });
    await press(container, 'Undo');
    expect(calls).toContainEqual(['sandbox_remove', { id: 's4', file: 'project:shop-api' }]);
    await waitFor(() =>
      expect(text(container.querySelector('[data-sandbox-decided]'))).toBe(
        'Allowance removed · back-1 keeps it until it next starts',
      ),
    );
  });

  it('a failed Undo keeps the line and says why', async () => {
    replies.sandbox_remove = new Error('no entry s4');
    const { container } = render(DecisionCard, { d: settled('project'), line });
    await press(container, 'Undo');
    await waitFor(() =>
      expect(container.querySelector('[role="alert"]').textContent).toBe('Could not remove it: no entry s4.'),
    );
    expect(buttons(container)).toContain('Undo');
  });

  it('the chat line says allowed / denied with the card inside', () => {
    const { container } = render(ChatLine, {
      item: { id: 'sandbox-7', key: 'k', d: settled('project'), entry: {}, line },
      expanded: true,
    });
    expect(container.querySelector('[data-sandbox-decided]')).not.toBeNull();
  });
});

describe('deciding', () => {
  it('Allow with a scope reaches the core with it; the default sends none', async () => {
    replies.inbox_act = { ok: true };
    const d = card();
    app.now.decisions[d.id] = d;
    decide(d, 'approve', null, 'global');
    expect(app.pending[0]).toMatchObject({ item: 'sandbox-7', action: 'approve', scope: 'global' });
    await flushDecisions(Date.now() + UNDO_MS + 1);
    expect(calls).toContainEqual([
      'inbox_act',
      { item: 'sandbox-7', action: 'approve', scope: 'global', message: null },
    ]);
  });

  it('the core refusing an Allow shows the reason on the card and toasts it', async () => {
    replies.inbox_act = new Error('`~/.ssh` is a credential store: never allowed');
    const d = card();
    app.now.decisions[d.id] = d;
    decide(d, 'approve');
    await flushDecisions(Date.now() + UNDO_MS + 1);
    expect(app.decideErrors['sandbox-7']).toBe(
      'Could not allow it: `~/.ssh` is a credential store: never allowed. Nothing was changed.',
    );
  });

  it('keys: ⇧a opens the scope menu, y replies', () => {
    const d = card();
    expect(keyIntent('A', d)).toEqual({ kind: 'scope-menu' });
    expect(keyIntent('y', d)).toEqual({ kind: 'reply' });
    expect(keyIntent('A', decision({ kind: 'permission' }))).toBeNull();
  });
});
