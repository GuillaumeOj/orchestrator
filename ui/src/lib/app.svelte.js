// The app's state: what the core reports (`app_state`, read on its `now`
// topics; attention records, decisions and counts in `app.now`, see
// now.svelte.js), the workspace (one tab strip in a project context), focus,
// overlays and the command
// registry. Terminal objects (xterm.js) live outside the reactive state, in `terms`.

import { tick as svelteTick } from 'svelte';
import { upsertEntries } from './chat.js';
import { checkToast, claudeToast, paletteActions } from './claude.js';
import { deliveryRef } from './delivery.js';
import { clock as clockOf, expandHome } from './format.js';
import { decisionToast, due, FILTERS, isOpen, older, pendingToast, UNDO_MS, version } from './inbox.js';
import { openPrefilled, planWithOverwatchIn, refreshIntake, setUpIntake } from './intake.svelte.js';
import { errText, invoke } from './ipc.js';
import { hint } from './keyboard.js';
import { deriveKittyTheme } from './kittytheme.js';
import * as L from './layout.js';
import { runNowCost, runNowDialog } from './mission.js';
import { attention, clearDigest, counts, needsYou, statusOf } from './now.svelte.js';
import { canOpenSession, overwatchOf } from './overwatch.js';
import { isLocalHost, liveTitle, looksLikePath, POLL_MS, parseOsc7 } from './shellcwd.js';
import { STATUS } from './status.js';
import { runStateLabel } from './teams.js';
import { TermPane } from './term.js';
import { breakpoint, size } from './tokens.js';
import {
  aboutText,
  backToMain,
  checkOutcome,
  isRestoreWarning,
  prKey,
  prReadyToast,
  readyToast,
  updateActions,
} from './update.js';
import { openProgram, openProgramPlan, openWork, togglePause } from './work.js';
import { scopeTitle } from './workmodel.js';

/** The dock's share of the work column when nothing is saved: agents need rows. */
const DOCK_HEIGHT = 0.45;
const CHAT_MIN = 300;
const CHAT_MAX = 720;
const CHAT_NARROW = 340;

/**
 * @typedef {{ rev: number, agents: Record<string, any>, runs: Record<string, any>,
 *   decisions: Record<string, any>, outcomes: any[], counts: { global: any, projects: any },
 *   chat?: { day: string | null, entries: Record<string, any> }, overwatch?: any, intake?: any, programs?: Record<string, any> }} Now
 */

export const app = $state({
  data: null, // app_state
  now: /** @type {Now} */ ({
    rev: 0,
    agents: {},
    runs: {},
    decisions: {},
    outcomes: [],
    counts: { global: {}, projects: {} },
    chat: { day: null, entries: {} }, // today's chat entries (P4.3)
  }), // now.svelte.js
  kitty: null,
  layout: null,
  panes: {}, // paneId → { type, agentId?, termId?, projectId?, cwd?, title }
  focused: null,
  // The workspace's project context (design-notes §5.1): 'all' or a project id.
  workspace: { project: /** @type {string} */ ('all') },
  // The palette's "Set the project context…" asks the context Select to open.
  contextOpen: false,
  // The sidebar tree (§5.4): expanded state per `projectId` / `f:<featureId>`; absent = expanded.
  sidebarTree: /** @type {Record<string, boolean>} */ ({}),
  leader: false,
  sidebar: true,
  // The Inspector is open (⌘I): docked at ≥ 1728 px, else an overlay over the
  // chat column's slot (P4.2 notes §4); one state across the breakpoint.
  inspector: false,
  width: typeof window !== 'undefined' ? window.innerWidth : 1512,
  // The Overwatch column's width in px; null = the breakpoint default (360 / 340).
  // Saved as chosen: the bounds apply at render (`chatWidthNow`), never to this.
  chatWidth: /** @type {number | null} */ (null),
  overlay: null, // { kind: 'palette' | 'jump' | 'new-session' | 'add-project' | 'confirm', … }
  // The floating window (design-notes §3): Settings, Project settings, Week
  // planner or Keyboard shortcuts, one at a time, never saved.
  window:
    /** @type {{ kind: 'settings' | 'project-settings' | 'planner' | 'shortcuts', section?: string | null, server?: string | null, projectId?: string } | null} */ (
      null
    ),
  // What had the focus before the first window opened (an element, never saved);
  // replacing a window keeps it.
  windowReturn: /** @type {HTMLElement | null} */ (null),
  // The Settings window's last section, while the app runs (§3.2).
  settingsSection: /** @type {string | null} */ (null),
  toasts: [],
  pending: [], // decisions in their undo window: { item, card, action, message, at, toast }
  // What the Inspector shows when no agent pane has focus (D10): set by the
  // sidebar, the palette and Mission control. { kind: 'agent' | 'run' | 'decision', id }
  selection: null,
  // A request for the chat column (P4.3): { id } focuses that decision's card
  // (null: the oldest open card) with the filter on All, or on `filter` when
  // given; { search: true } opens its search.
  chatFocus: null,
  chatFilter: 'all', // the chat's level filter (notes §5): never saved
  chatDraft: '', // the Overwatch input's draft (P4.4 notes §2): kept while the app is open, never saved
  chatMentions: /** @type {{ token: string, kind: 'feature', id: string }[]} */ ([]), // the features picked from the @ list, next to the draft
  selectedProject: null,
  theme: null, // null = system, 'dark' | 'light' (user choice)
  themeRev: 0, // bumped when the applied theme may have changed (diagrams redraw)
  kittyTheme: false, // derive surface, text and accent from kitty.conf (§9.1); a user choice wins
  // Agents and shells live in one dock under the panes, one tab each (not in
  // `layout`). `v: 2` marks a height saved under the 0.45 default.
  // `view`, `agentProject`, `activeAgent`, `activeShell`: design-notes §5.2 (`active` = the current view's).
  dock: /** @type {{ open: boolean, height: number, tabs: string[], active: string | null, v: number,
   *  view?: 'agents' | 'shells', agentProject?: string, activeAgent?: string | null, activeShell?: string | null }} */ ({
    open: false,
    height: DOCK_HEIGHT,
    tabs: [],
    active: null,
    v: 2,
    view: 'agents',
    agentProject: 'all',
    activeAgent: null,
    activeShell: null,
  }),
  error: null,
  prTitles: {}, // PR number → title, from the last `pr_builds_list` (the status chip's tooltip)
});

// One clock for relative times ("checked 14:58 · 2m ago"), every 30 s, started on first use.
const tick = $state({ now: Date.now() });
let ticking = null;

/** "now" for relative times; reading it in a template re-renders every 30 s. */
export function now30() {
  if (!ticking) {
    ticking = setInterval(() => (tick.now = Date.now()), 30000);
    ticking.unref?.(); // never keeps a test runner alive
  }
  return tick.now;
}

// A 1 s clock for the Claude update's elapsed time: it runs only while a component holds it.
const sec = $state({ now: Date.now() });
let secTimer = null;
let secUsers = 0;

/** "now" to the second; read it in a template after `holdSecondClock()`. */
export const now1s = () => sec.now;

/** Start the 1 s clock; the returned function releases it (the timer stops with the last holder). */
export function holdSecondClock() {
  sec.now = Date.now();
  if (!secUsers++) secTimer = setInterval(() => (sec.now = Date.now()), 1000);
  let held = true;
  return () => {
    if (!held) return;
    held = false;
    if (!--secUsers) clearInterval(secTimer);
  };
}

/** A pane shown in the dock rather than the workspace. */
export const inDock = (id) => app.dock.tabs.includes(id);

/** The dock takes the whole work column: no workspace tab at all. Never keyed
 *  on the context: a context with no tab still shows the strip and its Select. */
export const dockFills = () => app.dock.open && app.dock.tabs.length > 0 && !app.layout;

// ---------------------------------------------------------------- dock views (§5.2)

const dockKind = (id) => (app.panes[id]?.type === 'shell' ? 'shells' : 'agents');
/** The project a dock agent tab belongs to. */
const tabProject = (id) => app.panes[id]?.projectId ?? agent(app.panes[id]?.agentId)?.project_id ?? null;
export const dockAgents = () => app.dock.tabs.filter((id) => app.panes[id]?.type === 'agent');
export const dockShells = () => app.dock.tabs.filter((id) => app.panes[id]?.type === 'shell');
/** The dock's view: saved, else the kind of its active tab. */
export const dockView = () => app.dock.view ?? dockKind(app.dock.active);
/** The agents filter ('all' or a project id). */
export const dockFilter = () => app.dock.agentProject ?? 'all';
/** The tabs the strip shows: the view's kind, agents narrowed by the project filter. */
export const dockTabs = () =>
  dockView() === 'agents'
    ? dockAgents().filter((id) => dockFilter() === 'all' || tabProject(id) === dockFilter())
    : dockShells();

/** The view's remembered tab, else its first: `dock.active` follows the view and the filter. */
function settleDock() {
  const tabs = dockTabs();
  const kept = dockView() === 'agents' ? app.dock.activeAgent : app.dock.activeShell;
  const id = tabs.includes(kept) ? kept : (tabs[0] ?? null);
  app.dock.active = id;
  if (id) app.dock[dockView() === 'agents' ? 'activeAgent' : 'activeShell'] = id;
}

/** After the view or filter changed: a focus that was in the dock follows the new active tab. */
function refocusDock() {
  if (!inDock(app.focused)) return;
  if (app.dock.active) focusPane(app.dock.active);
  else app.focused = activeTab();
}

/** The view Select (Agents / Shells): restores that view's active tab. */
export function setDockView(view) {
  app.dock.view = view;
  settleDock();
  refocusDock();
  saveLayout();
}

/** The project Select of the Agents view. */
export function setDockProject(projectId) {
  app.dock.agentProject = projectId;
  settleDock();
  refocusDock();
  saveLayout();
}

const waits = (id) => attention(app.panes[id]?.agentId)?.level === 'needs_you';

/** The view Select's options: the number of open dock tabs of each kind, whatever the filter. */
export const dockViewOptions = () => [
  { value: 'agents', label: 'Agents', count: dockAgents().length },
  { value: 'shells', label: 'Shells', count: dockShells().length },
];

/** The project Select's options: All projects + the projects with an open agent tab;
 *  the current filter stays listed at 0. Each carries its waiting dot. */
export function dockProjectOptions() {
  const tabs = dockAgents();
  const per = new Map();
  for (const id of tabs) {
    const p = tabProject(id);
    if (p) per.set(p, [...(per.get(p) ?? []), id]);
  }
  const f = dockFilter();
  if (f !== 'all' && !per.has(f)) per.set(f, []);
  const known = projects()
    .filter((p) => per.has(p.id))
    .map((p) => p.id);
  const ids = [...known, ...[...per.keys()].filter((k) => !known.includes(k))];
  return [
    { value: 'all', label: 'All projects', count: tabs.length, sep: true },
    ...ids.map((id) => ({
      value: id,
      label: projectName(id),
      count: per.get(id).length,
      dot: per.get(id).some(waits),
    })),
  ];
}

/** The view Select's waiting dot (§12.7): in Shells, an agent tab that needs you; text, or null. */
export function dockViewWaiting() {
  const id = dockView() === 'shells' ? dockAgents().find(waits) : null;
  return id ? waitingText(app.panes[id].agentId) : null;
}

/** The closed project Select's waiting dot: an agent tab of another project than the filter (never in All). */
export function dockProjectWaiting() {
  const f = dockFilter();
  const id = f === 'all' ? null : dockAgents().find((t) => tabProject(t) !== f && waits(t));
  return id ? waitingText(app.panes[id].agentId) : null;
}

/** xterm.js panes by pane id (not reactive). */
export const terms = new Map();

let nextPane = 1;
const newPaneId = () => `p${nextPane++}`;

// ---------------------------------------------------------------- derived

export const wide = () => app.width >= breakpoint.md;
/** The Inspector docks beside the chat at ≥ 1728 px; below, it overlays the chat's slot. */
export const inspectorDocked = () => app.width >= breakpoint.lg;
/** The Overwatch column's bounds in px: min 300, max the smaller of 720 and
 *  what leaves the work column `centre-min` beside the sidebar / rail and a
 *  docked Inspector. When even the min does not fit, the min wins. */
export function chatWidthBounds({ width, sidebar, inspector }) {
  const left = !sidebar ? 0 : width >= breakpoint.md ? size.sidebar : size['sidebar-rail'];
  const right = inspector && width >= breakpoint.lg ? size.inspector : 0;
  const fit = width - left - right - size['centre-min'];
  return { min: CHAT_MIN, max: Math.max(CHAT_MIN, Math.min(CHAT_MAX, fit)) };
}

/** `w` rounded and held inside the bounds. */
export const clampChatWidth = (w, bounds) => Math.round(Math.min(bounds.max, Math.max(bounds.min, w)));

/** The default Overwatch width: 360 at ≥ 1512 px, 340 below. */
const chatWidthDefault = () => (wide() ? size.chat : CHAT_NARROW);

/** The Overwatch column's bounds right now. */
export const chatBounds = () => chatWidthBounds({ width: app.width, sidebar: app.sidebar, inspector: app.inspector });

/** The Overwatch column's width right now: the saved one held inside the bounds
 *  (the bounds never overwrite it), or the breakpoint default as it always was. */
export const chatWidthNow = () =>
  app.chatWidth == null ? chatWidthDefault() : clampChatWidth(app.chatWidth, chatBounds());

/** Sets the width (null = default) and remembers it. */
export function setChatWidth(w) {
  app.chatWidth = w == null ? null : Math.round(w);
  saveLayout();
}

/** The Inspector covers the chat column. */
export const inspectorOverlay = () => app.inspector && !inspectorDocked();
export const agents = () => app.data?.agents ?? [];
export const agent = (id) => agents().find((a) => a.id === id);
/** Agents not deleted (archived ones stay in the snapshot for history). */
export const activeAgents = () => agents().filter((a) => a.state !== 'archived');
export const projects = () => app.data?.projects ?? [];
/** A project's name by id (the id itself when it is not listed), null for no id. */
export const projectName = (id) => (id ? (projects().find((p) => p.id === id)?.name ?? id) : null);
/** Features with their latest team run (P2). `null` in a snapshot when the
 *  core could not read them: the poll then keeps the previous list. */
export const features = () => app.data?.features ?? [];
/** A feature by its project and slug (a decision's or an outcome's subject). */
export const featureBySlug = (projectId, slug) => features().find((f) => f.project_id === projectId && f.slug === slug);
export const focusedSpec = () => app.panes[app.focused];
export const isTerminal = (spec) => ['agent', 'shell', 'editor'].includes(spec?.type);

// ---------------------------------------------------------------- project context (design-notes §5.1, §5.3)

/** The project a workspace pane belongs to, or null for an app-wide pane
 *  (Mission control, Setup, the weekly report, a shell, an editor or a canvas without a project). */
export function paneProject(spec) {
  if (!spec) return null;
  if (spec.type === 'feature')
    return features().find((f) => f.id === spec.featureId)?.project_id ?? spec.projectId ?? null;
  if (['project', 'diff', 'trust', 'canvas', 'editor', 'shell'].includes(spec.type)) return spec.projectId ?? null;
  // A Work tab on one project is that project's; on several (or all) it is app-wide.
  if (spec.type === 'work') return spec.projects?.length === 1 ? spec.projects[0] : null;
  // A Program view belongs to its program's project.
  if (spec.type === 'program') return app.now.programs?.[spec.programId]?.project ?? spec.projectId ?? null;
  return null;
}

/** The tab shows in the current context: All projects, an app-wide pane, or one of the context's project. */
export function inContext(id) {
  const ctx = app.workspace.project;
  if (ctx === 'all') return true;
  const p = paneProject(app.panes[id]);
  return p == null || p === ctx;
}

/** The workspace tabs the context shows, in strip order (⌘1…9 count these). */
export const visibleTabs = () => (app.layout?.tabs ?? []).filter(inContext);

/** The workspace's active tab when the context shows it; null = the empty state. */
export function activeTab() {
  const a = app.layout?.active;
  return a && inContext(a) ? a : null;
}

/** A project id the app knows, else 'all' (a removed project, a stale saved value).
 *  Before any `app_state` read nothing is known gone: the id is kept (prunePanes checks it once read). */
export const knownContext = (id) =>
  id && id !== 'all' && (!app.data || projects().some((p) => p.id === id)) ? id : 'all';

/** The context's project id, or null in All projects (or for a project that is gone). */
export function contextProjectId() {
  const ctx = knownContext(app.workspace.project);
  return ctx === 'all' ? null : ctx;
}

/** A hidden active tab gives way to the first tab the context shows (none: left as is, the empty state). */
function settleActive() {
  const vis = visibleTabs();
  if (app.layout && vis.length && !vis.includes(app.layout.active)) app.layout = L.activate(app.layout, vis[0]);
}

/** A sidebar or icon-rail project click: its context, then its pane (rule 4). */
export function openProjectContext(projectId) {
  setContext(projectId);
  openProject(projectId);
}

/**
 * Sets the workspace's project context ('all' or a project id) and remembers it.
 * The active tab stays when the new context shows it, else the first tab it
 * shows becomes active (no per-context last tab, §12.8), else the empty state.
 */
export function setContext(projectId) {
  const was = activeTab();
  app.workspace = { project: knownContext(projectId) };
  settleActive();
  // A workspace focus follows the strip; a dock focus stays where it is.
  const now = activeTab();
  if (now !== was && !inDock(app.focused)) {
    if (now) focusPane(now);
    else app.focused = app.dock.open ? app.dock.active : null;
  }
  saveLayout();
}

/**
 * The dock already shows this agent's waiting dot (design-notes §12.7): it is
 * open and the agent's tab is open there (its view or project Select carries the
 * dot). Then the context Select does not.
 */
export const dockShowsWaiting = (agentId) =>
  app.dock.open && dockAgents().some((id) => app.panes[id].agentId === agentId);

/** An agent's short name: its team node, else its id. */
export const agentShort = (agentId) => agent(agentId)?.team?.node ?? agentId;

/** "lead (orch) is waiting for you". */
const waitingText = (agentId) =>
  `${agentShort(agentId)} (${projectName(agent(agentId)?.project_id) ?? '?'}) is waiting for you`;

/**
 * The closed context Select's waiting dot (§12.7): an agent of another project
 * than the context needs you and the dock does not show its dot. Never in All
 * projects. Its tooltip text names the first one; null = no dot.
 */
export function contextWaiting() {
  const ctx = app.workspace.project;
  if (ctx === 'all') return null;
  const r = needsYou().find((x) => {
    const p = agent(x.agent)?.project_id;
    return p && p !== ctx && !dockShowsWaiting(x.agent);
  });
  return r ? waitingText(r.agent) : null;
}

/** The context Select's options: All projects (every tab) + each project
 *  (its own tabs; a dot while one of its agents needs you). */
export function contextOptions() {
  const tabs = app.layout?.tabs ?? [];
  const per = new Map();
  for (const id of tabs) {
    const p = paneProject(app.panes[id]);
    if (p) per.set(p, (per.get(p) ?? 0) + 1);
  }
  return [
    { value: 'all', label: 'All projects', count: tabs.length, sep: true },
    ...projects().map((p) => ({
      value: p.id,
      label: p.name,
      count: per.get(p.id) ?? 0,
      dot: counts({ project: p.id }).needs_you > 0,
    })),
  ];
}

/** classify()'s context for a keydown on `target` (§9.7). A session in watch
 *  mode is not a terminal for the keyboard: its strip takes ⏎ and Esc (D5). */
export function keyContext(target) {
  const spec = focusedSpec();
  const inXterm = !!target?.closest?.('.xterm');
  return {
    leader: app.leader,
    terminal: isTerminal(spec) && !spec.watch && inXterm,
    agent: spec?.type === 'agent',
    remaps: app.kitty?.key_remaps,
  };
}

/** What the Inspector shows: the focused agent pane's agent, else the selection (D10). */
export function inspected() {
  const id = focusedSpec()?.agentId;
  return id ? { kind: 'agent', id } : app.selection;
}

/** The agent the inspector and the palette are about: the focused pane's, else the selected agent. */
export function contextAgent() {
  const s = inspected();
  return s?.kind === 'agent' ? agent(s.id) : null;
}

/** Selects an agent, a run or a decision for the Inspector. */
export function select(kind, id) {
  app.selection = id == null ? null : { kind, id };
}

export function contextProject() {
  const s = focusedSpec();
  const a = contextAgent();
  // Else the workspace's project context (an app-wide pane in focus), else the first project.
  const id = s?.projectId ?? a?.project_id ?? app.workspace.project;
  return projects().find((p) => p.id === id) ?? projects()[0];
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
      // An interactive shell shows its live directory; a command shell keeps its fixed title.
      if (spec.live && spec.cwd) return liveTitle(spec.shell, spec.cwd, userHome());
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
      if (spec.type === 'canvas') return spec.teamId ? `Team: ${spec.teamId}` : 'Team canvas';
      if (spec.type === 'program')
        return `${app.now.programs?.[spec.programId]?.title ?? spec.programTitle ?? 'Program'} · program`;
      if (spec.type === 'work') return scopeTitle(spec.projects ?? null, (spec.projects ?? []).map(projectName));
      if (spec.type === 'diff')
        return `diff · ${
          spec.branch ??
          String(spec.path ?? '')
            .split('/')
            .pop()
        }`;
      return (
        {
          mission: 'Mission control',
          setup: 'Set up Orchestrator',
          report: 'Weekly report',
        }[spec.type] ?? spec.type
      );
  }
}

/** Adds a pane as the workspace's last tab, focused (its project becomes the context, rule 3). */
export function addPane(spec) {
  leaveWindow();
  const id = newPaneId();
  app.panes[id] = { ...spec, title: spec.title ?? title(spec) };
  app.layout = L.addTab(app.layout, id);
  focusPane(id);
  saveLayout();
  return id;
}

/** Adds a shell or an agent to the dock, as its active tab, and opens the dock. */
export function addDockPane(spec) {
  leaveWindow();
  const id = newPaneId();
  app.panes[id] = { ...spec, title: spec.title ?? title(spec) };
  app.dock.tabs = [...app.dock.tabs, id];
  app.dock.open = true;
  // focusPane picks the view (and the filter) that shows the new tab.
  focusPane(id);
  trackShells();
  saveLayout();
  return id;
}

/** ⌘J: shows or hides the dock (a new shell when it has none). */
export function toggleDock() {
  if (!app.dock.tabs.length) return openShell();
  app.dock.open = !app.dock.open;
  if (app.dock.open) {
    const t = app.dock.active ?? dockTabs()[0];
    if (t) focusPane(t);
    void readShellCwd(...liveShells()); // the titles were not kept current while hidden
  } else {
    if (inDock(app.focused)) {
      // No workspace tab to hand the focus to: nothing is focused, not a hidden tab.
      const next = activeTab();
      if (next) focusPane(next);
      else app.focused = null;
    }
  }
  saveLayout();
}

/** Next or previous dock tab of the view and filter (`d` = 1 or -1). */
export function cycleDock(d) {
  const tabs = dockTabs();
  if (!tabs.length) return;
  const i = tabs.indexOf(app.dock.active);
  focusPane(tabs[i < 0 ? (d > 0 ? 0 : tabs.length - 1) : (i + d + tabs.length) % tabs.length]);
}

export function focusPane(id) {
  if (!app.panes[id]) return;
  const s = app.panes[id];
  if (inDock(id)) {
    // The tab's view; an agent hidden by another project's filter moves the filter to its project.
    app.dock.view = dockKind(id);
    if (s.type === 'agent') {
      const f = dockFilter();
      if (f !== 'all' && tabProject(id) !== f) app.dock.agentProject = tabProject(id) ?? 'all';
      app.dock.activeAgent = id;
    } else app.dock.activeShell = id;
    app.dock.active = id;
    app.dock.open = true;
    saveLayout();
    if (s.live) readShellCwd(id);
  } else {
    // A tab of another project brings its context (rule 3); All projects stays All.
    if (!inContext(id)) {
      app.workspace = { project: knownContext(paneProject(s)) };
      saveLayout();
    }
    app.layout = L.activate(app.layout, id);
  }
  app.focused = id;
  if (s.agentId) select('agent', s.agentId);
  // Focus whatever is focused when this runs: several calls in a row end on
  // the last, and each terminal's focusin then matches `app.focused`. A
  // session in watch mode never gets the terminal: its strip has the keys (D5).
  queueMicrotask(() => {
    const t = app.panes[app.focused]?.watch ? null : terms.get(app.focused);
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
  const slot = inDock(id) ? dockTabs().indexOf(id) : -1; // where it sat in the view, before it goes
  const closedKind = dockKind(id);
  terms.get(id)?.dispose();
  terms.delete(id);
  delete app.panes[id];
  osc7Seen.delete(id);
  polled.delete(id);
  localHosts.delete(id);
  if (inDock(id)) {
    const view = dockView();
    app.dock.tabs = app.dock.tabs.filter((t) => t !== id);
    if (app.dock.activeAgent === id) app.dock.activeAgent = null;
    if (app.dock.activeShell === id) app.dock.activeShell = null;
    if (app.dock.active === id) {
      const rest = dockTabs();
      app.dock.active = rest[Math.min(Math.max(slot, 0), rest.length - 1)] ?? null;
      if (app.dock.active) app.dock[view === 'agents' ? 'activeAgent' : 'activeShell'] = app.dock.active;
    }
    // The last tab of the view's kind: the other view when it has tabs, else the dock closes.
    // (A filter with no agent left keeps its empty view: "No agent running in …".)
    // Only when the closed tab was of the view's kind: closing another kind never moves an empty view the user chose.
    if (closedKind === view && !(view === 'agents' ? dockAgents() : dockShells()).length) {
      const other = view === 'agents' ? 'shells' : 'agents';
      if ((other === 'agents' ? dockAgents() : dockShells()).length) {
        app.dock.view = other;
        settleDock();
      }
    }
    if (!app.dock.tabs.length) app.dock.open = false;
    if (app.focused === id) app.focused = app.dock.active ?? activeTab();
  } else {
    // The next tab the context shows takes an active tab's place (else the one before it).
    const vis = visibleTabs();
    const i = vis.indexOf(id);
    const wasActive = app.layout?.active === id;
    app.layout = L.remove(app.layout, id);
    const rest = vis.filter((t) => t !== id);
    if (wasActive && i >= 0 && rest.length) app.layout = L.activate(app.layout, rest[Math.min(i, rest.length - 1)]);
  }
  if (app.focused === id) app.focused = activeTab() ?? (app.dock.open ? app.dock.active : null);
  if (app.focused) focusPane(app.focused);
  saveLayout();
}

/** Panes the user closed, newest last, for Reopen closed tab (⌘⇧T). A
 *  pane that closes itself (a shell exits, Setup is done, its agent or
 *  feature is gone) goes through `closePane` and is not offered back. */
const closed = [];
const CLOSED_MAX = 20;

/** ⌘W, a tab's ×, a pane's close: the pane can be reopened. */
export function closePaneByUser(id = app.focused) {
  if (!id || !app.panes[id]) return;
  // Watch mode is a session over the board, not part of what the pane is: reopened, it is a plain tab.
  closed.push({ spec: { ...app.panes[id], watch: undefined, over: undefined }, docked: inDock(id) });
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
    return addPane(s);
  }
  toast('No closed tab to reopen');
}

/** What a pane is about: two panes with these equal show the same thing. */
const PANE_KEYS = ['agentId', 'featureId', 'projectId', 'teamId', 'path'];

/** The sizes a PTY is spawned at, before its pane exists to fit it: the pane
 *  sends its real size once mounted (and nudges the program, see TermPane.nudge). */
const SHELL_SIZE = { rows: 24, cols: 80 };
const RUN_SIZE = { rows: 30, cols: 100 };

/** A saved pane spec made live: shells and editors restart as a shell in
 *  their cwd; `null` when the shell does not start. */
async function revive(s) {
  if (s.type !== 'shell' && s.type !== 'editor') return s;
  try {
    const t = await spawnShell({ projectId: s.projectId, cwd: s.cwd });
    // A restored shell is a plain interactive one again, whatever it was running.
    const spec = { ...s, type: 'shell', termId: t.id, spawn: t.spawn, cwd: t.cwd, shell: t.shell, live: true };
    return { ...spec, title: title(spec) };
  } catch {
    return null;
  }
}

export function findPane(pred) {
  return Object.entries(app.panes).find(([, s]) => pred(s))?.[0];
}

/** Singleton panes (Mission control, Setup) open as a tab, or get focus. */
export function openSingleton(type) {
  const existing = findPane((s) => s.type === type);
  if (existing) return showPane(existing);
  addPane({ type });
}

/** ⌘⇧I, the badge, a waiting-on chip, Mission control `d`: the chat column
 *  focused on that decision's card (null: the oldest open one). The
 *  Inspector's overlay closes: the user asks for the cards (P4.2 notes §4). */
export function focusCard(id = null) {
  askChat({ id });
}

/** `/` and the header button: the chat's search (P4.4 notes §2.3). */
export const searchDecisions = () => askChat({ search: true });

/** ⌘L: the Overwatch input, from anywhere; an open search closes (P4.4 notes §2.3). */
export const askOverwatch = () => askChat({ ask: true });

/** The Overwatch's session over the board, when it has one today (notes §1). */
export function openOverwatch() {
  const ow = overwatchOf(app.now);
  if (!canOpenSession(ow)) return;
  openSession(ow.agent ?? 'overwatch', { watch: true });
}

/** Restart (the strip, the palette, Settings): a fresh session; the caller shows a failure where it is (notes §8). */
export const restartOverwatch = () => invoke('overwatch_restart');

/** The chat's level filter (notes §5). */
export const filterDecisions = (id) => askChat({ id: null, filter: id });

/** A request for the chat column; the Inspector's overlay closes (P4.2 notes §4). */
function askChat(req) {
  if (!inspectorDocked()) app.inspector = false;
  app.chatFocus = req;
}

// ---------------------------------------------------------------- windows

/** Saved pane types that are windows now (design-notes §3.1). */
const WINDOW_KINDS = new Set(['settings', 'project-settings', 'planner']);

/** The open window's guard (a footer section registers it): `{ isDirty, onDiscard }`, or null. */
let windowGuard = null;

/**
 * A section with a Save/Revert footer registers its dirty check while it is
 * shown, and what to do when the user discards; the returned function
 * unregisters it (design-notes §3.3).
 * @param {() => boolean} isDirty
 * @param {() => void} [onDiscard]
 */
export function guardWindow(isDirty, onDiscard) {
  const guard = { isDirty, onDiscard };
  windowGuard = guard;
  return () => {
    if (windowGuard === guard) windowGuard = null;
  };
}

/** The open window has unsaved changes. */
export const windowDirty = () => !!app.window && !!windowGuard?.isDirty();

/**
 * A backdrop click: a clean window closes; one with unsaved changes only
 * flashes its footer (Esc, ×, ⌘W ask instead; design-notes §3.3).
 */
export function backdropClose() {
  if (!windowDirty()) return closeWindow();
  const foot = /** @type {HTMLElement | null} */ (document.querySelector('.fw-layer .foot'));
  if (!foot) return;
  foot.classList.remove('flash');
  void foot.offsetWidth; // restart the animation
  foot.classList.add('flash');
}

/** Esc, ×, backdrop: the window's `onclose(reason)`. */
export const onWindowClose = (reason) => (reason === 'backdrop' ? backdropClose() : closeWindow());

/** Asks "Discard changes?" when the window is dirty, else runs `then` at once. */
export function unlessDirty(then) {
  if (!windowDirty()) return then();
  confirm(
    'Discard changes?',
    'This section has changes that are not saved.',
    () => {
      const guard = windowGuard;
      windowGuard = null;
      guard?.onDiscard?.();
      then();
    },
    'Discard',
    'Keep editing',
  );
}

const sameWindow = (a, b) => a?.kind === b?.kind && a?.projectId === b?.projectId && a?.section === b?.section;

/** Opens `next`, replacing the open window (asked first when dirty and something else shows). */
function setWindow(next) {
  const apply = () => {
    if (!app.window) {
      const el = typeof document === 'undefined' ? null : /** @type {HTMLElement | null} */ (document.activeElement);
      app.windowReturn = el && el !== document.body ? el : null;
    }
    app.window = next;
    if (next.kind === 'settings') app.settingsSection = next.section ?? null;
  };
  if (sameWindow(app.window, next)) return apply();
  unlessDirty(apply);
}

/** Moves the focus back where it was before the window opened, else to the focused pane.
 *  Not when the focus moved on meanwhile (a pane opened from the window took it). */
function restoreFocus(el, was) {
  svelteTick().then(() => {
    if (app.window || app.focused !== was) return;
    if (el?.isConnected) el.focus();
    else if (app.focused) focusPane(app.focused);
  });
}

/** Closes the window (Esc, ×, ⌘W, the backdrop); `force` skips the Discard question. */
export function closeWindow(force = false) {
  if (!app.window) return;
  const close = () => {
    windowGuard = null;
    const el = app.windowReturn;
    app.window = null;
    app.windowReturn = null;
    restoreFocus(el, app.focused);
  };
  if (force) close();
  else unlessDirty(close);
}

/** A pane opens (the palette above a window, a link in it): the window gives way, unless it has unsaved changes. */
function leaveWindow() {
  if (app.window && !windowDirty()) closeWindow(true);
}

/** An open pane gets the focus (a jump, a link): like a new tab, it gives the window way. */
export function showPane(id) {
  leaveWindow();
  focusPane(id);
}

/** ⌘, and ⌘/: the window of that kind closes when it is open, else opens (replacing another). */
export function toggleWindow(kind) {
  if (app.window?.kind !== kind) return kind === 'settings' ? openSettings() : openShortcuts();
  closeWindow();
}

/**
 * Settings (⌘,), the floating window (design-notes §3.1), on a section with a
 * server or PR selected (banners, toasts, decision cards, the inspector).
 * An open Settings window updates in place. A project with section `mcp` is
 * Project settings › MCP overrides; with any other section it only says where
 * the caller came from and is ignored.
 */
export function openSettings(section = null, server = null, projectId = null) {
  if (projectId && section === 'mcp') return openProjectSettings(projectId, 'mcp', server);
  const w = app.window;
  if (w?.kind === 'settings') return setWindow({ ...w, section: section ?? w.section, server });
  setWindow({ kind: 'settings', section: section ?? app.settingsSection ?? 'integrations', server });
}

/** A project's settings, one window for any project (opening another project's replaces it). */
export function openProjectSettings(projectId, section = null, server = null) {
  if (!projectId) return toast('No project in focus');
  const w = app.window;
  if (w?.kind === 'project-settings' && w.projectId === projectId)
    return setWindow({ ...w, section: section ?? w.section, server });
  setWindow({ kind: 'project-settings', projectId, section: section ?? 'tracker', server });
}

/** The Week planner window (pacing indicator, banners, leader g, the palette). */
export function openPlanner() {
  if (app.window?.kind !== 'planner') setWindow({ kind: 'planner' });
}

/** The Keyboard shortcuts sheet (⌘/, Help › Keyboard Shortcuts, the palette). */
export function openShortcuts() {
  if (app.window?.kind !== 'shortcuts') setWindow({ kind: 'shortcuts' });
}

/** The section of the open window (its nav): a switch away from a dirty section asks first. */
export function windowSection(section) {
  const w = app.window;
  if (!w || w.section === section) return;
  setWindow({ ...w, section, server: null });
}

/** FR-818: the weekly report ('current' = this week so far), one pane; `'list'` opens it on the
 *  list of past reports (§9 bug 2: the palette's "Past weekly reports"). */
export function openReport(reportId = 'current') {
  const list = reportId === 'list';
  const id = list ? 'current' : reportId;
  const existing = findPane((s) => s.type === 'report');
  if (existing) {
    app.panes[existing] = { ...app.panes[existing], reportId: id, list };
    return showPane(existing);
  }
  addPane({ type: 'report', reportId: id, list });
}

/** A link in the browser (the core runs `open`; only http(s)). */
export async function openUrl(url) {
  try {
    await invoke('open_url', { url });
  } catch (e) {
    toast(`Could not open ${url}: ${errText(e)}`, { kind: 'error' });
  }
}

/**
 * Where a card's action sends the user (D3): `{open: 'settings', server}`,
 * `{open: 'pane', run}` or `{open: 'url', url}`. Anything else: nothing.
 */
export function routeOpen(r, projectId = null) {
  if (!r || typeof r !== 'object') return false;
  if (r.open === 'settings')
    projectId ? openProjectSettings(projectId, 'mcp', r.server ?? null) : openSettings('mcp', r.server ?? null);
  else if (r.open === 'pane' && r.run) runInPane(r.run, `${r.run}${r.server ? ` · ${r.server}` : ''}`, projectId);
  else if (r.open === 'url' && r.url) openUrl(r.url);
  else if (r.open === 'report') openReport(r.report ?? 'current');
  // A brief card's Launch / Edit answered by the core (P4.5 api-contract §5): the prefilled dialog.
  else if (r.open === 'launch' && r.prefill)
    openPrefilled(r.prefill, {
      intake: r.intake ?? (r.decision ? { brief_decision: r.decision } : undefined),
      origin:
        r.intake?.brief_decision || r.decision
          ? { kind: 'brief', decision: r.intake?.brief_decision ?? r.decision, title: r.title ?? null }
          : null,
      focus: r.focus === 'brief' || r.edit ? 'brief' : null,
    });
  else if (r.open === 'editor' && r.scope) openPolicy(r.scope, r.project ?? projectId);
  // Program cards (improve-intake notes §7.3): the Program view, its row editor, or the failed feature.
  else if (r.open === 'program' && r.program) openProgram(r.program);
  else if (r.open === 'program_add' && r.program) openProgram(r.program, '', { add: r.item ?? true });
  else if (r.open === 'feature' && r.feature) {
    const f = features().find((x) => x.slug === r.feature && (!projectId || x.project_id === projectId));
    if (f) openFeature(f.id);
    else if (r.program) openProgram(r.program);
  } else return false;
  return true;
}

/** A policy file in nvim (api-contract §5 `policy_open`): created with its first comment when missing. */
export async function openPolicy(scope, projectId = null) {
  // The core's scope label is `global` or `project:<id>`; a bare `project` takes the card's project.
  const m = /^project:(.+)$/.exec(scope);
  if (m) projectId = m[1];
  scope = m ? 'project' : scope;
  const project = scope === 'project' ? projectId : null;
  try {
    const r = await invoke('policy_open', { scope, project, ...RUN_SIZE });
    if (r.reused) {
      focusEditor(project);
      return toast('Opened the policy file in nvim');
    }
    addPane({ type: 'editor', termId: r.term.id, spawn: RUN_SIZE, cwd: r.term.cwd, projectId: project, file: null });
  } catch (e) {
    toast(`Could not open the policy file: ${errText(e)}`, { kind: 'error' });
  }
}

/** Undoes an Overwatch line (api-contract §3.3): `text` is the correction of an answer.
 *  The line comes back updated; a refusal throws, `{ ok: false, gone }` says why it can't. */
export async function undoLine(entry, text = null) {
  const r = await invoke('chat_undo', text == null ? { entry: entry.id } : { entry: entry.id, text });
  if (r?.entry) upsertEntries((app.now.chat ??= { day: null, entries: {} }), [r.entry]);
  return r;
}

/** An action the core answers at once (no undo window): Open (a Notion page, a
 *  report…), an MCP Reconnect or Open Settings; then routed where it says. */
export async function coreAct(d, action) {
  try {
    routeOpen(await invoke('inbox_act', { item: d.id, action, message: null }), d.subject?.project ?? null);
  } catch (e) {
    toast(errText(e), { kind: 'error' });
  }
}

/** A command in a dock shell (gh auth login, claude /mcp); the user exits it. */
export async function runInPane(run, title, projectId = null) {
  const p = projects().find((x) => x.id === projectId);
  try {
    const t = await spawnShell({ projectId: p?.id, agentId: null, cwd: p?.path, run });
    addDockPane({ type: 'shell', termId: t.id, spawn: t.spawn, cwd: t.cwd, projectId: p?.id, title });
  } catch (e) {
    toast(`Could not run ${run}: ${errText(e)}`, { kind: 'error' });
  }
}

/** A project's pane (§4.1): git state, worktrees, PRs, removal (J11). */
export function openProject(projectId) {
  app.selectedProject = projectId;
  const existing = findPane((s) => s.type === 'project' && s.projectId === projectId);
  if (existing) return showPane(existing);
  addPane({ type: 'project', projectId });
}

/** Project settings › General › Remove project…: the removal flow lives in the project pane,
 *  so its pane opens and starts it (a one-shot flag the pane clears). */
export function askRemoveProject(projectId) {
  openProject(projectId);
  const id = findPane((s) => s.type === 'project' && s.projectId === projectId);
  if (id) app.panes[id] = { ...app.panes[id], remove: true };
}

/** P2: a feature's pane (docs, tasks, team, timeline), one per feature. */
export function openFeature(featureId, doc = null, anchor = null, timeline = null, land = null) {
  const existing = findPane((s) => s.type === 'feature' && s.featureId === featureId);
  const landReq = land && { ...land, seq: Date.now() };
  if (existing) {
    if (doc) app.panes[existing] = { ...app.panes[existing], doc, anchor };
    if (timeline) app.panes[existing].timeline = timeline;
    if (landReq) app.panes[existing] = { ...app.panes[existing], land: landReq };
    return showPane(existing);
  }
  const f = features().find((x) => x.id === featureId);
  addPane({
    type: 'feature',
    featureId,
    projectId: f?.project_id,
    ...(doc ? { doc, anchor } : {}),
    ...(timeline ? { timeline } : {}),
    ...(landReq ? { land: landReq } : {}),
  });
}

/** The undelivered chip's target (ux-brief §2): the decision if there is one, else
 *  the agent's feature timeline filtered to Delivery. */
export function openDelivery(rec) {
  const ref = deliveryRef(rec);
  if (ref) return focusCard(ref);
  const f = features().find((x) => x.run?.id && x.run.id === rec?.run);
  if (f) openFeature(f.id, null, null, 'delivery');
  else openAgent(rec?.agent);
}

/** Landing on a task (notes §9.3): the Feature pane of the task's run, Tasks scrolled in, the task
 *  focused and flashed (`conflict`: its menu open at "Resolve conflict…"). `hint` carries what the
 *  caller knows (`run`, `feature` slug or id, `remote_url`, `ref`, `tracker`). A run that is gone: a toast offering the upstream issue. */
export function openTask(projectId, taskId, hint = {}) {
  const id = String(taskId ?? '').replace(/^T-/, '');
  const f = taskFeature(projectId, hint);
  if (f) return openFeature(f.id, null, null, null, { task: id, conflict: !!hint.conflict });
  const name = `T-${id}`;
  const url = hint.remote_url ?? null;
  toast(
    `${name} belongs to ${hint.feature ?? 'a feature'} (deleted).${url ? ` Open ${hint.ref ?? 'it'}${hint.tracker ? ` in ${hint.tracker}` : ' upstream'}?` : ''}`,
    url ? { action: { label: 'Open', run: () => openUrl(url) }, ms: 12000 } : {},
  );
}

/** The feature of a task link: its run (task links carry `run`, F24), else the feature named by the hint. */
function taskFeature(projectId, hint) {
  const all = features();
  const byRun = hint.run ? all.find((f) => f.run?.id === hint.run) : null;
  if (byRun) return byRun;
  if (!hint.feature) return null;
  return all.find((f) => f.id === hint.feature || (f.project_id === projectId && f.slug === hint.feature)) ?? null;
}

/** The first time an old Board path is used (Feature `b`, the inspector): one toast, then never (notes §1.3). */
export async function boardNotice() {
  const seen = await invoke('settings_get', { key: 'work.board_notice_seen' }).catch(() => null);
  if (seen) return;
  invoke('settings_set', { key: 'work.board_notice_seen', value: true }).catch(() => {});
  toast('The Board is gone: tracker items are in Work (w), team tasks are here in Tasks.', { ms: 9000 });
}

/** P2: the Team canvas (FR-301), on a team (or the library when none). Bound to
 *  `projectId`, else to the context's project; from All projects it is app-wide (§12.2). */
export function openCanvas(teamId = null, projectId = null) {
  // A team's canvas is that team's; a team-less one is the project's (in a project context) or the app-wide one (All projects).
  const project = projectId ?? contextProjectId();
  const existing = findPane(
    (s) => s.type === 'canvas' && (teamId ? s.teamId === teamId : (paneProject(s) ?? null) === (project ?? null)),
  );
  if (existing) return showPane(existing);
  addPane({ type: 'canvas', teamId, projectId: project });
}

/** FR-107: a worktree's diff against its base, one pane per worktree. */
export function openDiff(projectId, path, branch = null) {
  const existing = findPane((s) => s.type === 'diff' && s.path === path);
  if (existing) return showPane(existing);
  addPane({ type: 'diff', projectId, path, branch });
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
    const t = await spawnShell({ projectId: p.id, agentId: null, cwd: p.path, run });
    addDockPane({
      type: 'shell',
      termId: t.id,
      spawn: t.spawn,
      cwd: t.cwd,
      projectId: p.id,
      title: `trust · ${p.name}`,
    });
    toast(`Answer "Yes, I trust this folder" in the pane, then /exit · ${p.name}`, { ms: 10000 });
  } catch (e) {
    toast(`Could not start claude: ${errText(e)}`, { kind: 'error' });
  }
}

/** Mission control (leader o, the palette): focused, else opened as a tab. */
export const openMission = () => openSingleton('mission');

/**
 * A session over the board (D5, spec §3.3): the agent's dock tab, in watch
 * mode. One already open gets focus, in watch mode again (notes §1.10).
 * Without Mission control: as `openAgent`.
 */
export function openSession(agentId, { watch = false, keepLive = false } = {}) {
  const over = findPane((s) => s.type === 'mission');
  // `watch`: never a live tab, so a keystroke can't reach the session by accident (it counts as an intervention).
  if (!over && !watch) return openAgent(agentId);
  const existing = findPane((s) => s.type === 'agent' && s.agentId === agentId);
  // `keepLive`: a tab you are typing in stays as it is (a link only brings it forward).
  if (existing && keepLive && !app.panes[existing].watch) return showPane(existing);
  const from = app.focused;
  if (existing) {
    Object.assign(app.panes[existing], { watch: true, over: over ?? null, from });
    return showPane(existing);
  }
  return addDockPane({
    type: 'agent',
    agentId,
    projectId: agent(agentId)?.project_id,
    watch: true,
    over: over ?? null,
    from,
  });
}

/** Take-over (⏎ or a click in watch mode): the PTY gets the keyboard. */
export function takeOver(paneId) {
  const s = app.panes[paneId];
  if (!s?.watch) return;
  s.watch = false;
  focusPane(paneId);
}

/** Esc in watch mode: back to the Mission control the session opened over; the dock tab stays. */
export function leaveWatch(paneId) {
  const { over, from } = app.panes[paneId] ?? {};
  // Back to where it was opened from: the board, else the pane that had the focus (the chat's status).
  if (over && app.panes[over]) focusPane(over);
  else if (from && app.panes[from] && from !== paneId) focusPane(from);
  else openMission();
}

/** An agent: a tab of the dock, focused (its tab is focused when it is open). */
export function openAgent(agentId) {
  const existing = findPane((s) => s.type === 'agent' && s.agentId === agentId);
  if (existing) return showPane(existing);
  addDockPane({ type: 'agent', agentId, projectId: agent(agentId)?.project_id });
}

/**
 * A login shell PTY in the core (running `run` when given); the pane fits it
 * once mounted. Returns the term with the size it was spawned at.
 * @param {{ projectId?: string, agentId?: string, cwd?: string, run?: string }} req
 */
async function spawnShell({ projectId, agentId, cwd, run }) {
  const spawn = run ? RUN_SIZE : SHELL_SIZE;
  const t = await invoke('term_spawn', {
    req: {
      kind: 'shell',
      project_id: projectId ?? null,
      agent_id: agentId ?? null,
      cwd: cwd ?? null,
      run: run ?? null,
      ...spawn,
    },
  });
  return { ...t, spawn };
}

/** A shell, as a dock tab. `agentId: null` = no agent (undefined = the one in focus). */
export async function openShell(opts = {}) {
  const projectId = opts.projectId ?? contextProject()?.id;
  const agentId = opts.agentId ?? contextAgent()?.id;
  try {
    const t = await spawnShell({ projectId, agentId, cwd: opts.cwd });
    addDockPane({
      type: 'shell',
      termId: t.id,
      spawn: t.spawn,
      cwd: t.cwd,
      shell: t.shell,
      live: true,
      projectId,
      agentId,
    });
  } catch (e) {
    toast(`Could not open a shell: ${errText(e)}`, { kind: 'error' });
  }
}

/** Focuses the project's open editor pane (a global one has no project: null or undefined). */
function focusEditor(projectId) {
  const pane = findPane((s) => s.type === 'editor' && (s.projectId ?? null) === (projectId ?? null));
  if (pane) focusPane(pane);
}

/** FR-1101/1102: reuse the project's nvim when it runs, else a new editor pane. */
export async function openEditor(file = null, line = null, opts = {}) {
  // An explicit project never takes the focused pane's agent: that one may belong to another project.
  const a = opts.agentId ? agent(opts.agentId) : opts.projectId ? null : contextAgent();
  const p = projects().find((x) => x.id === (opts.projectId ?? a?.project_id)) ?? contextProject();
  try {
    const r = await invoke('editor_open', {
      projectId: p?.id ?? null,
      agentId: a?.id ?? null,
      file,
      line,
      ...RUN_SIZE,
    });
    if (r.reused) {
      focusEditor(p?.id);
      toast(
        file
          ? `Opened ${file}${line ? `:${line}` : ''} in nvim · ${p?.name ?? ''}`
          : `nvim is already open · ${p?.name ?? ''}`,
      );
      return;
    }
    addPane({ type: 'editor', termId: r.term.id, spawn: RUN_SIZE, cwd: r.term.cwd, projectId: p?.id, file });
  } catch (e) {
    toast(`Could not open the editor: ${errText(e)}`, { kind: 'error' });
  }
}

// ---------------------------------------------------------------- shell live title (§6)

const userHome = () => app.data?.core?.user_home ?? '';
/** Dock shells whose OSC 7 was seen (instant: no poll), and those the core's poll has answered. */
const osc7Seen = new Set();
const polled = new Set();
/** Per shell, the host names its OSC 7 reported first (the local prompt's, before any ssh). */
const localHosts = new Map();

const liveShells = () => app.dock.tabs.filter((id) => app.panes[id]?.live && app.panes[id].type === 'shell');
/** The shells whose titles are on screen: none while the dock is hidden. */
const shownShells = () => (app.dock.open ? liveShells() : []);

/** A new directory for a live shell: its title follows, the layout is saved (throttled). */
function setShellCwd(id, cwd) {
  const s = app.panes[id];
  if (!s?.live || !cwd || s.cwd === cwd) return;
  s.cwd = cwd;
  s.title = title(s);
  saveLayout();
}

/** OSC 7 from the shell: `file://host/path`. A foreign host (an ssh session) is ignored. */
export function onShellOsc7(id, data) {
  const r = parseOsc7(data);
  if (!r || !app.panes[id]?.live) return;
  let names = localHosts.get(id);
  if (!names) localHosts.set(id, (names = []));
  if (!names.length && r.host) names.push(r.host);
  if (!isLocalHost(r.host, names)) return;
  osc7Seen.add(id);
  setShellCwd(id, r.path);
}

/** OSC 0/2: the last source, only when it looks like a path and nothing better answered. */
export function onShellTitle(id, text) {
  if (osc7Seen.has(id) || polled.has(id) || !looksLikePath(text)) return;
  // `~` cannot be expanded before the home directory is known: "~/dev" would read "/dev".
  if (/^\s*~/.test(text) && !userHome()) return;
  setShellCwd(id, expandHome(text, userHome()));
}

/** One `term_cwd` read per shell, skipping those that sent OSC 7. */
export async function readShellCwd(...ids) {
  await Promise.all(
    ids.map(async (id) => {
      const termId = app.panes[id]?.termId;
      if (osc7Seen.has(id) || termId == null) return;
      const dir = await invoke('term_cwd', { id: termId }).catch(() => null);
      // The tab may have closed during the read: nothing is recorded for a gone pane.
      if (!dir || osc7Seen.has(id) || !app.panes[id]) return;
      polled.add(id);
      setShellCwd(id, dir);
    }),
  );
}

let cwdTimer = null;
/** Polls the live shells every POLL_MS while the window is visible; stops when none is left. */
export function trackShells() {
  if (cwdTimer || !liveShells().length) return;
  cwdTimer = setInterval(() => {
    const ids = liveShells();
    if (!ids.length) {
      clearInterval(cwdTimer);
      cwdTimer = null;
    } else if (document.visibilityState === 'visible') void readShellCwd(...shownShells());
  }, POLL_MS);
}

if (typeof document !== 'undefined') {
  // The window shown again: one read at once, not after the next tick.
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') void readShellCwd(...shownShells());
  });
}

/** Creates (or returns) the xterm.js pane for a pane spec. */
export function termFor(paneId, hooks) {
  let t = terms.get(paneId);
  if (t) {
    t.hooks = hooks;
    return t;
  }
  const s = app.panes[paneId];
  const source = s.type === 'agent' ? { kind: 'agent', id: s.agentId } : { kind: 'term', id: s.termId, spawn: s.spawn };
  t = new TermPane(source, app.kitty, hooks);
  terms.set(paneId, t);
  return t;
}

// ---------------------------------------------------------------- focus moves

/** Leader j / k: down into the dock's active tab, up back to the workspace's active tab. */
export function focusDirection(dir) {
  if (dir === 'down' && !inDock(app.focused) && app.dock.open && app.dock.active) focusPane(app.dock.active);
  else if (dir === 'up' && inDock(app.focused)) {
    const a = activeTab();
    if (a) focusPane(a);
  }
}

/** ⌘1…9: the n-th tab the context shows; out of range does nothing. */
export function focusNumber(n) {
  const id = visibleTabs()[n - 1];
  if (id) focusPane(id);
}

/** Leader w: the next agent that needs you, longest waiting first (attention ladder level 1). */
export function nextWaiting() {
  const waiting = needsYou();
  if (!waiting.length) return toast('No agent is waiting for you');
  const cur = contextAgent()?.id;
  const i = waiting.findIndex((r) => r.agent === cur);
  openAgent(waiting[(i + 1) % waiting.length].agent);
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

/** Kill after a confirm (the `⋯` Menu, the palette). */
export const confirmKill = (id) =>
  confirm(
    `Kill ${id}?`,
    'Its process group and every process left in its worktree get SIGTERM, then SIGKILL after 5 s. The session can be resumed.',
    () => agentAction(id, 'kill'),
    'Kill',
  );

/** A fresh session after a confirm (the `⋯` Menu, the palette). */
export const confirmRestartFresh = (id) =>
  confirm(
    `Restart ${id} fresh?`,
    'Its current session is left; a new one starts with its brief and task.',
    () => restartFresh(id),
    'Restart fresh',
  );

/** The Delete agent dialog (the `⋯` Menu, the palette). */
export const askDeleteAgent = (id) => (app.overlay = { kind: 'delete-agent', agentId: id });

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

/** Pause all autonomous work: the planner, the palette and the Banner share this and its toast. */
export const pauseAll = () =>
  governor('pause_all').then((r) => (r !== undefined && toast('Paused all · Resume all from the palette'), r));

export async function governor(action, arg = null) {
  try {
    const r = await invoke('governor_act', { action, arg });
    await refresh();
    return r;
  } catch (e) {
    toast(errText(e), { kind: 'error' });
  }
}

/** Reveals a path in Finder; a failure toasts. */
export const revealPath = (path) => invoke('reveal_path', { path }).catch((e) => toast(errText(e), { kind: 'error' }));

/** Copies text to the clipboard and toasts `Copied <what>`. */
export async function copyText(text, what) {
  try {
    await navigator.clipboard?.writeText(text);
    toast(`Copied ${what}`);
  } catch (e) {
    toast(errText(e), { kind: 'error' });
  }
}

export function openInKitty(cwd) {
  invoke('open_in_kitty', { cwd }).catch((e) => toast(errText(e), { kind: 'error' }));
}

/** Run a queued turn now, or start a not-started agent, after the user confirms its cost in both windows
 *  (FR-809, notes §1.12). `rec` is the agent's attention record: its `queue` names the entry (D7). */
export function runNow(rec) {
  if (!rec?.queue) return;
  const d = runNowDialog(rec);
  const fresh = rec.status === 'not_started';
  const cost = runNowCost(rec.queue);
  confirm(
    d.title,
    d.detail,
    () => queueAct('run_now', rec, `${fresh ? 'Started' : 'Running now'} · ${rec.agent}${cost ? ` · ${cost}` : ''}`),
    d.yes,
    fresh ? 'Keep waiting' : 'Keep queued',
  );
}

/** FR-816: a queued turn to the next off-hours window, or back (D4: you scheduled it). */
export const runOffHours = (a, off = a.queue_entry?.reason?.kind !== 'scheduled_off_hours') =>
  queueOffHours({ entry: a.queue_entry.id, off }, a.id, refresh);

/**
 * FR-816: mark a queued entry or a task for off-hours (or clear it), with
 * the Penpot 05 toast "deps-1 runs off-hours · starts 18:00 · Undo";
 * `reload` refreshes the caller's view.
 */
export async function queueOffHours({ entry = null, task = null, off }, label, reload = refresh) {
  const id = entry != null ? { entry: String(entry) } : { task: String(task) };
  try {
    const r = await invoke('queue_off_hours', { ...id, off });
    toast(
      off ? `${label} runs off-hours · starts ${clockOf(Date.parse(r.starts_at))}` : `${label} runs in working hours`,
      off ? { action: { label: 'Undo', run: () => queueOffHours({ entry, task, off: false }, label, reload) } } : {},
    );
    await reload?.();
  } catch (e) {
    toast(errText(e), { kind: 'error' });
  }
}

/** FR-505: the feature's docs to Notion (also `N` in the Feature pane). */
export async function publishNotion(f) {
  try {
    const r = await invoke('notion_publish', { feature: f.id });
    toast(`Published ${r.docs.length} docs to Notion`, { action: { label: 'Open', run: () => openUrl(r.page_url) } });
  } catch (e) {
    toast(`Publish failed: ${errText(e)}`, { kind: 'error' });
  }
}

/** A queue entry's Run now or Cancel; the toast only once the core confirms (notes §1.13). */
async function queueAct(action, rec, done) {
  try {
    await invoke('governor_act', { action, arg: rec.queue.entry });
    toast(done);
    await refresh();
  } catch (e) {
    toast(`Failed: ${errText(e)}`, { kind: 'error' });
  }
}

/** Removes the agent's queue entry (notes §1.5), after asking: its waiting turn is dropped. */
export function cancelQueued(rec) {
  if (!rec?.queue) return;
  const go = () => queueAct('cancel', rec, `Removed from the queue · ${rec.agent}`);
  const run = rec.run ? app.now.runs[rec.run] : null;
  const detail = run
    ? `The ${run.feature} run keeps going without it.`
    : 'Its waiting turn and message are dropped and logged; the agent keeps its session.';
  confirm(`Cancel ${rec.agent}?`, detail, go, 'Remove from queue', 'Keep it');
}

/** Mounts the pane's xterm in `host`, fits it on resize; returns the cleanup. */
export function mountTerm(paneId, host, hooks) {
  const t = termFor(paneId, {
    // Watch mode: xterm takes no key (D5); a click in it takes over.
    onKey: (e) => !e.metaKey && !app.panes[paneId]?.watch,
    // In watch mode only ⏎ or a click takes over (TermHost): Tab into xterm goes back to the strip.
    onFocus: () => (app.panes[paneId]?.watch || app.focused !== paneId) && focusPane(paneId),
    onOsc7: (data) => onShellOsc7(paneId, data),
    onTitle: (text) => onShellTitle(paneId, text),
    ...hooks,
  });
  t.mount(host);
  if (app.focused === paneId && !app.panes[paneId]?.watch) t.focus();
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

export function confirm(text, detail, onYes, yes = 'Confirm', no = 'Cancel') {
  app.overlay = { kind: 'confirm', text, detail, onYes, yes, no };
}

// ---------------------------------------------------------------- persistence

let saveTimer = null;
export function saveLayout() {
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => {
    const panes = Object.fromEntries(
      Object.entries(app.panes).map(([id, s]) => [
        id,
        { ...s, termId: undefined, spawn: undefined, watch: undefined, over: undefined, land: undefined },
      ]),
    );
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
        chatWidth: app.chatWidth,
        sidebarTree: $state.snapshot(app.sidebarTree),
        workspace: { project: app.workspace.project },
      },
    }).catch(() => {});
  }, 400);
}

/** Sidebar caret click (§5.4): a project (`p.id`) or feature (`f:<id>`) row, remembered. */
export function toggleTree(key) {
  app.sidebarTree = { ...app.sidebarTree, [key]: !treeOpen(key) };
  saveLayout();
}

/** A sidebar row is expanded unless the user closed it. */
export const treeOpen = (key) => app.sidebarTree[key] ?? true;

/** The saved tree without the projects and features that are gone (projects or
 *  features never read, the first `app_state` read failed: their keys stay). */
function pruneTree(tree) {
  if (!tree || typeof tree !== 'object') return {};
  const read = app.data?.features != null;
  return Object.fromEntries(
    Object.entries(tree).filter(([k, v]) => {
      if (typeof v !== 'boolean') return false;
      if (k.startsWith('f:')) return !read || features().some((f) => f.id === k.slice(2));
      return knownContext(k) !== 'all';
    }),
  );
}

/** Restores the saved layout: shells and editors restart in their cwd (NFR-2).
 *  A layout saved with splits comes back as one tab strip (design-notes §4.2);
 *  the old `mission` key (Mission control's filter) is not read any more. */
export async function restoreLayout() {
  const saved = await invoke('settings_get', { key: 'layout' }).catch(() => null);
  app.sidebarTree = pruneTree(saved?.sidebarTree);
  // The saved context, else the focused pane's project, else All (rule 5, 6).
  const ctx = saved?.workspace?.project;
  app.workspace = {
    project: knownContext(
      ctx ?? paneProject(saved?.panes?.[saved?.focused]) ?? saved?.panes?.[saved?.focused]?.projectId,
    ),
  };
  if (saved) {
    app.sidebar = saved.sidebar ?? true;
    // Never opens over the chat by itself (P4.2 notes §4 rule 1): docked only.
    app.inspector = !!saved.inspector && inspectorDocked();
    app.theme = saved.theme ?? null;
    app.kittyTheme = saved.kittyTheme ?? false;
  }
  const cw = saved?.chatWidth;
  app.chatWidth = Number.isFinite(cw) && cw >= CHAT_MIN && cw <= CHAT_MAX ? Math.round(cw) : null;
  // A dock with tabs and no workspace pane is a layout too.
  if (!saved?.layout && !saved?.dock?.tabs?.length) {
    addPane({ type: 'mission' });
    if (!app.data?.setup?.done) openSingleton('setup');
    return;
  }
  // Agents saved in the workspace move to the dock, after the saved dock tabs.
  const migrated = L.allPanes(saved.layout).filter((id) => saved.panes[id]?.type === 'agent');
  const docked = [...(saved.dock?.tabs ?? []), ...migrated];
  const ids = [...L.allPanes(saved.layout), ...docked];
  for (const id of ids) nextPane = Math.max(nextPane, Number(id.slice(1)) + 1);
  const panes = {};
  // Shells and editors restart in parallel; one that does not start is dropped.
  await Promise.all(
    ids.map(async (id) => {
      let s = saved.panes[id];
      // A saved `inbox` pane (removed in P4.3) or `board` pane (removed in improve-intake): its slot closes.
      // Settings, Project settings and the Week planner are windows now (design-notes §3.1): dropped, never opened at launch.
      if (!s || s.type === 'inbox' || s.type === 'board' || WINDOW_KINDS.has(s.type) || paneGone(s)) return;
      // The Overview became Mission control (D9).
      if (s.type === 'overview') s = { ...s, type: 'mission', title: 'Mission control' };
      // A shell that does not start is dropped from the restored layout.
      const live = await revive(s);
      if (live) panes[id] = live;
    }),
  );
  app.panes = panes;
  const tabs = docked.filter((id) => panes[id]);
  const landed = migrated.some((id) => panes[id]);
  // The focused agent that moved is the active tab; else the saved one.
  const active = panes[saved.focused] && migrated.includes(saved.focused) ? saved.focused : saved.dock?.active;
  // A height saved before the 0.45 default (no `v`) is the old 0.3 default: moved once.
  const height = saved.dock?.height;
  /** A saved per-view active tab, only when it is still a tab of that kind. */
  const keep = (id, k) => (tabs.includes(id) && dockKind(id) === k ? id : null);
  const sd = saved.dock ?? {};
  // The view: saved, else the kind of the active tab (§12.4), else Agents when one is open.
  const view =
    sd.view === 'agents' || sd.view === 'shells'
      ? sd.view
      : tabs.includes(active)
        ? dockKind(active)
        : tabs.some((id) => dockKind(id) === 'agents')
          ? 'agents'
          : 'shells';
  app.dock = {
    open: (!!sd.open || landed) && tabs.length > 0,
    height: sd.v === 2 ? (height ?? DOCK_HEIGHT) : !height || height === 0.3 ? DOCK_HEIGHT : height,
    tabs,
    active: null,
    v: 2,
    view,
    agentProject: knownContext(sd.agentProject),
    activeAgent: keep(sd.activeAgent, 'agents') ?? keep(active, 'agents'),
    activeShell: keep(sd.activeShell, 'shells') ?? keep(active, 'shells'),
  };
  // An active tab outside the filter falls back to the filter's first agent tab, else the empty view.
  settleDock();
  trackShells();
  const kept = (id) => !!panes[id] && !migrated.includes(id);
  app.layout = L.prune(L.flatten(saved.layout, kept(saved.focused) ? saved.focused : null), kept);
  // The context's first tab when the saved active one is hidden or gone.
  settleActive();
  // Mission control only when both the workspace and the dock are empty.
  if (!app.layout && !tabs.length) addPane({ type: 'mission' });
  else {
    const f = saved.focused;
    const back =
      panes[f] && (inDock(f) ? app.dock.open && f === app.dock.active : inContext(f))
        ? f
        : (activeTab() ?? (app.dock.open ? app.dock.active : null));
    if (back) focusPane(back);
  }
  if (!app.data?.setup?.done) openSingleton('setup');
}

// ---------------------------------------------------------------- app_state

/**
 * `app_state`'s payload, replaced whole on each read and never edited: a class
 * instance is not turned into a deep reactive proxy, so Svelte only tracks
 * `app.data` itself instead of wrapping the whole tree each time.
 */
class Snapshot {
  constructor(data) {
    Object.assign(this, data);
  }
}

let refreshing = null;

/** One read at a time: a call made while one is in flight waits for that one. */
export function refresh() {
  refreshing ??= readState().finally(() => (refreshing = null));
  return refreshing;
}

async function readState() {
  try {
    const data = await invoke('app_state');
    data.features ??= app.data?.features ?? null;
    const prevClaude = app.data?.setup?.claude;
    app.data = new Snapshot(data);
    announceClaude(prevClaude, data.setup?.claude);
    announceWarnings(data.warnings);
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
  if (s.type === 'project') return !projects().some((p) => p.id === s.projectId);
  // Features never read yet (the core's read failed): nothing is known gone.
  if (s.type === 'feature') return app.data?.features != null && !features().some((f) => f.id === s.featureId);
  return false;
}

function prunePanes() {
  for (const [id, s] of Object.entries(app.panes)) if (paneGone(s)) closePane(id);
  // Project settings of a project that is gone: nothing left to edit.
  if (app.window?.projectId && !projects().some((p) => p.id === app.window.projectId)) closeWindow(true);
  const sel = app.selection;
  if (sel?.kind === 'agent' && !activeAgents().some((a) => a.id === sel.id)) app.selection = null;
  // The context's project was removed: All projects.
  if (app.workspace.project !== 'all' && app.data && knownContext(app.workspace.project) === 'all') setContext('all');
}

// ---------------------------------------------------------------- updates

const TOASTED_KEY = 'ui.update_toasted';
let toasted = null; // the last version toasted (persisted under TOASTED_KEY)
let toastedLoad = null;

/** The core pushed a new `update` object: keep it in `app.data`, toast a staged version once. */
export async function applyUpdate(u) {
  if (!u) return;
  if (app.data) app.data = new Snapshot({ ...app.data, update: u });
  announcePrBuild(u);
  if (u.state !== 'ready' || !u.available) return;
  // The persisted version is read once; the check-and-record below has no await between
  // them, so the startup call and an `update` event for the same version toast once.
  toastedLoad ??= invoke('settings_get', { key: TOASTED_KEY }).then(
    (v) => (toasted = v),
    () => null,
  );
  await toastedLoad;
  const text = readyToast(u, toasted);
  if (!text) return;
  toasted = u.available.version;
  invoke('settings_set', { key: TOASTED_KEY, value: toasted }).catch(() => {});
  toast(text, { ms: 12000, action: { label: 'Restart now', run: restartToUpdate } });
}

/** "PR #n's build is ready", once per staged build (persisted); the action opens Settings › Builds on that row. */
const PR_TOASTED_KEY = 'ui.pr_build_toasted';
let prToasted;
function announcePrBuild(u) {
  const i = u.pr_install;
  if (i?.state !== 'ready') return;
  const key = prKey(i);
  if (prToasted === key) return; // sync guard: a second push for the same build
  const load =
    prToasted === undefined
      ? invoke('settings_get', { key: PR_TOASTED_KEY }).catch(() => null)
      : Promise.resolve(prToasted);
  prToasted = key; // claim now so a concurrent push does not toast twice
  load.then((last) => {
    const text = prReadyToast(u, last);
    if (!text) return;
    invoke('settings_set', { key: PR_TOASTED_KEY, value: key }).catch(() => {});
    toast(text, { ms: 12000, action: { label: 'Restart to run', run: () => openSettings('builds', `pr:${i.pr}`) } });
  });
}

const announcedWarnings = new Set();
/** Start-up warnings after a restore (api-contract §6): an amber toast, 15 s, once. */
export function announceWarnings(warnings) {
  for (const w of warnings ?? []) {
    if (!isRestoreWarning(w) || announcedWarnings.has(w)) continue;
    announcedWarnings.add(w);
    toast(w, { kind: 'warn', ms: 15000 });
  }
}

/** Restart into `target`: 'main' (default, today's behaviour) or 'pr' (the staged PR build). */
export function restartToUpdate(target = 'main') {
  const t = target === 'pr' ? 'pr' : 'main';
  return invoke('update_restart', { target: t }).catch((e) => toast(errText(e), { kind: 'error' }));
}

/** Palette / Settings "Back to main". */
export function backToMainBuild() {
  return restartToUpdate('main');
}

/** Palette "Check for updates": the toast says what the check found. */
export async function checkForUpdates() {
  try {
    let u = await invoke('update_check');
    // The core settles asynchronously: follow the `update` pushes (kept in app.data) for the outcome.
    for (let n = 0; n < 120 && (u?.state === 'checking' || u?.state === 'downloading'); n++) {
      await new Promise((r) => setTimeout(r, 500));
      u = app.data?.update ?? u;
    }
    const out = checkOutcome(u);
    if (out) toast(out.text, out.error ? { kind: 'error' } : {});
  } catch (e) {
    toast(errText(e), { kind: 'error' });
  }
}

export function aboutOrchestrator() {
  toast(aboutText(app.data?.update), { ms: 10000 });
}

// ---------------------------------------------------------------- Claude Code updates

/** A toast for what a new `setup.claude` read changed: a finished or failed update of this run. */
function announceClaude(prev, next) {
  const t = claudeToast(prev, next);
  if (!t) return;
  const { details, ...opts } = t;
  toast(t.text, details ? { ...opts, action: { label: 'Details', run: () => openSettings('claude') } } : opts);
}

/** Keep a `setup.claude` the core returned in the snapshot (the Setup topic pushes follow). */
function keepClaude(c) {
  if (c && app.data) app.data = new Snapshot({ ...app.data, setup: { ...app.data.setup, claude: c } });
}

// One Claude Code command at a time: a second click or key while one is in flight is dropped.
const claudeBusy$ = $state({ on: false });
export const claudeBusy = () => claudeBusy$.on;

async function exclusive(fn) {
  if (claudeBusy$.on) return;
  claudeBusy$.on = true;
  try {
    await fn();
  } finally {
    claudeBusy$.on = false;
  }
}

/** Run `claude update`: progress comes through the Setup topic; the toast is `announceClaude`'s. */
export const claudeUpdate = () =>
  exclusive(async () => {
    try {
      await invoke('claude_update_run');
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
    const before = app.data?.setup?.claude;
    await refresh();
    // A retry that failed before any push showed `updating` leaves the same error state: say so anyway.
    const u = app.data?.setup?.claude?.update;
    const was = before?.update;
    if (u?.state === 'error' && u.error_from === 'update' && was?.state === 'error' && was.error_from === 'update') {
      const text = 'Claude Code update failed.';
      if (!app.toasts.some((x) => x.text === text))
        toast(text, { kind: 'warn', sticky: true, action: { label: 'Details', run: () => openSettings('claude') } });
    }
  });

/** Check for a newer Claude Code now; `notify` toasts the outcome (the palette). */
export const claudeCheck = (notify = false) =>
  exclusive(async () => {
    try {
      const c = await invoke('claude_update_check');
      // An update may have started while the check ran: its push is fresher than this reply.
      if (app.data?.setup?.claude?.update?.state !== 'updating') keepClaude(c);
      const t = notify && checkToast(c);
      if (t) toast(t.text, t.kind ? { kind: t.kind } : {});
    } catch (e) {
      toast(errText(e), { kind: 'error' });
      await refresh();
    }
  });

/** A button or key of the update block: 'update' runs `claude update`, 'check' checks. */
export const claudeAct = (id) => (id === 'update' ? claudeUpdate() : claudeCheck());

function claudeCommands() {
  const off = paletteActions(app.data?.setup?.claude);
  return [
    { id: 'claude.check', label: 'Check for Claude Code updates', disabled: off.check, run: () => claudeCheck(true) },
    { id: 'claude.update', label: 'Update Claude Code', disabled: off.update, run: claudeUpdate },
  ];
}

function updateCommands() {
  const u = app.data?.update;
  const off = updateActions(u);
  const back = backToMain(u);
  return [
    { id: 'update.check', label: 'Check for updates', disabled: off.check, run: checkForUpdates },
    { id: 'update.restart', label: 'Restart to update', disabled: off.restart, run: restartToUpdate },
    { id: 'update.pr', label: 'Install a PR build…', run: () => openSettings('builds') },
    { id: 'update.main', label: 'Back to main', disabled: back.why, run: backToMainBuild },
    { id: 'update.about', label: 'About Orchestrator', run: aboutOrchestrator },
  ];
}

// ---------------------------------------------------------------- commands

/** The projects "Plan with the Overwatch…" and "Refresh Ready to start" take:
 *  the context's project, else all (design-notes §5.1 rule 7). */
function planProjects() {
  const ctx = contextProjectId();
  return ctx ? [ctx] : projects().map((p) => p.id);
}

/** The palette's hint while the Overwatch cannot answer (notes §8.5). */
function planHint() {
  const s = app.now?.overwatch?.status;
  return s === 'off' ? 'off: your message waits' : s === 'down' ? 'down: your message waits' : undefined;
}

/**
 * Every action is in the palette with its shortcut (§9.1 principle 1).
 * `scope`: 'agent' commands act on the focused/selected agent and are listed first.
 */
export function commands() {
  const a = contextAgent();
  /** @type {{ id: string, label: string, run: (...args: any[]) => any, detail?: string, disabled?: any, scope?: string, hint?: string }[]} */
  const list = [
    { id: 'session.new', label: 'New session…', run: () => (app.overlay = { kind: 'new-session' }) },
    { id: 'project.add', label: 'Add project…', run: () => (app.overlay = { kind: 'add-project' }) },
    { id: 'mission', label: 'Open Mission control', run: openMission },
    // The context Select is in the workspace's strip: not drawn while the dock fills the column.
    ...(dockFills()
      ? []
      : [{ id: 'mission.filter', label: 'Set the project context…', run: () => (app.contextOpen = true) }]),
    { id: 'decisions', label: 'Show decisions', run: () => focusCard() },
    {
      id: 'intake.plan',
      label: 'Plan with the Overwatch…',
      detail: planHint(),
      run: () => planWithOverwatchIn(planProjects()),
    },
    {
      id: 'intake.refresh',
      label: 'Refresh Ready to start',
      run: () => refreshIntake(planProjects()),
    },
    ...(contextProject()
      ? [
          {
            id: 'intake.setup',
            label: `Set up intake for ${contextProject().name}`,
            run: () => setUpIntake(contextProject().id),
          },
        ]
      : []),
    { id: 'overwatch.ask', label: 'Ask the Overwatch', run: askOverwatch },
    { id: 'decisions.search', label: 'Search decisions', run: searchDecisions },
    ...(app.now?.overwatch?.status === 'down'
      ? [
          {
            id: 'overwatch.restart',
            label: 'Restart the Overwatch',
            run: () => restartOverwatch().catch((e) => toast(`Could not restart: ${errText(e)}`, { kind: 'error' })),
          },
        ]
      : []),
    ...(canOpenSession(overwatchOf(app.now))
      ? [{ id: 'overwatch.session', label: "Open the Overwatch's session", run: openOverwatch }]
      : []),
    { id: 'overwatch.settings', label: 'Turn the Overwatch on or off', run: () => openSettings('overwatch') },
    ...FILTERS.map((f) => ({
      id: `decisions.filter.${f.id}`,
      label: `Filter decisions: ${f.label}`,
      run: () => filterDecisions(f.id),
    })),
    {
      id: 'decisions.clear-digest',
      label: "Clear today's digest",
      run: () => clearDigest().catch((e) => toast(errText(e), { kind: 'error' })),
    },
    { id: 'planner', label: 'Open Week planner', run: openPlanner },
    { id: 'shortcuts', label: 'Keyboard shortcuts', run: openShortcuts },
    { id: 'setup', label: 'Set up Orchestrator (first run)…', run: () => openSingleton('setup') },
    ...updateCommands(),
    ...claudeCommands(),
    { id: 'settings', label: 'Settings', run: () => openSettings() },
    { id: 'settings.integrations', label: 'Settings: Integrations', run: () => openSettings('integrations') },
    { id: 'settings.mcp', label: 'Settings: MCP servers', run: () => openSettings('mcp') },
    ...(contextProject()
      ? [
          {
            id: 'settings.mcp.project',
            label: `Settings: MCP servers for ${contextProject().name}`,
            run: () => openProjectSettings(contextProject().id, 'mcp'),
          },
        ]
      : []),
    { id: 'plan.edit', label: 'Edit the weekly plan…', run: () => (app.overlay = { kind: 'plan' }) },
    { id: 'report.current', label: 'Weekly report (this week so far)', run: () => openReport('current') },
    { id: 'report.list', label: 'Past weekly reports', run: () => openReport('list') },
    { id: 'team.canvas', label: 'Open the Team canvas', run: () => openCanvas() },
    {
      id: 'team.launch',
      label: 'Launch a team on a feature…',
      run: () => (app.overlay = { kind: 'launch', projectId: contextProject()?.id ?? null }),
    },
    // Work browser (notes §1.2, §12.7): leader ⌘; b opens it on the context project (else all).
    { id: 'work.browser', label: 'Work browser', run: () => openWork({ projects: workContext() }) },
    { id: 'work.open', label: 'Open Work (all projects)', run: () => openWork({ projects: null }) },
    { id: 'work.find', label: 'Find work item…', run: () => openWork({ projects: workContext(), find: true }) },
    {
      id: 'work.refresh',
      label: 'Refresh Work',
      run: () =>
        Promise.all((workContext() ?? [null]).map((project) => invoke('work_refresh', { project }))).catch((e) =>
          toast(`Could not refresh: ${errText(e)}`, { kind: 'error' }),
        ),
    },
    { id: 'shell.new', label: 'New shell (dock)', run: () => openShell() },
    { id: 'dock.toggle', label: 'Show or hide the dock', run: () => toggleDock() },
    { id: 'dock.next', label: 'Next dock tab', run: () => cycleDock(1) },
    { id: 'dock.prev', label: 'Previous dock tab', run: () => cycleDock(-1) },
    { id: 'editor.open', label: 'Open editor (nvim)', run: () => openEditor() },
    { id: 'pane.close', label: 'Close tab', run: () => closePaneByUser() },
    { id: 'pane.reopen', label: 'Reopen closed tab', run: reopenClosed },
    { id: 'sidebar', label: 'Toggle sidebar', run: () => ((app.sidebar = !app.sidebar), saveLayout()) },
    {
      id: 'inspector',
      label: 'Toggle inspector',
      run: () => {
        app.inspector = !app.inspector;
        saveLayout();
      },
    },
    { id: 'agent.next-waiting', label: 'Jump to next waiting agent', run: nextWaiting },
    {
      id: 'governor.pause-all',
      label: 'Pause all autonomous work',
      run: pauseAll,
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
      { id: 'agent.kill', label: `Kill ${a.id}…`, run: () => confirmKill(a.id) },
      { id: 'agent.restart-fresh', label: `Restart ${a.id} fresh…`, run: () => confirmRestartFresh(a.id) },
      { id: 'agent.delete', label: `Delete ${a.id}…`, run: () => askDeleteAgent(a.id) },
      { id: 'agent.shell', label: `Shell in ${a.id}'s worktree`, run: () => openShell({ agentId: a.id }) },
      {
        id: 'agent.diff',
        label: `Diff of ${a.id} against its base`,
        run: () => openDiff(a.project_id, a.worktree, a.branch),
      },
      { id: 'agent.kitty', label: `Open ${a.id}'s worktree in kitty`, run: () => openInKitty(a.worktree) },
      a.queue_entry && {
        id: 'agent.off-hours',
        label:
          a.queue_entry.reason?.kind === 'scheduled_off_hours'
            ? `Run ${a.id} in working hours`
            : `Run ${a.id} off-hours`,
        run: () => runOffHours(a),
      },
    ].filter(Boolean);
    for (const c of agentCmds) c.scope = 'agent';
    list.unshift(...agentCmds);
  }
  for (const f of features()) {
    list.push({ id: `feature.open.${f.id}`, label: `Open feature ${f.slug}`, run: () => openFeature(f.id) });
    list.push({ id: `feature.notion.${f.id}`, label: `Publish ${f.slug} to Notion`, run: () => publishNotion(f) });
    list.push({
      id: `feature.delete.${f.id}`,
      label: `Delete feature ${f.slug}…`,
      run: () => (app.overlay = { kind: 'delete-feature', featureId: f.id }),
    });
  }
  // Ready parents (notes §12.7): "Run as program: #12 Checkout v2".
  for (const it of app.now.intake?.items ?? []) {
    if (!it.node || it.actions?.program !== 'ok' || it.hidden) continue;
    const name = `${it.ref ? `${it.ref} ` : ''}${it.title ?? ''}`.trim();
    list.push({
      id: `program.plan.${it.node}`,
      label: `Run as program: ${name}`,
      run: () => openProgramPlan([it.node], { title: name }),
    });
  }
  // Programs in progress (notes §12.7): "Program: Checkout v2", "Pause program Checkout v2".
  for (const p of Object.values(app.now.programs ?? {})) {
    list.push({ id: `program.open.${p.id}`, label: `Program: ${p.title}`, run: () => openProgram(p.id, p.title) });
    list.push({
      id: `program.pause.${p.id}`,
      label: `${p.status === 'paused' ? 'Resume' : 'Pause'} program ${p.title}`,
      run: () => togglePause(p),
    });
  }
  for (const p of projects()) {
    list.push({ id: `work.open.${p.id}`, label: `Work: ${p.name}`, run: () => openWork({ projects: [p.id] }) });
    list.push({ id: `project.open.${p.id}`, label: `Open project ${p.name}`, run: () => openProject(p.id) });
    list.push({ id: `project.settings.${p.id}`, label: `${p.name} settings`, run: () => openProjectSettings(p.id) });
    list.push({
      id: `project.trust.${p.id}`,
      label: `Trust ${p.name} (Claude Code's dialog)…`,
      run: () => openTrust(p.id),
    });
  }
  for (const c of list) c.hint = hint(c.id);
  return list;
}

/** The project Work opens on from the leader / palette: the focused pane's, else the context's, else all. */
function workContext() {
  // A Work tab in focus keeps its own scope (several projects, or all).
  const s = focusedSpec();
  if (s?.type === 'work') return s.projects ?? null;
  const id = paneProject(s) ?? contextProjectId();
  return id ? [id] : null;
}

export function run(commandId) {
  if (commandId.startsWith('pane.focus.')) return focusNumber(Number(commandId.slice(11)));
  const map = {
    palette: () => (app.overlay = app.overlay?.kind === 'palette' ? null : { kind: 'palette' }),
    jump: () => (app.overlay = app.overlay?.kind === 'jump' ? null : { kind: 'jump' }),
    'focus.up': () => focusDirection('up'),
    'focus.down': () => focusDirection('down'),
    'agent.pause': () => {
      // FR-805/811: the Governor holds this agent at its next tool boundary; again resumes it.
      const a = contextAgent();
      if (!a) return toast('No agent in focus');
      agentAction(a.id, a.paused ? 'unpause' : 'pause');
    },
    'window.close': () => {},
    // ⌘, and ⌘/ (and their menu items) toggle their window; ⌘W closes the window before a tab.
    settings: () => toggleWindow('settings'),
    shortcuts: () => toggleWindow('shortcuts'),
    'pane.close': () => (app.window ? closeWindow() : closePaneByUser()),
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
  app.themeRev++;
  return app.theme ? 'user' : derived ? 'kitty' : 'system';
}

// Following the system, the scheme can change under the app.
if (typeof window !== 'undefined')
  window.matchMedia?.('(prefers-color-scheme: dark)').addEventListener?.('change', () => {
    if (!app.theme) app.themeRev++;
  });

/** The notification settings: desktop and sound, both on unless stored off. */
export async function getNotify() {
  const cur = await invoke('settings_get', { key: 'notifications' }).catch(() => null);
  return { desktop: true, sound: true, ...(cur ?? {}) };
}

// Writes are read-modify-write on one object: queued, so two quick toggles don't undo each other.
let notifyQueue = Promise.resolve();

/** Stores `key` on or off (`on` may be a function of the current settings) and returns the new
 *  settings; throws when the store refuses. */
export function setNotify(key, on) {
  const run = notifyQueue.then(async () => {
    const cur = await getNotify();
    const next = { ...cur, [key]: typeof on === 'function' ? on(cur) : on };
    await invoke('settings_set', { key: 'notifications', value: next });
    return next;
  });
  notifyQueue = run.catch(() => {});
  return run;
}

/** Desktop notifications and their sound (both on by default), read by the core's notifier. */
export async function toggleNotify(key) {
  const next = await setNotify(key, (cur) => !cur[key]).catch(() => null);
  if (next) toast(`${key === 'desktop' ? 'Desktop notifications' : 'Notification sound'} ${next[key] ? 'on' : 'off'}`);
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
  // Agents that need you first, longest waiting first; then the rest as listed.
  const first = new Map(needsYou().map((r, i) => [r.agent, i]));
  const rank = (a) => first.get(a.id) ?? first.size;
  for (const a of [...agents()].sort((x, y) => rank(x) - rank(y))) {
    const p = projects().find((x) => x.id === a.project_id);
    out.push({
      label: `${p?.name ?? ''} › ${a.id}`,
      detail: STATUS[statusOf(a.id)].label,
      run: () => openAgent(a.id),
    });
  }
  for (const [id, s] of Object.entries(app.panes))
    out.push({ label: `pane: ${s.title}`, detail: s.type, run: () => focusPane(id) });
  return out;
}

// ---------------------------------------------------------------- decisions

/** Records a decision; it reaches the core when its 5 s undo window ends (FR-707 `u`).
 *  Its toast says what will be sent; the effect is said when the core pushes it. */
export function decide(d, action, message = null) {
  if (app.pending.some((p) => p.item === d.id)) return;
  // Always allow writes a rule: it asks once (Cancel · Always allow), then goes at once, with no undo window.
  if (action === 'always') {
    const project = projectName(d.subject?.project) ?? 'the project';
    return confirm(
      `Always allow "${d.payload?.always ?? 'this'}"?`,
      `Adds a rule to ${project}'s policy file. Undo in Settings › Permissions.`,
      () => {
        if (app.pending.some((p) => p.item === d.id)) return;
        app.pending = [
          ...app.pending,
          { item: d.id, action, message, at: Date.now() - UNDO_MS, toast: null, project: d.subject?.project ?? null },
        ];
        flushDecisions();
      },
      'Always allow',
    );
  }
  const t = toast(pendingToast(d, action), { ms: UNDO_MS, undo: true, item: d.id });
  app.pending = [
    ...app.pending,
    { item: d.id, action, message, at: Date.now(), toast: t, project: d.subject?.project ?? null },
  ];
}

export function undoLast() {
  const last = app.pending.filter((p) => !p.sent).at(-1);
  if (!last) return false;
  app.pending = app.pending.filter((x) => x !== last);
  dismissToast(last.toast);
  toast('Undone');
  return true;
}

/** Drops an item's decision still in its undo window (one already sent stays). */
export function undoItem(itemId) {
  const p = app.pending.find((x) => x.item === itemId && !x.sent);
  if (!p) return;
  app.pending = app.pending.filter((x) => x !== p);
  dismissToast(p.toast);
}

// What this window sent, by decision id, until its effect is pushed: the
// effect's toast names the action ("Always allowed · <rule>", "reason sent").
const mine = new Map();

/** Sends the decisions whose undo window is over. Called by the app's clock,
 *  which may tick again while a slow act (a restart, a kill) is in flight: the
 *  batch is marked sent at once so no tick sends it twice, and its cards stay
 *  hidden until the core answered with their lifecycle. No toast here: the
 *  effect comes with the `now` push (notes §4), never before `sent`. */
export async function flushDecisions(now = Date.now()) {
  const batch = due(app.pending, now);
  if (!batch.length) return;
  for (const p of batch) {
    p.sent = true;
    mine.set(p.item, { action: p.action, message: p.message });
  }
  // The decision as sent: an answer with no version is newer only than that card.
  const sent = batch.map((p) => app.now.decisions[p.item]);
  const acts = await Promise.allSettled(
    batch.map((p) => invoke('inbox_act', { item: p.item, action: p.action, message: p.message })),
  );
  batch.forEach((p, i) => {
    const r = acts[i];
    if (r.status === 'rejected') {
      mine.delete(p.item);
      toast(errText(r.reason), { kind: 'error' });
      return;
    }
    // The answer's lifecycle counts as a push unless older than what we hold: a
    // synchronous effect (Allow once on a held hook, Dismiss, Delete) toasts now.
    // A versionless answer (synthesized cards; until inbox_act returns pushed
    // versions) only holds while no push replaced the card during the await.
    const d = app.now.decisions[p.item];
    const pushedMeanwhile = d !== sent[i];
    let lc = r.value?.lifecycle;
    if (lc && !lc.version) lc = pushedMeanwhile ? null : { ...lc, version: version(d) };
    if (d && lc && !older(lc, d)) {
      const next = { ...d, lifecycle: lc };
      app.now.decisions[p.item] = next;
      decisionPushed(d, next);
    }
    routeOpen(r.value, p.project);
  });
  app.pending = app.pending.filter((x) => !batch.includes(x));
}

// Lifecycle states after which a decision is no longer waiting on its answer.
const SETTLED = new Set(['effect', 'failed', 'dismissed', 'stale', 'interrupted']);

/**
 * A decision's lifecycle push (`now.decisions`): the effect's toast (notes §4),
 * once, when the state moves to effect, dismissed, failed or parked. A decision taken
 * elsewhere (the pane, a rule, the hold's expiry) drops ours still in its undo
 * window; a parked one stays open, so a pending answer to it still applies.
 */
export function decisionPushed(prev, d) {
  const state = d.lifecycle?.state;
  if (state === prev?.lifecycle?.state) return;
  if (!isOpen(d)) undoItem(d.id);
  const text = decisionToast(d, mine.get(d.id));
  if (SETTLED.has(state)) mine.delete(d.id);
  if (!text) return;
  if (state === 'failed')
    toast(text, { kind: 'error', sticky: true, action: { label: 'Show card', run: () => focusCard(d.id) } });
  else if (d.lifecycle.effect === 'pr_opened' && d.lifecycle.effect_detail)
    toast(text, { action: { label: 'Open PR', run: () => openUrl(d.lifecycle.effect_detail) } });
  else toast(text);
}

/** A decision left the open list (its final upsert came first, T-35): ours
 *  still in its undo window would only fail, and what we sent is forgotten. */
export function decisionGone(id) {
  undoItem(id);
  mine.delete(id);
}

/** FR-415: the agent again as a fresh session with its brief (agent menu). */
export async function restartFresh(id) {
  try {
    await invoke('restart_fresh', { agent: id });
    toast(`Restarted fresh · ${id}`);
    await refresh();
  } catch (e) {
    toast(`${id}: ${errText(e)}`, { kind: 'error' });
  }
}
