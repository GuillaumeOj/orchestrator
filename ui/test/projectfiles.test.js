// Project settings › Files: the helpers and the section against a mocked core.

import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';
import { tick } from 'svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { app } from '../src/lib/app.svelte.js';
import { classify } from '../src/lib/keyboard.js';
import { addedToast, inRect, midTruncate, pathLines, tilde } from '../src/lib/projectfiles.js';
import { pick, picked } from './pick.js';

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

const drag = { handler: null, off: vi.fn() };
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: (fn) => {
      drag.handler = fn;
      return Promise.resolve(drag.off);
    },
  }),
}));

const { default: ProjectSettingsWindow } = await import('../src/components/ProjectSettingsWindow.svelte');

const file = (path, over = {}) => ({ path, kind: 'file', access: 'write', added_at_ms: 1, exists: true, ...over });
const HOME = '/Users/me';
const LIST = [
  file(`${HOME}/Brand`, { kind: 'folder', note: 'Brand guide' }),
  file(`${HOME}/design/app.figcraft`),
  file(`${HOME}/old/pricing.pdf`, { exists: false }),
];
const sent = (cmd) => calls.filter(([c]) => c === cmd).map(([, a]) => a);

describe('project files helpers', () => {
  it('splits lines, shortens home, truncates the middle', () => {
    expect(pathLines(' ~/a \n\n/b\r\n')).toEqual(['~/a', '/b']);
    expect(tilde(`${HOME}/x: holds credentials in ${HOME}`, HOME)).toBe('~/x: holds credentials in /Users/me');
    expect(tilde(`/mnt${HOME}/x`, HOME)).toBe(`/mnt${HOME}/x`);
    expect(tilde(HOME, HOME)).toBe('~');
    expect(tilde('/other/x', '')).toBe('/other/x');
    expect(tilde('/Users/media/x', HOME)).toBe('/Users/media/x');
    expect(midTruncate('~/a')).toBe('~/a');
    const m = midTruncate(`~/${'d/'.repeat(40)}app.figcraft`);
    expect(m.length).toBeLessThanOrEqual(44);
    expect(m).toContain('…');
    expect(m.endsWith('app.figcraft')).toBe(true);
    expect(addedToast(1, 'p')).toBe('Added 1 item to p');
    expect(addedToast(3, 'p')).toBe('Added 3 items to p');
  });

  it('hit-tests a physical position against a css rect', () => {
    const rect = { left: 10, top: 10, right: 110, bottom: 110, width: 100, height: 100 };
    expect(inRect({ x: 40, y: 40 }, rect, 2)).toBe(true);
    expect(inRect({ x: 40, y: 40 }, rect, 1)).toBe(true);
    expect(inRect({ x: 400, y: 40 }, rect, 2)).toBe(false);
    expect(inRect({ x: 40, y: 40 }, { ...rect, width: 0 }, 1)).toBe(false);
  });

  it('lets ⌘O through outside terminals', () => {
    const e = { key: 'o', code: 'KeyO', metaKey: true };
    expect(classify(e, {}).kind).toBe('pass');
    expect(classify(e, { terminal: true }).kind).toBe('swallow');
  });
});

describe('Project settings › Files', () => {
  beforeEach(() => {
    cleanup();
    calls.length = 0;
    for (const k of Object.keys(replies)) delete replies[k];
    drag.handler = null;
    drag.off.mockClear();
    app.toasts = [];
    app.data = { core: { user_home: HOME } };
    replies.project_files = { files: LIST, live_agents: 2 };
    app.window = { kind: 'project-settings', projectId: 'shop-api', section: 'files' };
  });

  async function open() {
    const r = render(ProjectSettingsWindow);
    await waitFor(() => expect(r.container.querySelector('.files h2')).toBeTruthy());
    await waitFor(() => expect(r.container.querySelector('[aria-busy]')).toBeNull());
    return { ...r, root: r.container.querySelector('[data-pane-focus]') };
  }
  const btn = (c, re) => [...c.querySelectorAll('button')].find((b) => re.test(b.textContent.trim()));
  const rows = (c) => [...c.querySelectorAll('.row')];
  const toasts = () => app.toasts.map((t) => t.text);
  const rect = { left: 0, top: 0, right: 5000, bottom: 5000, width: 5000, height: 5000 };

  it('sits second in the nav and lists rows with ~ paths, missing badge and hints', async () => {
    const { container, getByText } = await open();
    const nav = [...container.querySelectorAll('[role="tab"]')].map((b) => b.textContent.trim());
    expect(nav.slice(0, 3)).toEqual(['General', 'Files', 'Tracker']);
    expect(rows(container)).toHaveLength(3);
    expect(container.textContent).toContain('~/Brand');
    expect(container.textContent).toContain('▲ missing');
    expect(getByText(/Agents may change these files/)).toBeTruthy();
    expect(getByText(/2 live now/)).toBeTruthy();
    expect(rows(container)[2].querySelector('[aria-label="Reveal in Finder"]').disabled).toBe(true);
    expect(rows(container)[0].querySelector('[aria-label="Open in editor"]')).toBeNull();
  });

  it('keeps Tracker as the default section', async () => {
    app.window = { kind: 'project-settings', projectId: 'shop-api' };
    const { findByText } = render(ProjectSettingsWindow);
    await findByText(/Where this project's work lives/, {}, { timeout: 2000 }).catch(() =>
      findByText(/Could not read shop-api's tracker/),
    );
  });

  it('hides the write hint when all rows are read only and the live note at 0 agents', async () => {
    replies.project_files = { files: [file(`${HOME}/a`, { access: 'read' })], live_agents: 0 };
    const { container } = await open();
    expect(container.textContent).not.toContain('Agents may change these files');
    expect(container.textContent).not.toContain('live now');
  });

  it('shows a load error and retries with r', async () => {
    replies.project_files = new Error('could not read the settings store');
    const { container, getByText } = render(ProjectSettingsWindow);
    await waitFor(() => getByText(/Couldn't load shop-api's files: could not read the settings store/));
    replies.project_files = { files: LIST, live_agents: 0 };
    await fireEvent.keyDown(container.querySelector('[data-pane-focus]'), { key: 'r' });
    await waitFor(() => expect(rows(container)).toHaveLength(3));
  });

  it('adds a batch from the browse panel and lists the refusals', async () => {
    const { container, getByText } = await open();
    replies.pick_paths = [`${HOME}/Docs/g.pdf`, `${HOME}/.ssh`];
    replies.project_files_add = {
      files: [...LIST, file(`${HOME}/Docs/g.pdf`)],
      added: [`${HOME}/Docs/g.pdf`],
      refused: [{ path: `${HOME}/.ssh`, error: `${HOME}/.ssh: holds credentials, can't be shared with agents` }],
    };
    await fireEvent.click(container.querySelector('.zone'));
    await waitFor(() => getByText('Not added (1)'));
    expect(sent('project_files_add')).toEqual([{ project: 'shop-api', paths: [`${HOME}/Docs/g.pdf`, `${HOME}/.ssh`] }]);
    expect(container.textContent).toContain("~/.ssh: holds credentials, can't be shared with agents");
    expect(toasts()).toContain('Added 1 item to shop-api');
    expect(rows(container)).toHaveLength(4);
    await fireEvent.click(container.querySelector('[aria-label="Dismiss"]'));
    expect(container.textContent).not.toContain('Not added');
  });

  it('adds nothing and says why when the whole batch is refused; cancel adds nothing', async () => {
    const { container, getByText } = await open();
    replies.pick_paths = [];
    await fireEvent.click(container.querySelector('.zone'));
    await tick();
    expect(sent('project_files_add')).toEqual([]);
    replies.pick_paths = ['/x'];
    replies.project_files_add = {
      files: LIST,
      added: [],
      refused: [{ path: '/x', error: '/x: no such file or folder' }],
    };
    await fireEvent.click(container.querySelector('.zone'));
    await waitFor(() => getByText('Not added (1)'));
    expect(toasts()).toEqual([]);
  });

  it('adds pasted lines, one path each, on Enter', async () => {
    const { container } = await open();
    const field = container.querySelector('textarea');
    replies.project_files_add = { files: LIST, added: ['/a', '/b'], refused: [] };
    await fireEvent.input(field, { target: { value: '~/a\n\n~/b' } });
    await fireEvent.keyDown(field, { key: 'Enter' });
    await waitFor(() => expect(sent('project_files_add')).toEqual([{ project: 'shop-api', paths: ['~/a', '~/b'] }]));
    await waitFor(() => expect(field.value).toBe(''));
  });

  it('adds a drop over the section and ignores one elsewhere', async () => {
    const { container } = await open();
    expect(drag.handler).toBeTypeOf('function');
    const body = container.querySelector('.body');
    body.getBoundingClientRect = () => /** @type {DOMRect} */ (rect);
    replies.project_files_add = { files: LIST, added: ['/d'], refused: [] };
    drag.handler({ payload: { type: 'over', position: { x: 50, y: 50 } } });
    await waitFor(() => expect(container.querySelector('.zone.drag')).toBeTruthy());
    expect(container.textContent).toContain('Drop to add to shop-api');
    drag.handler({ payload: { type: 'drop', paths: ['/outside'], position: { x: 9000, y: 50 } } });
    await tick();
    expect(sent('project_files_add')).toEqual([]);
    expect(container.querySelector('.zone.drag')).toBeNull();
    drag.handler({ payload: { type: 'drop', paths: ['/d'], position: { x: 50, y: 50 } } });
    await waitFor(() => expect(sent('project_files_add')).toEqual([{ project: 'shop-api', paths: ['/d'] }]));
    drag.handler({ payload: { type: 'over', position: { x: 50, y: 50 } } });
    await waitFor(() => expect(container.querySelector('.zone.drag')).toBeTruthy());
    drag.handler({ payload: { type: 'leave' } });
    await waitFor(() => expect(container.querySelector('.zone.drag')).toBeNull());
  });

  it('listens to drops only while the section is shown', async () => {
    const { container } = await open();
    await fireEvent.click(btn(container, /^Tracker$/));
    await waitFor(() => expect(drag.off).toHaveBeenCalled());
  });

  it('toggles access by click and with a, and reports a core refusal', async () => {
    const { container, root } = await open();
    replies.project_file_update = (a) => ({
      files: LIST.map((f) => (f.path === a.path ? { ...f, access: a.access } : f)),
    });
    await pick(rows(container)[1], 'Read only');
    await waitFor(() => expect(toasts()).toContain('~/design/app.figcraft is now Read only'));
    expect(sent('project_file_update')[0]).toEqual({
      project: 'shop-api',
      path: `${HOME}/design/app.figcraft`,
      access: 'read',
    });
    expect(picked(rows(container)[1])).toBe('read');

    replies.project_file_update = new Error(
      '/Users/me/Brand: contains files set to Read & write; set them to Read only first',
    );
    await fireEvent.keyDown(root, { key: 'k' });
    await fireEvent.keyDown(root, { key: 'a' });
    await waitFor(() => expect(toasts().at(-1)).toMatch(/contains files set to Read & write/));
    expect(app.toasts.at(-1).kind).toBe('error');
    expect(sent('project_file_update').at(-1)).toEqual({ project: 'shop-api', path: `${HOME}/Brand`, access: 'read' });
  });

  it('saves the note on blur, clears with "" and leaves it alone when unchanged', async () => {
    const { container } = await open();
    replies.project_file_update = (a) => ({
      files: LIST.map((f) => (f.path === a.path ? { ...f, note: a.note || undefined } : f)),
    });
    const note = rows(container)[1].querySelector('.note');
    await fireEvent.focus(note);
    await fireEvent.input(note, { target: { value: '  Design file ' } });
    await fireEvent.blur(note);
    await waitFor(() =>
      expect(sent('project_file_update')).toEqual([
        { project: 'shop-api', path: `${HOME}/design/app.figcraft`, note: 'Design file' },
      ]),
    );
    const brand = rows(container)[0].querySelector('.note');
    await fireEvent.blur(brand);
    expect(sent('project_file_update')).toHaveLength(1);
    await fireEvent.input(brand, { target: { value: '' } });
    await fireEvent.blur(brand);
    await waitFor(() =>
      expect(sent('project_file_update')[1]).toEqual({ project: 'shop-api', path: `${HOME}/Brand`, note: '' }),
    );
  });

  it('saves the note on Enter, reverts on Escape, shows the core error', async () => {
    const { container, getByText } = await open();
    replies.project_file_update = new Error('note: 500 characters at most');
    const note = rows(container)[0].querySelector('.note');
    note.focus();
    await fireEvent.input(note, { target: { value: 'x'.repeat(501) } });
    await fireEvent.keyDown(note, { key: 'Enter' });
    await waitFor(() => getByText('note: 500 characters at most'));
    expect(note.value).toBe('x'.repeat(501)); // kept for a retry
    await fireEvent.input(note, { target: { value: 'changed' } });
    await fireEvent.keyDown(note, { key: 'Escape' });
    expect(note.value).toBe('Brand guide');
    expect(sent('project_file_update')).toHaveLength(1);
  });

  it('asks before removing, cancels on esc, removes on ⏎ and says the file stays', async () => {
    const { container, root, getByText } = await open();
    await fireEvent.keyDown(root, { key: 'j' });
    await fireEvent.keyDown(root, { key: 'Backspace' });
    await waitFor(() => getByText(/Remove from the project\? The file stays on disk\./));
    expect(sent('project_file_remove')).toEqual([]);
    await fireEvent.keyDown(root, { key: 'Escape' });
    expect(container.querySelector('[data-confirm]')).toBeNull();
    expect(rows(container)).toHaveLength(3);

    await fireEvent.click(rows(container)[0].querySelector('[aria-label="Remove"]'));
    await waitFor(() => getByText(/The folder stays on disk\./));
    replies.project_file_remove = { files: LIST.slice(1) };
    await fireEvent.click(btn(container, /^Remove/));
    await waitFor(() => expect(rows(container)).toHaveLength(2));
    expect(sent('project_file_remove')).toEqual([{ project: 'shop-api', path: `${HOME}/Brand` }]);
    expect(toasts()).toContain('Removed ~/Brand from shop-api');
  });

  it('retries a failed note save with the typed text', async () => {
    const { container, getByText } = await open();
    replies.project_file_update = new Error('disk full');
    const note = rows(container)[1].querySelector('.note');
    await fireEvent.input(note, { target: { value: 'Design file' } });
    await fireEvent.blur(note);
    await waitFor(() => getByText('disk full'));
    expect(note.value).toBe('Design file');
    replies.project_file_update = (a) => ({ files: LIST.map((f) => (f.path === a.path ? { ...f, note: a.note } : f)) });
    await fireEvent.blur(note);
    await waitFor(() => expect(sent('project_file_update')).toHaveLength(2));
    expect(sent('project_file_update')[1].note).toBe('Design file');
    await waitFor(() => expect(container.querySelector('.nerr')).toBeNull());
  });

  it('leaves confirm mode on ⏎ when the row has vanished', async () => {
    const { container, root, getByText } = await open();
    await fireEvent.click(rows(container)[1].querySelector('[aria-label="Remove"]'));
    await waitFor(() => getByText(/Remove from the project/));
    // A reload (here: an add) brings back a list without the confirmed row.
    const body = container.querySelector('.body');
    body.getBoundingClientRect = () => /** @type {DOMRect} */ (rect);
    replies.project_files_add = { files: [LIST[0], LIST[2]], added: [], refused: [] };
    drag.handler({ payload: { type: 'drop', paths: ['/d'], position: { x: 50, y: 50 } } });
    await waitFor(() => expect(rows(container)).toHaveLength(2));
    await fireEvent.keyDown(root, { key: 'Enter' });
    expect(sent('project_file_remove')).toEqual([]);
    expect(container.querySelector('[data-confirm]')).toBeNull();
  });

  it('reveals in Finder and opens a file in the editor', async () => {
    const { container } = await open();
    replies.reveal_path = null;
    replies.editor_open = { reused: true };
    await fireEvent.click(rows(container)[1].querySelector('[aria-label="Reveal in Finder"]'));
    expect(sent('reveal_path')).toEqual([{ path: `${HOME}/design/app.figcraft` }]);
    await fireEvent.click(rows(container)[1].querySelector('[aria-label="Open in editor"]'));
    await waitFor(() => expect(sent('editor_open')[0]).toMatchObject({ file: `${HOME}/design/app.figcraft` }));
  });
});
