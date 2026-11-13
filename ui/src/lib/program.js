// Programs (improve-intake notes §6–§7, api-contract §4): the plan dialog's edits and words, the
// Programs lane rows, the Program view's states and the program cards. Pure functions over the
// core's `Program`, `PlanIssue` and `ProgramRow`; the components only render them.

import { clock, plural, ppAmount } from './format.js';
import { isOpen } from './inbox.js';
import { ppRange } from './intake.js';
import { trackerName } from './workmodel.js';

// ---------------------------------------------------------------- ids and refs

/** A WorkNode id's ref: `gh:acme/shop#13` → `#13`, `lin:ENG-42` → `ENG-42`, `local:dark-mode.md` → `dark-mode.md`. */
export function refOfId(id) {
  const s = String(id ?? '');
  if (s.startsWith('gh:')) return s.includes('#') ? `#${s.split('#').pop()}` : s.slice(3);
  const colon = s.indexOf(':');
  return colon >= 0 ? s.slice(colon + 1) : s;
}

/** "acme/shop" from a GitHub node id ("gh:acme/shop#12", "gh-milestone:acme/shop/3"); '' for other trackers. */
export function repoOfId(id) {
  const m = /^gh(?:-milestone)?:([^/\s#]+\/[^/\s#]+)/.exec(String(id ?? ''));
  return m ? m[1] : '';
}

/** The tracker of a node id, in copy: GitHub, Linear, Local (else "the tracker"). */
export const trackerOfId = (id) => trackerName({ id });

/** "≈ 4–7 pp" (P4.5 rounding); "rough ≈ 4–7 pp" when the basis is the default. */
export function estText(e, { unit = true } = {}) {
  const r = ppRange(e?.low_pp, e?.high_pp);
  if (!r) return '—';
  const v = `${r.startsWith('<') ? r : `≈ ${r}`}${unit ? ' pp' : ''}`;
  return e?.rough ? `rough ${v}` : v;
}

// ---------------------------------------------------------------- order (§6.2 Order column)

const cutSet = (p) => new Set((p?.cut ?? []).map(([a, b]) => `${a}>${b}`));

/** The feature's number in the plan (1-based, row order). */
export const numberOf = (p, key) => (p?.features ?? []).findIndex((f) => f.key === key) + 1;

/**
 * The edges a feature waits on (cut ones left out), with the waited-on feature's number and skip. An edge on an
 * item left out because it already runs (`source: "external"`, from `x:<run id>`) has `external` and the item's
 * `ref` ("#13", the first of its items) instead of a number; externals come last.
 */
export function depsOf(p, key) {
  const cut = cutSet(p);
  const byKey = new Map((p?.features ?? []).map((f, i) => [f.key, { f, n: i + 1 }]));
  const seen = new Set();
  const out = [];
  for (const e of p?.edges ?? []) {
    if (e.to !== key || cut.has(`${e.from}>${e.to}`) || seen.has(e.from)) continue;
    const items = e.items ?? [];
    if (e.source === 'external' || String(e.from).startsWith('x:')) {
      seen.add(e.from);
      const ref = items[0] ? refOfId(items[0]) : '';
      out.push({
        key: e.from,
        n: Number.POSITIVE_INFINITY,
        external: true,
        ref,
        skipped: false,
        source: 'external',
        items,
        slug: ref,
      });
      continue;
    }
    if (!byKey.has(e.from)) continue;
    seen.add(e.from);
    const { f, n } = byKey.get(e.from);
    out.push({ key: e.from, n, skipped: !!f.skip, source: e.source, items, slug: f.slug });
  }
  return out.sort((a, b) => a.n - b.n);
}

/**
 * "first" · "after 1" · "after 1, 2" · "after 1 (skipped)" · "after #13 merges" (an item running outside the
 * program); a skipped row: "first (skipped)".
 */
export function orderText(p, f) {
  const deps = depsOf(p, f.key);
  const inner = deps.filter((d) => !d.external);
  const outer = deps.filter((d) => d.external);
  const parts = [
    inner.length ? `after ${inner.map((d) => d.n).join(', ')}${inner.every((d) => d.skipped) ? ' (skipped)' : ''}` : '',
    outer.length
      ? `${inner.length ? '' : 'after '}${outer.map((d) => d.ref || 'a running item').join(', ')} ${outer.length > 1 ? 'merge' : 'merges'}`
      : '',
  ].filter(Boolean);
  const base = parts.length ? parts.join(', ') : 'first';
  return f.skip ? `${base} (skipped)` : base;
}

/** "#13 blocks #14" for a tracker edge's items (pairs blocker, blocked). */
export function blocksText(items) {
  const refs = items.map(refOfId);
  const pairs = [];
  for (let i = 0; i + 1 < refs.length; i += 2) pairs.push(`${refs[i]} blocks ${refs[i + 1]}`);
  return pairs.join(', ');
}

/** The muted line under the Order cell: "⛓ #13 blocks #14" · "Sequential" · "Added by you"; '' when first. */
export function orderNote(p, f) {
  const deps = depsOf(p, f.key).filter((d) => !d.skipped);
  if (!deps.length) return '';
  if (deps.every((d) => d.source === 'sequential')) return 'Sequential';
  const tracker = deps.filter((d) => (d.source === 'tracker' || d.external) && d.items.length);
  if (tracker.length) return `⛓ ${tracker.map((d) => blocksText(d.items)).join(', ')}`;
  return 'Added by you';
}

/** The Order cell's tooltip: "#13 blocks #14 in GitHub" / "Sequential" / "Added by you". */
export function orderTip(p, f) {
  const deps = depsOf(p, f.key);
  if (!deps.length) return 'Starts first';
  return deps
    .map((d) => {
      if (d.source === 'sequential') return `Sequential: after ${d.slug}`;
      if (d.source === 'user') return `Added by you: after ${d.slug}`;
      if (d.external) return `${d.ref || 'The item'} already runs outside this program: waits for its merge`;
      return `${blocksText(d.items) || `after ${d.slug}`} in ${trackerOfId(d.items[0])}${d.skipped ? ' (skipped: no wait)' : ''}`;
    })
    .join(' · ');
}

// ---------------------------------------------------------------- plan edits (§6.2 Row ⋯, §12.3)

// (a plain copy: the draft may be a Svelte $state proxy, which structuredClone refuses)
const clone = (p) => JSON.parse(JSON.stringify(p));

/** A copy of the plan with feature `key` changed by `fn`. */
function editFeature(p, key, fn) {
  const q = clone(p);
  const f = q.features.find((x) => x.key === key);
  if (f) fn(f);
  return q;
}

/** Skip / unskip a feature (x): the row dims, its estimate leaves the total, dependents stop waiting on it. */
export const toggleSkip = (p, key) => withTotals(editFeature(p, key, (f) => (f.skip = !f.skip)));

/** Mode team / solo (m); solo has no team; team takes the first team when it had none. */
export const setMode = (p, key, mode, teams = []) =>
  editFeature(p, key, (f) => {
    f.mode = mode;
    f.team = mode === 'solo' ? null : (f.team ?? teams[0]?.id ?? null);
  });

export const setTeam = (p, key, team) => editFeature(p, key, (f) => (f.team = team || null));

export const setBrief = (p, key, brief) => editFeature(p, key, (f) => (f.brief = brief));

/** Merge into previous (⇧M): the row's items join the row above, its brief appended; edges follow. */
export function mergeIntoPrevious(p, key) {
  const q = clone(p);
  const i = q.features.findIndex((x) => x.key === key);
  if (i <= 0) return q;
  const [f] = q.features.splice(i, 1);
  const prev = q.features[i - 1];
  prev.items = [...prev.items, ...f.items.filter((x) => !prev.items.includes(x))];
  prev.brief = `${prev.brief ?? ''}\n\n${f.brief ?? ''}`.trim();
  const lo = (prev.estimate?.low_pp ?? 0) + (f.skip ? 0 : (f.estimate?.low_pp ?? 0));
  const hi = (prev.estimate?.high_pp ?? 0) + (f.skip ? 0 : (f.estimate?.high_pp ?? 0));
  prev.estimate = { ...(prev.estimate ?? {}), low_pp: round1(lo), high_pp: round1(hi) };
  const re = (k) => (k === f.key ? prev.key : k);
  const seen = new Set();
  q.edges = (q.edges ?? [])
    .map((e) => ({ ...e, from: re(e.from), to: re(e.to) }))
    .filter((e) => e.from !== e.to && !seen.has(`${e.from}>${e.to}`) && seen.add(`${e.from}>${e.to}`));
  q.cut = (q.cut ?? []).map(([a, b]) => [re(a), re(b)]).filter(([a, b]) => a !== b);
  return withTotals(q);
}

/** Remove dependency (row ⋯, the cycle fix): cuts edge `from → to`; the cut is kept on a re-plan (F10). */
export function cutEdge(p, from, to) {
  const q = clone(p);
  if (!(q.cut ?? []).some(([a, b]) => a === from && b === to)) q.cut = [...(q.cut ?? []), [from, to]];
  return q;
}

const round1 = (x) => Math.round(x * 10) / 10;

/** The program's totals over the features not skipped, against its room (the core's F9 rule). */
export function withTotals(q) {
  const live = q.features.filter((f) => !f.skip);
  const low = round1(live.reduce((s, f) => s + (f.estimate?.low_pp ?? 0), 0));
  const high = round1(live.reduce((s, f) => s + (f.estimate?.high_pp ?? 0), 0));
  const room = q.estimate?.room_pp;
  const over = room == null ? 'none' : low > room ? 'low' : high > room ? 'high' : 'none';
  q.estimate = { ...(q.estimate ?? {}), low_pp: low, high_pp: high, over_room: over };
  return q;
}

/** Features that would start (not skipped). */
export const liveCount = (p) => (p?.features ?? []).filter((f) => !f.skip).length;

/** How many features differ from the plan (the re-plan confirm counts them). */
export function editedCount(p, base) {
  if (!p || !base) return 0;
  const before = new Map(base.features.map((f) => [f.key, JSON.stringify(f)]));
  let n = p.features.filter((f) => before.get(f.key) !== JSON.stringify(f)).length;
  n += base.features.filter((f) => !p.features.some((x) => x.key === f.key)).length;
  if (!n && JSON.stringify(p.cut) !== JSON.stringify(base.cut)) n = 1;
  return n;
}

// ---------------------------------------------------------------- issues (§6.4)

/**
 * The client's view of the issues: the core's, with cycles re-checked on the edited draft (a cut
 * edge or a skip fixes a loop before submit) and `no_team` re-checked on the rows (Make it solo).
 * `teams`: the project's team ids. `over_room` and the left-out notes follow the edited totals.
 */
export function liveIssues(p, issues = [], teams = null) {
  if (!p) return issues ?? [];
  const mk = (kind, blocking, more = {}) => ({
    kind,
    blocking,
    features: [],
    items: [],
    feature: null,
    edges: [],
    message: '',
    ...more,
  });
  // The three kinds the client re-checks on the draft; the core's others stay as sent.
  const out = (issues ?? []).filter((i) => !['cycle', 'no_team', 'over_room'].includes(i.kind));
  const cyc = findCycle(p);
  if (cyc) {
    const items = cyc.flatMap(([a, b]) =>
      (p.edges ?? []).filter((e) => e.from === a && e.to === b).flatMap((e) => e.items ?? []),
    );
    out.unshift(mk('cycle', true, { features: cyc.map(([a]) => a), items, edges: cyc }));
  }
  for (const f of p.features) {
    if (f.skip || f.mode !== 'team') continue;
    if (!f.team || (teams != null && !teams.includes(f.team)))
      out.push(mk('no_team', true, { features: [f.key], feature: f.slug }));
  }
  const over = p.estimate?.over_room;
  if (over && over !== 'none' && p.estimate?.room_pp != null) out.push(mk('over_room', false));
  // Blocking first, the core's order inside.
  return [...out.filter((i) => i.blocking), ...out.filter((i) => !i.blocking)];
}

/** One loop among the live features' edges (cut and skipped left out), as its closing edges. */
function findCycle(p) {
  const cut = cutSet(p);
  const live = new Set(p.features.filter((f) => !f.skip).map((f) => f.key));
  const adj = new Map();
  for (const e of p.edges ?? []) {
    if (cut.has(`${e.from}>${e.to}`) || !live.has(e.from) || !live.has(e.to)) continue;
    if (!adj.has(e.from)) adj.set(e.from, []);
    adj.get(e.from).push(e.to);
  }
  const mark = new Map();
  const path = [];
  const dfs = (n) => {
    mark.set(n, 1);
    path.push(n);
    for (const m of adj.get(n) ?? []) {
      if (mark.get(m) === 1) {
        const c = path.slice(path.indexOf(m));
        return c.map((a, i) => [a, c[(i + 1) % c.length]]);
      }
      if (!mark.get(m)) {
        const r = dfs(m);
        if (r) return r;
      }
    }
    path.pop();
    mark.set(n, 2);
    return null;
  };
  for (const f of p.features)
    if (live.has(f.key) && !mark.get(f.key)) {
      const r = dfs(f.key);
      if (r) return r;
    }
  return null;
}

/** The issues block's header: "1 thing to fix before starting" · "Nothing to fix · 2 notes" · '' when none. */
export function issuesHeader(list) {
  const block = list.filter((i) => i.blocking).length;
  const notes = list.length - block;
  if (block) return `${plural(block, 'thing')} to fix before starting`;
  if (notes) return `Nothing to fix · ${plural(notes, 'note')}`;
  return '';
}

/** The issue line's copy (§6.4), named from the plan. `ctx`: { tracker, project, root_ref }. */
export function issueText(i, p, ctx = {}) {
  const tracker = ctx.tracker ?? 'the tracker';
  const slug = (k) => p?.features.find((f) => f.key === k)?.slug ?? k;
  switch (i.kind) {
    case 'cycle': {
      const names = (i.features ?? []).map(slug);
      const refs = [...new Set((i.items ?? []).map(refOfId))];
      const tail = refs.length ? ` (${refs.join(' ↔ ')} in ${tracker})` : '';
      return `Order loop: ${names.join(' and ')} wait on each other${tail}.`;
    }
    case 'already_running': {
      const ref = refOfId(i.items?.[0]);
      return `${ref} is already running as ${i.feature ?? 'a feature'}. It is left out; features that depend on it wait for its merge.`;
    }
    case 'closed': {
      const refs = (i.items ?? []).map(refOfId);
      const inc = new Set(p?.include ?? []);
      const kept = (i.items ?? []).length > 0 && (i.items ?? []).every((x) => inc.has(x));
      if (refs.length === 1)
        return `${refs[0]} is closed in ${tracker}. ${kept ? 'It stays in its feature.' : 'It is left out.'}`;
      return `${refs.join(', ')} are closed in ${tracker}. ${kept ? 'They stay in their features.' : 'They are left out.'}`;
    }
    case 'no_team':
      return `${i.feature ?? slug(i.features?.[0])} needs a team: ${ctx.project ?? 'the project'} has no team.`;
    case 'over_room': {
      const e = p?.estimate ?? {};
      const end = e.over_room === 'low' ? '' : ' at the high end';
      return `${estText(e)} is over this week's room for ${ctx.project ?? 'the project'}${end} (${Math.round(e.room_pp ?? 0)} pp left). Features wait for room under the Governor.`;
    }
    default:
      return i.message ?? '';
  }
}

/** The fixes offered on an issue line (§6.4). */
export function issueFixes(i, p = null) {
  switch (i.kind) {
    case 'closed': {
      const inc = new Set(p?.include ?? []);
      return (i.items ?? []).every((x) => inc.has(x)) ? [] : [{ id: 'include', label: 'Include anyway' }];
    }
    case 'cycle':
      return [
        { id: 'cut', label: 'Remove a dependency…' },
        { id: 'sequential', label: 'Use tree order' },
      ];
    case 'already_running':
      return [{ id: 'feature', label: 'Show feature', key: 'g' }];
    case 'no_team':
      return [
        { id: 'solo', label: 'Make it solo' },
        { id: 'team', label: 'Create a team' },
      ];
    default:
      return [];
  }
}

/** Rows with an issue get a mark in their Order cell. */
export function flaggedKeys(list) {
  return new Set(list.filter((i) => i.kind === 'cycle' || i.kind === 'no_team').flatMap((i) => i.features ?? []));
}

// ---------------------------------------------------------------- plan lines (§6.2)

/** "Total ≈ 10–17 pp of the week", the room and "over at the high end" / "over this week's room". */
export function totalsParts(p, projectName = 'the project') {
  const e = p?.estimate ?? {};
  const total = `Total ${estText(e)} of the week`;
  const room = e.room_pp != null ? `${Math.round(e.room_pp)} pp left this week for ${projectName}` : '';
  const over = e.over_room === 'low' ? "over this week's room" : e.over_room === 'high' ? 'over at the high end' : '';
  return { total, room, over };
}

/**
 * The write-back line (§6.2, D4): items are written when their own feature starts. `t`: the
 * project's TrackerSettings; `rootRef`: the single root's ref (null: several roots, no closure).
 */
export function writeBackLine(t, rootRef = null) {
  if (!t?.write_back?.enabled) return '';
  const kind = t.tracker?.kind;
  if (kind !== 'github' && kind !== 'linear') return '';
  let start;
  if (kind === 'github' && !t.tracker.project_board)
    start = `When each feature starts, its item gets the label ${t.write_back.running_label || 'orch:running'} and a comment in GitHub.`;
  else if (kind === 'github')
    start = `When each feature starts, its item moves to ${t.write_back.board_status || 'In progress'} on the board and gets a comment in GitHub.`;
  else start = 'When each feature starts, its item moves to a started state and gets a comment in Linear.';
  if (!rootRef) return start;
  const close =
    kind === 'github' ? `${rootRef} closes when all are done.` : `${rootRef} moves to Done when all are done.`;
  return `${start} ${close}`;
}

/** "Skipped: checkout-summary · Left out: none", or the left-out notes of §6.2. */
export function footLine(p) {
  const skipped = (p?.features ?? []).filter((f) => f.skip).map((f) => f.slug);
  const closed = (p?.left_out?.closed ?? []).map(refOfId);
  const running = (p?.left_out?.running ?? []).map((r) => `${refOfId(r.id)}${r.feature ? ` (${r.feature})` : ''}`);
  const parts = [];
  if (closed.length) parts.push(`Closed sub-issues left out: ${closed.join(', ')}`);
  if (running.length) parts.push(`Already running, left out: ${running.join(', ')}`);
  if (parts.length)
    return [skipped.length ? `Skipped: ${skipped.join(', ')}` : null, ...parts].filter(Boolean).join(' · ');
  return `Skipped: ${skipped.length ? skipped.join(', ') : 'none'} · Left out: none`;
}

/** Mode and At once choices (the plan rows, the Program view's row editor and settings). */
export const MODES = [
  { value: 'team', label: 'Team' },
  { value: 'solo', label: 'Solo' },
];
export const AT_ONCE = [1, 2, 3, 4, 5].map((n) => ({ value: n, label: String(n) }));

/** "Start program · 3 features". */
export const startLabel = (p) => `Start program · ${plural(liveCount(p), 'feature')}`;

/** The Split into choices (§6.2): containers say "issue", items "sub-issue". */
export function splitChoices(container, hasGrandchildren = true) {
  const unit = container ? 'issue' : 'sub-issue';
  const sub = container ? 'sub-issue' : 'sub-sub-issue';
  return [
    { value: 'child', label: `One feature per ${unit}` },
    {
      value: 'grandchild',
      label: `Per ${sub}`,
      disabled: !hasGrandchildren,
      reason: hasGrandchildren ? undefined : `No ${sub}s`,
    },
    { value: 'single', label: 'One feature for all' },
  ];
}

/** "From" cell: the first item's ref and "+2" when more items are in its scope. */
export function fromText(f) {
  const ref = refOfId(f.items?.[0]);
  return f.items?.length > 1 ? `${ref} +${f.items.length - 1}` : ref;
}

export const fromTip = (f) =>
  f.items?.length > 1 ? `Also in its scope: ${f.items.slice(1).map(refOfId).join(', ')}` : '';

/** "#12 is already in program Checkout v2." from a stale refusal. */
export function staleText(s, rootRef) {
  const m = /already in program (.+)$/.exec(s?.message ?? '');
  if (m) return `${rootRef} is already in program ${m[1]}.`;
  return `${rootRef} is gone from the tracker.`;
}

// ---------------------------------------------------------------- graph strip (§6.2, §7.2)

/** Nodes (numbered, x by level) and arrows for the strip; null without edges (the strip is hidden). */
export function graphOf(p) {
  const feats = p?.features ?? [];
  const cut = cutSet(p);
  const num = new Map(feats.map((f, i) => [f.key, i + 1]));
  const edges = (p?.edges ?? []).filter((e) => !cut.has(`${e.from}>${e.to}`) && num.has(e.from) && num.has(e.to));
  if (!edges.length) return null;
  // Level = longest path from a root (loops are cut by the visit guard).
  const level = new Map();
  const incoming = new Map();
  for (const e of edges) incoming.set(e.to, [...(incoming.get(e.to) ?? []), e.from]);
  const into = (k) => incoming.get(k) ?? [];
  const lv = (k, guard = new Set()) => {
    if (level.has(k)) return level.get(k);
    if (guard.has(k)) return 0;
    guard.add(k);
    const v = Math.max(0, ...into(k).map((x) => lv(x, guard) + 1));
    level.set(k, v);
    return v;
  };
  const nodes = feats.map((f, i) => ({
    key: f.key,
    n: i + 1,
    level: lv(f.key),
    skipped: !!f.skip || f.state === 'skipped',
    state: f.state,
  }));
  // Features with no edge go after the deepest level, as the boards draw them.
  const linked = new Set(edges.flatMap((e) => [e.from, e.to]));
  const maxLevel = Math.max(0, ...nodes.filter((n) => linked.has(n.key)).map((n) => n.level));
  let free = 0;
  for (const n of nodes) if (!linked.has(n.key)) n.level = maxLevel + 1 + free++;
  // Rows within a level, in plan order (after the moves: a moved feature leaves no gap in level 0).
  const rows = new Map();
  for (const n of nodes) {
    const r = rows.get(n.level) ?? 0;
    n.row = r;
    rows.set(n.level, r + 1);
  }
  return { nodes, edges: edges.map((e) => ({ from: e.from, to: e.to, fromN: num.get(e.from), toN: num.get(e.to) })) };
}

// ---------------------------------------------------------------- Programs lane (§7.1)

const COUNT_WORDS = [
  ['running', 'running'],
  ['pr', 'PR open'],
  ['waiting', 'waiting'],
  ['queued', 'queued'],
  ['failed', 'failed'],
  ['skipped', 'skipped'],
];

/**
 * Counts as parts: "1 of 3 merged", then running, PR open, waiting, queued, failed, skipped (non-zero).
 * `short` (rows under 880 px, the 1280 board): "0/3 merged · 1 running · 2 more" (the first count, the rest summed).
 */
export function countParts(row, { short = false } = {}) {
  const c = row?.counts ?? {};
  const total = progressOf(row).length;
  const out = [
    { key: 'merged', text: short ? `${c.merged ?? 0}/${total} merged` : `${c.merged ?? 0} of ${total} merged` },
  ];
  const rest = COUNT_WORDS.filter(([k]) => c[k] > 0).map(([k, w]) => ({
    key: k,
    text: `${c[k]} ${w}`,
    error: k === 'failed',
    n: c[k],
  }));
  if (!short || rest.length <= 2) return [...out, ...rest.map(({ n: _, ...p }) => p)];
  const [first, ...more] = rest;
  return [
    ...out,
    { key: first.key, text: first.text, error: first.error },
    { key: 'more', text: `${more.reduce((s, p) => s + p.n, 0)} more`, error: more.some((p) => p.error) },
  ];
}

/** Progress segments: one per feature not skipped, filled when merged. */
export function progressOf(row) {
  const live = (row?.features ?? []).filter((f) => f.state !== 'skipped');
  return live.map((f) => f.state === 'merged');
}

/** The lane's count ("Programs · 1 running · 1 paused"): "1 running", "1 running · 1 paused"; '' when none. */
export function laneCount(rows) {
  const run = rows.filter((r) => r.status === 'running').length;
  const paused = rows.filter((r) => r.status === 'paused').length;
  return [run && `${run} running`, paused && `${paused} paused`].filter(Boolean).join(' · ');
}

/** "2/3" (live agents / the Governor's live cap) for "queued · live cap 2/3"; null without a cap. */
export const liveCapText = (g) => (g?.live_cap != null ? `${g.live ?? 0}/${g.live_cap}` : null);

/** Sub-rows are expanded by default with ≤ 5 features. */
export const SUBROWS_OPEN_MAX = 5;

const FLAG_WORDS = {
  closed_upstream: (t) => `closed in ${t}`,
  deleted_upstream: (t) => `deleted in ${t}`,
  new_dependency: () => 'new dependency',
};

/** A feature's flags in words: "closed in GitHub · new dependency"; '' without any. */
export const flagText = (f, tracker = 'GitHub') =>
  (f?.flags ?? [])
    .map((x) => FLAG_WORDS[x]?.(tracker))
    .filter(Boolean)
    .join(' · ');

/** A sub-row (§7.1): glyph, text, tone (muted / error / working / gate), flag tail. `tracker`: GitHub / Linear. */
export function subRowOf(f, tracker = 'GitHub', { liveCap = null } = {}) {
  return { ...subRowState(f, liveCap), flag: flagText(f, tracker) };
}

function subRowState(f, liveCap) {
  const pr = f.pr?.number != null ? ` #${f.pr.number}` : '';
  switch (f.state) {
    case 'merged':
      return { glyph: '✓', text: `merged${pr}`, tone: 'muted' };
    case 'running':
      return {
        glyph: '●',
        text: [f.phase ?? 'running', f.mode === 'solo' ? 'solo' : f.team].filter(Boolean).join(' · '),
        tone: 'working',
      };
    case 'pr':
      return { glyph: '◐', text: `PR${pr} open`, tone: 'gate' };
    case 'waiting':
      return {
        glyph: '○',
        text: f.waiting_on?.length ? `waiting for ${andList(f.waiting_on)} to merge` : 'waiting',
        tone: '',
      };
    case 'queued':
    case 'planned':
      return { glyph: '○', text: liveCap ? `queued · live cap ${liveCap}` : 'queued', tone: '' };
    case 'skipped':
      return { glyph: '–', text: 'skipped', tone: 'muted' };
    case 'failed':
      return { glyph: '✕', text: 'failed', tone: 'error', open: true };
    default:
      return { glyph: '○', text: f.state ?? '', tone: '' };
  }
}

function andList(xs) {
  if (xs.length <= 1) return xs.join('');
  return `${xs.slice(0, -1).join(', ')} and ${xs.at(-1)}`;
}

/** The lane's rows from `now.programs` (running and paused), filtered by project, oldest title order. */
export function programRows(programs, shown) {
  return Object.values(programs ?? {})
    .filter((r) => isLive(r) && shown(r.project))
    .sort((a, b) => String(a.title).localeCompare(String(b.title)) || String(a.id).localeCompare(String(b.id)));
}

// ---------------------------------------------------------------- Program view (§7.2)

/** The header chip: "running · 1 of 3 merged" · "paused · …" · "done · 3 merged · 13 Nov 18:20" · "cancelled · 1 merged". */
export function statusChip(p) {
  const merged = (p?.features ?? []).filter((f) => f.state === 'merged').length;
  const total = (p?.features ?? []).filter((f) => !f.skip && f.state !== 'skipped').length; // (a draft's skip, a live program's state)
  switch (p?.status) {
    case 'done':
      return { tone: 'done', text: `done · ${merged} merged${p.done_at_ms ? ` · ${dateTime(p.done_at_ms)}` : ''}` };
    case 'cancelled':
      return { tone: 'muted', text: `cancelled · ${merged} merged` };
    case 'paused':
      return { tone: 'paused', text: `paused · ${merged} of ${total} merged` };
    default:
      return { tone: 'working', text: `running · ${merged} of ${total} merged` };
  }
}

/** "12 Nov 09:40" (local 24 h). */
const dateTime = (ms) =>
  `${new Date(ms).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })} ${clock(ms, ms)}`;

/** "From #12 Checkout v2 · acme/shop ↗ · started 12 Nov 09:40 · 2 at once · by dependencies". */
export function viewSubline(p, { where = '', rootTitle = '' } = {}) {
  return [
    `From ${p.root_ref}${rootTitle ? ` ${rootTitle}` : ''}`,
    where,
    p.started_at_ms ? `started ${dateTime(p.started_at_ms)}` : '',
    `${p.max_parallel} at once`,
    p.sequential ? 'one after the other' : 'by dependencies',
  ].filter(Boolean);
}

/** The State cell of a view row (§7.2), with its tone. `paused`: the program is paused. */
export function viewState(f, p, { liveCap = null } = {}) {
  const pr = f.pr?.number != null ? ` #${f.pr.number}` : '';
  const paused = p?.status === 'paused';
  switch (f.state) {
    case 'merged':
      return { tone: 'done', text: `merged${pr}${f.merged_at_ms ? ` · ${dateTime(f.merged_at_ms)}` : ''}` };
    case 'running':
      return { tone: 'working', text: `running · ${f.phase ?? 'starting'}` };
    case 'pr':
      return { tone: 'gate', text: `PR${pr} open` };
    case 'waiting': {
      if (paused) return { tone: '', text: 'waiting · program paused' };
      const deps = (f.depends_on ?? [])
        .map((k) => p.features.find((x) => x.key === k))
        .filter((x) => x && x.state !== 'merged');
      return { tone: '', text: deps.length ? `waiting for ${andList(deps.map((x) => x.slug))} to merge` : 'waiting' };
    }
    case 'queued':
      return paused
        ? { tone: '', text: 'queued · program paused' }
        : { tone: 'queued', text: liveCap ? `queued · live cap ${liveCap}` : 'queued' };
    case 'planned':
      return paused ? { tone: '', text: 'planned · program paused' } : { tone: '', text: 'planned' };
    case 'skipped':
      // (a cancel drops what had not started: the core marks those skipped)
      return { tone: 'muted', text: p?.status === 'cancelled' ? 'dropped' : 'skipped' };
    case 'failed':
      return { tone: 'error', text: f.error ? `failed · ${f.error}` : 'failed' };
    default:
      return { tone: '', text: f.state ?? '' };
  }
}

/** The Actual cell: "5.2 pp", "2.1 pp so far" (running), "—". */
export function actualText(f) {
  if (f.actual_pp == null) return '—';
  return `${ppAmount(f.actual_pp)}${['running', 'pr'].includes(f.state) ? ' so far' : ''}`;
}

/** A feature can be skipped while it has not started (core rule: planned, waiting, queued without a run, failed). */
export const skippable = (f) =>
  ['planned', 'waiting', 'failed'].includes(f?.state) || (f?.state === 'queued' && !f.run && !f.agent);

/** Live programs can be edited; done / cancelled are read-only. */
export const isLive = (p) => p?.status === 'running' || p?.status === 'paused';

/** The done / cancelled foot line. */
export function overLine(p) {
  if (p?.status === 'cancelled') return 'Running features kept running as normal features; waiting ones were dropped.';
  if (p?.status === 'done') return `Program ${p.title} done.`;
  return '';
}

// ---------------------------------------------------------------- cards (§7.3–§7.5)

/** The tag under a spec / PR card's title: "Checkout v2 · 2 of 3". */
export const programTag = (d) =>
  d?.program?.title ? `${d.program.title} · ${d.program.index} of ${d.program.total}` : '';

/** After approving a program spec: the next pending spec gate of the same program (§7.5), or null. */
export function nextSpec(decisions, d) {
  const id = d?.program?.id;
  if (!id) return null;
  return (
    Object.values(decisions ?? {})
      .filter((x) => x.id !== d.id && x.kind === 'gate' && x.gate === 'spec' && x.program?.id === id && isOpen(x))
      .sort(
        (a, b) => (a.program?.index ?? 0) - (b.program?.index ?? 0) || (a.created_ms ?? 0) - (b.created_ms ?? 0),
      )[0] ?? null
  );
}

/** "Next: checkout-summary's spec". */
export const nextSpecText = (d) => `Next: ${d.subject?.feature ?? d.title}'s spec`;

/** "Include anyway": the closed items of an issue join `program.include` (kept in their feature at start). */
export function includeItems(p, ids) {
  return { ...p, include: [...new Set([...(p.include ?? []), ...ids])] };
}

/** "added 13 Nov": when a new item appeared upstream ('' while unknown). */
export const addedText = (ms) =>
  ms ? `added ${new Date(ms).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })}` : '';
