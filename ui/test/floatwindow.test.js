// FloatingWindow (design-notes §2.4, §3.3, 02-floating-window.png): backdrop
// click and Esc close, an open Select takes the Esc first, focus trap and
// restore, one window at a time, the width clamp, the dialog roles, the
// section nav keys.
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { windowSize } from '../src/components/FloatingWindow.svelte';
import WindowHost from './fixtures/WindowHost.svelte';

const dialogs = () => /** @type {HTMLElement[]} */ ([...document.body.querySelectorAll('[role="dialog"]')]);
const settle = async () => {
  await tick();
  await tick();
};
const esc = (el = document.activeElement) => fireEvent.keyDown(el, { key: 'Escape' });
const tab = (el = document.activeElement, shiftKey = false) => fireEvent.keyDown(el, { key: 'Tab', shiftKey });
const $ = (sel) => document.body.querySelector(sel);

const [W, H] = [window.innerWidth, window.innerHeight];
beforeEach(() => {
  document.body.innerHTML = '';
});
afterEach(() => {
  window.innerWidth = W;
  window.innerHeight = H;
});

/** Renders the host with the opener focused first, then opens `kind`. */
async function open(kind, props = {}) {
  const r = render(WindowHost, { props: { kind: null, ...props } });
  $('#opener').focus();
  await r.rerender({ kind, ...props });
  await settle();
  return r;
}

describe('FloatingWindow', () => {
  it('role dialog, aria-modal, labelled by its title; the subtitle after "· "', async () => {
    await open('project');
    const [d] = dialogs();
    expect(d.getAttribute('aria-modal')).toBe('true');
    const title = document.getElementById(d.getAttribute('aria-labelledby'));
    expect(title.textContent).toBe('Project settings · shop-api');
    expect($('.x').getAttribute('aria-label')).toBe('Close');
  });

  it('the app root before the window goes inert; the overlay layer after it stays live; all restored on close', async () => {
    const onclose = vi.fn();
    const r = await open('settings', { onclose });
    expect($('#work').hasAttribute('inert')).toBe(true);
    expect($('#overlay').hasAttribute('inert')).toBe(false);
    await fireEvent.click($('.x'));
    expect(onclose).toHaveBeenCalledWith('close');
    await settle();
    expect(dialogs()).toHaveLength(0);
    expect($('#work').hasAttribute('inert')).toBe(false);
    r.unmount();
  });

  it('a backdrop click closes; Esc closes; focus goes back to the opener', async () => {
    const onclose = vi.fn();
    await open('project', { onclose });
    expect(document.activeElement).toBe($('#first')); // no nav: the first field
    await fireEvent.click($('.backdrop'));
    expect(onclose).toHaveBeenLastCalledWith('backdrop');
    await settle();
    expect(document.activeElement).toBe($('#opener'));

    document.body.innerHTML = '';
    const onclose2 = vi.fn();
    await open('planner', { onclose: onclose2 });
    await esc();
    expect(onclose2).toHaveBeenCalledWith('esc');
    await settle();
    expect(document.activeElement).toBe($('#opener'));
  });

  it('Esc is taken by an open Select first, then goes content → nav → close', async () => {
    const onclose = vi.fn();
    await open('settings', { onclose });
    const box = $('[role="combobox"]');
    box.focus();
    await fireEvent.keyDown(box, { key: 'ArrowDown' });
    expect($('[role="listbox"]')).not.toBeNull();
    await esc(box);
    expect($('[role="listbox"]')).toBeNull();
    expect(onclose).not.toHaveBeenCalled();
    expect(dialogs()).toHaveLength(1);
    // In the content, Esc goes back to the nav first.
    await esc(box);
    expect(document.activeElement.getAttribute('role')).toBe('tab');
    expect(onclose).not.toHaveBeenCalled();
    await esc();
    expect(onclose).toHaveBeenCalledWith('esc');
  });

  it('Esc ignores keys from the overlay layer (the palette above the window has its own)', async () => {
    const onclose = vi.fn();
    await open('settings', { onclose });
    await esc($('#palette'));
    expect(onclose).not.toHaveBeenCalled();
  });

  it('focus trap: Tab and Shift+Tab cycle inside the window', async () => {
    await open('settings');
    const tabEl = document.activeElement;
    expect(tabEl.getAttribute('aria-selected')).toBe('true'); // initial focus: the nav
    const last = $('#last');
    last.focus();
    const ev = await tab(last);
    expect(ev).toBe(false); // default prevented: we moved focus ourselves
    expect(document.activeElement).toBe($('.x'));
    await tab(document.activeElement, true);
    expect(document.activeElement).toBe(last);
  });

  it('one window at a time: replacing keeps the original opener for the focus return', async () => {
    const r = await open('settings');
    expect(dialogs()).toHaveLength(1);
    await r.rerender({ kind: 'planner' });
    await settle();
    expect(dialogs()).toHaveLength(1);
    expect(document.getElementById(dialogs()[0].getAttribute('aria-labelledby')).textContent).toBe('Week planner');
    expect(document.activeElement).toBe($('#pause'));
    await esc();
    await settle();
    expect(document.activeElement).toBe($('#opener'));
  });

  it('width = min(w, viewport − 80), never under 720; height capped the same way', async () => {
    expect(windowSize({ w: 960, h: 760 }, 1280, 800)).toEqual({ w: 960, h: 720, maxH: 720 });
    expect(windowSize({ w: 960, h: 760 }, 1024, 900).w).toBe(944);
    expect(windowSize({ w: 1100, h: 760 }, 1024, 900)).toEqual({ w: 944, h: 760, maxH: 820 });
    expect(windowSize({ w: 960, h: 760 }, 700, 900).w).toBe(720);
    expect(windowSize({ w: 560 }, 1280, 800, 400)).toEqual({ w: 560, h: null, maxH: 720 });

    window.innerWidth = 1024;
    window.innerHeight = 800;
    await open('settings');
    expect(dialogs()[0].style.width).toBe('944px');
    expect(dialogs()[0].style.height).toBe('720px');
  });

  it('nav: a vertical tablist; j/k and ↑↓ move the section, Enter goes into the content', async () => {
    await open('settings');
    const list = $('[role="tablist"]');
    expect(list.getAttribute('aria-orientation')).toBe('vertical');
    const panel = $('[role="tabpanel"]');
    const tabs = () => /** @type {HTMLElement[]} */ ([...document.body.querySelectorAll('[role="tab"]')]);
    expect(panel.getAttribute('aria-labelledby')).toBe(tabs()[0].id);
    expect(tabs().map((t) => t.tabIndex)).toEqual([0, -1, -1]);
    await fireEvent.keyDown(document.activeElement, { key: 'j' });
    await settle();
    expect($('#shown').textContent).toBe('overwatch');
    expect(document.activeElement).toBe(tabs()[1]);
    await fireEvent.keyDown(document.activeElement, { key: 'ArrowDown' });
    await settle();
    expect($('#shown').textContent).toBe('mcp');
    await fireEvent.keyDown(document.activeElement, { key: 'k' });
    await settle();
    expect($('#shown').textContent).toBe('overwatch');
    expect(panel.getAttribute('aria-labelledby')).toBe(tabs()[1].id);
    await fireEvent.keyDown(document.activeElement, { key: 'Enter' });
    expect(document.activeElement).toBe($('[role="combobox"]'));
    await fireEvent.click(tabs()[2]);
    await settle();
    expect($('#shown').textContent).toBe('mcp');
    expect(tabs()[2].querySelector('.badge').textContent).toBe('2');
  });

  it('opens on the deep-linked section', async () => {
    await open('settings', { section: 'mcp' });
    expect(document.activeElement.textContent).toContain('MCP servers');
    expect($('#shown').textContent).toBe('mcp');
  });
});
