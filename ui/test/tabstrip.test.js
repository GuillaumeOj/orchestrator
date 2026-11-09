// TabStrip (design-notes §2.3 + R6, 02-tab-strip.png, 03-tab-overflow-*):
// tabs never shrink, ‹ › only on overflow and disabled at each end, a click
// scrolls by the lane width minus one tab, press-and-hold repeats, the
// active / new tab is scrolled into view, roving tabindex, × visibility,
// middle-click closes.

import { readFileSync } from 'node:fs';
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import TabStrip, { HOLD_DELAY, HOLD_REPEAT } from '../src/components/TabStrip.svelte';

const TABS = ['Mission control', 'orch · project', 'improve-ui-ux', 'Board · improve-ui-ux', 'Diff · back-1'].map(
  (title, i) => ({
    id: `p${i + 1}`,
    title,
    icon: '•',
    closable: true,
  }),
);

const lane = (c) => c.querySelector('[role="tablist"]');
const tabs = (c) => [...c.querySelectorAll('[role="tab"]')];
const arrows = (c) => [...c.querySelectorAll('.arrow')];
const settle = async () => {
  await tick();
  await tick();
};

/** Gives the lane a layout: `sw` content in `cw` visible, at `sl`; each tab `tw` wide. */
function layout(c, { sw, cw, sl = 0, tw = 100 }) {
  const l = lane(c);
  let left = sl;
  Object.defineProperty(l, 'scrollWidth', { configurable: true, get: () => sw });
  Object.defineProperty(l, 'clientWidth', { configurable: true, get: () => cw });
  Object.defineProperty(l, 'scrollLeft', { configurable: true, get: () => left, set: (v) => (left = v) });
  for (const t of tabs(c)) Object.defineProperty(t, 'offsetWidth', { configurable: true, get: () => tw });
  return {
    get left() {
      return left;
    },
  };
}

beforeEach(() => {
  document.body.innerHTML = '';
});
afterEach(() => vi.useRealTimers());

describe('TabStrip', () => {
  it('tabs never shrink: flex none, min-width content, max 280 then ellipsis', () => {
    const css = readFileSync('src/components/TabStrip.svelte', 'utf8');
    const rule = css.match(/\n {2}\.tab \{([^}]*)\}/)[1];
    expect(rule).toMatch(/flex: none;/);
    expect(rule).toMatch(/min-width: min-content;/);
    expect(rule).toMatch(/max-width: 280px;/);
    const name = css.match(/\n {2}\.name \{([^}]*)\}/)[1];
    expect(name).toMatch(/text-overflow: ellipsis;/);
  });

  it('roles: a horizontal tablist, tabs with aria-selected and aria-controls, the arrows outside it', async () => {
    const { container } = render(TabStrip, { props: { tabs: TABS, active: 'p2', panelId: 'pane', idPrefix: 'ws' } });
    const l = lane(container);
    expect(l.getAttribute('aria-orientation')).toBe('horizontal');
    const t = tabs(container);
    expect(t.map((x) => x.getAttribute('aria-selected'))).toEqual(['false', 'true', 'false', 'false', 'false']);
    expect(t[1].id).toBe('ws-tab-p2');
    expect(t[1].getAttribute('aria-controls')).toBe('pane');
    layout(container, { sw: 900, cw: 500 });
    await fireEvent.scroll(l);
    expect(arrows(container)).toHaveLength(2);
    for (const a of arrows(container)) expect(l.contains(a)).toBe(false);
  });

  it('‹ › only when the tabs overflow; each disabled at its end', async () => {
    const { container } = render(TabStrip, { props: { tabs: TABS, active: 'p1' } });
    const l = lane(container);
    layout(container, { sw: 500, cw: 500 });
    await fireEvent.scroll(l);
    expect(arrows(container)).toHaveLength(0);

    const pos = layout(container, { sw: 900, cw: 500, sl: 0 });
    await fireEvent.scroll(l);
    let [left, right] = arrows(container);
    expect(left.disabled).toBe(true);
    expect(right.disabled).toBe(false);
    expect(container.querySelector('.lanewrap').classList.contains('fade-r')).toBe(true);
    expect(container.querySelector('.lanewrap').classList.contains('fade-l')).toBe(false);

    l.scrollLeft = 200;
    await fireEvent.scroll(l);
    [left, right] = arrows(container);
    expect([left.disabled, right.disabled]).toEqual([false, false]);
    expect(container.querySelector('.lanewrap').className).toMatch(/fade-l.*fade-r|fade-r.*fade-l/);

    l.scrollLeft = 400;
    await fireEvent.scroll(l);
    [left, right] = arrows(container);
    expect([left.disabled, right.disabled]).toEqual([false, true]);
    expect(pos.left).toBe(400);
    expect(left.getAttribute('aria-label')).toBe('Scroll tabs left');
    expect(right.getAttribute('aria-label')).toBe('Scroll tabs right');
  });

  it('a click scrolls by the lane width minus one tab; press-and-hold repeats', async () => {
    const { container } = render(TabStrip, { props: { tabs: TABS, active: 'p1' } });
    const l = lane(container);
    const pos = layout(container, { sw: 2000, cw: 500, tw: 120 });
    await fireEvent.scroll(l);
    const right = arrows(container)[1];
    vi.useFakeTimers();
    await fireEvent.pointerDown(right, { button: 0 });
    expect(pos.left).toBe(380);
    await fireEvent.pointerUp(right);
    vi.advanceTimersByTime(1000);
    expect(pos.left).toBe(380); // released: no repeat

    await fireEvent.pointerDown(right, { button: 0 });
    expect(pos.left).toBe(760);
    vi.advanceTimersByTime(HOLD_DELAY - 1);
    expect(pos.left).toBe(760);
    vi.advanceTimersByTime(1 + HOLD_REPEAT);
    expect(pos.left).toBe(1140);
    vi.advanceTimersByTime(HOLD_REPEAT);
    expect(pos.left).toBe(1500); // clamped at the end
    await fireEvent.pointerLeave(right);
    vi.useRealTimers();

    // A keyboard click (detail 0) scrolls too.
    const left = arrows(container)[0];
    await fireEvent.click(left, { detail: 0 });
    expect(pos.left).toBe(1120);
  });

  it('scrolls the active tab, and a tab just opened, into view', async () => {
    const seen = [];
    const spy = vi.spyOn(Element.prototype, 'scrollIntoView').mockImplementation(function (opts) {
      seen.push([this.dataset.tab, opts]);
    });
    const { rerender } = render(TabStrip, { props: { tabs: TABS, active: 'p3' } });
    await settle();
    expect(seen.at(-1)).toEqual(['p3', { inline: 'nearest', block: 'nearest' }]);
    await rerender({ tabs: TABS, active: 'p5' });
    await settle();
    expect(seen.at(-1)[0]).toBe('p5');
    await rerender({ tabs: [...TABS, { id: 'p9', title: 'New', closable: true }], active: 'p5' });
    await settle();
    expect(seen.at(-1)[0]).toBe('p9');
    spy.mockRestore();
  });

  it('roving tabindex: ← → Home End move focus, Enter activates, Delete closes', async () => {
    const onselect = vi.fn();
    const onclose = vi.fn();
    const { container } = render(TabStrip, { props: { tabs: TABS, active: 'p2', onselect, onclose } });
    const t = () => tabs(container);
    expect(t().map((x) => x.tabIndex)).toEqual([-1, 0, -1, -1, -1]);
    t()[1].focus();
    await fireEvent.keyDown(t()[1], { key: 'ArrowRight' });
    await settle();
    expect(document.activeElement).toBe(t()[2]);
    expect(t().map((x) => x.tabIndex)).toEqual([-1, -1, 0, -1, -1]);
    expect(onselect).not.toHaveBeenCalled();
    await fireEvent.keyDown(t()[2], { key: 'End' });
    expect(document.activeElement).toBe(t()[4]);
    await fireEvent.keyDown(t()[4], { key: 'ArrowRight' });
    expect(document.activeElement).toBe(t()[4]);
    await fireEvent.keyDown(t()[4], { key: 'Home' });
    expect(document.activeElement).toBe(t()[0]);
    await fireEvent.keyDown(t()[0], { key: 'ArrowLeft' });
    expect(document.activeElement).toBe(t()[0]);
    await fireEvent.keyDown(t()[0], { key: 'Enter' });
    expect(onselect).toHaveBeenCalledWith('p1');
    await fireEvent.keyDown(t()[0], { key: 'Delete' });
    expect(onclose).toHaveBeenCalledWith('p1');
    await fireEvent.keyDown(t()[0], { key: 'w', metaKey: true });
    expect(onclose).toHaveBeenCalledTimes(2);
  });

  it('a click selects; × shows on the active, hovered or focused tab only and closes', async () => {
    const onselect = vi.fn();
    const onclose = vi.fn();
    const { container } = render(TabStrip, { props: { tabs: TABS, active: 'p2', onselect, onclose } });
    const vis = () => tabs(container).map((x) => x.querySelector('.x').style.visibility);
    expect(vis()).toEqual(['hidden', 'visible', 'hidden', 'hidden', 'hidden']);
    await fireEvent.pointerEnter(tabs(container)[3]);
    expect(vis()).toEqual(['hidden', 'visible', 'hidden', 'visible', 'hidden']);
    await fireEvent.pointerLeave(tabs(container)[3]);
    expect(vis()[3]).toBe('hidden');
    await fireEvent.focus(tabs(container)[4]);
    expect(vis()[4]).toBe('visible');
    await fireEvent.click(tabs(container)[3]);
    expect(onselect).toHaveBeenCalledWith('p4');
    await fireEvent.click(tabs(container)[1].querySelector('.x'));
    expect(onclose).toHaveBeenCalledWith('p2');
    expect(onselect).toHaveBeenCalledTimes(1);
    expect(tabs(container)[1].querySelector('.x').getAttribute('aria-label')).toBe('Close orch · project');
  });

  it('middle-click closes; a tab that is not closable has no × and stays', async () => {
    const onclose = vi.fn();
    const list = [{ ...TABS[0], closable: false }, ...TABS.slice(1)];
    const { container } = render(TabStrip, { props: { tabs: list, active: 'p1', onclose } });
    await fireEvent(tabs(container)[2], new MouseEvent('auxclick', { button: 1, bubbles: true }));
    expect(onclose).toHaveBeenCalledWith('p3');
    expect(tabs(container)[0].querySelector('.x')).toBeNull();
    await fireEvent(tabs(container)[0], new MouseEvent('auxclick', { button: 1, bubbles: true }));
    expect(onclose).toHaveBeenCalledTimes(1);
  });

  it('a cut title shows in full in the tooltip', async () => {
    const { container } = render(TabStrip, { props: { tabs: TABS, active: 'p1' } });
    const t = tabs(container)[3];
    const name = t.querySelector('.name');
    Object.defineProperty(name, 'scrollWidth', { configurable: true, get: () => 400 });
    Object.defineProperty(name, 'clientWidth', { configurable: true, get: () => 250 });
    vi.useFakeTimers();
    await fireEvent.pointerEnter(t);
    vi.advanceTimersByTime(500);
    expect(document.getElementById('orch-tooltip').textContent).toBe('Board · improve-ui-ux');
    await fireEvent.pointerLeave(t);
    expect(document.getElementById('orch-tooltip').hidden).toBe(true);
  });
});
