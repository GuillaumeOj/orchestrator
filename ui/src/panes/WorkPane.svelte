<script>
  // Work browser (improve-intake notes §2–§5, §11, §12.1; api-contract §2, §6; boards 04.8 ii-work-*):
  // the scope's tracker items as a tree with app state, details, search, filters saved per scope,
  // multi-select and actions that open the existing dialogs. One tab per scope (lib/work.js).
  import { onDestroy, onMount, untrack } from 'svelte';
  import Menu from '../components/Menu.svelte';
  import AppChip from '../components/work/AppChip.svelte';
  import FiltersPopover from '../components/work/FiltersPopover.svelte';
  import HintRow from '../components/work/HintRow.svelte';
  import KindGlyph from '../components/work/KindGlyph.svelte';
  import WorkDetails from '../components/work/WorkDetails.svelte';
  import WorkRow from '../components/work/WorkRow.svelte';
  import { app, copyText, featureBySlug, now30, openAgent, openEditor, openFeature, openProjectSettings, openUrl, projects, toast } from '../lib/app.svelte.js';
  import { ageText } from '../lib/intake.js';
  import { fixSource } from '../lib/intake.svelte.js';
  import { errText, invoke, listen } from '../lib/ipc.js';
  import { isTypingTarget } from '../lib/keyboard.js';
  import { breakpoint } from '../lib/tokens.js';
  import { tooltip } from '../lib/tooltip.js';
  import { openProgram, openProgramPlan } from '../lib/work.js';
  import { DETAILS_MS, WorkView } from '../lib/work.svelte.js';
  import {
    actionOf,
    ancestors,
    chipOf,
    crumbs,
    emptyText,
    featureOf,
    filterCount,
    headerStatus,
    hintsFor,
    narrowed,
    noMatchText,
    refOf,
    resultsText,
    selectionText,
    TRACKER_NAME,
    trackerOf,
    usable,
    visibleRows,
  } from '../lib/workmodel.js';

  let { paneId } = $props();

  const spec = $derived(app.panes[paneId]);
  const ids = untrack(() => spec?.projects ?? projects().map((p) => p.id)).slice().sort();
  const view = new WorkView({ ids, projects: untrack(projects) });
  view.toast = toast;

  /** @type {HTMLElement | undefined} */
  let root = $state();
  /** @type {HTMLInputElement | undefined} */
  let searchEl = $state();
  /** @type {HTMLElement | undefined} */
  let treeEl = $state();
  /** @type {HTMLElement | undefined} */
  let detailsEl = $state();
  /** @type {HTMLElement | undefined} */
  let filtersBtn = $state();
  let filtersOpen = $state(false);
  let ready = false;
  let pendingLand = null;
  let unlisten = null;

  const now = $derived(Math.max(now30(), Date.now()));
  const narrow = $derived(app.width < breakpoint.md);
  const names = $derived(Object.fromEntries(view.scope.map((p) => [p.id, p.name])));
  const scopeNames = $derived(view.scope.map((p) => p.name));
  const lines = $derived(view.lines(now));
  const searching = $derived(!!view.query.trim());

  const firstLoad = $derived(
    !view.loaded ||
      (!(view.kids.get('roots') ?? []).length && Object.values(view.status).some((s) => s.status === 'refreshing' && !s.refreshed_at_ms)),
  );
  const rootsTotal = $derived((view.kids.get('roots') ?? []).length + (view.more.roots?.left ?? 0));
  const status = $derived(
    headerStatus(view.status, { now, firstLoad, names: Object.fromEntries(view.scope.map((p) => [p.id, p.label || p.name])), rootsTotal }),
  );

  /** Ready, not hidden, per project (the project row's "3 ready"): the Ready lane's own list. */
  const readyCount = (pid) => (app.now?.intake?.items ?? []).filter((i) => i.project === pid && !i.hidden).length;

  const rows = $derived(
    visibleRows({
      scope: view.scope,
      multi: view.multi,
      nodes: view.nodes,
      kids: view.kids,
      more: view.more,
      expanded: view.expanded,
      collapsedProjects: view.collapsedProjects,
      loading: view.loading,
      errors: view.errors,
      folds: view.folds,
      tasks: view.tasks,
      focusRoot: view.focusRoot,
      statusLines: view.multi ? lines : null,
    }),
  );

  const results = $derived(
    searching && view.results
      ? view.results.nodes.map((n) => {
          const p = view.scope.find((x) => x.id === n.project);
          const trail = [p?.label || p?.name, ...crumbs(n.breadcrumb, 3).map((c) => c.text)].filter(Boolean);
          // A body match adds its snippet (contract §2 `match`): "… · in body: "…after the address step"".
          const body = n.match?.field === 'body' && n.match.snippet ? ` · in body: "${n.match.snippet}"` : '';
          return { kind: 'result', key: n.id, node: n, trail: trail.join(' › ') + body };
        })
      : [],
  );
  const list = $derived(searching ? results : rows);
  const cur = $derived(list.find((r) => r.key === view.focused) ?? null);
  const curNode = $derived(cur?.kind === 'node' || cur?.kind === 'result' ? cur.node : null);
  const curProject = $derived(cur?.kind === 'project' ? view.scope.find((p) => p.id === cur.project.id) : null);

  // The focus falls back on the first row when its row went away (filter, collapse).
  $effect(() => {
    // Not while children load: a restored focus waits for its row.
    if (!list.length || !view.loaded || view.loading.size) return;
    if (!view.focused || !list.some((r) => r.key === view.focused)) untrack(() => (view.focused = list[0].key));
  });

  // Keep the focused row in view.
  $effect(() => {
    const k = view.focused;
    if (!k || !treeEl) return;
    treeEl.querySelector(`[data-key="${CSS.escape(k)}"]`)?.scrollIntoView?.({ block: 'nearest' });
  });

  // Details load when the focus rests 300 ms (no fetch while j is held, notes §3).
  $effect(() => {
    const n = curNode;
    if (!n || !view.detailsOpen) return;
    const t = setTimeout(() => view.loadDetails(n), DETAILS_MS);
    return () => clearTimeout(t);
  });

  const selNodes = $derived((view.selection?.ids ?? []).map((id) => view.nodes.get(id)).filter(Boolean));
  const selParent = $derived.by(() => {
    const p = view.selection?.parent;
    return p && !p.startsWith('roots:') ? view.nodes.get(p) : null;
  });
  const selText = $derived(
    view.selection ? selectionText(view.selection, selParent, names[view.selection.parent?.replace(/^roots:/, '')] ?? '') : '',
  );

  const focusNode = $derived(view.focusRoot ? view.nodes.get(view.focusRoot) : null);
  const focusTrail = $derived(
    focusNode
      ? crumbs(
          [
            { id: `project:${focusNode.project}`, title: names[focusNode.project] ?? focusNode.project },
            ...ancestors(view.nodes, focusNode.id).map((n) => ({ id: n.id, ref: refOf(n), title: n.title })),
            { id: focusNode.id, ref: refOf(focusNode), title: focusNode.title },
          ],
          4,
        )
      : [],
  );
  const title = $derived.by(() => {
    const base = spec?.projects ? (ids.length === 1 ? `Work · ${names[ids[0]] ?? ids[0]}` : `Work · ${ids.length} projects`) : 'Work · all projects';
    return focusNode ? `${base} › ${refOf(focusNode) || focusNode.title}` : base;
  });

  /** Breadcrumb of the details: the cached `work_node` one, else the loaded ancestors; the project first. */
  const trail = $derived.by(() => {
    const n = curNode;
    if (!n) return [];
    const det = view.detailsOf(n);
    const chain = det?.breadcrumb ?? n.breadcrumb ?? ancestors(view.nodes, n.id).map((a) => ({ id: a.id, ref: refOf(a), title: a.title }));
    return [{ id: `project:${n.project}`, title: names[n.project] ?? n.project }, ...chain];
  });

  /** The Ready filter in words, per project (the ready chip's tooltip). */
  const readyTip = (n) => view.scope.find((p) => p.id === n?.project)?.ready ?? '';

  const projectTail = (p) => {
    const s = view.status[p.id];
    const l = lines.get(p.id);
    const right = firstLoad && s?.status === 'refreshing' ? `Reading ${TRACKER_NAME[p.kind] ?? 'the tracker'}…` : s?.status === 'refreshing' ? 'refreshing…' : `${readyCount(p.id)} ready`;
    return {
      tracker: view.trackerLine(p),
      error: l?.tone === 'error' ? `${TRACKER_NAME[p.kind] ?? ''}: ${l.short ?? 'error'}`.replace(/^: /, '') : null,
      fix: l?.tone === 'error' && l.fix ? 'Fix' : null,
      right,
    };
  };

  // ---------------------------------------------------------------- filter options (from the cache)

  const filterOptions = $derived.by(() => {
    const labels = new Set();
    const assignees = new Set();
    const milestones = new Set();
    const types = new Set();
    const lin = new Map();
    for (const n of view.nodes.values()) {
      const u = n.upstream ?? {};
      for (const l of u.labels ?? []) if (!/^size[:/]/i.test(l) && !/^orch(:|$)/i.test(l)) labels.add(l);
      for (const a of u.assignees ?? []) assignees.add(a);
      if (u.milestone) milestones.add(u.milestone);
      if (n.kind === 'milestone') milestones.add(n.title);
      if (u.issue_type) types.add(u.issue_type);
      if (n.kind === 'linear_project') lin.set(n.id.replace(/^lin-project:/, ''), n.title);
    }
    const sort = (s) => [...s].sort((a, b) => a.localeCompare(b));
    return {
      labels: sort(labels),
      assignees: sort(assignees),
      milestones: sort(milestones),
      issue_types: sort(types),
      linear_projects: [...lin].map(([id, name]) => ({ id, name })),
    };
  });
  const filterGroups = $derived(
    ['github', 'linear']
      .map((kind) => {
        const ps = view.scope.filter((p) => p.kind === kind);
        return ps.length ? { kind, title: `${TRACKER_NAME[kind]} · ${ps.map((p) => p.name).join(', ')}` } : null;
      })
      .filter(Boolean),
  );
  const nFilters = $derived(filterCount(view.filter));

  // ---------------------------------------------------------------- the ⋯ menu

  const githubScope = $derived(view.scope.filter((p) => p.kind === 'github' && p.settings));
  const menu = $derived([
    ...(githubScope.length
      ? [{ id: 'group', label: 'Group by milestone (GitHub)', checked: githubScope.every((p) => p.settings.group_by_milestone !== false), run: toggleGrouping }]
      : []),
    { id: 'expand', label: 'Expand all to level 2', run: () => view.expandLevel2() },
    { id: 'collapse', label: 'Collapse all', run: () => view.collapseAll() },
    { id: 'hidden', label: 'Show hidden items', checked: view.filter.show_hidden, run: () => view.setFilter({ show_hidden: !view.filter.show_hidden }) },
    { separator: true },
    ...(view.scope.length === 1
      ? [{ id: 'tracker', label: 'Project settings › Tracker', run: () => openProjectSettings(view.scope[0].id, 'tracker') }]
      : [{ id: 'tracker', label: 'Project settings › Tracker', items: view.scope.map((p) => ({ id: `tracker.${p.id}`, label: p.name, run: () => openProjectSettings(p.id, 'tracker') })) }]),
  ]);

  async function toggleGrouping() {
    const on = !githubScope.every((p) => p.settings.group_by_milestone !== false);
    try {
      await Promise.all(githubScope.map((p) => invoke('tracker_settings_set', { project: p.id, settings: { ...p.settings, group_by_milestone: on } })));
      await view.loadTrackers();
      view.reload();
    } catch (e) {
      toast(`Could not change the grouping: ${errText(e)}`, { kind: 'error' });
    }
  }

  // ---------------------------------------------------------------- open / events / landing

  onMount(async () => {
    listen('work.changed', (ev) => view.onChanged(ev.payload?.project ?? null)).then((u) => {
      if (unlisten === false) u();
      else unlisten = u;
    });
    await view.init();
    ready = true;
    if (pendingLand) land(pendingLand);
  });
  onDestroy(() => {
    view.dispose();
    if (unlisten) unlisten();
    unlisten = false;
  });

  let landed = 0;
  $effect(() => {
    const l = spec?.land;
    if (!l || l.seq === landed) return;
    landed = l.seq;
    untrack(() => (ready ? land(l) : (pendingLand = l)));
  });

  async function land(l) {
    pendingLand = null;
    if (l.reveal) {
      if (await view.reveal(l.reveal)) view.detailsOpen = true;
      root?.focus();
    }
    if (l.find) searchEl?.focus();
  }

  // ---------------------------------------------------------------- actions

  function chipOpen(c, n = curNode) {
    if (!c) return;
    if (c.kind === 'feature' || (c.kind === 'pr' && !c.url)) return showFeature(n, c.slug);
    if (c.kind === 'pr') return openUrl(c.url);
    if (c.kind === 'program') return openProgram(c.id, n?.app?.program?.title);
    if (c.kind === 'agent') return openAgent(c.id);
  }

  function showFeature(n, slug = featureOf(n)?.slug) {
    const f = featureOf(n);
    if (f?.agent) return openAgent(f.agent);
    const feat = slug ? featureBySlug(n.project, slug) : null;
    if (feat) openFeature(feat.id);
    else toast(`Feature ${slug ?? ''} is not in this app any more.`.replace('  ', ' '));
  }

  function openNode(n) {
    if (n.url) return openUrl(n.url);
    if (trackerOf(n) === 'local') return openEditor(n.id.replace(/^local:/, ''), null, { projectId: n.project });
  }

  function program(n) {
    const p = actionOf(n, 'program');
    if (p && typeof p === 'object' && 'open_program' in p) return openProgram(p.open_program, n.app?.program?.title);
    if (!usable(p)) return p && typeof p === 'object' && p.disabled ? toast(p.disabled) : undefined;
    openProgramPlan([n.id], { title: `${refOf(n) ? `${refOf(n)} ` : ''}${n.title}` });
  }

  function start(n, mode) {
    const a = actionOf(n, mode);
    if (a === 'ok') return view.start(n, mode);
    if (a && typeof a === 'object' && a.disabled) toast(a.disabled);
  }

  function hideToggle(n) {
    const h = actionOf(n, 'hide');
    if (!usable(h)) return;
    return n.hidden ? view.unhide(n) : view.hide(n);
  }

  function runSelected() {
    if ((view.selection?.ids.length ?? 0) < 2) return toast('Select 2 or more, or use Run as program on the parent');
    openProgramPlan([...view.selection.ids], { title: selParent ? `${refOf(selParent)} ${selParent.title}`.trim() : '' });
  }

  function fixLine(pid, l) {
    if (!l?.fix) return;
    const s = view.status[pid];
    if (l.fix === 'retry') return view.refresh([pid]);
    if (l.fix === 'settings' || l.fix === 'limit_projects') return openProjectSettings(pid, 'tracker');
    return fixSource({ project: pid, error: { ...(s?.error ?? {}), fix: l.fix } }, () => view.refresh([pid]));
  }

  /** The details panel's buttons and links. */
  function detailsAct(what, arg) {
    const n = curNode;
    switch (what) {
      case 'feature':
        return n && showFeature(n);
      case 'program':
        return n && program(n);
      case 'team':
      case 'solo':
        return n && start(n, what);
      case 'open':
        return n && openNode(n);
      case 'hide':
        return n && hideToggle(n);
      case 'focus':
        return n && view.focusHere(n.id);
      case 'copy':
        return n?.url && copyText(n.url, 'Link');
      case 'chip':
        return chipOpen(arg, n);
      case 'url':
        return arg && openUrl(arg);
      case 'reveal':
        if (String(arg).startsWith('project:')) {
          view.leaveFocus();
          view.focused = view.multi ? arg : (view.kids.get('roots') ?? [])[0];
          return;
        }
        return view.reveal(arg);
      case 'expand':
        return n && view.toggle(n.id, true);
      case 'fold':
        if (!n) return;
        view.toggle(n.id, true);
        return view.toggleFold(n.id, true);
      case 'writeback':
        return n && view.retryWriteback(n);
      case 'tracker':
        return openProjectSettings(arg, 'tracker');
      case 'program-selected':
        return runSelected();
      case 'clear':
        return view.clearSelection();
    }
  }

  /** A click or ⏎ on a row. `enter`: from the keyboard (details open / focused). */
  function activate(r, enter = false) {
    if (!r) return;
    view.focused = r.key;
    switch (r.kind) {
      case 'result':
        if (enter) view.reveal(r.node.id);
        return;
      case 'node':
      case 'project':
        if (!enter) return view.save();
        if (!view.detailsOpen) view.detailsOpen = true;
        else detailsEl?.focus();
        return view.save();
      case 'fold':
        return view.toggleFold(r.node.id);
      case 'task': {
        const parent = view.nodes.get(r.parent);
        const f = parent?.app?.feature ? featureBySlug(parent.project, parent.app.feature) : null;
        if (f && enter) openFeature(f.id, null, null, null, { tasks: true });
        return;
      }
      case 'more':
        return enter && view.loadMore(r.parent);
      case 'error':
        return enter && view.retry(r.parent);
      case 'srcline':
        return enter && fixLine(r.key.replace(/^srcline:/, ''), r);
    }
  }

  function disclose(r) {
    if (r.kind === 'project') return view.toggleProject(r.project.id);
    if (r.kind === 'fold') return view.toggleFold(r.node.id);
    if (r.kind === 'node' && r.hasKids) return view.toggle(r.node.id);
  }

  function rowAction(r, what) {
    if (what === 'more') return view.loadMore(r.parent);
    if (what === 'retry') return view.retry(r.parent);
    if (what === 'fix') {
      const pid = r.kind === 'project' ? r.project.id : r.key.replace(/^srcline:/, '');
      return fixLine(pid, lines.get(pid));
    }
  }

  // ---------------------------------------------------------------- keys (notes §12.1)

  function move(d, extend = false) {
    if (!list.length) return;
    const i = list.findIndex((r) => r.key === view.focused);
    const j = Math.max(0, Math.min(list.length - 1, (i < 0 ? 0 : i) + d));
    const r = list[j];
    if (extend && !searching) {
      if (!view.selection && cur?.kind === 'node') view.select(cur.node);
      if (r.kind === 'node') view.extend(r.node);
    }
    view.focused = r.key;
  }

  function left() {
    const r = cur;
    if (!r) return;
    if (r.kind === 'project') return r.open && view.toggleProject(r.project.id, false);
    if ((r.kind === 'node' && r.open) || (r.kind === 'fold' && r.open)) return disclose(r);
    const parent = r.kind === 'node' ? r.node.parent : r.kind === 'task' ? `fold:${r.parent}` : r.kind === 'fold' ? r.node.id : r.parent;
    const target = parent ?? (r.kind === 'node' && view.multi ? `project:${r.node.project}` : null);
    if (target && list.some((x) => x.key === target)) view.focused = target;
  }

  function right() {
    const r = cur;
    if (!r) return;
    const closed = (r.kind === 'project' || r.kind === 'fold' || (r.kind === 'node' && r.hasKids)) && !r.open;
    if (closed) return disclose(r);
    if (r.open) move(1);
  }

  function esc() {
    if (filtersOpen) return (filtersOpen = false), true;
    if (searching) return view.setQuery(''), true;
    if (view.selection) return view.clearSelection(), true;
    if (view.detailsOpen && document.activeElement && detailsEl?.contains(document.activeElement)) return root?.focus(), true;
    if (view.detailsOpen) return (view.detailsOpen = false), view.save(), true;
    if (view.focusRoot) return view.leaveFocus(), true;
    return false;
  }

  function onSearchKey(e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      if (view.query) view.setQuery('');
      else root?.focus();
      return;
    }
    if (e.key === 'ArrowDown' || e.key === 'Enter') {
      e.preventDefault();
      e.stopPropagation();
      root?.focus();
      if (e.key === 'Enter' && searching && cur) activate(cur, true);
    }
  }

  function onKey(e) {
    if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey || isTypingTarget(e.target)) return;
    if (e.target?.closest?.('.pop')) return;
    const k = e.key;
    const n = curNode;
    const run = (f) => {
      e.preventDefault();
      f();
    };
    if (k === 'Escape') {
      if (esc()) e.preventDefault();
      return;
    }
    if (k === 'J' && e.shiftKey) return run(() => move(1, true));
    if (k === 'K' && e.shiftKey) return run(() => move(-1, true));
    if (k === 'Z' && e.shiftKey) return run(() => view.focusUp());
    if (k === 'j' || k === 'ArrowDown') return run(() => move(1));
    if (k === 'k' || k === 'ArrowUp') return run(() => move(-1));
    if (k === 'h' || k === 'ArrowLeft') return run(left);
    if (k === 'l' || k === 'ArrowRight') return run(right);
    if (k === 'Enter') {
      if (e.target?.closest?.('button, a') && e.target !== root) return;
      return run(() => activate(cur, true));
    }
    if (k === ' ') {
      if (e.target?.closest?.('button, a')) return;
      return run(() => {
        view.detailsOpen = !view.detailsOpen;
        view.save();
      });
    }
    if (k === '/') return run(() => searchEl?.focus());
    if (k === 'f') return run(() => (filtersOpen = !filtersOpen));
    if (k === 'r') return run(() => !status.refreshDisabled && view.refresh());
    if (k === 'u' && view.lastHide) return run(() => view.undoHide());
    if (k === 'p' && view.selection) return run(runSelected);
    if (cur?.kind === 'task') {
      if (k === 'o' && cur.node.url) return run(() => openUrl(cur.node.url));
      return;
    }
    if (!n) return;
    if (k === 'x' && !searching) return run(() => view.select(n));
    if (k === 't') return run(() => start(n, 'team'));
    if (k === 'a') return run(() => start(n, 'solo'));
    if (k === 'p') return run(() => program(n));
    if (k === 'o') return run(() => openNode(n));
    if (k === 'Backspace' || k === 'Delete') return run(() => hideToggle(n));
    if (k === 'z' && !searching && n.children?.total > 0) return run(() => view.focusHere(n.id));
    if (k === 'g' && featureOf(n)) return run(() => showFeature(n));
  }

  const hints = $derived(
    hintsFor(cur, { selection: view.selection?.ids.length ?? 0, focusMode: !!view.focusRoot, searching, details: view.detailsOpen }),
  );
  const singleLines = $derived(view.multi ? [] : [...lines].map(([pid, l]) => ({ pid, ...l })));
  const noRoots = $derived(view.loaded && !firstLoad && !(view.kids.get('roots') ?? []).length && !view.focusRoot);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="work" tabindex="0" data-pane-focus bind:this={root} onkeydown={onKey} role="region" aria-label={title}>
  <header class="head">
    <h2>{title}</h2>
    <span class="grow"></span>
    <span class="status" class:err={status.tone === 'error'} data-status>{status.text}</span>
    {#if status.text}<span class="muted" aria-hidden="true">·</span>{/if}
    <button type="button" class="btn ghost" disabled={status.refreshDisabled} use:tooltip={{ label: 'Refresh', key: 'r', detail: 'Read the trackers of this scope now' }} onclick={() => view.refresh()}>Refresh</button>
  </header>

  {#if view.selection}
    <div class="bar selbar" data-selbar>
      <span>{selText}</span>
      <button type="button" class="btn primary" disabled={view.selection.ids.length < 2} use:tooltip={view.selection.ids.length < 2 ? 'Select 2 or more, or use Run as program on the parent' : { label: 'Run selected as program', key: 'p' }} onclick={runSelected}>Run selected as program</button>
      <button type="button" class="btn" use:tooltip={{ label: 'Clear', key: 'Esc' }} onclick={() => view.clearSelection()}>Clear</button>
      {#if view.selection.ids.length < 2}<span class="muted">Select 2 or more, or use Run as program on the parent</span>{/if}
    </div>
  {:else}
    <div class="bar">
      {#if focusNode}
        <nav class="fcrumbs" aria-label="Focus">
          {#each focusTrail as c, i (i)}
            {#if i}<span class="muted" aria-hidden="true">›</span>{/if}
            {#if c.id && i < focusTrail.length - 1}
              <button type="button" class="link" onclick={() => (i === 0 ? view.leaveFocus() : view.focusHere(c.id))}>{c.text}</button>
            {:else}<b>{c.text}</b>{/if}
          {/each}
          <span class="muted keytext">⇧Z up · Esc leave focus</span>
        </nav>
        <span class="grow"></span>
      {:else}
        <label class="search">
          <span class="icon muted" aria-hidden="true">{'\u{F0349}'}</span>
          <input
            bind:this={searchEl}
            type="search"
            value={view.query}
            placeholder="Search work… (title, #12, ENG-42, text)"
            aria-label="Search work"
            oninput={(e) => view.setQuery(e.currentTarget.value)}
            onkeydown={onSearchKey}
          />
        </label>
      {/if}
      <span class="fwrap">
        <button type="button" class="btn" bind:this={filtersBtn} aria-expanded={filtersOpen} use:tooltip={{ label: 'Filters', key: 'f' }} onclick={() => (filtersOpen = !filtersOpen)}>{nFilters ? `Filters · ${nFilters}` : 'Filters'}</button>
        {#if filtersOpen}
          <FiltersPopover filter={view.filter} set={(p) => view.setFilter(p)} reset={() => view.resetFilters()} close={() => ((filtersOpen = false), root?.focus())} groups={filterGroups} options={filterOptions} anchor={filtersBtn} />
        {/if}
      </span>
      {#if !focusNode}
        <button type="button" class="btn toggle" aria-pressed={view.filter.state === 'open'} onclick={() => view.setFilter({ state: view.filter.state === 'open' ? 'all' : 'open' })}>Hide closed{view.filter.state === 'open' ? ' ✓' : ''}</button>
        <button type="button" class="btn toggle" aria-pressed={view.filter.only_ready} onclick={() => view.setFilter({ only_ready: !view.filter.only_ready })}>Only ready{view.filter.only_ready ? ' ✓' : ''}</button>
        <Menu items={menu} label="View options" />
      {/if}
    </div>
  {/if}

  {#each singleLines as l (l.pid)}
    <div class="banner {l.tone}" data-line={l.tone}>
      <span aria-hidden="true">{l.tone === 'info' ? '\u{F02FC}' : '⚠'}</span>
      <span class="btext">{l.text}</span>
      {#if l.fix && l.fixLabel}<button type="button" class="btn sm" onclick={() => fixLine(l.pid, l)}>{l.fixLabel}</button>{/if}
    </div>
    {#if l.command}<div class="banner cmd"><span class="mono">{l.command}</span></div>{/if}
  {/each}

  <div class="body">
    <div class="tree" role="tree" aria-label="Work items" bind:this={treeEl}>
      {#if view.failed && !view.nodes.size}
        <div class="empty"><p class="err">Could not read Work: {view.failed}</p><button type="button" class="btn" onclick={() => view.reload()}>Retry</button></div>
      {:else if firstLoad}
        {#each view.multi ? view.scope : [null] as p (p?.id ?? 'one')}
          {#if p}
            <WorkRow row={{ kind: 'project', key: `project:${p.id}`, project: p, level: 0, indent: 0, open: true }} projectTail={projectTail(p)} />
          {/if}
          <div class="skel" role="status" aria-busy="true" aria-label="Loading">
            {#each [0, 1, 2] as i (i)}<span class="bar-row" style:padding-left="{i * 16 + 16}px"><span class="sk short"></span><span class="sk"></span><span class="grow"></span><span class="sk tail"></span></span>{/each}
          </div>
        {/each}
      {:else if searching && view.results && !view.results.total}
        <!-- No results (notes §4.1): centred in the tree, as the other empty states. -->
        <div class="empty" data-empty>
          <p class="muted" data-results>
            {resultsText(view.results, view.query.trim(), scopeNames)}
            {#if view.results.hidden > 0} · {view.results.hidden} more hidden by filters · <button type="button" class="link" onclick={() => view.showHiddenResults()}>Show</button>{/if}
            · <button type="button" class="link" onclick={() => view.setQuery('')}>Clear search</button>
          </p>
        </div>
      {:else if searching}
        <p class="rhead muted" data-results>
          {#if view.results}
            {resultsText(view.results, view.query.trim(), scopeNames)} · title first, then body · ⏎ reveals in the tree
            {#if view.results.hidden > 0} · {view.results.hidden} more hidden by filters · <button type="button" class="link" onclick={() => view.showHiddenResults()}>Show</button>{/if}
          {:else}Searching…{/if}
        </p>
        {#each results as r (r.key)}
          {@const chip = chipOf(r.node, { short: narrow, readyTip: readyTip(r.node) })}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div class="result" class:focused={view.focused === r.key} role="treeitem" tabindex="-1" aria-selected={view.focused === r.key} data-key={r.key} onclick={() => activate(r)} ondblclick={() => activate(r, true)}>
            <KindGlyph node={r.node} />
            <div class="rtext">
              <span><span class="ref">{refOf(r.node)}</span> {r.node.title}</span>
              <span class="muted crumbline">{r.trail}</span>
            </div>
            <span class="grow"></span>
            {#if chip}<AppChip {chip} onopen={(c) => chipOpen(c, r.node)} />{/if}
          </div>
        {/each}
      {:else if noRoots}
        <div class="empty" data-empty>
          {#if narrowed(view.filter)}
            <p class="muted">{noMatchText(nFilters || 1, scopeNames)} · <button type="button" class="link" onclick={() => view.resetFilters()}>Reset filters</button></p>
          {:else}
            {#each view.scope as p (p.id)}<p class="muted">{emptyText(p)}</p>{/each}
          {/if}
        </div>
      {:else}
        {#each rows as r (r.key)}
          <WorkRow
            row={r}
            focused={view.focused === r.key}
            inDetails={view.detailsOpen && view.focused === r.key}
            selected={!!view.selection?.ids.includes(r.key)}
            selecting={!!view.selection}
            {narrow}
            readyTip={readyTip(r.node)}
            projectTail={r.kind === 'project' ? projectTail(r.project) : null}
            onrow={() => activate(r)}
            ondisclose={() => ((view.focused = r.key), disclose(r))}
            onchip={(c) => chipOpen(c, r.node)}
            onaction={(w) => rowAction(r, w)}
          />
        {/each}
      {/if}
    </div>
    {#if view.detailsOpen && !firstLoad}
      <WorkDetails
        bind:el={detailsEl}
        node={view.selection && selNodes.length >= 2 ? null : curNode}
        project={!curNode && curProject ? { ...curProject, trackerLine: view.trackerLine(curProject) } : null}
        projectStatus={curProject ? [view.status[curProject.id]?.refreshed_at_ms ? `updated ${ageText(view.status[curProject.id].refreshed_at_ms, now)}` : '', `${readyCount(curProject.id)} ready`].filter(Boolean).join(' · ') : ''}
        detail={curNode ? view.detailsOf(curNode) : null}
        {trail}
        kids={curNode && view.kids.has(curNode.id) ? view.kids.get(curNode.id).map((id) => view.nodes.get(id)).filter(Boolean) : null}
        selection={view.selection && selNodes.length >= 2 ? selNodes : null}
        selectionText={selText}
        {narrow}
        readyTip={readyTip(curNode)}
        lookup={(id) => view.nodes.get(id) ?? null}
        act={detailsAct}
      />
    {/if}
  </div>

  <HintRow {hints} />
</div>

<style>
  .work {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    outline: none;
    font: var(--text-label);
    position: relative;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    height: 40px;
    padding: 0 var(--space-lg);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  h2 {
    margin: 0;
    font: var(--text-title);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .grow {
    flex: 1;
  }
  .status {
    color: var(--color-text-muted);
    white-space: nowrap;
  }
  .status.err {
    color: var(--color-status-error);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: 36px;
    padding: var(--space-xs) var(--space-md);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  .selbar {
    background: var(--color-surface-selected);
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    flex: 0 1 300px;
    min-width: 160px;
    height: var(--size-control);
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
  }
  .search:focus-within {
    border-color: var(--color-focus);
  }
  .search input {
    flex: 1;
    min-width: 0;
    background: none;
    border: 0;
    outline: none;
    color: var(--color-text-primary);
    font: var(--text-body);
  }
  .fwrap {
    position: relative;
  }
  .btn.toggle[aria-pressed='true'] {
    border-color: var(--color-focus);
    background: var(--color-accent-subtle);
  }
  .fcrumbs {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    min-width: 0;
    flex-wrap: wrap;
  }
  .keytext {
    margin-left: var(--space-md);
  }
  .banner {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: 28px;
    padding: var(--space-xs) var(--space-lg);
    background: var(--color-surface-raised);
    color: var(--color-text-muted);
  }
  .banner.warn {
    color: var(--color-pacing-amber);
  }
  .banner.error {
    color: var(--color-status-error);
  }
  .banner.cmd {
    padding-left: calc(var(--space-lg) + 20px);
    color: var(--color-text-secondary);
  }
  .btext {
    min-width: 0;
  }
  .btn.sm {
    height: var(--size-control-sm);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .tree {
    flex: 1;
    min-width: 0;
    overflow: auto;
    padding: var(--space-xs);
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-sm);
    height: 100%;
    text-align: center;
    padding: var(--space-xl);
  }
  .empty p {
    margin: 0;
  }
  .skel {
    display: flex;
    flex-direction: column;
  }
  .bar-row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    height: 32px;
  }
  .sk {
    height: 10px;
    width: 220px;
    border-radius: var(--radius-sm);
    background: var(--color-surface-raised);
  }
  .sk.short {
    width: 16px;
  }
  .sk.tail {
    width: 110px;
    margin-right: var(--space-md);
  }
  .rhead {
    margin: var(--space-xs) var(--space-sm);
  }
  .result {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: 44px;
    padding: 0 var(--space-md) 0 var(--space-sm);
    border: var(--border-width-default) solid transparent;
    border-radius: var(--radius-sm);
    outline: none;
  }
  .result.focused {
    border-color: var(--color-focus);
  }
  .rtext {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font: var(--text-body);
  }
  .crumbline {
    font: var(--text-caption);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ref {
    font: var(--text-mono-sm);
    color: var(--color-text-muted);
  }
</style>
