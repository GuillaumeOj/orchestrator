// Select (design-notes §2.1, 02-select.png): opening, keyboard, type-ahead,
// disabled options, search, error, flip, ARIA, the hidden input, and
// `onchange` only on a user pick.
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Select from '../src/components/Select.svelte';
import { MODEL_ALIASES, MODEL_OPTIONS, MODELS } from '../src/lib/options.js';
import { placePopover, typeahead } from '../src/lib/popover.js';

const MODELS_GROUPED = [
  { value: 'opus', label: 'Opus', group: 'Recommended' },
  { value: 'sonnet', label: 'Sonnet', group: 'Recommended' },
  { value: 'haiku', label: 'Haiku', group: 'Other' },
  { value: 'role', label: 'Inherit from role', group: 'Other' },
  { value: 'opus1m', label: 'Opus 1M', group: 'Other', disabled: true, reason: 'Not in your plan (Max only)' },
];

const PROJECTS = [
  'orchestrator',
  'shop-api',
  'billing',
  'design-system',
  'docs-site',
  'infra',
  'mobile-app',
  'rss-feed',
  'search',
].map((p, i) => ({ value: p, label: p, count: i, dot: p === 'orchestrator' }));

const box = (c) => c.querySelector('[role="combobox"]');
const list = () => document.body.querySelector('[role="listbox"]');
const opts = () => [...document.body.querySelectorAll('[role="option"]')];
const hiLabel = () => document.body.querySelector('[role="option"].hi .lab')?.textContent;
const key = (el, k, extra = {}) => fireEvent.keyDown(el, { key: k, ...extra });

beforeEach(() => {
  document.body.innerHTML = '';
});
afterEach(() => vi.useRealTimers());

describe('Select', () => {
  it('opens on click, ↓, Enter and Space; ARIA wiring between box, list and options', async () => {
    for (const how of ['click', 'ArrowDown', 'Enter', ' ']) {
      document.body.innerHTML = '';
      const { container, unmount } = render(Select, {
        props: { value: 'opus', options: MODELS_GROUPED, 'aria-label': 'Model' },
      });
      const b = box(container);
      expect(b.getAttribute('aria-haspopup')).toBe('listbox');
      expect(b.getAttribute('aria-expanded')).toBe('false');
      if (how === 'click') await fireEvent.click(b);
      else await key(b, how);
      await tick();
      expect(list(), how).not.toBeNull();
      expect(b.getAttribute('aria-expanded')).toBe('true');
      expect(b.getAttribute('aria-controls')).toBe(list().id);
      expect(b.getAttribute('aria-activedescendant')).toBe(opts()[0].id);
      unmount();
    }
  });

  it('groups, ✓ and aria-selected on the selected option, disabled options marked', async () => {
    const { container } = render(Select, { props: { value: 'opus', options: MODELS_GROUPED } });
    await fireEvent.click(box(container));
    expect([...document.body.querySelectorAll('.gh')].map((g) => g.textContent)).toEqual(['Recommended', 'Other']);
    const o = opts();
    expect(o.map((x) => x.getAttribute('aria-selected'))).toEqual(['true', 'false', 'false', 'false', 'false']);
    expect(o[0].querySelector('.ck').textContent).not.toBe('');
    expect(o[4].getAttribute('aria-disabled')).toBe('true');
    expect(o[4].classList.contains('off')).toBe(true);
  });

  it('↑↓ Home End move the highlight and skip disabled options', async () => {
    const { container } = render(Select, { props: { value: 'opus', options: MODELS_GROUPED } });
    const b = box(container);
    await key(b, 'ArrowDown');
    expect(hiLabel()).toBe('Opus');
    await key(b, 'ArrowDown');
    expect(hiLabel()).toBe('Sonnet');
    await key(b, 'End');
    expect(hiLabel()).toBe('Inherit from role'); // Opus 1M is disabled
    await key(b, 'ArrowDown');
    expect(hiLabel()).toBe('Inherit from role');
    await key(b, 'Home');
    expect(hiLabel()).toBe('Opus');
    await key(b, 'ArrowUp');
    expect(hiLabel()).toBe('Opus');
  });

  it('type-ahead moves to the next label starting with the typed text', async () => {
    const { container } = render(Select, { props: { value: 'opus', options: MODELS_GROUPED } });
    const b = box(container);
    await key(b, 'ArrowDown');
    await key(b, 'h');
    expect(hiLabel()).toBe('Haiku');
    await key(b, 'i');
    expect(hiLabel()).toBe('Haiku'); // "hi" matches nothing: stays
    vi.useFakeTimers();
    vi.advanceTimersByTime(600);
    await key(b, 's');
    expect(hiLabel()).toBe('Sonnet');
  });

  it('Enter and Space pick, close and give focus back; onchange fires only on a user pick', async () => {
    const onchange = vi.fn();
    const { container, rerender } = render(Select, { props: { value: 'opus', options: MODELS_GROUPED, onchange } });
    const b = box(container);
    await rerender({ value: 'haiku' });
    expect(onchange).not.toHaveBeenCalled();
    await key(b, 'ArrowDown');
    await key(b, 'ArrowUp');
    await key(b, 'ArrowUp');
    await key(b, 'Enter');
    expect(onchange).toHaveBeenLastCalledWith('opus');
    expect(list()).toBeNull();
    expect(document.activeElement).toBe(b);
    await key(b, ' ');
    await key(b, 'ArrowDown');
    await key(b, ' ');
    expect(onchange).toHaveBeenLastCalledWith('sonnet');
    expect(onchange).toHaveBeenCalledTimes(2);
    expect(b.textContent).toContain('Sonnet');
  });

  it('a click on an option picks it; a click on a disabled option does nothing', async () => {
    const onchange = vi.fn();
    const { container } = render(Select, { props: { value: 'opus', options: MODELS_GROUPED, onchange } });
    await fireEvent.click(box(container));
    await fireEvent.click(opts()[4]);
    expect(onchange).not.toHaveBeenCalled();
    expect(list()).not.toBeNull();
    await fireEvent.click(opts()[2].querySelector('.lab'));
    expect(onchange).toHaveBeenCalledWith('haiku');
    expect(list()).toBeNull();
  });

  it('Esc closes keeping the value and is preventDefault-ed (a dialog under it stays open)', async () => {
    const onchange = vi.fn();
    const { container } = render(Select, { props: { value: 'opus', options: MODELS_GROUPED, onchange } });
    const b = box(container);
    await key(b, 'ArrowDown');
    await key(b, 'ArrowDown');
    let prevented = null;
    const spy = (e) => (prevented = e.defaultPrevented);
    window.addEventListener('keydown', spy);
    await key(b, 'Escape');
    window.removeEventListener('keydown', spy);
    expect(prevented).toBe(true);
    expect(list()).toBeNull();
    expect(onchange).not.toHaveBeenCalled();
    expect(b.textContent).toContain('Opus');
    // Closed, Esc is not consumed.
    window.addEventListener('keydown', spy);
    await key(b, 'Escape');
    window.removeEventListener('keydown', spy);
    expect(prevented).toBe(false);
  });

  it('Tab closes and keeps the value; an outside pointer down closes', async () => {
    const { container } = render(Select, { props: { value: 'opus', options: MODELS_GROUPED } });
    const b = box(container);
    await key(b, 'ArrowDown');
    await key(b, 'Tab');
    expect(list()).toBeNull();
    await fireEvent.click(b);
    expect(list()).not.toBeNull();
    await fireEvent.pointerDown(document.body);
    expect(list()).toBeNull();
  });

  it('a disabled option shows its reason in a tooltip on hover', async () => {
    const { container } = render(Select, { props: { value: 'opus', options: MODELS_GROUPED } });
    await fireEvent.click(box(container));
    vi.useFakeTimers();
    await fireEvent.pointerEnter(opts()[4]);
    vi.advanceTimersByTime(500);
    const tip = document.getElementById('orch-tooltip');
    expect(tip.hidden).toBe(false);
    expect(tip.textContent).toBe('Not in your plan (Max only)');
    expect(opts()[4].getAttribute('aria-describedby')).toBe('orch-tooltip');
  });

  it('search appears past 8 options, filters as you type, and says when nothing matches', async () => {
    const { container, unmount } = render(Select, {
      props: { value: 'shop-api', options: PROJECTS.slice(0, 8), noun: 'project' },
    });
    await fireEvent.click(box(container));
    expect(document.body.querySelector('[role="searchbox"]')).toBeNull();
    unmount();
    document.body.innerHTML = '';

    const onchange = vi.fn();
    const r = render(Select, { props: { value: 'shop-api', options: PROJECTS, noun: 'project', onchange } });
    await fireEvent.click(box(r.container));
    await tick();
    const q = document.body.querySelector('[role="searchbox"]');
    expect(q).not.toBeNull();
    expect(q.getAttribute('placeholder')).toBe('Filter projects');
    expect(document.activeElement).toBe(q);
    expect(box(r.container).getAttribute('role')).toBe('combobox');
    await fireEvent.input(q, { target: { value: 'sh' } });
    expect(opts().map((o) => o.querySelector('.lab').textContent)).toEqual(['shop-api']);
    await fireEvent.input(q, { target: { value: 'zz' } });
    expect(opts()).toHaveLength(0);
    expect(document.body.querySelector('.none').textContent).toBe('No project matches “zz”');
    await fireEvent.input(q, { target: { value: 'i' } });
    expect(hiLabel()).toBe('shop-api'); // the selected one, when it is in the list
    await key(q, 'ArrowDown');
    await key(q, 'Enter');
    expect(onchange).toHaveBeenCalledWith(PROJECTS.filter((p) => p.label.includes('i'))[1].value);
    expect(document.activeElement).toBe(box(r.container));
  });

  it('counts and dots on options; the box shows the selected count and dot', async () => {
    const { container } = render(Select, { props: { value: 'orchestrator', options: PROJECTS, noun: 'project' } });
    const b = box(container);
    expect(b.querySelector('.cnt').textContent).toBe('0');
    expect(b.querySelector('.dot')).not.toBeNull();
    await fireEvent.click(b);
    expect(opts()[0].querySelector('.dot')).not.toBeNull();
    expect(opts()[1].querySelector('.dot')).toBeNull();
    expect(opts()[3].querySelector('.cnt').textContent).toBe('3');
  });

  it('placeholder when the value is null or unknown', () => {
    const { container } = render(Select, {
      props: { value: null, options: PROJECTS, placeholder: 'Choose a project' },
    });
    const v = box(container).querySelector('.val');
    expect(v.textContent).toBe('Choose a project');
    expect(v.classList.contains('ph')).toBe(true);
  });

  it('error: red border, the caption under the box, aria-invalid and aria-describedby', () => {
    const { container } = render(Select, {
      props: { value: 'opus', options: MODELS_GROUPED, error: 'Pick a model for this role' },
    });
    const b = box(container);
    expect(b.getAttribute('aria-invalid')).toBe('true');
    const cap = container.querySelector(`#${CSS.escape(b.getAttribute('aria-describedby'))}`);
    expect(cap.textContent).toBe('Pick a model for this role');
    expect(b.classList.contains('bad')).toBe(true);
  });

  it('disabled: does not open', async () => {
    const { container } = render(Select, { props: { value: 'opus', options: MODELS_GROUPED, disabled: true } });
    await fireEvent.click(box(container));
    await key(box(container), 'ArrowDown');
    expect(list()).toBeNull();
  });

  it('renders the popover on the body, under the box; flips above at the bottom of the window', async () => {
    const { container, unmount } = render(Select, { props: { value: 'opus', options: MODELS_GROUPED } });
    const b = box(container);
    b.getBoundingClientRect = () => ({ top: 100, bottom: 128, left: 40, right: 240, width: 200, height: 28 });
    await fireEvent.click(b);
    await tick();
    const pop = list().parentElement;
    expect(pop.parentElement).toBe(document.body);
    expect(pop.classList.contains('above')).toBe(false);
    expect(pop.style.top).toBe('132px');
    unmount();
    document.body.innerHTML = '';

    const r = render(Select, { props: { value: 'opus', options: MODELS_GROUPED } });
    const b2 = box(r.container);
    const vh = window.innerHeight;
    b2.getBoundingClientRect = () => ({ top: vh - 30, bottom: vh - 2, left: 40, right: 240, width: 200, height: 28 });
    await fireEvent.click(b2);
    await tick();
    const pop2 = list().parentElement;
    expect(pop2.classList.contains('above')).toBe(true);
    expect(pop2.style.bottom).toBe('34px');
  });

  it('renders a hidden input with the name and the value (forms keep working)', async () => {
    const { container } = render(Select, { props: { value: 'opus', name: 'model', options: MODELS_GROUPED } });
    const input = /** @type {HTMLInputElement} */ (container.querySelector('input[type="hidden"]'));
    expect(input.name).toBe('model');
    expect(input.value).toBe('opus');
    await fireEvent.click(box(container));
    await fireEvent.click(opts()[1]);
    expect(input.value).toBe('sonnet');
  });

  it('compact: the strip variant hugs its value', () => {
    const { container } = render(Select, { props: { value: 'opus', options: MODELS_GROUPED, compact: true } });
    expect(container.querySelector('.sel').classList.contains('compact')).toBe(true);
    expect(container.querySelector('.sel').classList.contains('hug')).toBe(true);
  });
});

describe('options.js', () => {
  it('one model list: the aliases for required pickers, default + aliases + custom for optional ones', () => {
    expect(MODEL_ALIASES).toEqual(['opus', 'sonnet', 'haiku', 'fable']);
    expect(MODEL_OPTIONS.map((o) => o.value)).toEqual(MODEL_ALIASES);
    expect(MODELS.map((o) => o.value)).toEqual(['', ...MODEL_ALIASES, 'custom']);
  });
});

describe('popover.js', () => {
  it('a tall list that fits once capped at 320 px stays below', () => {
    const r = { top: 300, bottom: 328, left: 10, right: 210 };
    expect(placePopover(r, 200, 600, 1280, 700).above).toBe(false); // 360 px below ≥ 320
    expect(placePopover({ ...r, top: 500, bottom: 528 }, 200, 600, 1280, 700).above).toBe(true);
  });

  it('type-ahead: the same letter typed again cycles through its labels', () => {
    const labels = ['Sonnet', 'Opus', 'Sonnet 1M', 'Haiku'];
    const find = typeahead(() => labels);
    expect(find('s', -1)).toBe(0);
    expect(find('s', 0)).toBe(2);
    expect(find('s', 2)).toBe(0);
  });
});
