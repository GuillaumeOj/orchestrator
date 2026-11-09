// Menu (design-notes §2.2, 02-menu.png): items and their roles, keys
// right-aligned, ↑↓ →/←, Enter runs and closes, Esc gives focus back, the
// trigger's tooltip has no key.
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Menu from '../src/components/Menu.svelte';

let ran;
const ITEMS = () => [
  { id: 'shell', label: 'Shell in its worktree', icon: 'S', run: () => ran.push('shell') },
  { id: 'diff', label: 'Diff against base', icon: 'D', disabled: true, reason: 'No commits yet' },
  { id: 'kitty', label: 'Open in kitty', icon: 'K', key: '⌘K', run: () => ran.push('kitty') },
  { separator: true },
  { id: 'offhours', label: 'Run off-hours', checked: true, run: () => ran.push('offhours') },
  {
    id: 'move',
    label: 'Move to',
    items: [
      { id: 'ready', label: 'Ready', run: () => ran.push('ready') },
      { id: 'done', label: 'Done', run: () => ran.push('done') },
    ],
  },
  { separator: true },
  { id: 'kill', label: 'Kill…', destructive: true, run: () => ran.push('kill') },
];

const trig = (c) => c.querySelector('button');
const menus = () => /** @type {HTMLElement[]} */ ([...document.body.querySelectorAll('[role="menu"]')]);
const focusedLabel = () => document.activeElement?.querySelector?.('.lab')?.textContent;
const key = (k, el = document.activeElement) => fireEvent.keyDown(el, { key: k });
const settle = async () => {
  await tick();
  await tick();
};

beforeEach(() => {
  document.body.innerHTML = '';
  ran = [];
});
afterEach(() => vi.useRealTimers());

describe('Menu', () => {
  it('the ⋯ trigger: aria-label and a tooltip with the label only, aria-haspopup / expanded', async () => {
    const { container } = render(Menu, { props: { items: ITEMS() } });
    const t = trig(container);
    expect(t.getAttribute('aria-label')).toBe('More actions');
    expect(t.getAttribute('aria-haspopup')).toBe('menu');
    expect(t.getAttribute('aria-expanded')).toBe('false');
    vi.useFakeTimers();
    await fireEvent.pointerEnter(t);
    vi.advanceTimersByTime(500);
    const tip = document.getElementById('orch-tooltip');
    expect(tip.textContent).toBe('More actions');
    expect(tip.querySelector('.kbd')).toBeNull();
  });

  it('renders the roles: menu, menuitem, menuitemcheckbox + aria-checked, separator, submenu', async () => {
    const { container } = render(Menu, { props: { items: ITEMS() } });
    await fireEvent.click(trig(container));
    await settle();
    const [m] = menus();
    expect(m.parentElement).toBe(document.body);
    expect(m.getAttribute('aria-label')).toBe('More actions');
    expect(trig(container).getAttribute('aria-expanded')).toBe('true');
    expect(trig(container).classList.contains('pressed')).toBe(true);
    const rows = [...m.children];
    expect(rows.map((r) => r.getAttribute('role'))).toEqual([
      'menuitem',
      'menuitem',
      'menuitem',
      'separator',
      'menuitemcheckbox',
      'menuitem',
      'separator',
      'menuitem',
    ]);
    expect(rows[4].getAttribute('aria-checked')).toBe('true');
    expect(rows[1].getAttribute('aria-disabled')).toBe('true');
    expect(rows[5].getAttribute('aria-haspopup')).toBe('menu');
    expect(rows[7].classList.contains('danger')).toBe(true);
    // The key is a Kbd at the right end of its row.
    const k = rows[2].querySelector('.key .kbd');
    expect(k.textContent).toBe('⌘K');
    expect(rows[2].lastElementChild.classList.contains('key')).toBe(true);
  });

  it('Enter / Space / ↓ on the trigger open on the first item', async () => {
    for (const k of ['Enter', ' ', 'ArrowDown']) {
      document.body.innerHTML = '';
      const { container, unmount } = render(Menu, { props: { items: ITEMS() } });
      await key(k, trig(container));
      await settle();
      expect(menus()).toHaveLength(1);
      expect(focusedLabel()).toBe('Shell in its worktree');
      unmount();
    }
  });

  it('↑↓ Home End skip separators and disabled items; type-ahead', async () => {
    const { container } = render(Menu, { props: { items: ITEMS() } });
    await key('ArrowDown', trig(container));
    await settle();
    await key('ArrowDown');
    await settle();
    expect(focusedLabel()).toBe('Open in kitty'); // Diff is disabled
    await key('ArrowDown');
    await settle();
    expect(focusedLabel()).toBe('Run off-hours'); // over the separator
    await key('End');
    await settle();
    expect(focusedLabel()).toBe('Kill…');
    await key('Home');
    await settle();
    expect(focusedLabel()).toBe('Shell in its worktree');
    await key('m');
    await settle();
    expect(focusedLabel()).toBe('Move to');
  });

  it('Enter runs the item, closes, gives focus back and calls onselect', async () => {
    const onselect = vi.fn();
    const { container } = render(Menu, { props: { items: ITEMS(), onselect } });
    await key('ArrowDown', trig(container));
    await settle();
    await key('End');
    await settle();
    await key('Enter');
    await settle();
    expect(ran).toEqual(['kill']);
    expect(onselect).toHaveBeenCalledWith(expect.objectContaining({ id: 'kill' }));
    expect(menus()).toHaveLength(0);
    expect(document.activeElement).toBe(trig(container));
  });

  it('a click runs an item; a disabled item does nothing and shows its reason', async () => {
    const { container } = render(Menu, { props: { items: ITEMS() } });
    await fireEvent.click(trig(container));
    await settle();
    const rows = [...menus()[0].children];
    await fireEvent.click(rows[1]);
    expect(ran).toEqual([]);
    expect(menus()).toHaveLength(1);
    vi.useFakeTimers();
    await fireEvent.pointerEnter(rows[1]);
    vi.advanceTimersByTime(500);
    expect(document.getElementById('orch-tooltip').textContent).toBe('No commits yet');
    vi.useRealTimers();
    await fireEvent.click(rows[0].querySelector('.lab'));
    expect(ran).toEqual(['shell']);
    expect(menus()).toHaveLength(0);
  });

  it('→ opens a submenu on its first item, ← closes it back to the parent row, Enter runs a subitem', async () => {
    const { container } = render(Menu, { props: { items: ITEMS() } });
    await key('ArrowDown', trig(container));
    await settle();
    await key('m');
    await settle();
    await key('ArrowRight');
    await settle();
    expect(menus()).toHaveLength(2);
    expect(menus()[1].getAttribute('aria-label')).toBe('Move to');
    expect(focusedLabel()).toBe('Ready');
    await key('ArrowLeft');
    await settle();
    expect(menus()).toHaveLength(1);
    expect(focusedLabel()).toBe('Move to');
    await key('Enter');
    await settle();
    await key('ArrowDown');
    await settle();
    expect(focusedLabel()).toBe('Done');
    await key('Enter');
    await settle();
    expect(ran).toEqual(['done']);
    expect(menus()).toHaveLength(0);
  });

  it('Esc closes (consumed, so a window under it stays) and gives focus back to the trigger', async () => {
    const { container } = render(Menu, { props: { items: ITEMS() } });
    await key('ArrowDown', trig(container));
    await settle();
    let prevented = null;
    const spy = (e) => (prevented = e.defaultPrevented);
    window.addEventListener('keydown', spy);
    await key('Escape');
    window.removeEventListener('keydown', spy);
    expect(menus()).toHaveLength(0);
    expect(document.activeElement).toBe(trig(container));
    // stopPropagation: the window listener never saw it (or saw it prevented).
    expect(prevented === null || prevented === true).toBe(true);
  });

  it('opens with right edges aligned under the trigger', async () => {
    const { container } = render(Menu, { props: { items: ITEMS() } });
    const t = trig(container);
    t.getBoundingClientRect = () => ({ top: 20, bottom: 42, left: 478, right: 500, width: 22, height: 22 });
    await fireEvent.click(t);
    await settle();
    const m = menus()[0];
    expect(m.style.top).toBe('46px');
    expect(m.style.left).toBe(`${500 - 200}px`);
  });

  it('a disabled trigger does not open', async () => {
    const { container } = render(Menu, { props: { items: ITEMS(), disabled: true } });
    await fireEvent.click(trig(container));
    expect(menus()).toHaveLength(0);
  });
});

describe('tooltip', () => {
  it('Label · key with the key as a Kbd after a muted dot; two lines for a detail', async () => {
    const { tooltip } = await import('../src/lib/tooltip.js');
    const b = document.createElement('button');
    document.body.append(b);
    const t = tooltip(b, { label: 'Hide dock', key: '⌘J' });
    vi.useFakeTimers();
    b.dispatchEvent(new Event('pointerenter'));
    vi.advanceTimersByTime(499);
    expect(document.getElementById('orch-tooltip')?.hidden ?? true).toBe(true);
    vi.advanceTimersByTime(1);
    const tip = document.getElementById('orch-tooltip');
    expect(tip.hidden).toBe(false);
    expect(tip.querySelector('.tip-dot').textContent).toBe('·');
    expect(tip.querySelector('.kbd').textContent).toBe('⌘J');
    expect(b.getAttribute('aria-describedby')).toBe('orch-tooltip');
    b.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    expect(tip.hidden).toBe(true);
    expect(b.hasAttribute('aria-describedby')).toBe(false);
    t.update({ label: 'zsh', detail: '~/dev/orchestrator.worktrees/ui-1' });
    b.dispatchEvent(new Event('pointerenter'));
    vi.advanceTimersByTime(500);
    expect(tip.querySelector('.tip-detail').textContent).toBe('~/dev/orchestrator.worktrees/ui-1');
    b.dispatchEvent(new Event('pointerdown'));
    expect(tip.hidden).toBe(true);
    t.destroy();
  });

  it('below the control with a 6 px gap, flipped above at the bottom, kept inside the window', async () => {
    const { placeTip } = await import('../src/lib/tooltip.js');
    const r = { top: 100, bottom: 122, left: 50, width: 22 };
    expect(placeTip(r, 80, 24, 1280, 800)).toEqual({ top: 128, left: 21, above: false });
    expect(placeTip({ ...r, top: 780, bottom: 798 }, 80, 24, 1280, 800)).toEqual({ top: 750, left: 21, above: true });
    expect(placeTip({ ...r, left: 0, width: 10 }, 80, 24, 1280, 800).left).toBe(4);
  });
});
