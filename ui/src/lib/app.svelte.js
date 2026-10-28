// The app's state: what the core reports (polled about once a second), the
// tiling workspace, focus, overlays and the command registry. Terminal objects
// (xterm.js) live outside the reactive state, in `terms`.

import { due, toastFor, UNDO_MS } from './inbox.js';
import { errText, invoke } from './ipc.js';
import { hint } from './keyboard.js';
import { deriveKittyTheme } from './kittytheme.js';
import * as L from './layout.js';
import { displayState, runCost } from './status.js';
import { runStateLabel } from './teams.js';
import { TermPane } from './term.js';
import { breakpoint } from './tokens.js';

export const app = $state({
  data: null, // app_state
  kitty: null,
  layout: null,
  panes: {}, // paneId → { type, agentId?, termId?, projectId?, cwd?, title }
  focused: null,
  zoomed: null,
  leader: false,
  sidebar: true,
  inspector: true,
  inspectorOverlay: false, // below 1512 px the inspector is an overlay, opened by ⌘I
  width: typeof window !== 'undefined' ? window.innerWidth : 1512,
  overlay: null, // { kind: 'palette' | 'jump' | 'new-session' | 'add-project' | 'confirm', … }
  toasts: [],
  pending: [], // Inbox decisions in their undo window: { item, action, message, at, toast }
  selectedAgent: null,
  selectedProject: null,
  theme: null, // null = system, 'dark' | 'light' (user choice)
  kittyTheme: false, // derive surface, text and accent from kitty.conf (§9.1); a user choice wins
  // Shells live in one dock under the panes, one tab each (not in `layout`).
  dock: { open: false, height: 0.3, tabs: [], active: null },
  error: null,
});

/** A pane shown in the shell dock rather than the tiling layout. */
export const inDock = (id) => app.dock.tabs.includes(id);

/** xterm.js panes by pane id (not reactive). */
export const terms = new Map();

let nextPane = 1;
const newPaneId = () => `p${nextPane++}`;

// ---------------------------------------------------------------- derived

export const wide = () => app.width >= breakpoint.md;
export const agents = () => app.data?.agents ?? [];
export const agent = (id) => agents().find((a) => a.id === id);
/** Agents not deleted (archived ones stay in the snapshot for history). */
export const activeAgents = () => agents().filter((a) => a.state !== 'archived');
export const projects = () => app.data?.projects ?? [];
/** Features with their latest team run (P2). `null` in a snapshot when the
 *  core could not read them: the poll then keeps the previous list. */
export const features = () => app.data?.features ?? [];
export const focusedSpec = () => app.panes[app.focused];
export const isTerminal = (spec) => ['agent', 'shell', 'editor'].includes(spec?.type);

/** The agent the inspector and the palette are about: the focused pane's, else the selection. */
export function contextAgent() {
  const s = focusedSpec();
  return agent(s?.agentId ?? app.selectedAgent);
}

export function contextProject() {
  const s = focusedSpec();
  const a = contextAgent();
  return projects().find((p) => p.id === (s?.projectId ?? a?.project_id)) ?? projects()[0];
}

// ---------------------------------------------------------------- toasts

export function toast(text, opts = {}) {
  const t = { id: Math.random().toString(36).slice(2), text, ...opts, at: Date.now() };
  app.toasts = [...app.toasts.slice(-2), t];
  if (!opts.sticky) setTimeout(() => dismissToast(t.id), opts.ms ?? 6000);
  return t.id;
}

export function dismissToast(id) {
  app.toasts = app.toasts.filter((t) => t.id !== id);
}

// ---------------------------------------------------------------- panes

function title(spec) {
  switch (spec.type) {
    case 'agent':
      return spec.agentId;
    case 'shell': {
      const dir =
        String(spec.cwd ?? '')
          .split('/')
          .slice(-1)[0] || '~';
      const where = spec.agentId ?? projects().find((p) => p.id === spec.projectId)?.name;
      return where && where !== dir ? `${where} · ${dir}` : `zsh · ${dir}`;
    }
    case 'editor':
      return `nvim · ${spec.file ? spec.file.split('/').pop() : (spec.projectId ?? '')}`;
    default:
      if (spec.type === 'project') return projects().find((p) => p.id === spec.projectId)?.name ?? spec.projectId;
      if (spec.type === 'trust') return `trust · ${spec.projectId}`;
      if (spec.type === 'feature') return features().find((f) => f.id === spec.featureId)?.slug ?? 'feature';
      if (spec.type === 'board')
        return `${projects().find((p) => p.id === spec.projectId)?.name ?? spec.projectId} tasks`;
      if (spec.type === 'canvas') return spec.teamId ? `Team: ${spec.teamId}` : 'Team canvas';
      if (spec.type === 'diff')
        return `diff · ${
          spec.branch ??
          String(spec.path ?? '')
            .split('/')
            .pop()
        }`;
      return (
        { overview: 'All projects', inbox: 'Inbox', planner: 'Week planner', setup: 'Set up Orchestrator' }[
          spec.type
        ] ?? spec.type
      );
  }
}

/** Adds a pane: next to the focused one (`how`: 'right' | 'below' | 'tab'). */
export function addPane(spec, how = 'right') {
  const id = newPaneId();
  app.panes[id] = { ...spec, title: spec.title ?? title(spec) };
  const at = app.focused && app.panes[app.focused] ? app.focused : L.visible(app.layout)[0];
  if (!app.layout) app.layout = L.leaf(id);
  else if (how === 'tab') app.layout = L.addTab(app.layout, at, id);
  else app.layout = L.split(app.layout, at, how === 'below' ? 'col' : 'row', id);
  app.zoomed = null;
  focusPane(id);
  saveLayout();
  return id;
}

/** Adds a shell to the dock, as its active tab, and opens the dock. */
export function addDockPane(spec) {
  const id = newPaneId();
  app.panes[id] = { ...spec, title: spec.title ?? title(spec) };
  app.dock.tabs = [...app.dock.tabs, id];
  app.dock.open = true;
  focusPane(id);
  saveLayout();
  return id;
}

/** ⌘J: shows or hides the dock (a new shell when it has none). */
export function toggleDock() {
  if (!app.dock.tabs.length) return openShell();
  app.dock.open = !app.dock.open;
  if (app.dock.open) focusPane(app.dock.active ?? app.dock.tabs[0]);
  else if (inDock(app.focused)) focusPane(L.visible(app.layout)[0]);
  saveLayout();
}

/** Next or previous dock tab (`d` = 1 or -1). */
export function cycleDock(d) {
  const tabs = app.dock.tabs;
  if (!tabs.length) return;
  const i = tabs.indexOf(app.dock.active);
  focusPane(tabs[(i + d + tabs.length) % tabs.length]);
}

export function focusPane(id) {
  if (!app.panes[id]) return;
  if (inDock(id)) {
    app.dock.active = id;
    app.dock.open = true;
  } else app.layout = L.activate(app.layout, id);
  app.focused = id;
  const s = app.panes[id];
  if (s.agentId) app.selectedAgent = s.agentId;
  if (app.zoomed && app.zoomed !== id) app.zoomed = id;
  // Focus whatever is focused when this runs: several calls in a row end on
  // the last, and each terminal's focusin then matches `app.focused`.
  queueMicrotask(() => {
    const t = terms.get(app.focused);
    if (t) t.focus();
    else {
      /** @type {HTMLElement | null} */
      const el = document.querySelector(`[data-pane="${app.focused}"] [data-pane-focus]`);
      el?.focus();
    }
  });
}

export function closePane(id = app.focused) {
  if (!id || !app.panes[id]) return;
  terms.get(id)?.dispose();
  terms.delete(id);
  delete app.panes[id];
  if (inDock(id)) {
    const i = app.dock.tabs.indexOf(id);
    app.dock.tabs = app.dock.tabs.filter((t) => t !== id);
    if (app.dock.active === id) app.dock.active = app.dock.tabs[Math.min(i, app.dock.tabs.length - 1)] ?? null;
    if (!app.dock.tabs.length) app.dock.open = false;
    if (app.focused === id) app.focused = app.dock.active ?? L.visible(app.layout)[0] ?? null;
  } else app.layout = L.remove(app.layout, id);
  if (app.zoomed === id) app.zoomed = null;
  if (app.focused === id) app.focused = L.visible(app.layout)[0] ?? null;
  if (app.focused) focusPane(app.focused);
  saveLayout();
}

/** Panes the user closed, newest last, for Reopen closed pane (⌘⇧T). A
 *  pane that closes itself (a shell exits, Setup is done, its agent or
 *  feature is gone) goes through `closePane` and is not offered back. */
const closed = [];
const CLOSED_MAX = 20;

/** ⌘W, a tab's ×, a pane's close: the pane can be reopened. */
export function closePaneByUser(id = app.focused) {
  if (!id || !app.panes[id]) return;
  closed.push({ spec: app.panes[id], docked: inDock(id) });
  if (closed.length > CLOSED_MAX) closed.shift();
  closePane(id);
}

/** Reopens the last closed pane whose agent, feature or project still
 *  exists, as a restart restores it (a terminal as a shell in its cwd). */
export async function reopenClosed() {
  for (let c = closed.pop(); c; c = closed.pop()) {
    const { spec, docked } = c;
    if (paneGone(spec)) continue;
    const open = findPane((s) => s.type === spec.type && PANE_KEYS.every((k) => s[k] === spec[k]));
    if (open) return focusPane(open);
    const s = await revive(spec);
    if (!s) continue;
    if (docked) return addDockPane(s);
    return addPane(s, isTerminal(s) || !app.layout ? 'right' : 'tab');
  }
  toast('No closed pane to reopen');
}

/** What a pane is about: two panes with these equal show the same thing. */
const PANE_KEYS = ['agentId', 'featureId', 'projectId', 'teamId', 'path'];

/** A saved pane spec made live: shells and editors restart as a shell in
 *  their cwd; `null` when the shell does not start. */
async function revive(s) {
  if (s.type !== 'shell' && s.type !== 'editor') return s;
  try {
    const t = await spawnShell({ projectId: s.projectId, cwd: s.cwd });
    const spec = { ...s, type: 'shell', termId: t.id, cwd: t.cwd };
    return { ...spec, title: title(spec) };
  } catch {
    return null;
  }
}

export function findPane(pred) {
  return Object.entries(app.panes).find(([, s]) => pred(s))?.[0];
}

/** Singleton panes (Overview, Inbox, Week planner) open as a tab, or get focus. */
export function openSingleton(type) {
  const existing = findPane((s) => s.type === type);
  if (existing) return focusPane(existing);
  addPane({ type }, app.layout ? 'tab' : 'right');
}

/** A project's pane (§4.1): git state, worktrees, PRs, removal (J11). */
export function openProject(projectId) {
  app.selectedProject = projectId;
  const existing = findPane((s) => s.type === 'project' && s.projectId === projectId);
  if (existing) return focusPane(existing);
  addPane({ type: 'project', projectId }, app.layout ? 'tab' : 'right');
}

/** P2: a feature's pane (docs, tasks, team, timeline), one per feature. */
export function openFeature(featureId) {
  const existing = findPane((s) => s.type === 'feature' && s.featureId === featureId);
  if (existing) return focusPane(existing);
  const f = features().find((x) => x.id === featureId);
  addPane({ type: 'feature', featureId, projectId: f?.project_id }, app.layout ? 'tab' : 'right');
}

/** P2: a project's Board (FR-502), one per project. */
export function openBoard(projectId) {
  if (!projectId) return toast('No project in focus');
  const existing = findPane((s) => s.type === 'board' && s.projectId === projectId);
  if (existing) return focusPane(existing);
  addPane({ type: 'board', projectId }, app.layout ? 'tab' : 'right');
}

/** P2: the Team canvas (FR-301), on a team (or the library when none). */
export function openCanvas(teamId = null, projectId = null) {
  const existing = findPane((s) => s.type === 'canvas' && (!teamId || s.teamId === teamId));
  if (existing) return focusPane(existing);
  addPane(
    { type: 'canvas', teamId, projectId: projectId ?? contextProject()?.id ?? null },
    app.layout ? 'tab' : 'right',
  );
}

/** FR-107: a worktree's diff against its base, one pane per worktree. */
export function openDiff(projectId, path, branch = null) {
  const existing = findPane((s) => s.type === 'diff' && s.path === path);
  if (existing) return focusPane(existing);
  addPane({ type: 'diff', projectId, path, branch }, 'right');
}

/**
 * J0 step 2: plain `claude` in the project root for the user to answer the
 * workspace trust dialog (the app never answers it). The user exits it.
 */
export async function openTrust(projectId) {
  const p = projects().find((x) => x.id === projectId);
  if (!p) return;
  try {
    const run = await invoke('trust_command');
    const t = await invoke('term_spawn', {
      req: { kind: 'shell', project_id: p.id, agent_id: null, cwd: p.path, run, rows: 30, cols: 100 },
    });
    addPane({ type: 'shell', termId: t.id, cwd: t.cwd, projectId: p.id, title: `trust · ${p.name}` }, 'right');
    toast(`Answer "Yes, I trust this folder" in the pane, then /exit · ${p.name}`, { ms: 10000 });
  } catch (e) {
    toast(`Could not start claude: ${errText(e)}`, { kind: 'error' });
  }
}

export function openAgent(agentId) {
  const existing = findPane((s) => s.type === 'agent' && s.agentId === agentId);
  if (existing) return focusPane(existing);
  const a = agent(agentId);
  const n = L.visible(app.layout).length;
  addPane({ type: 'agent', agentId, projectId: a?.project_id }, n >= 3 ? 'tab' : 'right');
}

/**
 * A login shell PTY in the core; the pane fits it once mounted.
 * @param {{ projectId?: string, agentId?: string, cwd?: string }} req
 */
const spawnShell = ({ projectId, agentId, cwd }) =>
  invoke('term_spawn', {
    req: {
      kind: 'shell',
      project_id: projectId ?? null,
      agent_id: agentId ?? null,
      cwd: cwd ?? null,
      run: null,
      rows: 24,
      cols: 80,
    },
  });

/** A shell: a dock tab by default, a pane with `how` ('right' | 'below').
 *  `agentId: null` = no agent (undefined = the one in focus). */
export async function openShell(opts = {}) {
  const projectId = opts.projectId ?? contextProject()?.id;
  const agentId = opts.agentId ?? contextAgent()?.id;
  try {
    const t = await spawnShell({ projectId, agentId, cwd: opts.cwd });
    const spec = { type: 'shell', termId: t.id, cwd: t.cwd, projectId, agentId };
    if (opts.how) addPane(spec, opts.how);
    else addDockPane(spec);
  } catch (e) {
    toast(`Could not open a shell: ${errText(e)}`, { kind: 'error' });
  }
}

/** FR-1101/1102: reuse the project's nvim when it runs, else a new editor pane. */
export async function openEditor(file = null, line = null, opts = {}) {
  const a = opts.agentId ? agent(opts.agentId) : contextAgent();
  const p = projects().find((x) => x.id === (opts.projectId ?? a?.project_id)) ?? contextProject();
  try {
    const r = await invoke('editor_open', {
      projectId: p?.id ?? null,
      agentId: a?.id ?? null,
      file,
      line,
      rows: 30,
      cols: 100,
    });
    if (r.reused) {
      const pane = findPane((s) => s.type === 'editor' && s.projectId === p?.id);
      if (pane) focusPane(pane);
      toast(
        file
          ? `Opened ${file}${line ? `:${line}` : ''} in nvim · ${p?.name ?? ''}`
          : `nvim is already open · ${p?.name ?? ''}`,
      );
      return;
    }
    addPane({ type: 'editor', termId: r.term.id, cwd: r.term.cwd, projectId: p?.id, file }, 'right');
  } catch (e) {
    toast(`Could not open the editor: ${errText(e)}`, { kind: 'error' });
  }
}

/** Creates (or returns) the xterm.js pane for a pane spec. */
export function termFor(paneId, hooks) {
  let t = terms.get(paneId);
  if (t) {
    t.hooks = hooks;
    return t;
  }
  const s = app.panes[paneId];
  const source = s.type === 'agent' ? { kind: 'agent', id: s.agentId } : { kind: 'term', id: s.termId };
  t = new TermPane(source, app.kitty, hooks);
  terms.set(paneId, t);
  return t;
}

// ---------------------------------------------------------------- focus moves

export function focusDirection(dir) {
  if (!app.focused) return;
  // The dock is below every pane: down from the bottom row, up back out.
  if (inDock(app.focused)) {
    if (dir === 'up') focusPane(L.visible(app.layout).at(-1));
    return;
  }
  const n = L.neighbor(app.layout, app.focused, dir);
  if (n) focusPane(n);
  else if (dir === 'down' && app.dock.open && app.dock.active) focusPane(app.dock.active);
}

export function focusNumber(n) {
  const id = L.visible(app.layout)[n - 1];
  if (id) {
    app.zoomed = app.zoomed ? id : null;
    focusPane(id);
  }
}

export function toggleZoom() {
  if (!app.focused) return;
  app.zoomed = app.zoomed ? null : app.focused;
  focusPane(app.focused);
}

/** Leader w: the next agent waiting for you, by waiting time (attention ladder level 1). */
export function nextWaiting() {
  const waiting = agents()
    .filter((a) => a.state === 'waiting_for_you' || a.trust_dialog)
    .sort((x, y) => (x.last_event_at_ms ?? 0) - (y.last_event_at_ms ?? 0));
  if (!waiting.length) return toast('No agent is waiting for you');
  const cur = contextAgent()?.id;
  const i = waiting.findIndex((a) => a.id === cur);
  openAgent(waiting[(i + 1) % waiting.length].id);
}

// ---------------------------------------------------------------- agent actions

export async function agentAction(id, action, message = null) {
  try {
    await invoke('agent_action', { id, action, message });
    const verb = {
      interrupt: 'Interrupted',
      suspend: 'Suspended',
      kill: 'Killed',
      resume: 'Resumed',
      deliver: 'Sent to',
      pause: 'Paused',
      unpause: 'Unpaused',
    }[action];
    // A kill is checked: the core scans again and fails if anything survived.
    toast(`${verb ?? action} · ${id}${action === 'kill' ? ' · no process left' : ''}`);
    await refresh();
  } catch (e) {
    toast(`${id}: ${errText(e)}`, { kind: 'error' });
  }
}

/** FR-202: kill (checked) and hide the agent; its worktree on request. Its
 *  panes close on the refresh (see `prunePanes`). */
export async function deleteAgent(id, removeWorktree, confirmDiscard) {
  const r = await invoke('agent_delete', { id, removeWorktree, confirmDiscard });
  const w = r.worktree;
  const note = !w
    ? ''
    : w.error || w.kept
      ? ` · worktree kept: ${w.error ?? w.kept}`
      : w.worktree_removed
        ? ` · worktree removed${w.branch_deleted ? ' with its branch' : ''}`
        : '';
  toast(`Deleted ${id} · no process left${note}`, w?.error ? { kind: 'error' } : {});
  await refresh();
  return r;
}

/** Ends the feature's runs, deletes its agents, hides the feature. */
export async function deleteFeature(featureId, removeWorktrees, discard) {
  const r = await invoke('feature_delete', { feature: featureId, removeWorktrees, discard });
  const kept = r.worktrees.filter((w) => w.error || w.kept).length;
  toast(
    `Deleted ${r.deleted} · ${r.agents.length} agent(s), no process left${kept ? ` · ${kept} worktree(s) kept` : ''}`,
    kept ? { kind: 'error' } : {},
  );
  await refresh();
  return r;
}

export async function governor(action, arg = null) {
  try {
    const r = await invoke('governor_act', { action, arg });
    await refresh();
    return r;
  } catch (e) {
    toast(errText(e), { kind: 'error' });
  }
}

export function openInKitty(cwd) {
  invoke('open_in_kitty', { cwd }).catch((e) => toast(errText(e), { kind: 'error' }));
}

/** Run a queued turn now, after the user confirms its cost in both windows (FR-809). */
export function runNow(a, notStarted) {
  const cost = runCost(a.queue_entry, true);
  confirm(
    `${notStarted ? 'Start' : 'Run'} ${a.id} now?`,
    `≈ ${cost}. It runs above the live cap for one turn and is logged as your override.`,
    () => governor('run_now', a.queue_entry.id),
    notStarted ? 'Start now' : 'Run now',
  );
}

/** Drop a queued turn (its message is not delivered). */
export function cancelQueued(a) {
  confirm(
    `Cancel ${a.id}'s queued turn?`,
    'The waiting message is dropped and logged; the agent keeps its session.',
    () => governor('cancel', a.queue_entry.id),
    'Cancel turn',
  );
}

/** Mounts the pane's xterm in `host`, fits it on resize; returns the cleanup. */
export function mountTerm(paneId, host, hooks) {
  const t = termFor(paneId, {
    onKey: (e) => !e.metaKey,
    onFocus: () => app.focused !== paneId && focusPane(paneId),
    ...hooks,
  });
  t.mount(host);
  if (app.focused === paneId) t.focus();
  const ro = new ResizeObserver(() => t.scheduleFit());
  ro.observe(host);
  return {
    t,
    destroy() {
      ro.disconnect();
      t.unmount();
    },
  };
}

export function confirm(text, detail, onYes, yes = 'Confirm') {
  app.overlay = { kind: 'confirm', text, detail, onYes, yes };
}

// ---------------------------------------------------------------- persistence

let saveTimer = null;
export function saveLayout() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    const panes = Object.fromEntries(Object.entries(app.panes).map(([id, s]) => [id, { ...s, termId: undefined }]));
    invoke('settings_set', {
      key: 'layout',
      value: {
        layout: $state.snapshot(app.layout),
        panes,
        focused: app.focused,
        sidebar: app.sidebar,
        inspector: app.inspector,
        theme: app.theme,
        kittyTheme: app.kittyTheme,
        dock: $state.snapshot(app.dock),
      },
    }).catch(() => {});
  }, 400);
}

/** Restores the saved layout: shells and editors restart in their cwd (NFR-2). */
export async function restoreLayout() {
  const saved = await invoke('settings_get', { key: 'layout' }).catch(() => null);
  if (saved) {
    app.sidebar = saved.sidebar ?? true;
    app.inspector = saved.inspector ?? true;
    app.theme = saved.theme ?? null;
    app.kittyTheme = saved.kittyTheme ?? false;
  }
  if (!saved?.layout) {
    addPane({ type: 'overview' });
    if (!app.data?.setup?.done) openSingleton('setup');
    return;
  }
  const docked = saved.dock?.tabs ?? [];
  const ids = [...L.allPanes(saved.layout), ...docked];
  for (const id of ids) nextPane = Math.max(nextPane, Number(id.slice(1)) + 1);
  const panes = {};
  // Shells and editors restart in parallel; one that does not start is dropped.
  await Promise.all(
    ids.map(async (id) => {
      const s = saved.panes[id];
      if (!s || paneGone(s)) return;
      // A shell that does not start is dropped from the restored layout.
      const live = await revive(s);
      if (live) panes[id] = live;
    }),
  );
  app.panes = panes;
  const tabs = docked.filter((id) => panes[id]);
  app.dock = {
    open: !!saved.dock?.open && tabs.length > 0,
    height: saved.dock?.height ?? 0.3,
    tabs,
    active: tabs.includes(saved.dock?.active) ? saved.dock.active : (tabs[0] ?? null),
  };
  app.layout = L.reid(L.prune(saved.layout, (id) => !!panes[id]));
  if (!app.layout) addPane({ type: 'overview' });
  else
    focusPane(
      panes[saved.focused] && (!inDock(saved.focused) || app.dock.open) ? saved.focused : L.visible(app.layout)[0],
    );
  if (!app.data?.setup?.done) openSingleton('setup');
}

// ---------------------------------------------------------------- polling

/**
 * The poll's payload, replaced whole every second and never edited: a class
 * instance is not turned into a deep reactive proxy, so Svelte only tracks
 * `app.data` itself instead of wrapping the whole tree each time.
 */
class Snapshot {
  constructor(data) {
    Object.assign(this, data);
  }
}

let refreshing = null;

/** One poll at a time: a call made while one is in flight waits for that one. */
export function refresh() {
  refreshing ??= poll().finally(() => (refreshing = null));
  return refreshing;
}

async function poll() {
  try {
    const data = await invoke('app_state');
    data.features ??= app.data?.features ?? null;
    app.data = new Snapshot(data);
    app.error = null;
    prunePanes();
  } catch (e) {
    app.error = errText(e);
  }
}

/** A pane whose agent, feature or project is gone (deleted or archived),
 *  however that happened: a button, the CLI, a project removal. */
function paneGone(s) {
  if (s.type === 'agent') return !activeAgents().some((a) => a.id === s.agentId);
  if (s.type === 'project' || s.type === 'board') return !projects().some((p) => p.id === s.projectId);
  // Features never read yet (the core's read failed): nothing is known gone.
  if (s.type === 'feature') return app.data?.features != null && !features().some((f) => f.id === s.featureId);
  return false;
}

function prunePanes() {
  for (const [id, s] of Object.entries(app.panes)) if (paneGone(s)) closePane(id);
  if (app.selectedAgent && !activeAgents().some((a) => a.id === app.selectedAgent)) app.selectedAgent = null;
}

// ---------------------------------------------------------------- commands

/**
 * Every action is in the palette with its shortcut (§9.1 principle 1).
 * `scope`: 'agent' commands act on the focused/selected agent and are listed first.
 */
export function commands() {
  const a = contextAgent();
  const list = [
    { id: 'session.new', label: 'New session…', run: () => (app.overlay = { kind: 'new-session' }) },
    { id: 'project.add', label: 'Add project…', run: () => (app.overlay = { kind: 'add-project' }) },
    { id: 'overview', label: 'Open Overview', run: () => openSingleton('overview') },
    { id: 'inbox', label: 'Open Inbox', run: () => openSingleton('inbox') },
    { id: 'planner', label: 'Open Week planner', run: () => openSingleton('planner') },
    { id: 'setup', label: 'Set up Orchestrator (first run)…', run: () => openSingleton('setup') },
    { id: 'plan.edit', label: 'Edit the weekly plan…', run: () => (app.overlay = { kind: 'plan' }) },
    { id: 'team.canvas', label: 'Open the Team canvas', run: () => openCanvas() },
    {
      id: 'team.launch',
      label: 'Launch a team on a feature…',
      run: () => (app.overlay = { kind: 'launch', projectId: contextProject()?.id ?? null }),
    },
    { id: 'board.open', label: 'Open the Board of the focused project', run: () => openBoard(contextProject()?.id) },
    { id: 'shell.new', label: 'New shell (dock)', run: () => openShell() },
    { id: 'dock.toggle', label: 'Show or hide the shell dock', run: () => toggleDock() },
    { id: 'dock.next', label: 'Next shell tab', run: () => cycleDock(1) },
    { id: 'dock.prev', label: 'Previous shell tab', run: () => cycleDock(-1) },
    { id: 'editor.open', label: 'Open editor (nvim)', run: () => openEditor() },
    { id: 'pane.split.right', label: 'Split right (shell)', run: () => openShell({ how: 'right' }) },
    { id: 'pane.split.below', label: 'Split below (shell)', run: () => openShell({ how: 'below' }) },
    { id: 'pane.zoom', label: 'Zoom / restore the focused pane', run: toggleZoom },
    { id: 'pane.close', label: 'Close pane', run: () => closePaneByUser() },
    { id: 'pane.reopen', label: 'Reopen closed pane', run: reopenClosed },
    { id: 'sidebar', label: 'Toggle sidebar', run: () => ((app.sidebar = !app.sidebar), saveLayout()) },
    {
      id: 'inspector',
      label: 'Toggle inspector',
      run: () =>
        wide() ? ((app.inspector = !app.inspector), saveLayout()) : (app.inspectorOverlay = !app.inspectorOverlay),
    },
    { id: 'agent.next-waiting', label: 'Jump to next waiting agent', run: nextWaiting },
    {
      id: 'governor.pause-all',
      label: 'Pause all autonomous work',
      run: () => governor('pause_all').then(() => toast('Paused all · resumes on Resume all')),
    },
    {
      id: 'governor.resume-all',
      label: 'Resume all',
      run: () => governor('resume_all').then(() => toast('Resumed all')),
    },
    { id: 'theme.dark', label: 'Theme: Dark', run: () => setTheme('dark') },
    { id: 'theme.light', label: 'Theme: Light', run: () => setTheme('light') },
    { id: 'theme.kitty', label: 'Theme: derive from kitty.conf', run: () => setKittyTheme(true) },
    { id: 'theme.system', label: 'Theme: follow the system', run: () => setTheme(null) },
    { id: 'notify.desktop', label: 'Notifications: desktop on/off', run: () => toggleNotify('desktop') },
    { id: 'notify.sound', label: 'Notifications: sound on/off', run: () => toggleNotify('sound') },
  ];
  if (a) {
    const live = a.live;
    const agentCmds = [
      { id: 'agent.open', label: `Open ${a.id}`, run: () => openAgent(a.id) },
      live && { id: 'agent.interrupt', label: `Interrupt ${a.id} (Esc)`, run: () => agentAction(a.id, 'interrupt') },
      live && { id: 'agent.suspend', label: `Suspend ${a.id}`, run: () => agentAction(a.id, 'suspend') },
      !live && { id: 'agent.resume', label: `Resume ${a.id}`, run: () => agentAction(a.id, 'resume') },
      {
        id: 'agent.kill',
        label: `Kill ${a.id}…`,
        run: () =>
          confirm(
            `Kill ${a.id}?`,
            'Its process group and every process left in its worktree get SIGTERM, then SIGKILL after 5 s. The session can be resumed.',
            () => agentAction(a.id, 'kill'),
            'Kill',
          ),
      },
      {
        id: 'agent.delete',
        label: `Delete ${a.id}…`,
        run: () => (app.overlay = { kind: 'delete-agent', agentId: a.id }),
      },
      { id: 'agent.shell', label: `Shell in ${a.id}'s worktree`, run: () => openShell({ agentId: a.id }) },
      {
        id: 'agent.diff',
        label: `Diff of ${a.id} against its base`,
        run: () => openDiff(a.project_id, a.worktree, a.branch),
      },
      { id: 'agent.kitty', label: `Open ${a.id}'s worktree in kitty`, run: () => openInKitty(a.worktree) },
    ].filter(Boolean);
    for (const c of agentCmds) c.scope = 'agent';
    list.unshift(...agentCmds);
  }
  for (const f of features()) {
    list.push({ id: `feature.open.${f.id}`, label: `Open feature ${f.slug}`, run: () => openFeature(f.id) });
    list.push({
      id: `feature.delete.${f.id}`,
      label: `Delete feature ${f.slug}…`,
      run: () => (app.overlay = { kind: 'delete-feature', featureId: f.id }),
    });
  }
  for (const p of projects()) {
    list.push({ id: `board.open.${p.id}`, label: `Board: ${p.name}`, run: () => openBoard(p.id) });
    list.push({ id: `project.open.${p.id}`, label: `Open project ${p.name}`, run: () => openProject(p.id) });
    list.push({
      id: `project.trust.${p.id}`,
      label: `Trust ${p.name} (Claude Code's dialog)…`,
      run: () => openTrust(p.id),
    });
  }
  for (const c of list) c.hint = hint(c.id);
  return list;
}

export function run(commandId) {
  if (commandId.startsWith('pane.focus.')) return focusNumber(Number(commandId.slice(11)));
  const map = {
    palette: () => (app.overlay = app.overlay?.kind === 'palette' ? null : { kind: 'palette' }),
    jump: () => (app.overlay = app.overlay?.kind === 'jump' ? null : { kind: 'jump' }),
    'focus.left': () => focusDirection('left'),
    'focus.right': () => focusDirection('right'),
    'focus.up': () => focusDirection('up'),
    'focus.down': () => focusDirection('down'),
    'agent.pause': () => {
      // FR-805/811: the Governor holds this agent at its next tool boundary; again resumes it.
      const a = contextAgent();
      if (!a) return toast('No agent in focus');
      agentAction(a.id, a.paused ? 'unpause' : 'pause');
    },
    'window.close': () => {},
  };
  if (map[commandId]) return map[commandId]();
  const c = commands().find((x) => x.id === commandId);
  if (c) c.run();
}

let kittyVars = [];

/**
 * Theme precedence (§9.1): user choice > derived from kitty (if enabled and the
 * palette passes the contrast check) > system. The derived scheme picks which
 * fixed status colors apply.
 */
export function applyTheme() {
  const root = document.documentElement;
  for (const v of kittyVars) root.style.removeProperty(v);
  kittyVars = [];
  let scheme = app.theme;
  let derived = null;
  if (!scheme && app.kittyTheme) {
    derived = deriveKittyTheme(app.kitty?.options?.theme);
    scheme = derived?.scheme ?? null;
  }
  if (scheme) root.dataset.theme = scheme;
  else delete root.dataset.theme;
  if (derived) {
    for (const [k, v] of Object.entries(derived.vars)) root.style.setProperty(k, v);
    kittyVars = Object.keys(derived.vars);
  }
  return app.theme ? 'user' : derived ? 'kitty' : 'system';
}

/** Desktop notifications and their sound (both on by default), read by the core's notifier. */
export async function toggleNotify(key) {
  const cur = (await invoke('settings_get', { key: 'notifications' }).catch(() => null)) ?? {};
  const next = { desktop: true, sound: true, ...cur };
  next[key] = !next[key];
  await invoke('settings_set', { key: 'notifications', value: next }).catch(() => {});
  toast(`${key === 'desktop' ? 'Desktop notifications' : 'Notification sound'} ${next[key] ? 'on' : 'off'}`);
}

/** An explicit choice (dark, light, or null = system) ends the kitty-derived theme. */
export function setTheme(t) {
  app.theme = t;
  app.kittyTheme = false;
  applyTheme();
  saveLayout();
}

/** Turns the kitty-derived theme on or off; tells the user when the palette is refused. */
export function setKittyTheme(on) {
  app.kittyTheme = on;
  app.theme = null;
  const src = applyTheme();
  saveLayout();
  if (on && src !== 'kitty')
    toast(
      !(app.kitty?.options?.theme?.background && app.kitty?.options?.theme?.foreground)
        ? 'No background and foreground colors in kitty.conf: using the system theme'
        : 'kitty.conf colors are not readable enough (contrast) or lack an accent: using the system theme',
    );
  else if (on) toast('Theme derived from kitty.conf: surface, text and accent; status colors stay fixed');
}

/** Jump targets (⌘P): the sidebar tree and the open panes. */
export function jumpTargets() {
  const out = [];
  for (const p of projects()) out.push({ label: p.name, detail: p.path, run: () => openProject(p.id) });
  for (const f of features()) {
    const p = projects().find((x) => x.id === f.project_id);
    out.push({
      label: `${p?.name ?? ''} › ${f.slug}`,
      detail: f.run ? runStateLabel(f.run.status) : 'feature',
      run: () => openFeature(f.id),
    });
  }
  for (const a of agents()) {
    const p = projects().find((x) => x.id === a.project_id);
    out.push({ label: `${p?.name ?? ''} › ${a.id}`, detail: displayState(a), run: () => openAgent(a.id) });
  }
  for (const [id, s] of Object.entries(app.panes))
    out.push({ label: `pane: ${s.title}`, detail: s.type, run: () => focusPane(id) });
  return out;
}

// ---------------------------------------------------------------- inbox decisions

/** Records a decision; it reaches the core when its 5 s undo window ends (FR-707 `u`). */
export function decide(item, action, message = null) {
  if (app.pending.some((p) => p.item === item.id)) return;
  const text = toastFor(item, action);
  const t = toast(text, { ms: UNDO_MS, undo: true, item: item.id });
  app.pending = [...app.pending, { item: item.id, action, message, at: Date.now(), toast: t }];
}

export function undoLast() {
  const last = app.pending[app.pending.length - 1];
  if (!last) return false;
  app.pending = app.pending.slice(0, -1);
  dismissToast(last.toast);
  toast('Undone');
  return true;
}

export function undoItem(itemId) {
  const p = app.pending.find((x) => x.item === itemId);
  if (!p) return;
  app.pending = app.pending.filter((x) => x !== p);
  dismissToast(p.toast);
}

/** Sends the decisions whose undo window is over. Called by the app's clock. */
export async function flushDecisions(now = Date.now()) {
  for (const p of due(app.pending, now)) {
    app.pending = app.pending.filter((x) => x !== p);
    try {
      await invoke('inbox_act', { item: p.item, action: p.action, message: p.message });
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }
}
