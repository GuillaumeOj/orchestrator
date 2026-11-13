// Work browser (improve-intake notes §2–§5, §11, §12.1; api-contract §2, §6): the pure rules of
// the tree, its rows, chips, roll-ups, search, filters, view prefs and selection. Nothing here
// reads the app state, the clock or the core; callers pass the data. Tested in test/work-model.test.js.

import { clock, dayHm, plural } from './format.js';
import { ageText, GH_SCOPE_COMMAND, isShownLabel, shownLabels } from './intake.js';
import { TRACKERS } from './source.js';
import { backlogDir, trackerError } from './tracker.js';

// ---------------------------------------------------------------- trackers, kinds

/** The tracker of a node, from its id (`gh:` · `gh-milestone:` · `lin…:` · `local:` · `project:`). */
export function trackerOf(node) {
  const id = String(node?.id ?? '');
  if (id.startsWith('gh')) return 'github';
  if (id.startsWith('lin')) return 'linear';
  if (id.startsWith('local:')) return 'local';
  return null;
}

export const TRACKER_NAME = TRACKERS;
export const trackerName = (node) => TRACKER_NAME[trackerOf(node)] ?? 'the tracker';

/** Containers (no Start solo / team run, F12): milestones, Linear projects, milestones, initiatives. */
export const isContainer = (node) =>
  ['milestone', 'linear_project', 'linear_milestone', 'initiative'].includes(node?.kind);

/** Closed upstream: GitHub `closed`, Linear `completed` / `canceled` (notes §4.2 State). */
export function isClosed(node) {
  const u = node?.upstream ?? {};
  return u.state === 'closed' || u.state_type === 'completed' || u.state_type === 'canceled';
}

// ---------------------------------------------------------------- kind glyphs (notes §2.3, page 02)

export const GLYPH = {
  project: '\u{F401}',
  milestone: '\u{F45D}',
  issue_open: '\u{F41B}',
  issue_done: '\u{F41D}',
  issue_not_planned: '\u{F468}',
  initiative: '\u{F04FE}',
  linear_project: '\u{F01A7}',
  linear_milestone: '\u{25C7}',
  lin_backlog: '\u{25CC}',
  lin_unstarted: '\u{25CB}',
  lin_started: '\u{25D0}',
  lin_completed: '\u{F05E0}',
  lin_canceled: '\u{F0159}',
  local: '\u{F4A5}',
  fold: '\u{F51E}',
};

/** The kind glyph of a node and its accessible name. */
export function glyphOf(node) {
  const u = node?.upstream ?? {};
  switch (node?.kind) {
    case 'milestone':
      return { g: GLYPH.milestone, label: `Milestone ${node.ref || node.title}` };
    case 'linear_project':
      return { g: GLYPH.linear_project, label: 'Linear project' };
    case 'linear_milestone':
      return { g: GLYPH.linear_milestone, label: 'Project milestone' };
    case 'initiative':
      return { g: GLYPH.initiative, label: 'Initiative' };
    case 'local':
      return { g: GLYPH.local, label: 'Local item' };
    default:
      break;
  }
  if (trackerOf(node) === 'linear') {
    const t = u.state_type ?? 'unstarted';
    return { g: GLYPH[`lin_${t}`] ?? GLYPH.lin_unstarted, label: `Linear issue, ${u.state ?? 'Todo'}` };
  }
  if (u.state === 'closed')
    return u.closed_reason === 'not_planned'
      ? { g: GLYPH.issue_not_planned, label: 'Closed issue, not planned' }
      : { g: GLYPH.issue_done, label: 'Closed issue, completed' };
  return { g: GLYPH.issue_open, label: 'Open issue' };
}

// ---------------------------------------------------------------- dates

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** "1 Dec" (`year`: "1 Dec 2026") from "2026-12-01". */
export function shortDate(iso, year = false) {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(String(iso ?? ''));
  if (!m) return '';
  const d = `${Number(m[3])} ${MONTHS[Number(m[2]) - 1]}`;
  return year ? `${d} ${m[1]}` : d;
}

// ---------------------------------------------------------------- row slots (notes §2.2)

/** The title slot: a milestone with a due date reads "v2 · due 1 Dec". */
export function titleOf(node) {
  if (node?.kind === 'milestone' && node.upstream?.due_on)
    return `${node.title} · due ${shortDate(node.upstream.due_on)}`;
  return node?.title ?? '';
}

/** The ref slot: none for milestones / Linear projects (the title is the ref). */
export const refOf = (node) => (isContainer(node) ? '' : (node?.ref ?? ''));

/** The upstream state slot: Linear's state, GitHub's board Status (only with a board), a Linear project's state. */
export function upstreamText(node) {
  const u = node?.upstream ?? {};
  if (node?.kind === 'linear_project') {
    const t = u.target_date ? `target ${shortDate(u.target_date, true)}` : '';
    return [u.state, t].filter(Boolean).join(' · ');
  }
  if (trackerOf(node) === 'linear' && node?.kind === 'issue') return u.state ?? '';
  if (trackerOf(node) === 'github') return u.board_status ?? '';
  return '';
}

/** Children progress: "1/2" on issues, "2 of 6" on containers; '' without children (team tasks excluded, F31). */
export function progressText(node) {
  const c = node?.children;
  if (!c?.total) return '';
  const done = c.done ?? c.total - (c.open ?? 0);
  return isContainer(node) ? `${done} of ${c.total}` : `${done}/${c.total}`;
}

/** Labels: ≤ 2 neutral chips then "+n", never `orch*` / `size:*` (P4.5 §2). */
export const labelsOf = (node) => shownLabels({ labels: node?.upstream?.labels ?? [] });

/** The dependency mark: "blocked by #13" (one open blocker), "blocked by 2", null when every blocker is closed. */
export function depMark(node) {
  const open = (node?.upstream?.blocked_by ?? []).filter((d) => d.open);
  if (!open.length) return null;
  return {
    text: open.length === 1 ? `blocked by ${open[0].ref}` : `blocked by ${open.length}`,
    tip: `Blocked by ${open.map((d) => `${d.ref} ${d.title ?? ''}`.trim()).join(', ')}`,
  };
}

// ---------------------------------------------------------------- app-state chip (notes §2.5)

const PHASE_SHORT = { executing: 'exec.', planning: 'plan.', integrating: 'integ.', spec: 'spec', pr: 'PR' };

/**
 * The app-state chip of a node, or null. `tone` picks the status token
 * (ready | waiting | queued | working | pr | done | program). `short`: the 1280 text.
 * `readyTip`: what the Ready filter is ("label ready"). `click`: what a click opens.
 */
export function chipOf(node, { short = false, readyTip = '' } = {}) {
  const a = node?.app ?? { state: 'none' };
  switch (a.state) {
    case 'queued': {
      const prog = a.program?.title ?? 'a program';
      const waiting = (a.waiting_on ?? []).length > 0;
      return waiting
        ? {
            tone: 'waiting',
            text: short ? 'waiting' : `waiting · ${prog}`,
            tip: `In program ${prog}: waiting for ${a.waiting_on.join(', ')} to merge`,
            click: { kind: 'program', id: a.program?.id },
          }
        : {
            tone: 'queued',
            text: short ? 'queued' : `queued · ${prog}`,
            tip: `In program ${prog}: queued by the Governor`,
            click: { kind: 'program', id: a.program?.id },
          };
    }
    case 'running':
      return {
        tone: 'working',
        text: short ? `running · ${PHASE_SHORT[a.phase] ?? a.phase}` : `running · ${a.feature} · ${a.phase}`,
        tip: `Team run ${a.feature}, phase ${a.phase}. g opens the feature.`,
        click: { kind: 'feature', slug: a.feature },
      };
    case 'solo':
      return {
        tone: 'working',
        text: short ? 'solo' : `solo · ${a.name ?? a.agent}`,
        tip: `Solo session ${a.name ?? a.agent}`,
        click: { kind: 'agent', id: a.agent },
      };
    case 'pr':
      return {
        tone: 'pr',
        text: short ? `PR #${a.pr?.number}` : `PR #${a.pr?.number} open`,
        tip: `Feature ${a.feature} · PR #${a.pr?.number} waiting for review`,
        click: { kind: 'pr', url: a.pr?.url, slug: a.feature },
      };
    case 'done':
      return {
        tone: 'done',
        text: 'merged',
        tip: [`Merged${a.merged_at_ms ? ` ${dayHm(a.merged_at_ms)}` : ''}`, a.feature ? `feature ${a.feature}` : '']
          .filter(Boolean)
          .join(' · '),
        click: a.feature ? { kind: 'feature', slug: a.feature } : null,
      };
    case 'program_root':
      return {
        tone: 'program',
        text: short ? `program ${a.merged}/${a.total}` : `program · ${a.merged} of ${a.total} merged`,
        tip: `Program ${a.program?.title ?? ''}${node.rollup ? `: ${rollupTip(node.rollup)}` : ''}`,
        click: { kind: 'program', id: a.program?.id },
      };
    default:
      if (node?.ready && !node.hidden)
        return {
          tone: 'ready',
          text: 'ready',
          tip: `Matches the Ready filter${readyTip ? ` (${readyTip})` : ''}`,
          click: null,
        };
      return null;
  }
}

/** The "closed upstream" mark next to a chip of a node that runs / waits while closed upstream (§2.5). */
export function closedMark(node) {
  const s = node?.app?.state;
  if (!['running', 'queued', 'solo', 'pr'].includes(s) || !isClosed(node) || node.upstream?.deleted) return null;
  return { text: 'closed upstream', tip: `Closed in ${trackerName(node)} while the feature runs. It is not stopped.` };
}

// ---------------------------------------------------------------- roll-up (notes §2.6)

/** One word list and order for app states (chip, roll-up, program row). */
export const ROLLUP_WORDS = [
  ['running', 'running'],
  ['pr', 'PR open'],
  ['waiting', 'waiting'],
  ['queued', 'queued'],
  ['merged', 'merged'],
  ['failed', 'failed'],
  ['skipped', 'skipped'],
];

const parts = (r) => ROLLUP_WORDS.filter(([k]) => r?.[k] > 0).map(([k, w]) => [r[k], w]);

/** "1 running · 1 waiting · 1 merged"; > 3 parts: "1 running · 3 more"; '' when nothing. */
export function rollupText(r) {
  const p = parts(r);
  if (!p.length) return '';
  if (p.length > 3) return `${p[0][0]} ${p[0][1]} · ${p.slice(1).reduce((s, [n]) => s + n, 0)} more`;
  return p.map(([n, w]) => `${n} ${w}`).join(' · ');
}

/** Every count with its noun: "1 running, 1 waiting, 1 queued, 1 merged". */
export const rollupTip = (r) =>
  parts(r)
    .map(([n, w]) => `${n} ${w}`)
    .join(', ');

/** The right column of a row: its chip when it has an own state, else the roll-up of its descendants. */
export function rightOf(node, opts = {}) {
  const own = node?.app?.state && node.app.state !== 'none';
  if (!own) {
    const r = rollupText(node?.rollup);
    if (r) return { rollup: r, tip: rollupTip(node.rollup) };
  }
  const chip = chipOf(node, opts);
  return chip ? { chip } : null;
}

/** Tails after the slots (§11): deleted upstream (S14), write-back failed (S15), closed parent (S13). */
export function tailOf(node) {
  if (node?.upstream?.deleted) return { warn: true, text: `Deleted in ${trackerName(node)}` };
  if (node?.writeback?.status === 'failed') return { warn: true, text: 'write-back failed' };
  if (isClosed(node) && node.children?.open > 0)
    return { warn: false, text: `closed · ${plural(node.children.open, 'open sub-issue')}` };
  return null;
}

// ---------------------------------------------------------------- actions (notes §5.1, F40)

/** An action of a node: 'ok', 'hidden', { disabled } or { open_program }; team-task lines and projects have none. */
export function actionOf(node, which) {
  if (!node || node.team_task) return 'hidden';
  return node.actions?.[which] ?? 'hidden';
}

export const offered = (a) => a !== 'hidden' && a != null;
export const usable = (a) => a === 'ok' || (a && typeof a === 'object' && 'open_program' in a);
export const disabledTip = (a) => (a && typeof a === 'object' && 'disabled' in a ? a.disabled : '');

/** The feature (or solo session) behind a node's app state, for `g` and the chip. */
export function featureOf(node) {
  const a = node?.app ?? {};
  if (a.state === 'solo') return { agent: a.agent };
  if (['running', 'pr', 'done'].includes(a.state) && a.feature) return { slug: a.feature };
  return null;
}

/** Keys the focused row allows, for the hint row (P4.2 §1.9): [key, words]. */
export function hintsFor(row, { selection = 0, focusMode = false, searching = false, details = true } = {}) {
  if (selection)
    return [
      ['x', 'select'],
      ['⇧J ⇧K', 'extend'],
      ['p', 'run selected as program'],
      ['Esc', 'clear'],
    ];
  if (!row) return [['/', 'search']];
  if (row.kind === 'project')
    return [
      ['⏎', 'details'],
      ['/', 'search'],
    ];
  if (row.kind === 'fold') return [['⏎', row.open ? 'collapse' : 'expand']];
  if (row.kind === 'task')
    return [
      ['⏎', 'open in feature'],
      ['o', 'open'],
    ];
  if (row.kind === 'more') return [['⏎', 'load more']];
  if (row.kind === 'error') return [['⏎', 'retry']];
  if (row.kind === 'srcline')
    return row.fix
      ? [
          ['⏎', 'fix'],
          ['/', 'search'],
        ]
      : [['/', 'search']];
  const n = row.node;
  if (!n) return [];
  const out = [];
  if (searching) out.push(['⏎', 'reveal in tree']);
  else if (!details || !focusMode) out.push(['⏎', 'details']);
  if (featureOf(n)) out.push(['g', 'show feature']);
  if (usable(actionOf(n, 'team'))) out.push(['t', 'team run']);
  if (usable(actionOf(n, 'solo'))) out.push(['a', 'solo']);
  const p = actionOf(n, 'program');
  if (usable(p)) {
    const open = n.children?.open;
    out.push([
      'p',
      typeof p === 'object'
        ? 'open program'
        : isClosed(n) && open
          ? `run as program (${plural(open, 'open sub-issue')})`
          : 'program',
    ]);
  }
  if (n.url || trackerOf(n) === 'local') out.push(['o', 'open']);
  const h = actionOf(n, 'hide');
  if (usable(h)) out.push(['⌫', n.hidden ? 'unhide' : 'hide from Ready']);
  if (!searching && selectable(n).ok) out.push(['x', 'select']);
  if (!searching && n.children?.total > 0) out.push(['z', 'focus here']);
  if (focusMode) out.push(['⇧Z', 'up one level'], ['Esc', 'leave focus']);
  if (searching) out.push(['Esc', 'clear']);
  else out.push(['/', 'search']);
  return out;
}

// ---------------------------------------------------------------- selection (notes §5.4)

/** Whether `x` can select a node: never team-task lines, project rows or closed items. */
export function selectable(node) {
  if (!node || node.team_task) return { ok: false, why: null };
  if (isClosed(node)) return { ok: false, why: `${node.ref || node.title} is closed.` };
  return { ok: true, why: null };
}

export const NOT_SIBLING = 'Select items under the same parent to run them as one program.';

/**
 * `x` on `node` with the current selection ({ parent, ids }): the next selection, or a refusal
 * ({ refused: toast }). Siblings only (same parent); toggling the last one off clears it.
 */
export function toggleSelect(sel, node) {
  const s = selectable(node);
  if (!s.ok) return { refused: s.why };
  const parent = node.parent ?? `roots:${node.project}`;
  if (!sel || !sel.ids.length) return { sel: { parent, ids: [node.id] } };
  if (sel.parent !== parent) return { refused: NOT_SIBLING };
  const ids = sel.ids.includes(node.id) ? sel.ids.filter((x) => x !== node.id) : [...sel.ids, node.id];
  return { sel: ids.length ? { parent, ids } : null };
}

/** "3 selected under #12 Checkout v2" / "2 selected in shop-api". */
export function selectionText(sel, parentNode, projectName = '') {
  const n = sel?.ids?.length ?? 0;
  if (parentNode) return `${n} selected under ${[refOf(parentNode), parentNode.title].filter(Boolean).join(' ')}`;
  return `${n} selected in ${projectName}`;
}

// ---------------------------------------------------------------- breadcrumbs (notes §3, one rule)

/**
 * The Breadcrumb rule: root first; keep the root and the parent, collapse the middle to "…"
 * past `max` segments. Each segment: { id, text } ('…' has id null).
 */
export function crumbs(list, max = 3) {
  const segs = (list ?? []).map((c) => ({
    id: c.id ?? null,
    text: c.title && c.ref && c.title !== c.ref ? `${c.ref} ${c.title}` : (c.title ?? c.ref ?? ''),
  }));
  if (segs.length <= max) return segs;
  return [segs[0], { id: null, text: '…' }, ...segs.slice(-(max - 1))];
}

// ---------------------------------------------------------------- filters (notes §4.2, F3)

export const defaultFilter = () => ({
  state: 'open',
  only_ready: false,
  only_not_started: false,
  labels: [],
  assignees: [],
  milestone: null,
  cycle: null,
  issue_types: [],
  linear_projects: [],
  include_team_tasks: false,
  show_hidden: false,
});

/** A saved filter read back: unknown keys dropped, missing ones defaulted. */
export function cleanFilter(f) {
  const d = defaultFilter();
  if (!f || typeof f !== 'object') return d;
  const out = { ...d };
  for (const k of Object.keys(d)) {
    if (!(k in f)) continue;
    if (Array.isArray(d[k])) out[k] = Array.isArray(f[k]) ? f[k].map(String) : [];
    else if (typeof d[k] === 'boolean') out[k] = !!f[k];
    else out[k] = f[k] ?? null;
  }
  if (!['open', 'closed', 'all'].includes(out.state)) out.state = 'open';
  if (![null, 'current', 'next'].includes(out.cycle)) out.cycle = null;
  return out;
}

/** The count on "Filters · n": every filter off its default but Hide closed (the toolbar shows it) and Show hidden (⋯). */
export function filterCount(f) {
  const counted = Object.keys(defaultFilter()).filter((k) => k !== 'state' && k !== 'show_hidden');
  return counted.filter((k) => (Array.isArray(f[k]) ? f[k].length > 0 : !!f[k])).length;
}

/** Any filter beyond Hide closed narrows the tree (S8 "no match" vs S7 "empty tracker"). */
export const narrowed = (f) => filterCount(f) > 0 || f.state === 'closed';

// ---------------------------------------------------------------- scope and view prefs (F4)

/** The scope key: sorted project ids ("all" stays the ids of every project, read at open). */
export const scopeKey = (ids) => [...(ids ?? [])].sort().join(',');
export const prefsKey = (ids) => `work.view.${scopeKey(ids)}`;

/** The tab title (§1.2): "Work · shop-api" / "Work · 3 projects" / "Work · all projects". */
export function scopeTitle(projects, names) {
  if (!projects) return 'Work · all projects';
  if (projects.length === 1) return `Work · ${names[0] ?? projects[0]}`;
  return `Work · ${projects.length} projects`;
}

export const MAX_EXPANDED = 500;

/** View prefs to save (≤ 500 expanded ids). */
export function prefsOf(v) {
  return {
    filter: v.filter,
    expanded: [...(v.expanded ?? [])].slice(-MAX_EXPANDED),
    collapsed_projects: [...(v.collapsedProjects ?? [])],
    focused: v.focused ?? null,
    details_open: !!v.details_open,
    focus_root: v.focus_root ?? null,
  };
}

/** Saved prefs read back (anything missing: the first-open defaults, §2.1). */
export function readPrefs(saved) {
  const s = saved && typeof saved === 'object' ? saved : {};
  return {
    filter: cleanFilter(s.filter),
    expanded: Array.isArray(s.expanded) ? s.expanded.map(String).slice(-MAX_EXPANDED) : [],
    collapsedProjects: Array.isArray(s.collapsed_projects) ? s.collapsed_projects.map(String) : [],
    focused: typeof s.focused === 'string' ? s.focused : null,
    details_open: s.details_open ?? true,
    focus_root: typeof s.focus_root === 'string' ? s.focus_root : null,
  };
}

// ---------------------------------------------------------------- the tree's rows

/**
 * The visible rows, top to bottom. `t`: { scope: [{id, name, tracker}], multi, nodes: Map, kids: Map
 * (parent key → ids; roots under `roots`), more: { key: {left, cursor} }, expanded: Set, collapsedProjects: Set,
 * loading: Set, errors: Map(key → message), folds: Set (open folds), tasks: Map (node id → team-task nodes),
 * focusRoot: id | null, statusLines: Map(project → line) }.
 * Rows: { kind: 'project'|'node'|'fold'|'task'|'loading'|'error'|'more'|'limit'|'srcline', key, level, indent, … }.
 */
export function visibleRows(t) {
  const out = [];
  const focus = t.focusRoot ? t.nodes.get(t.focusRoot) : null;
  const top = focus ? (focus.depth ?? 1) : t.multi ? 0 : 1;
  const indentOf = (level) => Math.min(4, Math.max(0, level - top));
  /** One line of the tree: its indent follows its level. */
  const line = (kind, key, level, extra = {}) => out.push({ kind, key, level, indent: indentOf(level), ...extra });
  /** A loaded list's state line: an error, else "Loading…" until the list is there; true when one was added. */
  const state = (key, level, loaded) => {
    if (t.errors.has(key))
      return (line('error', `error:${key}`, level, { parent: key, message: t.errors.get(key) }), true);
    if (t.loading.has(key) && !loaded) return (line('loading', `loading:${key}`, level, { parent: key }), true);
    return false;
  };
  const moreLine = (key, level, indent = indentOf(level)) => {
    const left = t.more?.[key]?.left ?? 0;
    const shown = (t.kids.get(key) ?? []).length;
    if (left > 0)
      out.push({ kind: 'more', key: `more:${key}`, level, indent, parent: key, left, shown, roots: key === 'roots' });
  };

  const children = (key, level, parent) => {
    if (state(key, level, t.kids.has(key))) return;
    for (const id of t.kids.get(key) ?? []) node(id, level);
    moreLine(key, level);
    if (parent && trackerOf(parent) === 'github' && (parent.children?.total ?? 0) >= 100)
      line('limit', `limit:${key}`, level);
    if (!(parent?.team_tasks > 0)) return;
    const fk = `fold:${parent.id}`;
    const open = t.folds.has(parent.id);
    line('fold', fk, level, { node: parent, open, count: parent.team_tasks });
    if (!open) return;
    state(fk, level + 1, t.tasks.has(parent.id));
    for (const task of t.tasks.get(parent.id) ?? [])
      line('task', `task:${task.id}`, level + 1, { node: task, parent: parent.id });
  };

  const node = (id, level) => {
    const n = t.nodes.get(id);
    if (!n) return;
    const lv = n.depth ?? level;
    const hasKids = (n.children?.total ?? 0) > 0 || (n.team_tasks ?? 0) > 0 || (t.kids.get(id)?.length ?? 0) > 0;
    const open = t.expanded.has(id) && hasKids;
    line('node', id, lv, { node: n, open, hasKids });
    if (open) children(id, lv + 1, n);
  };

  if (focus) {
    node(focus.id, top);
    return out;
  }
  if (!t.multi) {
    children('roots', 1, null);
    return out;
  }
  // Several projects: a level-0 row each, its status line, then its roots (grouped in one pass).
  const byProject = new Map();
  for (const id of t.kids.get('roots') ?? []) {
    const pid = t.nodes.get(id)?.project;
    if (!byProject.has(pid)) byProject.set(pid, []);
    byProject.get(pid).push(id);
  }
  for (const p of t.scope) {
    const open = !t.collapsedProjects.has(p.id);
    line('project', `project:${p.id}`, 0, { project: p, open });
    const sl = t.statusLines?.get(p.id);
    if (sl) out.push({ kind: 'srcline', key: `srcline:${p.id}`, level: 1, indent: 1, ...sl });
    if (open) for (const id of byProject.get(p.id) ?? []) node(id, 1);
  }
  moreLine('roots', 1, 0);
  return out;
}

/** Ancestors of `id` in the loaded map, root first (the project row excluded). */
export function ancestors(nodes, id) {
  const chain = [];
  let cur = nodes.get(id)?.parent;
  const seen = new Set();
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    const n = nodes.get(cur);
    if (!n) break;
    chain.unshift(n);
    cur = n.parent;
  }
  return chain;
}

// ---------------------------------------------------------------- header status and lines (notes §11)

/**
 * The header's status text (right): { text, tone: 'muted'|'error', refreshDisabled }.
 * `status`: { project: SourceStatus }; `names`: project id → tracker words ("acme/shop").
 */
export function headerStatus(status, { now = Date.now(), firstLoad = false, names = {}, rootsTotal = 0 } = {}) {
  const list = Object.entries(status ?? {});
  if (firstLoad) {
    const first = list.find(([, s]) => s.status === 'refreshing')?.[0] ?? list[0]?.[0];
    return { text: `Loading ${names[first] ?? 'work'}…`, tone: 'muted' };
  }
  const failed = list.filter(([, s]) => s.status === 'error' && s.error?.code !== 'rate_limited' && !s.offline).length;
  if (failed) return { text: `${plural(failed, 'source')} failed`, tone: 'error' };
  if (list.some(([, s]) => s.status === 'refreshing'))
    return { text: 'refreshing…', tone: 'muted', refreshDisabled: true };
  const oldest = Math.min(...list.map(([, s]) => s.refreshed_at_ms ?? Number.POSITIVE_INFINITY));
  const updated = Number.isFinite(oldest) ? `updated ${ageText(oldest, now)}` : '';
  const limited = list.some(([, s]) => s.error?.code === 'rate_limited' && (s.error.retry_at_ms ?? 0) > now);
  const prefix = list.some(([, s]) => s.offline) ? 'Offline' : rootsTotal > 100 ? `${rootsTotal} at the top` : '';
  return { text: [prefix, updated].filter(Boolean).join(' · '), tone: 'muted', refreshDisabled: limited };
}

/**
 * The line under the toolbar (one project) or under its project row (several) for a project's
 * SourceStatus (S3 offline, S4 error, S5 rate limit, S6 complexity, gh_sub_issues, gh_scope), or null.
 * `p`: { id, name, label (the tracker words), kind }. Returns { tone: 'info'|'warn'|'error', text, fix, fixLabel,
 * command?, short? (the project row's tail) }.
 */
export function statusLine(p, s, now = Date.now()) {
  if (!s) return null;
  const kind = p.kind ?? 'github';
  if (s.offline)
    return {
      tone: 'info',
      text: `Showing what was cached at ${clock(s.refreshed_at_ms ?? now, now)}. Starting still works; write-back waits.`,
      fix: 'retry',
      fixLabel: 'Refresh',
    };
  if (s.status === 'error' && s.error) {
    const e = trackerError(s.error, kind, p.label);
    const code = s.error.code;
    if (code === 'rate_limited') return { tone: 'info', text: `${p.name} · ${e.text}`, fix: null };
    if (code === 'linear_complexity')
      return {
        tone: 'warn',
        text: `${e.text} Some items may be missing.`,
        fix: 'settings',
        fixLabel: 'Fix',
        short: 'request too large',
      };
    if (code === 'gh_scope')
      return {
        tone: 'error',
        text: `${p.name} · ${e.text} to read the board`,
        fix: 'gh_scope',
        fixLabel: 'Copy command',
        command: GH_SCOPE_COMMAND,
        short: e.text,
      };
    const kept = s.refreshed_at_ms ? ` · items kept from ${ageText(s.refreshed_at_ms, now)}` : '';
    return {
      tone: 'error',
      text: `${p.name} · ${e.text}${kept}`,
      fix: e.fix ?? 'retry',
      fixLabel: e.fix === 'retry' ? 'Retry' : 'Fix',
      short: e.text,
    };
  }
  if (s.warning === 'gh_sub_issues')
    return {
      tone: 'info',
      text: trackerError({ code: 'gh_sub_issues' }, kind, p.label).text,
      fix: 'retry',
      fixLabel: 'Retry',
    };
  return null;
}

/** A child list that failed to load (S6, S16). */
export const loadErrorText = (message) =>
  /too large|complexity/i.test(String(message ?? ''))
    ? 'Could not load sub-issues: request too large'
    : `Could not load: ${message}`;

/** The empty-tracker line (S7) of one project. */
export function emptyText(p) {
  const t = p?.tracker;
  if (t?.kind === 'linear') return `No issues in ${(t.teams ?? []).join(', ') || 'Linear'}.`;
  if (t?.kind === 'local') return `No backlog files in ${backlogDir(t.backlog_glob)}/. Add a .md file there.`;
  return `No open issues in ${p?.label ?? 'this repository'}. Create them in GitHub; Work picks them up at the next refresh (r).`;
}

/** No match for filters (S8): "Nothing matches 3 filters in shop-api". */
export const noMatchText = (n, names) => `Nothing matches ${plural(Math.max(1, n), 'filter')} in ${names.join(', ')}`;

// ---------------------------------------------------------------- search (notes §4.1)

/** The results header: "12 results for "address"" · "100+ results: refine the search" · no results. */
export function resultsText(res, query, names) {
  const total = res?.total ?? res?.nodes?.length ?? 0;
  if (!total) return `No results for "${query}" in ${names.join(', ')}`;
  if (total > 100) return '100+ results: refine the search';
  return `${plural(total, 'result')} for "${query}"`;
}

// ---------------------------------------------------------------- details (notes §3)

/** The facts line: GitHub `<type> · <state> · <board status> · <labels> · assignee <a> · milestone v2`, Linear, Linear project. */
export function factsOf(node) {
  const u = node?.upstream ?? {};
  const labels = (u.labels ?? []).filter(isShownLabel);
  const assignee = u.assignees?.length ? `assignee ${u.assignees.join(', ')}` : '';
  if (node?.kind === 'linear_project')
    return [u.target_date ? `target ${shortDate(u.target_date, true)}` : '', u.state].filter(Boolean).join(' · ');
  if (node?.kind === 'milestone')
    return [u.due_on ? `due ${shortDate(u.due_on)}` : '', u.state].filter(Boolean).join(' · ');
  if (trackerOf(node) === 'linear') {
    const prio = ['', 'Urgent', 'High', 'Medium', 'Low'][u.priority ?? 0] ?? '';
    return [
      u.state,
      prio,
      u.estimate != null ? `estimate ${u.estimate}` : '',
      u.cycle ? u.cycle.replace(/^Cycle /, 'cycle ') : '',
      labels.join(', '),
      u.assignees?.join(', ') ?? '',
    ]
      .filter(Boolean)
      .join(' · ');
  }
  const state =
    u.state === 'closed' ? (u.closed_reason === 'not_planned' ? 'closed (not planned)' : 'closed') : u.state;
  return [
    u.issue_type,
    state,
    u.board_status,
    labels.join(', '),
    assignee,
    u.milestone ? `milestone ${u.milestone}` : '',
  ]
    .filter(Boolean)
    .join(' · ');
}

/** The children heading: "Sub-issues · 1 of 2 done" (GitHub) / "Issues · 2 of 6 done" (containers) / "Sub-issues" (Linear). */
export function childrenHeading(node) {
  const c = node?.children ?? {};
  const done = `${c.done ?? 0} of ${c.total ?? 0} done`;
  if (isContainer(node)) return `Issues · ${done}`;
  if (trackerOf(node) === 'linear') return 'Sub-issues';
  return `Sub-issues · ${done}`;
}

/** One history line (F6). `tracker`: "GitHub" / "Linear". */
export function historyText(h, node) {
  const tr = trackerName(node);
  const feat = h.feature ? `feature ${h.feature}` : '';
  switch (h.what) {
    case 'started':
      return [h.program ? `Started in program ${h.program.title}` : 'Started', feat].filter(Boolean).join(' · ');
    case 'program_queued':
      return `Queued in program ${h.program?.title ?? ''}`.trim();
    case 'spec_approved':
      return 'Spec approved';
    case 'pr_opened':
      return h.detail ? `PR ${h.detail} opened` : 'PR opened';
    case 'merged':
      return h.detail ? `Merged · PR ${h.detail}` : 'Merged';
    case 'skipped':
      return `Skipped${h.program ? ` in program ${h.program.title}` : ''}`;
    case 'writeback_done':
      return h.detail ?? `Updated in ${tr}`;
    case 'writeback_failed':
      return h.detail ?? `Could not update ${node?.ref ?? 'the item'} in ${tr}`;
    case 'returned':
      return h.detail ?? 'Returned';
    default:
      return h.detail ?? h.what ?? '';
  }
}

/** Body (P4.5 §3): plain text, the first 12 lines unless `all`. */
export function bodyLines(body, all = false) {
  const lines = String(body ?? '')
    .replace(/\r/g, '')
    .split('\n');
  while (lines.length && !lines.at(-1).trim()) lines.pop();
  return { lines: all ? lines : lines.slice(0, 12), more: !all && lines.length > 12 };
}

/** Pairs for the "Blocking" / "Blocked by" rows; items outside the tracker are not openable in the tree. */
export const depLines = (list) =>
  (list ?? []).map((d) => ({ ...d, outside: !d.id, text: `${d.ref}${d.title ? ` ${d.title}` : ''}` }));
