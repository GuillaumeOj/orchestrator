import { fireEvent, render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fixture } from './fixture.js';

const h = vi.hoisted(() => ({ saved: null, sets: [] }));
vi.mock('../src/lib/ipc.js', () => ({
  inTauri: false,
  errText: (e) => String(e),
  listen: () => Promise.resolve(() => {}),
  invoke: (cmd, args) => {
    if (cmd === 'settings_set') {
      h.sets.push(args);
      return Promise.resolve(null);
    }
    if (cmd === 'settings_get') return Promise.resolve(args.key === 'layout' ? h.saved : null);
    return Promise.reject(new Error(`not mocked: ${cmd}`));
  },
}));

const { app, chatWidthBounds, chatWidthNow, clampChatWidth, restoreLayout, saveLayout } = await import(
  '../src/lib/app.svelte.js'
);
const { default: ChatResize } = await import('../src/components/chat/ChatResize.svelte');

beforeEach(() => {
  vi.useRealTimers();
  app.data = fixture();
  app.panes = {};
  app.layout = null;
  app.dock = { open: false, height: 0.45, tabs: [], active: null, v: 2 };
  app.sidebar = true;
  app.inspector = false;
  app.width = 1512;
  app.chatWidth = null;
  h.saved = null;
  h.sets = [];
});

describe('chatWidthBounds / clampChatWidth', () => {
  const b = (width, sidebar = true, inspector = false) => chatWidthBounds({ width, sidebar, inspector });

  it('caps at 720 on a big window', () => {
    expect(b(2400)).toEqual({ min: 300, max: 720 });
  });
  it('keeps the work column at 640 beside the sidebar (1512 − 240 − 640)', () => {
    expect(b(1512).max).toBe(632);
  });
  it('uses the icon rail below 1512 and nothing with the sidebar hidden', () => {
    expect(b(1280).max).toBe(592);
    expect(b(1280, false).max).toBe(640);
  });
  it('subtracts a docked Inspector only at ≥ 1728', () => {
    expect(b(1728, true, true).max).toBe(1728 - 240 - 320 - 640);
    expect(b(1600, true, true).max).toBe(1600 - 240 - 640);
  });
  it('lets the min win when even it does not fit', () => {
    expect(b(900)).toEqual({ min: 300, max: 300 });
  });
  it('clamps and rounds', () => {
    const bounds = { min: 300, max: 632 };
    expect(clampChatWidth(100, bounds)).toBe(300);
    expect(clampChatWidth(9999, bounds)).toBe(632);
    expect(clampChatWidth(400.4, bounds)).toBe(400);
  });
});

describe('chatWidth state', () => {
  it('defaults to 360 / 340 and applies the bound without overwriting the saved width', () => {
    expect(chatWidthNow()).toBe(360);
    app.width = 1280;
    expect(chatWidthNow()).toBe(340);
    app.chatWidth = 600;
    app.width = 1512;
    expect(chatWidthNow()).toBe(600);
    app.width = 1100; // max 1100 − 48 − 640 = 412
    expect(chatWidthNow()).toBe(412);
    expect(app.chatWidth).toBe(600);
    app.width = 1512;
    expect(chatWidthNow()).toBe(600);
  });

  it('is saved in the layout value and restored', async () => {
    app.chatWidth = 480;
    saveLayout();
    await vi.waitFor(() => expect(h.sets.length).toBe(1), { timeout: 2000 });
    expect(h.sets[0].value.chatWidth).toBe(480);
    app.chatWidth = null;
    h.saved = { layout: null, panes: {}, chatWidth: 480 };
    await restoreLayout();
    expect(app.chatWidth).toBe(480);
  });

  it.each([['abc'], [100], [9000], [NaN], [null]])('restores an invalid value (%s) as null', async (v) => {
    app.chatWidth = 500;
    h.saved = { layout: null, panes: {}, chatWidth: v };
    await restoreLayout();
    expect(app.chatWidth).toBeNull();
  });
});

describe('ChatResize', () => {
  const sep = (container) => container.querySelector('[role="separator"]');

  it('renders a labelled vertical separator with its values', () => {
    app.width = 1600;
    const { container } = render(ChatResize);
    const el = sep(container);
    expect(el.getAttribute('aria-orientation')).toBe('vertical');
    expect(el.getAttribute('aria-label')).toBe('Resize Overwatch');
    expect(el.getAttribute('aria-valuenow')).toBe('360');
    expect(el.getAttribute('aria-valuemin')).toBe('300');
    expect(el.getAttribute('aria-valuemax')).toBe('720');
    expect(el.getAttribute('title')).toBe('Drag to resize · double-click to reset');
    expect(el.getAttribute('tabindex')).toBe('0');
  });

  it('reports the clamped max, not 720', () => {
    const { container } = render(ChatResize);
    expect(sep(container).getAttribute('aria-valuemax')).toBe('632');
  });

  it('keys change the width: ← wider, → narrower, ⇧ ×4, Home/End, ⏎ resets', async () => {
    const { container } = render(ChatResize);
    const el = sep(container);
    await fireEvent.keyDown(el, { key: 'ArrowLeft' });
    expect(app.chatWidth).toBe(376);
    await fireEvent.keyDown(el, { key: 'ArrowRight' });
    expect(app.chatWidth).toBe(360);
    await fireEvent.keyDown(el, { key: 'ArrowLeft', shiftKey: true });
    expect(app.chatWidth).toBe(424);
    await fireEvent.keyDown(el, { key: 'Home' });
    expect(app.chatWidth).toBe(300);
    await fireEvent.keyDown(el, { key: 'ArrowRight' });
    expect(app.chatWidth).toBe(300);
    await fireEvent.keyDown(el, { key: 'End' });
    expect(app.chatWidth).toBe(632);
    await fireEvent.keyDown(el, { key: 'Enter' });
    expect(app.chatWidth).toBeNull();
  });

  it('double-click resets', async () => {
    app.chatWidth = 500;
    const { container } = render(ChatResize);
    await fireEvent.dblClick(sep(container));
    expect(app.chatWidth).toBeNull();
  });

  it('dragging left widens live, clamped, and saves on release', async () => {
    const { container } = render(ChatResize);
    const el = sep(container);
    el.setPointerCapture = () => {};
    await fireEvent.pointerDown(el, { button: 0, clientX: 1000, pointerId: 1 });
    expect(el.classList.contains('dragging')).toBe(true);
    await fireEvent.pointerMove(el, { clientX: 900, pointerId: 1 });
    expect(app.chatWidth).toBe(460);
    await fireEvent.pointerMove(el, { clientX: 0, pointerId: 1 });
    expect(app.chatWidth).toBe(632);
    await fireEvent.pointerMove(el, { clientX: 5000, pointerId: 1 });
    expect(app.chatWidth).toBe(300);
    expect(h.sets.length).toBe(0);
    await fireEvent.pointerUp(el, { pointerId: 1 });
    expect(el.classList.contains('dragging')).toBe(false);
    await vi.waitFor(() => expect(h.sets.length).toBe(1), { timeout: 2000 });
    expect(h.sets[0].value.chatWidth).toBe(300);
  });
});
