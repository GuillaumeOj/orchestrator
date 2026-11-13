// One Work tab's state (improve-intake notes §2–§5, §11; api-contract §2, §6): the tree loaded from
// `work_tree`, search, details cached by id + updated_at_ms, view prefs per scope, selection and the
// calls behind each key. The pane renders it; rules and copy live in workmodel.js.

import { UNDO_MS } from './inbox.js';
import { staleOf, staleToast } from './intake.js';
import { openPrefilled } from './intake.svelte.js';
import { errText, invoke } from './ipc.js';
import { readySummary, repoOfRemote, trackerName as trackerWords } from './tracker.js';
import {
  ancestors,
  cleanFilter,
  defaultFilter,
  prefsKey,
  prefsOf,
  readPrefs,
  statusLine,
  TRACKER_NAME,
  toggleSelect,
  trackerName,
  trackerOf,
} from './workmodel.js';

export const SEARCH_MS = 250;
export const DETAILS_MS = 300;
const SAVE_MS = 500;

/** The Ready filter in a few words for the chip's tooltip ("label ready"). */
function readyWords(ready) {
  if (!ready) return '';
  if (ready.kind === 'github' && ready.labels?.length) return `label ${ready.labels.join(', ')}`;
  return readySummary(ready).replace(/\.$/, '');
}

/**
 * @typedef {{ id: string, name: string, kind: string | null, label: string, tracker: any, ready: string,
 *             refresh_secs: number, settings: any, remote: string | null }} ScopeProject
 */

export class WorkView {
  /** @type {string[]} the scope's project ids */
  ids = [];
  /** @type {ScopeProject[]} */
  scope = $state([]);
  nodes = $state.raw(new Map());
  kids = $state.raw(new Map());
  more = $state.raw(/** @type {Record<string, { left: number, cursor?: any }>} */ ({}));
  status = $state.raw(/** @type {Record<string, any>} */ ({}));
  loading = $state.raw(new Set());
  errors = $state.raw(new Map());
  folds = $state.raw(new Set());
  tasks = $state.raw(new Map());
  expanded = $state.raw(new Set());
  collapsedProjects = $state.raw(new Set());
  filter = $state(defaultFilter());
  focused = $state(/** @type {string | null} */ (null));
  detailsOpen = $state(true);
  focusRoot = $state(/** @type {string | null} */ (null));
  selection = $state(/** @type {{ parent: string, ids: string[] } | null} */ (null));
  query = $state('');
  results = $state.raw(/** @type {any} */ (null));
  searchBusy = $state(false);
  loaded = $state(false);
  failed = $state(/** @type {string | null} */ (null));
  details = $state.raw(new Map());
  lastHide = $state(/** @type {{ node: any, until: number } | null} */ (null));
  refreshing = $state.raw(new Set());

  #seq = 0;
  #searchSeq = 0;
  #saveTimer = null;
  #searchTimer = null;
  #disposed = false;

  /** @param {{ ids: string[], projects: any[] }} o */
  constructor({ ids, projects }) {
    this.ids = ids;
    this.scope = ids.map((id) => {
      const p = projects.find((x) => x.id === id);
      const origin = (p?.remotes ?? []).find((x) => x.name === 'origin') ?? p?.remotes?.[0];
      return {
        id,
        name: p?.name ?? id,
        kind: 'github',
        label: '',
        tracker: null,
        ready: '',
        refresh_secs: 600,
        settings: null,
        remote: origin?.url ?? null,
      };
    });
  }

  get multi() {
    return this.ids.length > 1;
  }

  /** The `work_tree` / `work_search` scope argument. */
  get scopeArg() {
    return { projects: this.ids };
  }

  dispose() {
    this.#disposed = true;
    clearTimeout(this.#saveTimer);
    clearTimeout(this.#searchTimer);
  }

  // ---------------------------------------------------------------- open

  /** First open: prefs, tracker words, the roots, the saved expanded nodes, then a refresh of stale caches. */
  async init() {
    const [saved] = await Promise.all([
      invoke('settings_get', { key: prefsKey(this.ids) }).catch(() => null),
      this.loadTrackers(),
    ]);
    const p = readPrefs(saved);
    this.filter = p.filter;
    this.expanded = new Set(p.expanded);
    this.collapsedProjects = new Set(p.collapsedProjects);
    this.focused = p.focused;
    this.detailsOpen = p.details_open;
    this.focusRoot = p.focus_root;
    await this.reload();
    this.refreshStale();
  }

  /** Each project's tracker in words (project row, empty lines, Ready tooltip). */
  async loadTrackers() {
    const list = await Promise.all(
      this.scope.map(async (p) => {
        const r = await invoke('tracker_settings', { project: p.id }).catch(() => null);
        const t = r?.settings?.tracker ?? null;
        if (!t) return p;
        const known = t.kind === 'github' ? repoOfRemote(p.remote ?? '') : null;
        return {
          ...p,
          kind: t.kind,
          tracker: t,
          settings: r.settings,
          label:
            t.kind === 'local'
              ? String(t.backlog_glob ?? '.orchestrator/backlog/*.md').replace(/\/[^/]*$/, '')
              : trackerWords(t, known).replace(/^Linear /, ''),
          ready: readyWords(r.settings.ready),
          refresh_secs: r.settings.refresh_secs ?? 600,
        };
      }),
    );
    if (!this.#disposed) this.scope = list;
  }

  /** "GitHub acme/shop" for the project row. */
  trackerLine(p) {
    return `${TRACKER_NAME[p.kind] ?? ''} ${p.label}`.trim();
  }

  /** Opening refreshes only the projects whose cache is older than their refresh interval (F38). */
  refreshStale(now = Date.now()) {
    for (const p of this.scope) {
      const s = this.status[p.id];
      if (!s || s.status === 'refreshing') continue;
      const age = now - (s.refreshed_at_ms ?? 0);
      if (!s.refreshed_at_ms || age > p.refresh_secs * 1000) this.refresh([p.id], { quiet: true });
    }
  }

  // ---------------------------------------------------------------- the tree

  #treeArgs(parents, after = null) {
    return { scope: this.scopeArg, parents, filter: $state.snapshot(this.filter), after };
  }

  /** Merges nodes; `lists`: parent key → the ids it lists (replaced, or appended with `append`). */
  #merge(nodes, lists, append = false) {
    const map = new Map(this.nodes);
    for (const n of nodes) map.set(n.id, n);
    const kids = new Map(this.kids);
    for (const [k, ids] of lists) {
      const prev = append ? (kids.get(k) ?? []) : [];
      kids.set(k, [...prev, ...ids.filter((id) => !prev.includes(id))]);
    }
    this.nodes = map;
    this.kids = kids;
  }

  /** Groups a page of nodes by parent: roots under `roots`, the rest under their parent id. */
  #lists(nodes, parents) {
    const lists = new Map();
    for (const k of parents ?? ['roots']) lists.set(k, []);
    for (const n of nodes) {
      const k = n.parent ?? 'roots';
      if (!lists.has(k)) continue;
      lists.get(k).push(n.id);
    }
    return lists;
  }

  /** The roots, then every expanded parent the tree still shows, in one call (F37). */
  async reload() {
    const seq = ++this.#seq;
    try {
      const r = await invoke('work_tree', this.#treeArgs(null));
      if (seq !== this.#seq || this.#disposed) return;
      this.failed = null;
      // Lists of collapsed parents are dropped (re-read on expand); open ones stay until their reload.
      this.kids = new Map([...this.kids].filter(([k]) => this.expanded.has(k)));
      this.#merge(r?.nodes ?? [], this.#lists(r?.nodes ?? [], ['roots']));
      this.more = { ...(r?.more ?? {}) };
      this.status = { ...(r?.status ?? {}) };
      if (this.focusRoot && !this.nodes.has(this.focusRoot)) await this.#loadInto([this.focusRoot]);
      // Every saved expanded id in one call (F37): the cache answers each one, nested ones included.
      const open = [...new Set([...this.expanded, ...(this.focusRoot ? [this.focusRoot] : [])])];
      if (open.length) await this.#loadChildren(open, seq);
    } catch (e) {
      if (seq === this.#seq) this.failed = errText(e);
    } finally {
      if (seq === this.#seq) this.loaded = true;
    }
  }

  /** Nodes the tree has not loaded (a saved focus root): their chain from `work_reveal`. */
  async #loadInto(ids) {
    for (const id of ids) {
      const r = await invoke('work_reveal', { id }).catch(() => null);
      if (r) this.#mergeReveal(r);
    }
  }

  async #loadChildren(parents, seq = this.#seq) {
    this.loading = new Set([...this.loading, ...parents]);
    const errors = new Map(this.errors);
    for (const p of parents) errors.delete(p);
    this.errors = errors;
    try {
      const r = await invoke('work_tree', this.#treeArgs(parents));
      if (seq !== this.#seq || this.#disposed) return;
      this.#merge(r?.nodes ?? [], this.#lists(r?.nodes ?? [], parents));
      this.more = { ...this.more, ...(r?.more ?? {}) };
      if (r?.status) this.status = { ...this.status, ...r.status };
    } catch (e) {
      const errs = new Map(this.errors);
      for (const p of parents) errs.set(p, errText(e));
      this.errors = errs;
    } finally {
      const l = new Set(this.loading);
      for (const p of parents) l.delete(p);
      this.loading = l;
    }
  }

  /** Expand / collapse a node (its children load once). */
  async toggle(id, open = !this.expanded.has(id)) {
    const s = new Set(this.expanded);
    if (open) s.add(id);
    else s.delete(id);
    this.expanded = s;
    this.save();
    if (open && !this.kids.has(id)) await this.#loadChildren([id]);
  }

  /** Expand / collapse a project row (several projects in scope). */
  toggleProject(id, open = this.collapsedProjects.has(id)) {
    const s = new Set(this.collapsedProjects);
    if (open) s.delete(id);
    else s.add(id);
    this.collapsedProjects = s;
    this.save();
  }

  /** Retry a failed child load. */
  retry(key) {
    if (key.startsWith('fold:')) return this.toggleFold(key.slice(5), true);
    return this.#loadChildren([key]);
  }

  /** "Load 50 more" / "Load 100 more" (F18): continues that list from its cursor. */
  async loadMore(key) {
    const m = this.more[key];
    if (!m?.cursor) return;
    this.loading = new Set([...this.loading, `more:${key}`]);
    try {
      const r = await invoke('work_tree', this.#treeArgs(key === 'roots' ? null : [key], m.cursor));
      this.#merge(r?.nodes ?? [], this.#lists(r?.nodes ?? [], [key]), true);
      const more = { ...this.more };
      if (r?.more?.[key]) more[key] = r.more[key];
      else delete more[key];
      this.more = more;
    } catch (e) {
      this.toast?.(`Could not load more: ${errText(e)}`, { kind: 'error' });
    } finally {
      const l = new Set(this.loading);
      l.delete(`more:${key}`);
      this.loading = l;
    }
  }

  /** Expand all to level 2: every project row, and the roots with children. */
  async expandLevel2() {
    this.collapsedProjects = new Set();
    const roots = (this.kids.get('roots') ?? []).filter((id) => (this.nodes.get(id)?.children?.total ?? 0) > 0);
    this.expanded = new Set([...this.expanded, ...roots]);
    this.save();
    const missing = roots.filter((id) => !this.kids.has(id));
    if (missing.length) await this.#loadChildren(missing);
  }

  collapseAll() {
    this.expanded = new Set();
    this.collapsedProjects = this.multi ? new Set(this.ids) : new Set();
    this.save();
  }

  /** The Team tasks fold (§2.7): its lines come from the parent's children with team tasks included. */
  async toggleFold(id, open = !this.folds.has(id)) {
    const s = new Set(this.folds);
    if (open) s.add(id);
    else s.delete(id);
    this.folds = s;
    if (!open || this.tasks.has(id)) return;
    const key = `fold:${id}`;
    this.loading = new Set([...this.loading, key]);
    try {
      const r = await invoke('work_tree', {
        scope: this.scopeArg,
        parents: [id],
        filter: { ...$state.snapshot(this.filter), include_team_tasks: true, state: 'all' },
        after: null,
      });
      const tasks = (r?.nodes ?? []).filter((n) => n.parent === id && n.team_task);
      this.tasks = new Map(this.tasks).set(id, tasks);
    } catch (e) {
      this.errors = new Map(this.errors).set(key, errText(e));
    } finally {
      const l = new Set(this.loading);
      l.delete(key);
      this.loading = l;
    }
  }

  // ---------------------------------------------------------------- refresh, events

  /** `r`: one refresh per project (the core joins one in flight, F38); work.changed brings the result. */
  async refresh(ids = this.ids, { quiet = false } = {}) {
    this.refreshing = new Set([...this.refreshing, ...ids]);
    const st = { ...this.status };
    for (const id of ids) st[id] = { ...(st[id] ?? {}), status: 'refreshing' };
    this.status = st;
    try {
      await Promise.all(ids.map((project) => invoke('work_refresh', { project })));
    } catch (e) {
      if (!quiet) this.toast?.(`Could not refresh: ${errText(e)}`, { kind: 'error' });
    }
  }

  /** `work.changed {project}` from the core: reload when it is in scope. */
  onChanged(project) {
    if (project && !this.ids.includes(project)) return;
    const r = new Set(this.refreshing);
    if (project) r.delete(project);
    else r.clear();
    this.refreshing = r;
    this.details = new Map();
    this.tasks = new Map();
    this.reload();
    if (this.query.trim()) this.runSearch();
  }

  // ---------------------------------------------------------------- filters, prefs

  /** Applies a filter change live: the tree reloads (and the results, while searching). */
  setFilter(patch) {
    this.filter = cleanFilter({ ...$state.snapshot(this.filter), ...patch });
    this.selection = null;
    this.save();
    this.reload();
    if (this.query.trim()) this.runSearch();
  }

  resetFilters() {
    this.filter = { ...defaultFilter(), show_hidden: this.filter.show_hidden };
    this.save();
    this.reload();
    if (this.query.trim()) this.runSearch();
  }

  /** View prefs per scope (F4), debounced. */
  save() {
    clearTimeout(this.#saveTimer);
    this.#saveTimer = setTimeout(() => {
      if (this.#disposed) return;
      const value = prefsOf({
        filter: $state.snapshot(this.filter),
        expanded: this.expanded,
        collapsedProjects: this.collapsedProjects,
        focused: this.focused,
        details_open: this.detailsOpen,
        focus_root: this.focusRoot,
      });
      invoke('settings_set', { key: prefsKey(this.ids), value }).catch(() => {});
    }, SAVE_MS);
  }

  // ---------------------------------------------------------------- search (notes §4.1)

  setQuery(q) {
    this.query = q;
    clearTimeout(this.#searchTimer);
    if (!q.trim()) {
      this.#searchSeq++;
      this.results = null;
      this.searchBusy = false;
      return;
    }
    this.searchBusy = true;
    this.#searchTimer = setTimeout(() => this.runSearch(), SEARCH_MS);
  }

  async runSearch() {
    const q = this.query.trim();
    if (!q) return;
    const seq = ++this.#searchSeq;
    const filter = $state.snapshot(this.filter);
    const scope = this.focusRoot ? { ...this.scopeArg, root: this.focusRoot } : this.scopeArg;
    try {
      const r = await invoke('work_search', { scope, query: q, filter });
      if (seq !== this.#searchSeq || this.#disposed) return;
      this.results = {
        nodes: r?.nodes ?? [],
        total: r?.total ?? r?.nodes?.length ?? 0,
        hidden: r?.hidden_by_filters ?? 0,
      };
    } catch (e) {
      if (seq === this.#searchSeq) this.results = { nodes: [], total: 0, hidden: 0, error: errText(e) };
    } finally {
      if (seq === this.#searchSeq) this.searchBusy = false;
    }
  }

  /** "3 more hidden by filters · Show": the same query without filters (but Hide closed). */
  showHiddenResults() {
    this.filter = { ...defaultFilter(), state: 'all', show_hidden: this.filter.show_hidden };
    this.save();
    this.reload();
    this.runSearch();
  }

  // ---------------------------------------------------------------- reveal (⏎ on a result, Show in Work)

  #mergeReveal(r) {
    const nodes = r?.nodes ?? [];
    const lists = new Map();
    for (const n of nodes) {
      const k = n.parent ?? 'roots';
      if (k === 'roots' && this.kids.has('roots')) continue;
      if (!lists.has(k)) lists.set(k, []);
      lists.get(k).push(n.id);
    }
    // The roots list keeps its page: a revealed root joins it when missing.
    const roots = nodes.filter((n) => !n.parent).map((n) => n.id);
    this.#merge(nodes, lists);
    if (roots.length && this.kids.has('roots')) {
      const have = this.kids.get('roots');
      const add = roots.filter((id) => !have.includes(id));
      if (add.length) this.kids = new Map(this.kids).set('roots', [...have, ...add]);
    }
  }

  /** Reveals a node: the chain's ancestors expanded in one load (F37), search cleared, the row focused. */
  async reveal(id) {
    try {
      const r = await invoke('work_reveal', { id });
      this.#mergeReveal(r);
      const chain = r?.chain ?? [...ancestors(this.nodes, id).map((n) => n.id), id];
      this.expanded = new Set([...this.expanded, ...chain.slice(0, -1)]);
      const node = this.nodes.get(id);
      if (node && this.collapsedProjects.has(node.project)) this.toggleProject(node.project, true);
      if (this.focusRoot && !chain.includes(this.focusRoot)) this.focusRoot = null;
      this.setQuery('');
      this.focused = id;
      this.save();
      return true;
    } catch (e) {
      this.toast?.(`Could not show ${id} in Work: ${errText(e)}`, { kind: 'error' });
      return false;
    }
  }

  // ---------------------------------------------------------------- focus here (notes §2.4)

  focusHere(id) {
    this.focusRoot = id;
    this.focused = id;
    this.selection = null;
    if (!this.expanded.has(id)) this.toggle(id, true);
    this.save();
    if (this.query.trim()) this.runSearch();
  }

  /** ⇧Z: up one level (the root's parent; at the top, leaves focus mode). */
  focusUp() {
    if (!this.focusRoot) return;
    const parent = this.nodes.get(this.focusRoot)?.parent ?? null;
    if (parent && this.nodes.has(parent)) this.focusHere(parent);
    else this.leaveFocus();
  }

  leaveFocus() {
    const was = this.focusRoot;
    this.focusRoot = null;
    if (was) this.focused = was;
    this.save();
    if (this.query.trim()) this.runSearch();
  }

  // ---------------------------------------------------------------- details (notes §3, F36)

  /** The cached `work_node` of a node, or null; `load` fetches it when missing. */
  detailsOf(node) {
    return node ? (this.details.get(`${node.id}@${node.upstream?.updated_at_ms ?? 0}`) ?? null) : null;
  }

  async loadDetails(node) {
    if (!node || node.kind === 'project') return;
    const key = `${node.id}@${node.upstream?.updated_at_ms ?? 0}`;
    if (this.details.has(key)) return;
    const children = (node.children?.total ?? 0) > 0 && !this.kids.has(node.id);
    try {
      const r = await invoke('work_node', { id: node.id, opts: { children } });
      if (!this.#disposed) this.details = new Map(this.details).set(key, r ?? {});
    } catch (e) {
      if (!this.#disposed) this.details = new Map(this.details).set(key, { error: errText(e) });
    }
  }

  // ---------------------------------------------------------------- selection (notes §5.4)

  /** `x`: toggles a node in the selection; a refusal is a toast (non-sibling, closed). */
  select(node) {
    const r = toggleSelect(this.selection, node);
    if (r.refused) {
      this.toast?.(r.refused);
      return false;
    }
    this.selection = r.sel ?? null;
    return true;
  }

  /** ⇧J / ⇧K: adds `node` to the selection when it can (a sibling, selectable). */
  extend(node) {
    if (!node || this.selection?.ids.includes(node.id)) return;
    const r = toggleSelect(this.selection, node);
    if (r.sel) this.selection = r.sel;
  }

  clearSelection() {
    this.selection = null;
  }

  // ---------------------------------------------------------------- actions (notes §5)

  /** Start team run / Start solo (t / a): the prefilled dialog through `work_prefill`, never a start. */
  async start(node, mode) {
    try {
      const r = await invoke('work_prefill', { id: node.id, mode });
      const tracker = trackerOf(node) ?? 'local';
      openPrefilled(r, {
        intake: { source: node.id },
        origin: {
          kind: 'intake',
          item: node.id,
          source: tracker,
          ref: node.ref,
          url: node.url ?? null,
          breadcrumb: r?.origin?.breadcrumb ?? null,
        },
        item: { ...node, ...(r?.item ?? {}), estimate: r?.estimate ?? node.estimate },
      });
    } catch (e) {
      const stale = staleOf(e);
      this.toast?.(stale ? staleToast(stale, node.ref) : `Could not open the dialog: ${errText(e)}`, { kind: 'error' });
    }
  }

  /** ⌫ Hide from Ready: P4.5 §2.2 toast with Undo (5 s), tracker named (notes §5.3). */
  async hide(node) {
    try {
      await invoke('work_hide', { id: node.id });
    } catch (e) {
      return this.toast?.(`Could not hide ${node.ref}: ${errText(e)}`, { kind: 'error' });
    }
    this.#patch(node.id, { hidden: true });
    const h = { node, until: Date.now() + UNDO_MS };
    this.lastHide = h;
    this.toast?.(`${node.ref} hidden from Ready. It comes back if it changes in ${trackerName(node)}.`, {
      ms: UNDO_MS,
      action: { label: 'Undo u', run: () => this.undoHide(h) },
    });
  }

  async unhide(node, { quiet = false } = {}) {
    try {
      await invoke('work_unhide', { id: node.id });
      this.#patch(node.id, { hidden: false });
      if (!quiet) this.toast?.(`${node.ref} is back in Ready to start.`);
    } catch (e) {
      this.toast?.(`Could not unhide ${node.ref}: ${errText(e)}`, { kind: 'error' });
    }
  }

  /** `u` (5 s) or the toast's Undo. */
  async undoHide(h = this.lastHide) {
    if (!h || h.until < Date.now()) {
      if (h && this.lastHide?.until === h.until) this.lastHide = null;
      return false;
    }
    if (this.lastHide?.until === h.until) this.lastHide = null;
    await this.unhide(h.node);
    return true;
  }

  #patch(id, patch) {
    const n = this.nodes.get(id);
    if (n) this.nodes = new Map(this.nodes).set(id, { ...n, ...patch });
    if (this.results)
      this.results = { ...this.results, nodes: this.results.nodes.map((x) => (x.id === id ? { ...x, ...patch } : x)) };
  }

  /** Write-back Retry (S15). */
  async retryWriteback(node) {
    try {
      await invoke('writeback_retry', { node: node.id });
    } catch (e) {
      this.toast?.(errText(e), { kind: 'error' });
    }
  }

  // ---------------------------------------------------------------- status lines

  /** Per project: the S3–S6 / §10.7 line, or none. */
  lines(now = Date.now()) {
    const m = new Map();
    for (const p of this.scope) {
      const l = statusLine({ ...p, label: p.label || p.name }, this.status[p.id], now);
      if (l) m.set(p.id, l);
    }
    return m;
  }

  /** @type {((text: string, opts?: any) => any) | null} set by the pane */
  toast = null;
}
