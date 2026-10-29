// The Team canvas's model (FR-301–305, Penpot 04 Team canvas): pure edits of
// a team graph (orch_team::Team JSON: nodes with positions, edges), keyboard
// selection, connect mode, undo/redo and a layered auto-layout. The core
// validates and versions the team on save; `localIssues` gives instant
// feedback while editing.

export const EDGE_KINDS = ['reports-to', 'hands-off-to', 'reviews', 'can-message'];
export const NODE_W = 176;
export const NODE_H = 56;
const GAP_X = 44;
const GAP_Y = 120;

const clone = (t) => JSON.parse(JSON.stringify(t));

/** A node id for a role: short and git-safe (`backend` → `back`, then `back2`…). */
export function nodeIdFor(team, roleId) {
  const base =
    { backend: 'back', frontend: 'front', reviewer: 'review' }[roleId] ??
    String(roleId)
      .toLowerCase()
      .replace(/[^a-z0-9_-]+/g, '-')
      .replace(/^-+/, '')
      .slice(0, 20) ??
    'node';
  const ids = new Set(team.nodes.map((n) => n.id));
  if (!ids.has(base)) return base;
  for (let i = 2; ; i++) if (!ids.has(`${base}${i}`)) return `${base}${i}`;
}

/** Adds a node of `roleId`; the first node of a team is its lead. Returns [team, id]. */
export function addNode(team, roleId, position = null) {
  const t = clone(team);
  const id = nodeIdFor(t, roleId);
  const pos = position ?? freeSpot(t);
  t.nodes.push({
    id,
    role_id: roleId,
    overrides: {},
    position: pos,
    max_concurrency: 1,
    is_lead: t.nodes.length === 0,
  });
  return [t, id];
}

function freeSpot(t) {
  if (!t.nodes.length) return [0, 0];
  const maxY = Math.max(...t.nodes.map((n) => n.position[1]));
  const row = t.nodes.filter((n) => n.position[1] === maxY);
  return [Math.max(...row.map((n) => n.position[0])) + NODE_W + GAP_X, maxY];
}

export function removeNode(team, id) {
  const t = clone(team);
  const wasLead = t.nodes.find((n) => n.id === id)?.is_lead;
  t.nodes = t.nodes.filter((n) => n.id !== id);
  t.edges = t.edges.filter((e) => e.from !== id && e.to !== id);
  if (wasLead && t.nodes.length) t.nodes[0].is_lead = true;
  return t;
}

export function setLead(team, id) {
  const t = clone(team);
  for (const n of t.nodes) {
    n.is_lead = n.id === id;
    if (n.is_lead) n.max_concurrency = 1;
  }
  return t;
}

export function updateNode(team, id, patch) {
  const t = clone(team);
  const n = t.nodes.find((x) => x.id === id);
  if (n) Object.assign(n, patch);
  return t;
}

export function moveNode(team, id, position) {
  return updateNode(team, id, { position: [Math.round(position[0]), Math.round(position[1])] });
}

/** Same edge already drawn (can-message has no direction). */
function sameEdge(e, from, to, kind) {
  if (e.kind !== kind) return false;
  if (e.from === from && e.to === to) return true;
  return kind === 'can-message' && e.from === to && e.to === from;
}

/** Adds an edge unless it exists or loops; returns the team unchanged then. */
export function addEdge(team, from, to, kind) {
  if (from === to || !EDGE_KINDS.includes(kind)) return team;
  if (team.edges.some((e) => sameEdge(e, from, to, kind))) return team;
  const t = clone(team);
  t.edges.push({ from, to, kind });
  return t;
}

export function removeEdge(team, index) {
  const t = clone(team);
  t.edges.splice(index, 1);
  return t;
}

/** Arrow keys / hjkl: the nearest node in that direction (cone of 45°). */
export function neighbor(team, fromId, dir) {
  const from = team.nodes.find((n) => n.id === fromId);
  if (!from) return team.nodes[0]?.id ?? null;
  const [fx, fy] = from.position;
  const v = { left: [-1, 0], right: [1, 0], up: [0, -1], down: [0, 1] }[dir];
  let best = null;
  let bestScore = Infinity;
  for (const n of team.nodes) {
    if (n.id === fromId) continue;
    const dx = n.position[0] - fx;
    const dy = n.position[1] - fy;
    const along = dx * v[0] + dy * v[1];
    const across = Math.abs(dx * v[1] - dy * v[0]);
    if (along <= 0 || across > along) continue;
    const score = along + 2 * across;
    if (score < bestScore) [best, bestScore] = [n.id, score];
  }
  return best ?? fromId;
}

/** Connect mode: the other nodes, numbered 1…9 by reading order. */
export function targets(team, fromId) {
  return team.nodes
    .filter((n) => n.id !== fromId)
    .sort((a, b) => a.position[1] - b.position[1] || a.position[0] - b.position[0])
    .slice(0, 9)
    .map((n, i) => ({ n: i + 1, id: n.id }));
}

/**
 * Auto-layout (`=`): the lead on top, then each node one row below the node
 * it reports to (the longest reports-to chain to the lead), nodes not linked
 * by reports-to on the last row; rows centered.
 */
export function autoLayout(team) {
  const t = clone(team);
  const lead = t.nodes.find((n) => n.is_lead)?.id;
  const up = new Map();
  for (const e of t.edges) if (e.kind === 'reports-to') up.set(e.from, [...(up.get(e.from) ?? []), e.to]);
  const depth = new Map();
  const visiting = new Set();
  const d = (id) => {
    if (id === lead) return 0;
    if (depth.has(id)) return depth.get(id);
    if (visiting.has(id)) return null; // a cycle: placed on the last row
    visiting.add(id);
    const ups = (up.get(id) ?? []).map(d).filter((x) => x !== null);
    visiting.delete(id);
    const v = ups.length ? Math.max(...ups) + 1 : null;
    depth.set(id, v);
    return v;
  };
  const known = t.nodes.map((n) => d(n.id)).filter((x) => x !== null);
  const last = (known.length ? Math.max(...known) : 0) + 1;
  const rows = new Map();
  for (const n of t.nodes) {
    const r = d(n.id) ?? last;
    rows.set(r, [...(rows.get(r) ?? []), n]);
  }
  const widest = Math.max(...[...rows.values()].map((r) => r.length), 1);
  const span = widest * NODE_W + (widest - 1) * GAP_X;
  for (const [r, list] of [...rows.entries()].sort((a, b) => a[0] - b[0])) {
    const w = list.length * NODE_W + (list.length - 1) * GAP_X;
    list.sort((a, b) => a.position[0] - b.position[0] || a.id.localeCompare(b.id));
    list.forEach((n, i) => {
      n.position = [Math.round((span - w) / 2 + i * (NODE_W + GAP_X)), r * (NODE_H + GAP_Y)];
    });
  }
  return t;
}

/** The graph's bounds (for the minimap and fitting). */
export function bounds(team) {
  if (!team.nodes.length) return { x: 0, y: 0, w: NODE_W, h: NODE_H };
  const xs = team.nodes.map((n) => n.position[0]);
  const ys = team.nodes.map((n) => n.position[1]);
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, w: Math.max(...xs) + NODE_W - x, h: Math.max(...ys) + NODE_H - y };
}

/** Where an edge starts and ends: on the facing sides of its two nodes. */
export function edgeLine(team, e) {
  const a = team.nodes.find((n) => n.id === e.from);
  const b = team.nodes.find((n) => n.id === e.to);
  if (!a || !b) return null;
  const ca = [a.position[0] + NODE_W / 2, a.position[1] + NODE_H / 2];
  const cb = [b.position[0] + NODE_W / 2, b.position[1] + NODE_H / 2];
  const clip = (c, other) => {
    const dx = other[0] - c[0];
    const dy = other[1] - c[1];
    const sx = dx ? NODE_W / 2 / Math.abs(dx) : Infinity;
    const sy = dy ? NODE_H / 2 / Math.abs(dy) : Infinity;
    const s = Math.min(sx, sy, 1);
    return [c[0] + dx * s, c[1] + dy * s];
  };
  const [x1, y1] = clip(ca, cb);
  const [x2, y2] = clip(cb, ca);
  return { x1, y1, x2, y2, mx: (x1 + x2) / 2, my: (y1 + y2) / 2 };
}

/**
 * Instant checks while editing (the core's `validate` is the reference on
 * save): no lead / several leads, nodes not linked to the lead, reports-to
 * cycles. `{ node, text }` like the core's issues.
 */
export function localIssues(team) {
  const out = [];
  if (!team.nodes.length) return [{ node: null, text: 'the team has no nodes' }];
  const leads = team.nodes.filter((n) => n.is_lead);
  if (!leads.length) out.push({ node: null, text: 'no lead: mark exactly one node as the lead' });
  if (leads.length > 1)
    out.push({ node: leads[1].id, text: `more than one lead: ${leads.map((n) => n.id).join(', ')}` });
  if (leads.length === 1) {
    const adj = new Map();
    for (const e of team.edges) {
      adj.set(e.from, [...(adj.get(e.from) ?? []), e.to]);
      adj.set(e.to, [...(adj.get(e.to) ?? []), e.from]);
    }
    const seen = new Set([leads[0].id]);
    const stack = [leads[0].id];
    while (stack.length) for (const m of adj.get(stack.pop()) ?? []) if (!seen.has(m)) seen.add(m), stack.push(m);
    for (const n of team.nodes)
      if (!seen.has(n.id)) out.push({ node: n.id, text: `${n.id} is not reachable from the lead` });
  }
  const next = new Map();
  for (const e of team.edges) if (e.kind === 'reports-to') next.set(e.from, [...(next.get(e.from) ?? []), e.to]);
  const mark = new Map();
  const path = [];
  const dfs = (n) => {
    mark.set(n, 1);
    path.push(n);
    for (const m of next.get(n) ?? []) {
      if (mark.get(m) === 1) return [...path.slice(path.indexOf(m)), m];
      if (!mark.get(m)) {
        const c = dfs(m);
        if (c) return c;
      }
    }
    path.pop();
    mark.set(n, 2);
    return null;
  };
  for (const n of team.nodes) {
    if (mark.get(n.id)) continue;
    const c = dfs(n.id);
    if (c) {
      out.push({ node: c[0], text: `reports-to cycle: ${c.join(' → ')}` });
      break;
    }
  }
  return out;
}

/** Undo/redo over whole team snapshots (FR-301), capped. */
export class History {
  constructor(limit = 100) {
    this.past = [];
    this.future = [];
    this.limit = limit;
  }
  /** Records `before` (the team before an edit). */
  push(before) {
    this.past.push(clone(before));
    if (this.past.length > this.limit) this.past.shift();
    this.future = [];
  }
  undo(current) {
    if (!this.past.length) return null;
    this.future.push(clone(current));
    return this.past.pop();
  }
  redo(current) {
    if (!this.future.length) return null;
    this.past.push(clone(current));
    return this.future.pop();
  }
}

export const MODEL_ALIASES = ['opus', 'sonnet', 'haiku', 'fable'];

/** The model select's option for a stored value: '' (default), an alias, or 'custom'. */
export function modelOption(value) {
  if (value == null || value === '') return '';
  return MODEL_ALIASES.includes(value) ? value : 'custom';
}

/** The value to save from the select's option and the custom text: null for default or empty custom. */
export function modelValue(option, text = '') {
  if (option === '') return null;
  if (option === 'custom') return text.trim() || null;
  return option;
}

/** A node with its model override set (null clears the key). */
export function setNodeModel(team, id, model) {
  const t = clone(team);
  const n = t.nodes.find((x) => x.id === id);
  if (n) {
    n.overrides = { ...n.overrides };
    if (model == null) delete n.overrides.model;
    else n.overrides.model = model;
  }
  return t;
}
