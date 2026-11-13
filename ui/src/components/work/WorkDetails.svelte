<script>
  import { app } from '../../lib/app.svelte.js';
  import { dayHm, ppAmount } from '../../lib/format.js';
  // Work › details panel (notes §3): the focused node, first painted from the tree's WorkNode; body and history
  // from `work_node` (cached by id + updated_at_ms, F36). Kinds: issue, container, project row, multi-selection.
  // Item text is untrusted: plain text only; only "Open in GitHub ↗" / "Open in Linear ↗" navigates.
  import { basisText, exactRange, modeOf } from '../../lib/intake.js';
  import { liveCapText } from '../../lib/program.js';
  import { tooltip } from '../../lib/tooltip.js';
  import {
    actionOf,
    bodyLines,
    childrenHeading,
    chipOf,
    crumbs,
    depLines,
    disabledTip,
    factsOf,
    featureOf,
    GLYPH,
    historyText,
    isContainer,
    offered,
    progressText,
    refOf,
    TRACKER_NAME,
    trackerOf,
    upstreamText,
    usable,
  } from '../../lib/workmodel.js';
  import Menu from '../Menu.svelte';
  import AppChip from './AppChip.svelte';
  import KindGlyph from './KindGlyph.svelte';

  /**
   * @type {{ node?: any, project?: any, detail?: any, trail?: any[], kids?: any[] | null, selection?: any[] | null,
   *          selectionText?: string, narrow?: boolean, readyTip?: string, projectStatus?: string, lookup?: (id: string) => any,
   *          act: (what: string, arg?: any) => void, el?: HTMLElement }}
   */
  let { node = null, project = null, detail = null, trail = [], kids = null, selection = null, selectionText = '', narrow = false, readyTip = '', projectStatus = '', lookup = (_id) => null, act, el = $bindable() } = $props();

  let bodyAll = $state(false);
  let historyAll = $state(false);
  $effect(() => {
    void node?.id;
    bodyAll = false;
    historyAll = false;
  });

  const liveCap = $derived(liveCapText(app.now.counts?.global));
  const tracker = $derived(node ? (TRACKER_NAME[trackerOf(node)] ?? '') : '');
  const chip = $derived(node ? chipOf(node, { short: narrow, readyTip, liveCap }) : null);
  const body = $derived(bodyLines(detail?.body, bodyAll));
  // `work_node(id, {children: true})` lists the first page in `child_nodes` (F36); `children` stays the counts.
  const children = $derived(kids ?? detail?.child_nodes ?? null);
  const history = $derived(detail?.history ?? []);
  const feature = $derived(node ? featureOf(node) : null);
  const est = $derived(detail?.estimate ?? node?.estimate ?? null);
  const crumbList = $derived(crumbs(trail, narrow ? 3 : 4));

  const a = (w) => (node ? actionOf(node, w) : 'hidden');
  const program = $derived(a('program'));
  const openProgram = $derived(program && typeof program === 'object' && 'open_program' in program);

  /** The buttons the node allows (§5.1), primary first. */
  const buttons = $derived.by(() => {
    if (!node) return [];
    const out = [];
    if (feature) out.push({ id: 'feature', label: feature.agent ? 'Show session' : 'Show feature', key: 'g' });
    // Deleted upstream (S14): Show feature is the only action left.
    if (node.upstream?.deleted) return out;
    const prog = offered(program)
      ? { id: 'program', label: openProgram ? 'Open program' : 'Run as program', key: 'p', tip: disabledTip(program), disabled: !usable(program) }
      : null;
    // Containers and program roots lead with the program; items lead with the starts.
    const progFirst = isContainer(node) || openProgram;
    if (prog && progFirst) out.push(prog);
    // The suggested mode leads (P4.5 modeOf): solo first when the project has no team or the item asks for solo.
    const starts = [
      ['team', 'Start team run', 't'],
      ['solo', 'Start solo', 'a'],
    ];
    if (modeOf({ mode: detail?.mode ?? node.mode, can_team: usable(a('team')) }) === 'solo') starts.reverse();
    for (const [w, label, key] of starts)
      if (offered(a(w))) out.push({ id: w, label, key, tip: disabledTip(a(w)), disabled: !usable(a(w)) });
    if (prog && !progFirst) out.push(prog);
    if (node.url) out.push({ id: 'open', label: `Open in ${tracker} ↗`, key: 'o' });
    else if (trackerOf(node) === 'local') out.push({ id: 'open', label: 'Open file', key: 'o' });
    return out;
  });

  const menu = $derived.by(() => {
    if (!node || node.upstream?.deleted) return [];
    const h = a('hide');
    return [
      ...(offered(h) ? [{ id: 'hide', label: node.hidden ? 'Unhide' : 'Hide from Ready', key: '⌫', disabled: !usable(h), reason: disabledTip(h), run: () => act('hide') }] : []),
      ...(node.children?.total > 0 ? [{ id: 'focus', label: 'Focus here', key: 'z', run: () => act('focus') }] : []),
      ...(node.url ? [{ id: 'copy', label: 'Copy link', run: () => act('copy') }] : []),
    ];
  });

  const shownKids = $derived((children ?? []).slice(0, 10));
  const deps = $derived({ blocking: depLines(node?.upstream?.blocking), blocked: depLines(node?.upstream?.blocked_by) });
  const issueLike = $derived(node && !isContainer(node) && trackerOf(node) !== 'local');
</script>

<!-- A child or dependency line; without an app state it shows the upstream state (Linear) or the progress (containers). -->
{#snippet line(n, short = true)}
  {@const c = chipOf(n, { short, liveCap })}
  {@const rest = isContainer(n) ? progressText(n) : trackerOf(n) === 'linear' ? upstreamText(n) : ''}
  <div class="kid">
    <KindGlyph node={n} />
    {#if refOf(n)}<span class="ref">{refOf(n)}</span>{/if}
    <button type="button" class="ktitle" tabindex="-1" onclick={() => act('reveal', n.id)}>{n.title}</button>
    <span class="grow"></span>
    {#if c}<AppChip chip={c} onopen={(x) => act('chip', x)} />{:else}<span class="muted">{rest || '—'}</span>{/if}
  </div>
{/snippet}

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<aside class="details" class:narrow tabindex="-1" bind:this={el} aria-label="Details">
  <div class="scroll">
    {#if selection}
      <h3 class="ttl">{selectionText}</h3>
      {#each selection as s (s.id)}
        <div class="kid"><KindGlyph node={s} /><span class="ref">{refOf(s)}</span><span>{s.title}</span></div>
      {/each}
    {:else if project}
      <div class="titleline"><KindGlyph glyph={GLYPH.project} label="Project {project.name}" /><h3 class="ttl">{project.name}</h3></div>
      <p class="muted">{[project.trackerLine, project.ready ? `Ready filter ${project.ready}` : ''].filter(Boolean).join(' · ')}</p>
      {#if projectStatus}<p class="muted">{projectStatus}</p>{/if}
      <button type="button" class="link" onclick={() => act('tracker', project.id)}>Tracker settings</button>
    {:else if node}
      {#if crumbList.length}
        <nav class="crumbs" aria-label="Breadcrumb">
          {#each crumbList as c, i (i)}
            {#if i}<span class="sep" aria-hidden="true">›</span>{/if}
            {#if c.id}<button type="button" class="link" tabindex="-1" onclick={() => act('reveal', c.id)}>{c.text}</button>{:else}<span class="muted">{c.text}</span>{/if}
          {/each}
        </nav>
      {/if}
      <div class="titleline">
        <KindGlyph {node} dim={!!node.upstream?.deleted} />
        {#if refOf(node)}<span class="ref">{refOf(node)}</span>{/if}
        <h3 class="ttl">{node.title}</h3>
      </div>
      {#if chip}<div class="chipline"><AppChip {chip} onopen={(c) => act('chip', c)} /></div>{/if}
      {#if node.upstream?.deleted}<p class="warn">Deleted in {tracker}. Only Show feature is left.</p>{/if}
      {@const facts = factsOf(node)}
      {#if facts}<p class="facts">{facts}</p>{/if}

      {#if detail?.body || !detail || detail.error}
        <section>
          <h4>{isContainer(node) ? 'Description' : 'Body'}</h4>
          {#if detail?.error}<p class="err">Could not load the body: {detail.error}</p>
          {:else if !detail}<p class="muted">Loading…</p>
          {:else}
            <p class="body">{body.lines.join('\n')}</p>
            {#if detail.body_clipped}<p class="muted">Shortened: the full text is in {tracker}.</p>{/if}
            {#if body.more}<button type="button" class="link" onclick={() => (bodyAll = true)}>Show more</button>{/if}
          {/if}
        </section>
      {/if}

      {#if node.children?.total > 0 || node.team_tasks > 0}
        <section>
          <h4>{childrenHeading(node)}</h4>
          {#each shownKids as k (k.id)}{@render line(k)}{/each}
          {#if (children?.length ?? 0) > 10 || node.children?.total > Math.max(10, children?.length ?? 0)}
            <button type="button" class="link" onclick={() => act('expand')}>+ {(node.children?.total ?? children.length) - shownKids.length} more · Show in tree</button>
          {/if}
          {#if node.team_tasks > 0}
            <button type="button" class="kid fold" onclick={() => act('fold')}><span aria-hidden="true">▸</span><KindGlyph glyph={GLYPH.fold} label="Team tasks" /><span class="muted">Team tasks · {node.team_tasks}</span></button>
          {/if}
        </section>
      {/if}

      {#if issueLike}
        <section>
          <h4>Dependencies</h4>
          {#each [['Blocking', deps.blocking], ['Blocked by', deps.blocked]] as [label, list] (label)}
            <div class="deprow">
              <span class="dl muted">{label}</span>
              <div class="dv">
                {#each list as d (d.ref)}
                  {@const known = d.outside ? null : lookup(d.id)}
                  {#if known}{@render line(known, narrow)}
                  {:else}
                    <div class="kid">
                      {#if d.outside}
                        <button type="button" class="ktitle" tabindex="-1" use:tooltip={'Outside this tracker'} onclick={() => act('url', d.url)}>{d.text} ↗</button>
                      {:else}
                        <button type="button" class="ktitle" tabindex="-1" onclick={() => act('reveal', d.id)}>{d.text}</button>
                      {/if}
                      <span class="grow"></span>
                      <span class="muted">{d.open ? '—' : 'closed'}</span>
                    </div>
                  {/if}
                {:else}<span class="muted">none</span>{/each}
              </div>
            </div>
          {/each}
        </section>
      {/if}

      {#if node.upstream?.linked_prs?.length}
        <section>
          <h4>Linked pull requests</h4>
          {#each node.upstream.linked_prs as pr (pr.number)}
            <button type="button" class="ktitle block" tabindex="-1" onclick={() => act('url', pr.url)}>#{pr.number} {pr.title ?? ''} · {pr.state}</button>
          {/each}
        </section>
      {/if}

      {#if !isContainer(node)}
        <section>
          <h4>In Orchestrator</h4>
          {#if !detail}<p class="muted">Loading…</p>
          {:else if !history.length}<p class="muted">Nothing yet. Start it here.</p>
          {:else}
            {#each historyAll ? history : history.slice(-8) as h, i (i)}
              <div class="hist" class:warn={h.what === 'writeback_failed'}>
                <span class="muted when">{dayHm(h.at_ms)}</span>
                <span>{historyText(h, node)}</span>
                {#if h.what === 'writeback_failed'}<button type="button" class="link" onclick={() => act('writeback')}>Retry</button>{/if}
              </div>
            {/each}
            {#if history.length > 8 && !historyAll}<button type="button" class="link" onclick={() => (historyAll = true)}>Show all</button>{/if}
          {/if}
        </section>
      {/if}

      <section>
        <h4>Estimate</h4>
        {#if est?.team || est?.solo}
          {#each [['team', 'Team run'], ['solo', 'Solo']] as [m, label] (m)}
            {#if est[m]}
              <div class="deprow"><span class="dl muted">{label}</span><div class="dv"><span>{exactRange(est[m])}</span><span class="muted">{basisText(est[m], m, est.size ?? null, trail[0]?.title ?? node.project)}</span></div></div>
            {/if}
          {/each}
          {#if est.room_pp != null}<div class="deprow"><span class="dl muted">Room</span><span>{ppAmount(est.room_pp)} left this week</span></div>{/if}
        {:else if offered(program)}
          <p class="muted">Run as program to see the plan's estimate.</p>
        {:else}<p class="muted">No estimate yet.</p>{/if}
      </section>
    {/if}
  </div>

  {#if selection}
    <div class="acts">
      <!-- The pane passes a selection of 2 or more only. -->
      <button type="button" class="btn primary" use:tooltip={{ label: 'Run selected as program', key: 'p' }} onclick={() => act('program-selected')}>Run selected as program</button>
      <button type="button" class="btn" use:tooltip={{ label: 'Clear', key: 'Esc' }} onclick={() => act('clear')}>Clear</button>
    </div>
  {:else if node && buttons.length}
    <div class="acts">
      {#each buttons as b, i (b.id + i)}
        <button type="button" class="btn" class:primary={i === 0} disabled={b.disabled} data-act={b.id} use:tooltip={b.disabled ? b.tip : { label: b.label, key: b.key }} onclick={() => act(b.id)}>{b.label}</button>
      {/each}
      {#if menu.length}<Menu items={menu} label="More actions" />{/if}
    </div>
  {/if}
</aside>

<style>
  .details {
    flex: none;
    width: 420px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: var(--border-width-default) solid var(--color-border-subtle);
    outline: none;
    font: var(--text-body);
  }
  .details.narrow {
    width: 360px;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: var(--space-md) var(--space-lg);
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }
  .crumbs {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
    font: var(--text-label);
  }
  .sep {
    color: var(--color-text-muted);
  }
  .titleline {
    display: flex;
    align-items: baseline;
    gap: var(--space-sm);
  }
  .ttl {
    margin: 0;
    font: var(--text-title);
    overflow-wrap: anywhere;
  }
  .ref {
    font: var(--text-mono-sm);
    color: var(--color-text-muted);
  }
  .facts {
    margin: 0;
    color: var(--color-text-muted);
    font: var(--text-label);
  }
  section {
    border-top: var(--border-width-default) solid var(--color-border-subtle);
    padding-top: var(--space-sm);
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  h4 {
    margin: 0;
    font: var(--text-body-strong);
  }
  p {
    margin: 0;
  }
  .body {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .kid {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
    font: var(--text-label);
  }
  .kid.fold {
    background: none;
    border: 0;
    padding: 0;
    color: inherit;
    cursor: pointer;
  }
  .ktitle {
    background: none;
    border: 0;
    padding: 0;
    color: var(--color-text-primary);
    font: var(--text-label);
    text-align: left;
    cursor: pointer;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ktitle.block {
    display: block;
  }
  .deprow {
    display: flex;
    gap: var(--space-md);
    font: var(--text-label);
  }
  .dl {
    flex: none;
    width: 88px;
  }
  .dv {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    min-width: 0;
    flex: 1;
  }
  .hist {
    display: flex;
    gap: var(--space-md);
    font: var(--text-label);
  }
  .when {
    flex: none;
    width: 88px;
  }
  .warn {
    color: var(--color-pacing-amber);
  }
  .acts {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-lg);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
</style>
