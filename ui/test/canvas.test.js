import { describe, expect, it } from 'vitest';
import * as C from '../src/lib/canvas.js';

const empty = () => ({ id: 't', name: 'T', scope: { kind: 'global' }, nodes: [], edges: [], version: 1 });

function fullStack() {
  let t = empty();
  for (const r of ['lead', 'backend', 'frontend', 'ui', 'ux']) [t] = C.addNode(t, r);
  t = C.addEdge(t, 'back', 'lead', 'reports-to');
  t = C.addEdge(t, 'front', 'lead', 'reports-to');
  t = C.addEdge(t, 'ui', 'lead', 'reports-to');
  t = C.addEdge(t, 'ux', 'ui', 'hands-off-to');
  t = C.addEdge(t, 'back', 'front', 'can-message');
  return t;
}

describe('team canvas model', () => {
  it('adds nodes with short unique ids; the first is the lead', () => {
    let [t, id] = C.addNode(empty(), 'lead');
    expect(id).toBe('lead');
    expect(t.nodes[0].is_lead).toBe(true);
    [t, id] = C.addNode(t, 'backend');
    expect(id).toBe('back');
    [t, id] = C.addNode(t, 'backend');
    expect(id).toBe('back2');
    expect(t.nodes.filter((n) => n.is_lead)).toHaveLength(1);
    expect(t.nodes[2].position[0]).toBeGreaterThan(t.nodes[1].position[0]);
    expect(C.nodeIdFor(t, 'Design Ops!')).toBe('design-ops-');
  });

  it('edges: no duplicates, no loops, can-message has no direction', () => {
    const t = fullStack();
    expect(C.addEdge(t, 'back', 'lead', 'reports-to')).toBe(t);
    expect(C.addEdge(t, 'front', 'back', 'can-message')).toBe(t);
    expect(C.addEdge(t, 'ux', 'ux', 'reviews')).toBe(t);
    expect(C.addEdge(t, 'ux', 'front', 'teleport')).toBe(t);
    expect(C.addEdge(t, 'front', 'ui', 'reviews').edges).toHaveLength(t.edges.length + 1);
    expect(C.removeEdge(t, 0).edges).toHaveLength(t.edges.length - 1);
  });

  it('removing a node drops its edges and keeps a lead', () => {
    const t = C.removeNode(fullStack(), 'lead');
    expect(t.edges.some((e) => e.to === 'lead')).toBe(false);
    expect(t.nodes.filter((n) => n.is_lead)).toHaveLength(1);
    const s = C.setLead(t, 'ux');
    expect(s.nodes.find((n) => n.is_lead).id).toBe('ux');
  });

  it('keyboard selection moves to the nearest node in a direction', () => {
    const t = C.autoLayout(fullStack());
    expect(C.neighbor(t, 'lead', 'down')).not.toBe('lead');
    const row = t.nodes
      .filter((n) => n.position[1] === t.nodes.find((x) => x.id === 'back').position[1])
      .map((n) => n.id);
    const right = C.neighbor(t, row[0], 'right');
    expect(row).toContain(right);
    expect(C.neighbor(t, 'lead', 'up')).toBe('lead');
    expect(C.neighbor(t, 'missing', 'up')).toBe(t.nodes[0].id);
  });

  it('auto-layout puts the lead on top and each node below whom it reports to', () => {
    const t = C.autoLayout(fullStack());
    const y = (id) => t.nodes.find((n) => n.id === id).position[1];
    expect(y('lead')).toBe(0);
    expect(y('back')).toBeGreaterThan(y('lead'));
    expect(y('back')).toBe(y('front'));
    expect(y('ux')).toBeGreaterThan(y('ui')); // ux only hands off: the last row
    const xs = t.nodes.filter((n) => n.position[1] === y('back')).map((n) => n.position[0]);
    expect(new Set(xs).size).toBe(xs.length);
  });

  it('connect targets are numbered in reading order', () => {
    const t = C.autoLayout(fullStack());
    const ts = C.targets(t, 'back');
    expect(ts[0]).toEqual({ n: 1, id: 'lead' });
    expect(ts.map((x) => x.id)).not.toContain('back');
  });

  it('local issues mirror the core: lead, reachability, reports-to cycles', () => {
    expect(C.localIssues(fullStack())).toEqual([]);
    expect(C.localIssues(empty())[0].text).toMatch(/no nodes/);
    let t = C.updateNode(fullStack(), 'lead', { is_lead: false });
    expect(C.localIssues(t)[0].text).toMatch(/no lead/);
    [t] = C.addNode(fullStack(), 'reviewer');
    expect(C.localIssues(t)).toEqual([{ node: 'review', text: 'review is not reachable from the lead' }]);
    t = C.addEdge(C.addEdge(fullStack(), 'ui', 'ux', 'reports-to'), 'ux', 'ui', 'reports-to');
    expect(C.localIssues(t).some((i) => /reports-to cycle/.test(i.text))).toBe(true);
  });

  it('edge lines start and end on the node borders', () => {
    const t = C.autoLayout(fullStack());
    const g = C.edgeLine(t, { from: 'back', to: 'lead', kind: 'reports-to' });
    const lead = t.nodes.find((n) => n.id === 'lead');
    expect(g.y2).toBeCloseTo(lead.position[1] + C.NODE_H, 0);
    expect(C.edgeLine(t, { from: 'x', to: 'lead', kind: 'reports-to' })).toBe(null);
    const b = C.bounds(t);
    expect(b.w).toBeGreaterThan(C.NODE_W);
  });

  it('undo and redo whole snapshots', () => {
    const h = new C.History(2);
    let t = fullStack();
    h.push(t);
    const before = t;
    t = C.removeNode(t, 'ux');
    const back = h.undo(t);
    expect(back.nodes).toHaveLength(before.nodes.length);
    expect(h.redo(back).nodes).toHaveLength(t.nodes.length);
    expect(h.redo(back)).toBe(null);
    h.push(t);
    h.push(t);
    h.push(t);
    expect(h.past).toHaveLength(2);
  });
});

describe('model select mapping', () => {
  it('maps stored values to options', () => {
    expect(C.modelOption(null)).toBe('');
    expect(C.modelOption('')).toBe('');
    expect(C.modelOption('opus')).toBe('opus');
    expect(C.modelOption('claude-opus-4-1')).toBe('custom');
    expect(C.modelOption('mystery')).toBe('custom');
  });

  it('maps options back to saved values', () => {
    expect(C.modelValue('')).toBeNull();
    expect(C.modelValue('sonnet', 'x')).toBe('sonnet');
    expect(C.modelValue('custom', '  claude-x  ')).toBe('claude-x');
    expect(C.modelValue('custom', '   ')).toBeNull();
  });

  it('sets and clears a node override', () => {
    let t = empty();
    [t] = C.addNode(t, 'lead');
    t = C.setNodeModel(t, 'lead', 'haiku');
    expect(t.nodes[0].overrides.model).toBe('haiku');
    t = C.setNodeModel(t, 'lead', null);
    expect('model' in t.nodes[0].overrides).toBe(false);
  });
});
