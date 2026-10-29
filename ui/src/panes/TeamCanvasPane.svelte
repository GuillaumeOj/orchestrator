<script>
  // Team canvas (FR-301–308, Penpot 04 Team canvas · 1512): the builder, the
  // role inspector and the Rules panel. Mouse optional (§9.1): n add, hjkl /
  // arrows select, ⇧+hjkl move, c connect (target number, then edge type 1–4),
  // ⏎ inspect, x delete, = auto-layout, u undo, ⌃r redo, ⇧R library (roles and teams), ⌘⏎ launch,
  // D duplicate the selected role (the team when none), then 1 global / 2 project (FR-306).
  // Every edit saves (debounced); the core validates and versions the team.
  import { onMount } from 'svelte';
  import AgentChip from '../components/AgentChip.svelte';
  import Kbd from '../components/Kbd.svelte';
  import ModelSelect from '../components/ModelSelect.svelte';
  import RoleMcp from '../components/RoleMcp.svelte';
  import { app, projects, saveLayout, toast } from '../lib/app.svelte.js';
  import * as C from '../lib/canvas.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { onTopics } from '../lib/now.svelte.js';

  let { paneId } = $props();
  const spec = $derived(app.panes[paneId]);

  let view = $state(null); // teams_view
  let team = $state(null); // the edited copy
  let sel = $state(null); // node id
  let selEdge = $state(null); // edge index
  let mode = $state(null); // null | { kind: 'connect', from, to? } | { kind: 'add' } | { kind: 'library' } | { kind: 'import', text }
  let status = $state('saved'); // saved | saving | unsaved | error
  let serverIssues = $state([]);
  let registry = $state(null); // MCP servers this scope sees (FR-601)
  let pan = $state({ x: 40, y: 40 });
  let zoom = $state(1);
  let root;
  let area;
  const history = new C.History();
  let saveTimer = null;

  const roles = $derived(view?.roles ?? []);
  const roleOf = (id) => roles.find((r) => r.id === id);
  const node = $derived(team?.nodes.find((n) => n.id === sel) ?? null);
  const role = $derived(node ? roleOf(node.role_id) : null);
  const saved = $derived(view?.teams.find((t) => t.team.id === team?.id));
  const issues = $derived(team ? (status === 'saved' ? serverIssues : C.localIssues(team)) : []);
  const badNodes = $derived(new Set(issues.map((i) => i.node).filter(Boolean)));
  const lines = $derived(status === 'saved' ? (saved?.rules ?? []) : []);
  const targets = $derived(mode?.kind === 'connect' && team ? C.targets(team, mode.from) : []);
  const project = $derived(spec?.projectId ?? null);

  async function loadRegistry() {
    try {
      const r = await invoke('mcp_registry_view', { project });
      const over = r.project_overrides ?? {};
      registry = [...r.servers.map((s) => over[s.id] ?? s), ...Object.values(over).filter((o) => !r.servers.some((s) => s.id === o.id))];
    } catch {
      registry = [];
    }
  }

  async function load() {
    try {
      view = await invoke('teams_view', { project });
      if (!team) {
        const want = spec?.teamId ?? view.teams[0]?.team.id;
        const t = view.teams.find((x) => x.team.id === want);
        if (t) open(t);
      } else {
        const t = view.teams.find((x) => x.team.id === team.id);
        if (t && status === 'saved') serverIssues = t.issues;
      }
    } catch (e) {
      toast(`Teams: ${errText(e)}`, { kind: 'error' });
    }
  }

  function open(t) {
    team = structuredClone($state.snapshot(t.team));
    serverIssues = t.issues;
    sel = team.nodes.find((n) => n.is_lead)?.id ?? team.nodes[0]?.id ?? null;
    selEdge = null;
    history.past = [];
    history.future = [];
    status = 'saved';
    app.panes[paneId] = { ...app.panes[paneId], teamId: team.id, title: `Team: ${team.name}` };
    saveLayout();
    mode = null;
  }

  /** One edit: recorded for undo, then saved. A starter is a template: the
   *  first edit asks where to save a copy instead (p3-design-notes §5.8). */
  function edit(next) {
    if (!next || next === team) return;
    if (saved?.starter && status === 'saved') {
      mode = { kind: 'starter', next };
      return;
    }
    history.push($state.snapshot(team));
    team = next;
    scheduleSave();
  }

  function scheduleSave() {
    status = 'unsaved';
    clearTimeout(saveTimer);
    saveTimer = setTimeout(save, 600);
  }

  async function save() {
    status = 'saving';
    try {
      const r = await invoke('team_save', { team: $state.snapshot(team) });
      team = { ...team, version: r.team.version };
      serverIssues = r.issues;
      status = 'saved';
      await load();
    } catch (e) {
      status = 'error';
      toast(`Could not save the team: ${errText(e)}`, { kind: 'error' });
    }
  }

  function undo() {
    const t = history.undo($state.snapshot(team));
    if (t) ((team = t), scheduleSave());
  }
  function redo() {
    const t = history.redo($state.snapshot(team));
    if (t) ((team = t), scheduleSave());
  }

  function addRole(roleId) {
    const [t, id] = C.addNode(team, roleId);
    edit(t);
    sel = id;
    mode = null;
    ensureVisible(id);
  }

  function newTeam() {
    const ids = new Set(view?.teams.map((t) => t.team.id));
    let id = 'team';
    for (let i = 2; ids.has(id); i++) id = `team${i}`;
    const t = { id, name: 'New team', scope: project ? { kind: 'project', id: project } : { kind: 'global' }, nodes: [], edges: [], version: 1 };
    const [withLead] = C.addNode(t, roles.find((r) => r.id === 'lead')?.id ?? roles[0]?.id ?? 'lead');
    open({ team: withLead, issues: [], rules: [] });
    scheduleSave();
  }

  function connect(kind) {
    if (mode?.kind !== 'connect' || !mode.to) return;
    const t = C.addEdge(team, mode.from, mode.to, kind);
    if (t === team) toast(`${mode.from} → ${mode.to} (${kind}) is already drawn`);
    else edit(t);
    mode = null;
  }

  function ensureVisible(id) {
    const n = team?.nodes.find((x) => x.id === id);
    if (!n || !area) return;
    const r = area.getBoundingClientRect();
    const x = n.position[0] * zoom + pan.x;
    const y = n.position[1] * zoom + pan.y;
    if (x < 0 || y < 0 || x + C.NODE_W * zoom > r.width || y + C.NODE_H * zoom > r.height)
      pan = { x: r.width / 2 - (n.position[0] + C.NODE_W / 2) * zoom, y: r.height / 2 - (n.position[1] + C.NODE_H / 2) * zoom };
  }

  function launch() {
    if (!team) return;
    if (issues.length) return toast('Fix the team first: see the Rules panel', { kind: 'error' });
    if (status !== 'saved') return toast('Saving the team…');
    app.overlay = { kind: 'launch', teamId: team.id, projectId: project };
  }

  async function exportTeam() {
    try {
      const json = await invoke('team_export', { team: team.id, project: team.scope?.kind === 'project' ? team.scope.id : null });
      await navigator.clipboard?.writeText(json);
      toast(`Copied ${team.name} as JSON to the clipboard`);
    } catch (e) {
      toast(`Export: ${errText(e)}`, { kind: 'error' });
    }
  }

  async function importTeam(text) {
    try {
      const t = await invoke('team_import', { json: text });
      mode = null;
      await load();
      const v = view.teams.find((x) => x.team.id === t.id);
      if (v) open(v);
      toast(`Imported ${t.name}`);
    } catch (e) {
      toast(`Import: ${errText(e)}`, { kind: 'error' });
    }
  }

  async function saveRole(patch) {
    if (!role) return;
    // A starter role is a template too: ask where to save a copy (§5.8).
    if (role.origin && role.scope?.kind !== 'project') {
      mode = { kind: 'starter', role: $state.snapshot(role), node: node?.id, rolePatch: patch };
      root?.focus();
      return;
    }
    try {
      await invoke('role_save', { role: { ...$state.snapshot(role), ...patch } });
      await load();
    } catch (e) {
      toast(`Role: ${errText(e)}`, { kind: 'error' });
    }
  }

  // ---- presets (FR-306): duplicate a team or role, save as preset, launch
  const scopeOf = (t) => t?.scope ?? { kind: 'global' };
  const projectName = $derived(project ? (projects().find((p) => p.id === project)?.name ?? project) : null);

  /** D then 1 (global) or 2 (this project): the selected role, else the team. */
  async function duplicate(to, what = null) {
    mode = null;
    const target = to === 'project' ? (project ? { kind: 'project', id: project } : null) : { kind: 'global' };
    if (!target) return toast('Open the canvas from a project to duplicate into it');
    const where = target.kind === 'project' ? projectName : 'global';
    try {
      if (what?.kind === 'role' || (!what && role)) {
        const r = what?.role ?? role;
        await invoke('role_duplicate', { role: r.id, from: r.scope ?? { kind: 'global' }, to: target, newId: null });
        toast(`Duplicated role ${r.id} to ${where}`);
        await load();
        return;
      }
      const t = what?.team ?? team;
      if (!t) return;
      const r = await invoke('team_duplicate', { team: t.id, from: scopeOf(t), to: target, newId: null });
      await load();
      const v = view.teams.find((x) => x.team.id === r.team.id);
      if (v) open(v);
      toast(`Duplicated ${t.name} to ${where}`);
    } catch (e) {
      toast(`Duplicate: ${errText(e)}`, { kind: 'error' });
    }
  }

  async function setPreset(t, preset) {
    try {
      await invoke('team_set_preset', { team: t.id, scope: scopeOf(t), preset });
      toast(preset ? `Saved ${t.name} as a preset` : `Removed ${t.name} from presets`);
      await load();
    } catch (e) {
      toast(`Preset: ${errText(e)}`, { kind: 'error' });
    }
  }

  /** The starter edit, applied to a copy in `to` ('project' | 'global'). */
  async function copyStarter(to) {
    const m = mode?.kind === 'starter' ? mode : null;
    const next = m?.next ?? null;
    mode = null;
    const target = to === 'project' && project ? { kind: 'project', id: project } : { kind: 'global' };
    const where = target.kind === 'project' ? projectName : 'global';
    if (m?.rolePatch) return copyStarterRole(m, target, where);
    try {
      const r = await invoke('team_duplicate', { team: team.id, from: scopeOf(team), to: target, newId: null });
      await load();
      open(view.teams.find((x) => x.team.id === r.team.id) ?? { team: r.team, issues: [], rules: [] });
      if (next) edit({ ...next, id: r.team.id, name: r.team.name, scope: r.team.scope, version: r.team.version });
      toast(`Saved your changes as ${r.team.name} in ${target.kind === 'project' ? projectName : 'global'}`);
    } catch (e) {
      toast(`Copy: ${errText(e)}`, { kind: 'error' });
    }
  }

  /** A starter role's edit, saved on a copy that the node then uses. */
  async function copyStarterRole(m, target, where) {
    try {
      const copy = await invoke('role_duplicate', { role: m.role.id, from: m.role.scope ?? { kind: 'global' }, to: target, newId: null });
      await invoke('role_save', { role: { ...copy, ...m.rolePatch } });
      await load();
      toast(`Saved your changes as role ${copy.id} in ${where}`);
      if (m.node) edit(C.updateNode(team, m.node, { role_id: copy.id }));
    } catch (e) {
      toast(`Copy: ${errText(e)}`, { kind: 'error' });
    }
  }

  /** "Update available": a fresh copy of the app's starter, to compare with the edited one. */
  async function getNewStarter(t) {
    try {
      const r = await invoke('starter_copy', { starter: t.starter, to: scopeOf(t.team), newId: null });
      await load();
      const v = view.teams.find((x) => x.team.id === r.team.id);
      if (v) open(v);
      toast(`Copied the new ${t.team.name} starter as ${r.team.id}`);
    } catch (e) {
      toast(`New starter: ${errText(e)}`, { kind: 'error' });
    }
  }

  function launchPreset(t) {
    mode = null;
    app.overlay = { kind: 'launch', teamId: t.id, projectId: project };
  }

  /** x: the selected edge, else the selected node (and its edges). */
  function removeSelection() {
    if (selEdge !== null) {
      edit(C.removeEdge(team, selEdge));
      selEdge = null;
    } else if (sel) {
      const next = C.neighbor(team, sel, 'left');
      edit(C.removeNode(team, sel));
      sel = next === sel ? (team.nodes[0]?.id ?? null) : next;
    }
  }

  function onKey(e) {
    if (e.target.closest('input, textarea, select')) {
      if (e.key === 'Escape') (root?.focus(), e.preventDefault());
      return;
    }
    if (!team) return;
    const k = e.key;
    const handled = () => e.preventDefault();
    if (e.metaKey && k === 'Enter') return (launch(), handled());
    if (e.metaKey || e.altKey) return;
    if (e.ctrlKey) {
      if (k === 'r') (redo(), handled());
      return;
    }
    // Modes first: digits pick a target, then an edge type.
    if (mode?.kind === 'connect') {
      if (k === 'Escape') return ((mode = null), handled());
      const d = Number(k);
      if (!mode.to && d >= 1 && d <= 9) {
        const t = targets.find((x) => x.n === d);
        if (t) mode = { ...mode, to: t.id };
        return handled();
      }
      if (mode.to && d >= 1 && d <= 4) return (connect(C.EDGE_KINDS[d - 1]), handled());
      return handled();
    }
    if (mode?.kind === 'add') {
      if (k === 'Escape') return ((mode = null), handled());
      const d = Number(k);
      if (d >= 1 && d <= roles.length) return (addRole(roles[d - 1].id), handled());
      return handled();
    }
    if (mode?.kind === 'starter') {
      if (k === 'Enter' && project) copyStarter('project');
      else if (k === 'g' || (k === 'Enter' && !project)) copyStarter('global');
      else if (k === 'Escape') mode = null;
      return handled();
    }
    if (mode?.kind === 'dup') {
      if (k === '1') duplicate('global', mode.what);
      else if (k === '2') duplicate('project', mode.what);
      else if (k === 'Escape') mode = null;
      return handled();
    }
    if (mode?.kind === 'library' && k === 'Escape') return ((mode = null), handled());
    const dirs = { h: 'left', arrowleft: 'left', l: 'right', arrowright: 'right', k: 'up', arrowup: 'up', j: 'down', arrowdown: 'down' };
    const dir = dirs[k.toLowerCase()];
    if (dir && e.shiftKey && node) {
      const [dx, dy] = { left: [-20, 0], right: [20, 0], up: [0, -20], down: [0, 20] }[dir];
      return (edit(C.moveNode(team, node.id, [node.position[0] + dx, node.position[1] + dy])), handled());
    }
    if (dir) {
      sel = C.neighbor(team, sel, dir);
      selEdge = null;
      ensureVisible(sel);
      return handled();
    }
    switch (k) {
      case 'n':
        mode = { kind: 'add' };
        return handled();
      case 'c':
        if (sel && team.nodes.length > 1) mode = { kind: 'connect', from: sel };
        return handled();
      case 'Enter':
        root?.querySelector('.inspector input, .inspector select')?.focus();
        return handled();
      case 'x':
      case 'Backspace':
      case 'Delete':
        removeSelection();
        return handled();
      case '=':
        edit(C.autoLayout(team));
        return handled();
      case 'u':
        undo();
        return handled();
      case 'R':
        mode = mode?.kind === 'library' ? null : { kind: 'library' };
        return handled();
      case 'D':
        mode = { kind: 'dup', what: role ? { kind: 'role', role } : { kind: 'team', team } };
        return handled();
      case 'Escape':
        sel = null;
        selEdge = null;
        return handled();
    }
  }

  // ---- mouse: drag nodes, pan the background, zoom with the wheel
  let drag = null;
  function nodeDown(e, n) {
    if (e.button !== 0) return;
    e.stopPropagation();
    if (mode?.kind === 'connect' && n.id !== mode.from) {
      mode = { ...mode, to: n.id };
      return;
    }
    sel = n.id;
    selEdge = null;
    drag = { kind: 'node', id: n.id, sx: e.clientX, sy: e.clientY, ox: n.position[0], oy: n.position[1], moved: false, before: $state.snapshot(team) };
    e.currentTarget.setPointerCapture?.(e.pointerId);
  }
  function bgDown(e) {
    if (e.button !== 0) return;
    sel = null;
    selEdge = null;
    if (mode?.kind === 'connect') mode = null;
    drag = { kind: 'pan', sx: e.clientX, sy: e.clientY, ox: pan.x, oy: pan.y };
  }
  function move(e) {
    if (!drag) return;
    const dx = e.clientX - drag.sx;
    const dy = e.clientY - drag.sy;
    if (drag.kind === 'pan') pan = { x: drag.ox + dx, y: drag.oy + dy };
    else {
      drag.moved = drag.moved || Math.abs(dx) + Math.abs(dy) > 3;
      // In place while dragging (one node re-renders); `up` records the undo step.
      const n = drag.moved && team.nodes.find((x) => x.id === drag.id);
      if (n) n.position = [Math.round(drag.ox + dx / zoom), Math.round(drag.oy + dy / zoom)];
    }
  }
  function up() {
    if (drag?.kind === 'node' && drag.moved) {
      if (saved?.starter && status === 'saved') {
        // A starter's node moved: ask where to copy it, and put the starter back.
        const next = $state.snapshot(team);
        team = drag.before;
        mode = { kind: 'starter', next };
      } else {
        history.push(drag.before);
        scheduleSave();
      }
    }
    drag = null;
  }
  function wheel(e) {
    e.preventDefault();
    const r = area.getBoundingClientRect();
    const mx = e.clientX - r.left;
    const my = e.clientY - r.top;
    const z = Math.min(2, Math.max(0.4, zoom * (e.deltaY < 0 ? 1.1 : 0.9)));
    pan = { x: mx - ((mx - pan.x) * z) / zoom, y: my - ((my - pan.y) * z) / zoom };
    zoom = z;
  }

  // Minimap (FR-301): the graph in 168 × 104; click to center there.
  const MM = { w: 168, h: 104, pad: 8 };
  const mm = $derived.by(() => {
    if (!team) return null;
    const b = C.bounds(team);
    const s = Math.min((MM.w - 2 * MM.pad) / b.w, (MM.h - 2 * MM.pad) / b.h, 0.25);
    const r = area?.getBoundingClientRect();
    return { b, s, vw: r ? (r.width / zoom) * s : 0, vh: r ? (r.height / zoom) * s : 0 };
  });
  function mmClick(e) {
    const r = e.currentTarget.getBoundingClientRect();
    const gx = (e.clientX - r.left - MM.pad) / mm.s + mm.b.x;
    const gy = (e.clientY - r.top - MM.pad) / mm.s + mm.b.y;
    const a = area.getBoundingClientRect();
    pan = { x: a.width / 2 - gx * zoom, y: a.height / 2 - gy * zoom };
  }

  const EDGE = {
    'reports-to': { color: 'var(--color-border-strong)', dash: null, head: 'arrow' },
    'hands-off-to': { color: 'var(--color-accent-default)', dash: '6 4', head: 'arrow' },
    reviews: { color: 'var(--color-status-at-gate)', dash: '2 3', head: 'circle' },
    'can-message': { color: 'var(--color-text-muted)', dash: null, head: 'arrow', tail: true },
  };

  onMount(() => {
    load();
    loadRegistry();
    // Refetched when the teams changed, unless edits here are unsaved.
    const stop = onTopics(['teams'], () => status === 'saved' && load(), { now: false });
    return () => (stop(), clearTimeout(saveTimer));
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="canvas-pane" tabindex="0" data-pane-focus bind:this={root} onkeydown={onKey} role="application" aria-label="Team canvas">
  <header class="ch">
    {#if team}
      <input class="name" aria-label="Team name" value={team.name} onchange={(e) => edit({ ...$state.snapshot(team), name: e.currentTarget.value.trim() || team.name })} />
      <span class="muted">v{team.version} · {team.scope?.kind === 'project' ? team.scope.id : 'global'} · {team.nodes.length} node{team.nodes.length === 1 ? '' : 's'} · <span class:err={status === 'error'}>{status}</span></span>
    {:else}
      <b>Team canvas</b>
    {/if}
    <span class="grow"></span>
    <button class="btn ghost" onclick={() => (mode = mode?.kind === 'library' ? null : { kind: 'library' })}>Library <Kbd keys="⇧+R" /></button>
    <button class="btn ghost" onclick={() => (mode = { kind: 'import', text: '' })}>Import</button>
    <button class="btn ghost" disabled={!team} onclick={exportTeam}>Export</button>
    <button class="btn primary" disabled={!team} onclick={launch}>Launch on feature <Kbd keys="⌘+⏎" /></button>
  </header>

  <div class="main">
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="area" bind:this={area} onpointerdown={bgDown} onpointermove={move} onpointerup={up} onpointerleave={up} onwheel={wheel}>
      {#if team}
        <div class="world" style:transform="translate({pan.x}px, {pan.y}px) scale({zoom})">
          <svg class="edges" width="1" height="1" overflow="visible" aria-hidden="true">
            <defs>
              {#each Object.entries(EDGE) as [k, s] (k)}
                <marker id="arrow-{k}-{paneId}" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M0,1 L9,5 L0,9 Z" fill={s.color} /></marker>
              {/each}
              <marker id="circle-{paneId}" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="8" markerHeight="8"><circle cx="5" cy="5" r="3.5" fill="none" stroke={EDGE.reviews.color} stroke-width="1.5" /></marker>
            </defs>
            {#each team.edges as e, i (`${e.from}-${e.to}-${e.kind}`)}
              {@const g = C.edgeLine(team, e)}
              {@const s = EDGE[e.kind]}
              {#if g}
                <!-- svelte-ignore a11y_click_events_have_key_events -->
                <g class="edge" class:sel={selEdge === i} onpointerdown={(ev) => (ev.stopPropagation(), (selEdge = i), (sel = null))} role="button" tabindex="-1" aria-label="{e.from} {e.kind} {e.to}">
                  <line x1={g.x1} y1={g.y1} x2={g.x2} y2={g.y2} stroke="transparent" stroke-width="12" />
                  <line x1={g.x1} y1={g.y1} x2={g.x2} y2={g.y2} stroke={selEdge === i ? 'var(--color-focus)' : s.color} stroke-width={selEdge === i ? 2 : 1.5} stroke-dasharray={s.dash} marker-end={s.head === 'circle' ? `url(#circle-${paneId})` : `url(#arrow-${e.kind}-${paneId})`} marker-start={s.tail ? `url(#arrow-${e.kind}-${paneId})` : null} />
                  <text x={g.mx + 6} y={g.my - 6} class="elabel">{e.kind === 'reports-to' ? '' : e.kind.replace('-to', '')}</text>
                </g>
              {/if}
            {/each}
          </svg>
          {#each team.nodes as n (n.id)}
            {@const r = roleOf(n.role_id)}
            {@const t = targets.find((x) => x.id === n.id)}
            <div class="node" class:sel={sel === n.id} class:bad={badNodes.has(n.id)} class:from={mode?.kind === 'connect' && mode.from === n.id} style:left="{n.position[0]}px" style:top="{n.position[1]}px" onpointerdown={(e) => nodeDown(e, n)} role="button" tabindex="-1" aria-label="node {n.id}" aria-pressed={sel === n.id}>
              <div class="top">
                <AgentChip id={n.id} role={n.role_id} color={r?.color ?? null} />
                <span class="grow"></span>
                {#if n.is_lead}<span class="lead">lead</span>{/if}
                {#if t}<span class="target">{t.n}</span>{/if}
              </div>
              <div class="meta">{n.overrides?.model ?? r?.model ?? 'default'} · {n.overrides?.effort ?? r?.effort ?? 'default'} · max {n.max_concurrency}</div>
              {#if badNodes.has(n.id)}<div class="ierr">{'\u{F0028}'} {issues.find((i) => i.node === n.id)?.text}</div>{/if}
            </div>
          {/each}
        </div>
        {#if mm}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <svg class="minimap" width={MM.w} height={MM.h} onpointerdown={(e) => e.stopPropagation()} onclick={mmClick} aria-label="Minimap">
            {#each team.nodes as n (n.id)}
              <rect x={MM.pad + (n.position[0] - mm.b.x) * mm.s} y={MM.pad + (n.position[1] - mm.b.y) * mm.s} width={Math.max(6, C.NODE_W * mm.s)} height={Math.max(4, C.NODE_H * mm.s)} rx="2" fill={`var(--color-role-${roleOf(n.role_id)?.color ?? 1})`} />
            {/each}
            <rect class="vp" x={MM.pad + (-pan.x / zoom - mm.b.x) * mm.s} y={MM.pad + (-pan.y / zoom - mm.b.y) * mm.s} width={mm.vw} height={mm.vh} />
          </svg>
        {/if}
        <div class="hints">
          <span><Kbd keys="n" /> add</span><span><Kbd keys="c" /> connect</span><span><Kbd keys="⏎" /> inspect</span><span><Kbd keys="=" /> auto-layout</span><span><Kbd keys="x" /> delete</span><span><Kbd keys="u" /> undo</span><span><Kbd keys="⇧+D" /> duplicate</span><span><Kbd keys="⌃+r" /> redo</span>
        </div>
      {:else}
        <div class="empty">
          <p><b>No team open.</b></p>
          <p class="muted">Open one from the library (⇧R) or start a new one.</p>
          <button class="btn" onclick={newTeam}>New team</button>
        </div>
      {/if}

      {#if mode?.kind === 'connect'}
        <div class="menu" onpointerdown={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-label="Connect">
          {#if !mode.to}
            <b>Connect {mode.from} → …</b>
            <span class="muted">pick a target: its number or a click · esc cancel</span>
          {:else}
            <b>Connect {mode.from} → {mode.to}</b>
            {#each C.EDGE_KINDS as k, i (k)}<button class="opt" onclick={() => connect(k)}><Kbd keys={String(i + 1)} /> {k}</button>{/each}
            <span class="muted">esc cancel</span>
          {/if}
        </div>
      {:else if mode?.kind === 'add'}
        <div class="menu" onpointerdown={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-label="Add a node">
          <b>Add a node</b>
          {#each roles.slice(0, 9) as r, i (r.id)}<button class="opt" onclick={() => addRole(r.id)}><Kbd keys={String(i + 1)} /> <AgentChip id={r.id} role={r.id} color={r.color} name={r.name} /></button>{/each}
          <span class="muted">esc cancel</span>
        </div>
      {:else if mode?.kind === 'library'}
        <div class="menu lib" onpointerdown={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-label="Library">
          <b>Library</b>
          <div class="presets">
            {#each view?.teams ?? [] as t (t.team.id)}
              <div class="preset" class:on={team?.id === t.team.id}>
                <button class="popen" onclick={() => open(t)}>
                  <span class="pname">{t.team.name}{#if t.preset}<span class="tag">{t.starter ? 'Starter · read-only' : 'Preset'}</span>{/if}{#if t.update_available}<span class="tag upd">Update available</span>{/if}</span>
                  <span class="roles">{#each t.team.nodes as n (n.id)}<AgentChip id={n.id} role={n.role_id} compact />{/each}</span>
                  <span class="muted">v{t.team.version} · {t.team.scope?.kind === 'project' ? t.team.scope.id : 'global'}{t.issues.length ? ` · ${t.issues.length} issue(s)` : ''}</span>
                </button>
                {#if t.update_available}<span class="muted upline">The {t.team.name} starter changed in this version. Your copy was edited, so it wasn't updated. Duplicate the new starter to compare.</span>{/if}
                <div class="pacts">
                  {#if t.update_available && t.starter}<button class="link" onclick={() => getNewStarter(t)}>Get the new starter</button>{/if}
                  <button class="link" onclick={() => launchPreset(t.team)}>Launch</button>
                  <button class="link" onclick={() => (mode = { kind: 'dup', what: { kind: 'team', team: t.team } })}>Duplicate</button>
                  {#if !t.starter}<button class="link" onclick={() => setPreset(t.team, !t.preset)}>{t.preset ? 'Remove from presets' : 'Save as preset'}</button>{/if}
                </div>
              </div>
            {/each}
          </div>
          <button class="btn ghost" onclick={newTeam}>New team</button>
        </div>
      {:else if mode?.kind === 'starter'}
        <div class="menu" onpointerdown={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-label="Starter">
          <b>Starter · read-only</b>
          <span>{mode.rolePatch ? `Role ${mode.role.name}` : team?.name} is a starter. Starters are templates. Your changes will be saved as a copy.</span>
          {#if project}<button class="opt" onclick={() => copyStarter('project')}><Kbd keys="⏎" /> Save in {projectName}</button>{/if}
          <button class="opt" onclick={() => copyStarter('global')}><Kbd keys={project ? 'g' : '⏎'} /> Global</button>
          <span class="muted">esc cancel (the starter is left as it is)</span>
        </div>
      {:else if mode?.kind === 'dup'}
        {@const w = mode.what}
        <div class="menu" onpointerdown={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-label="Duplicate">
          <b>Duplicate {w?.kind === 'role' ? `role ${w.role.id}` : (w?.team?.name ?? 'team')}</b>
          <button class="opt" onclick={() => duplicate('global', w)}>Duplicate to global <Kbd keys="D" /><Kbd keys="1" /></button>
          <button class="opt" disabled={!project} onclick={() => duplicate('project', w)}>Duplicate to {projectName ?? 'a project (open the canvas from one)'} <Kbd keys="D" /><Kbd keys="2" /></button>
          <span class="muted">A copy at version 1, as &lt;id&gt;-copy · esc cancel</span>
        </div>
      {:else if mode?.kind === 'import'}
        <div class="menu lib" onpointerdown={(e) => e.stopPropagation()} role="dialog" tabindex="-1" aria-label="Import a team">
          <b>Import a team (JSON)</b>
          <textarea class="field mono" rows="8" bind:value={mode.text} placeholder="Paste an exported team file"></textarea>
          <div class="row"><button class="btn primary" onclick={() => importTeam(mode.text)}>Import</button><button class="btn ghost" onclick={() => (mode = null)}>Cancel</button></div>
        </div>
      {/if}
    </div>

    <aside class="side">
      <section class="card inspector" aria-label="Role inspector">
        {#if node && role}
          <div class="ih">
            <AgentChip id={node.id} role={node.role_id} color={role.color} />
            <span class="muted">role {role.name} v{role.version}</span>
          </div>
          <label><span>role</span>
            <select class="field" value={node.role_id} onchange={(e) => edit(C.updateNode(team, node.id, { role_id: e.currentTarget.value }))}>
              {#each roles as r (r.id)}<option value={r.id}>{r.name}</option>{/each}
            </select>
          </label>
          {#key `${node.id}:${node.role_id}`}
            <ModelSelect label="model" value={role.model} onsave={(model) => saveRole({ model })} />
            <ModelSelect label="node model" value={node.overrides?.model} emptyLabel={role.model ? `role: ${role.model}` : undefined} onsave={(model) => edit(C.setNodeModel(team, node.id, model))} />
          {/key}
          <label><span>effort</span>
            <select class="field" value={role.effort ?? ''} onchange={(e) => saveRole({ effort: e.currentTarget.value || null })}>
              {#each ['', 'low', 'medium', 'high'] as v (v)}<option value={v}>{v || 'default'}</option>{/each}
            </select>
          </label>
          <div class="lab"><span>tools</span>
            <span class="checks">
              {#each ['edit', 'bash', 'web'] as k (k)}<label class="ck"><input type="checkbox" checked={role.tools[k]} onchange={(e) => saveRole({ tools: { ...role.tools, [k]: e.currentTarget.checked } })} /> {k}</label>{/each}
              <label class="ck"><input type="checkbox" checked={role.subagents_allowed} onchange={(e) => saveRole({ subagents_allowed: e.currentTarget.checked })} /> subagents</label>
            </span>
          </div>
          <label><span>permissions</span>
            <select class="field" value={role.permission_profile} onchange={(e) => saveRole({ permission_profile: e.currentTarget.value })}>
              {#each ['strict', 'gated', 'autonomous'] as v (v)}<option value={v}>{v}</option>{/each}
            </select>
          </label>
          <RoleMcp {role} servers={registry} isLead={node.is_lead} {project} onsave={saveRole} />
          <label><span>max live</span><input class="field" type="number" min="1" max="9" value={node.max_concurrency} disabled={node.is_lead} onchange={(e) => edit(C.updateNode(team, node.id, { max_concurrency: Math.max(1, Number(e.currentTarget.value) || 1) }))} /></label>
          <label><span>budget</span>
            <select class="field" value={role.budget_priority} onchange={(e) => saveRole({ budget_priority: e.currentTarget.value })}>
              {#each ['low', 'normal', 'high', 'critical'] as v (v)}<option value={v}>{v}</option>{/each}
            </select>
          </label>
          <label><span>turn ceiling</span><input class="field" type="number" min="0" placeholder="minutes (none)" value={role.turn_ceiling?.minutes ?? ''} onchange={(e) => saveRole({ turn_ceiling: { ...(role.turn_ceiling ?? {}), minutes: Number(e.currentTarget.value) > 0 ? Number(e.currentTarget.value) : null } })} /></label>
          <label class="col"><span>prompt</span><textarea class="field" rows="4" value={role.prompt} onchange={(e) => saveRole({ prompt: e.currentTarget.value })}></textarea></label>
          <div class="row">
            {#if !node.is_lead}<button class="btn ghost" onclick={() => edit(C.setLead(team, node.id))}>Make lead</button>{/if}
            <button class="btn ghost" onclick={() => (mode = { kind: 'connect', from: node.id })}>Connect <Kbd keys="c" /></button>
            <button class="btn ghost" onclick={removeSelection}>Delete <Kbd keys="x" /></button>
          </div>
          {#if team.edges.some((e) => e.from === node.id || e.to === node.id)}
            <div class="edges-list">
              {#each team.edges as e, i (`${e.from}-${e.to}-${e.kind}`)}
                {#if e.from === node.id || e.to === node.id}
                  <div class="erow"><span>{e.from} {e.kind} {e.to}</span><button class="x" aria-label="Remove edge" onclick={() => edit(C.removeEdge(team, i))}>×</button></div>
                {/if}
              {/each}
            </div>
          {/if}
          <p class="muted note">Role fields apply to every node with this role; max live is this node's.</p>
        {:else}
          <p class="muted">Select a node (click or hjkl) to edit its role.</p>
        {/if}
      </section>

      <section class="card rules" aria-label="Rules">
        <div class="rh"><b>Rules</b><span class="grow"></span>{#if issues.length}<span class="err">{issues.length} error{issues.length === 1 ? '' : 's'}</span>{:else if status === 'saved'}<span class="ok">valid</span>{:else}<span class="muted">{status}…</span>{/if}</div>
        {#each issues as i (i.text)}<div class="err">{'\u{F0028}'} {i.text}</div>{/each}
        <label class="lab"><span>review</span>
          <select class="field" value={team?.review ?? 'self'} disabled={!team} onchange={(e) => edit({ ...team, review: e.currentTarget.value })}>
            <option value="self">self: /code-review, then /simplify</option>
            <option value="node">by the reviewer node</option>
            <option value="none">by the lead</option>
          </select>
        </label>
        {#each lines as [who, what] (who)}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="rule" onclick={() => (sel = who)}><b>{who}</b> <span>{what}</span></div>
        {/each}
      </section>
    </aside>
  </div>
</div>

<style>
  .canvas-pane {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    padding: var(--space-lg);
    outline: none;
    font: var(--text-label);
  }
  .ch {
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }
  .name {
    font: var(--text-title);
    color: var(--color-text-primary);
    background: none;
    border: var(--border-width-default) solid transparent;
    border-radius: var(--radius-sm);
    padding: 0 var(--space-xs);
    width: 18em;
  }
  .name:focus {
    border-color: var(--color-focus);
    outline: none;
  }
  .main {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: var(--space-md);
  }
  .area {
    position: relative;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    background: var(--color-surface-chrome);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    touch-action: none;
  }
  .world {
    position: absolute;
    left: 0;
    top: 0;
    transform-origin: 0 0;
  }
  .edges {
    position: absolute;
    left: 0;
    top: 0;
    overflow: visible;
  }
  .edge {
    cursor: pointer;
  }
  .elabel {
    font: var(--text-caption);
    fill: var(--color-text-muted);
  }
  .node {
    position: absolute;
    width: 176px;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    cursor: grab;
    user-select: none;
    -webkit-user-select: none;
  }
  .node.sel {
    background: var(--color-surface-selected);
    border: var(--border-width-focus) solid var(--color-focus);
  }
  .node.bad {
    border-color: var(--color-status-error);
  }
  .node.from {
    border: var(--border-width-focus) dashed var(--color-focus);
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .lead {
    color: var(--color-status-at-gate);
    font: var(--text-caption);
  }
  .target {
    padding: 0 var(--space-xs);
    background: var(--color-accent-default);
    color: var(--color-text-inverse);
    border-radius: var(--radius-sm);
    font: var(--text-key);
  }
  .meta {
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .ierr,
  .err {
    color: var(--color-status-error);
    font: var(--text-caption);
  }
  .ok {
    color: var(--color-status-done);
    font: var(--text-caption);
  }
  .minimap {
    position: absolute;
    left: var(--space-lg);
    bottom: var(--space-lg);
    background: var(--color-surface-chrome);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .minimap .vp {
    fill: none;
    stroke: var(--color-focus);
  }
  .hints {
    position: absolute;
    left: 200px;
    bottom: var(--space-lg);
    display: flex;
    gap: var(--space-md);
    padding: var(--space-xs) var(--space-sm);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
    font: var(--text-caption);
  }
  .menu {
    position: absolute;
    right: var(--space-lg);
    top: var(--space-lg);
    width: 240px;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
  }
  .menu.lib {
    width: 320px;
  }
  .opt,
  .preset {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    background: none;
    border: 0;
    padding: var(--space-xxs) 0;
    color: var(--color-text-primary);
    font: var(--text-body);
    text-align: left;
    cursor: pointer;
  }
  .presets {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    max-height: 50vh;
    overflow: auto;
  }
  .preset {
    flex-direction: column;
    align-items: flex-start;
    padding: var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    background: var(--color-surface-pane);
  }
  .preset.on {
    border-color: var(--color-focus);
    background: var(--color-surface-selected);
  }
  .pname {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    font: var(--text-body-strong);
  }
  .popen {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-xxs);
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-text-primary);
    text-align: left;
    cursor: pointer;
  }
  .tag {
    padding: 0 var(--space-xs);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
    font: var(--text-caption);
  }
  .tag.upd {
    border-color: var(--color-status-waiting-for-you);
  }
  .upline {
    font: var(--text-caption);
  }
  .pacts {
    display: flex;
    gap: var(--space-md);
  }
  .link {
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: var(--text-caption);
    cursor: pointer;
  }
  .roles {
    display: flex;
    gap: var(--space-xs);
  }
  .side {
    width: 340px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    overflow: auto;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-md);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .ih,
  .rh,
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    flex-wrap: wrap;
  }
  .inspector label,
  .lab {
    display: grid;
    grid-template-columns: 80px 1fr;
    align-items: center;
    gap: var(--space-sm);
  }
  .inspector label > span:first-child,
  .lab > span:first-child {
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .inspector label.col {
    align-items: start;
  }
  .checks {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
  }
  .inspector label.ck {
    display: inline-flex;
    gap: var(--space-xxs);
  }
  .edges-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    font: var(--text-caption);
  }
  .erow {
    display: flex;
    justify-content: space-between;
  }
  .x {
    background: none;
    border: 0;
    color: var(--color-text-muted);
    cursor: pointer;
  }
  .rule {
    cursor: pointer;
    font: var(--text-caption);
    color: var(--color-text-secondary);
  }
  .rule b {
    color: var(--color-text-primary);
    font: var(--text-label);
  }
  .note {
    margin: 0;
    font: var(--text-caption);
  }
  .empty {
    padding: var(--space-2xl);
  }
</style>
