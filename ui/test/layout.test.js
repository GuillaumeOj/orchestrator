import { describe, expect, it } from 'vitest';
import * as L from '../src/lib/layout.js';

describe('tiling layout', () => {
  it('splits, numbers and finds neighbors', () => {
    let t = L.leaf('a');
    t = L.split(t, 'a', 'row', 'b'); // a | b
    t = L.split(t, 'b', 'col', 'c'); // a | (b / c)
    expect(L.visible(t)).toEqual(['a', 'b', 'c']);
    const r = L.rects(t);
    expect(r.a).toEqual({ x: 0, y: 0, w: 0.5, h: 1 });
    expect(r.c).toEqual({ x: 0.5, y: 0.5, w: 0.5, h: 0.5 });
    expect(L.neighbor(t, 'a', 'right')).toBe('b');
    expect(L.neighbor(t, 'b', 'down')).toBe('c');
    expect(L.neighbor(t, 'c', 'left')).toBe('a');
    expect(L.neighbor(t, 'c', 'up')).toBe('b');
    expect(L.neighbor(t, 'a', 'left')).toBe(null);
  });

  it('same-direction splits add siblings', () => {
    let t = L.split(L.leaf('a'), 'a', 'row', 'b');
    t = L.split(t, 'a', 'row', 'c');
    expect(t.children.length).toBe(3);
    expect(L.visible(t)).toEqual(['a', 'c', 'b']);
    expect(t.sizes.every((s) => Math.abs(s - 1 / 3) < 1e-9)).toBe(true);
  });

  it('tabs and removal collapse the tree', () => {
    let t = L.split(L.leaf('a'), 'a', 'row', 'b');
    t = L.addTab(t, 'b', 'c');
    expect(L.visible(t)).toEqual(['a', 'c']);
    t = L.activate(t, 'b');
    expect(L.visible(t)).toEqual(['a', 'b']);
    t = L.remove(t, 'b');
    expect(L.visible(t)).toEqual(['a', 'c']);
    t = L.remove(t, 'c');
    expect(t.kind).toBe('leaf');
    expect(L.remove(t, 'a')).toBe(null);
  });

  it('resizes within bounds and prunes missing panes', () => {
    let t = L.split(L.leaf('a'), 'a', 'row', 'b');
    t = L.resize(t, t.id, 0, 0.9);
    expect(t.sizes[0]).toBeCloseTo(0.9);
    expect(t.sizes[1]).toBeCloseTo(0.1);
    t = L.prune(t, (p) => p !== 'a');
    expect(L.allPanes(t)).toEqual(['b']);
  });
});

describe('ids', () => {
  it('stay unique next to a restored layout', () => {
    const restored = { kind: 'leaf', id: 'l1', tabs: ['a'], active: 'a' };
    const t = L.split(restored, 'a', 'row', 'b');
    expect(new Set(t.children.map((c) => c.id)).size).toBe(2);
  });
});

describe('restore', () => {
  it('re-keys a saved layout with clashing ids', () => {
    const saved = { kind: 'split', id: 's1', dir: 'row', sizes: [0.5, 0.5], children: [{ kind: 'leaf', id: 'l1', tabs: ['a'], active: 'a' }, { kind: 'leaf', id: 'l1', tabs: ['b'], active: 'b' }] };
    const t = L.reid(saved);
    expect(t.children[0].id).not.toBe(t.children[1].id);
    expect(L.visible(t)).toEqual(['a', 'b']);
  });
});
