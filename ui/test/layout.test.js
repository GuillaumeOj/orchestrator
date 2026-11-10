import { describe, expect, it } from 'vitest';
import * as L from '../src/lib/layout.js';

describe('single pane layout', () => {
  it('adds tabs at the end, active', () => {
    let t = L.addTab(null, 'a');
    t = L.addTab(t, 'b');
    expect(t.tabs).toEqual(['a', 'b']);
    expect(t.active).toBe('b');
    expect(L.activate(t, 'a').active).toBe('a');
    expect(L.activate(t, 'zz')).toBe(t);
  });

  it('removing the active tab activates the next, then the previous; the last leaves null', () => {
    let t = L.leaf('a');
    t = L.addTab(L.addTab(t, 'b'), 'c');
    t = L.activate(t, 'b');
    t = L.remove(t, 'b');
    expect(t.tabs).toEqual(['a', 'c']);
    expect(t.active).toBe('c');
    t = L.remove(t, 'c');
    expect(t.active).toBe('a');
    expect(L.remove(t, 'a')).toBe(null);
  });

  it('prunes missing panes', () => {
    const t = L.prune(L.addTab(L.leaf('a'), 'b'), (p) => p !== 'a');
    expect(L.allPanes(t)).toEqual(['b']);
    expect(t.active).toBe('b');
  });

  it('has no split, neighbor, rects or resize', () => {
    for (const f of ['split', 'neighbor', 'rects', 'resize']) expect(L[f]).toBeUndefined();
  });
});

describe('flatten (a saved layout with splits, design-notes §4.2)', () => {
  // a | (b+c tabs / d): reading order a, b, c, d.
  const saved = {
    kind: 'split',
    id: 's1',
    dir: 'row',
    sizes: [0.5, 0.5],
    children: [
      { kind: 'leaf', id: 'l1', tabs: ['a'], active: 'a' },
      {
        kind: 'split',
        id: 's2',
        dir: 'col',
        sizes: [0.5, 0.5],
        children: [
          { kind: 'leaf', id: 'l1', tabs: ['b', 'c'], active: 'c' },
          { kind: 'leaf', id: 'l3', tabs: ['d'], active: 'd' },
        ],
      },
    ],
  };

  it('keeps every pane in reading order with the focused one active', () => {
    const t = L.flatten(saved, 'd');
    expect(t.kind).toBe('leaf');
    expect(t.tabs).toEqual(['a', 'b', 'c', 'd']);
    expect(t.active).toBe('d');
    expect(t.id).not.toBe('l1');
  });

  it('falls back to the first group active tab when the focus is elsewhere', () => {
    expect(L.flatten(saved, 'gone').active).toBe('a');
    expect(L.flatten(saved).active).toBe('a');
  });

  it('a leaf stays a leaf; an empty tree is null', () => {
    const t = L.flatten({ kind: 'leaf', id: 'l9', tabs: ['x', 'y'], active: 'y' }, null);
    expect(t.tabs).toEqual(['x', 'y']);
    expect(t.active).toBe('y');
    expect(L.flatten(null)).toBe(null);
  });
});
