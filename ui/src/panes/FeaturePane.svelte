<script>
  // Feature pane (§9.3, Penpot 04 Feature · 1512): the feature's latest team
  // run: docs from the integration branch (rendered), tasks, the team with
  // live states, the timeline (FR-408) with a message box (FR-409), artifacts
  // and the PR, and the Design section (P3). Keys: e open the doc in nvim, b
  // Tasks (focus), w show the source in Work, c canvas, p pause / resume the team,
  // m message, d Design, N publish / refresh the docs in Notion (FR-505). While Tasks has
  // focus its own keys apply and the pane's e / m / p do not (notes §12.5).
  import { onMount, untrack } from 'svelte';
  import AgentChip from '../components/AgentChip.svelte';
  import DesignSection from '../components/DesignSection.svelte';
  import SourceHeader from '../components/feature/SourceHeader.svelte';
  import TasksSection from '../components/feature/TasksSection.svelte';
  import Markdown from '../components/Markdown.svelte';
  import Menu from '../components/Menu.svelte';
  import PrLine from '../components/PrLine.svelte';
  import Select from '../components/Select.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import { app, boardNotice, confirm, openAgent, openCanvas, openEditor, openUrl, refresh, toast } from '../lib/app.svelte.js';
  import { deliveryMark } from '../lib/delivery.js';
  import { isDesign } from '../lib/ecosystem.js';
  import { ago, dayHm, plural } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { isTypingTarget } from '../lib/keyboard.js';
  import { onTopics, runAttention, runPhase, statusOf } from '../lib/now.svelte.js';
  import { docRepoPath, runFinished, runStateColor, runStateLabel, TL_FILTERS, timelineFilter, timelineText } from '../lib/teams.js';
  import { tooltip } from '../lib/tooltip.js';
  import { openWork } from '../lib/work.js';

  let { paneId } = $props();
  const spec = $derived(app.panes[paneId]);
  let v = $state(null);
  let error = $state(null);
  let doc = $state({ name: null, text: '' });
  let tl = $state('all');
  // Opened from an undelivered chip: the timeline starts on Delivery.
  $effect(() => {
    if (spec?.timeline) {
      tl = spec.timeline;
      // One-shot: the saved layout must not reopen the pane on Delivery.
      delete spec.timeline;
    }
  });
  let to = $state(null); // a node id; null = the lead (whatever its id)
  let msg = $state('');
  let sending = $state(false); // a message on its way: a second click or ⏎ does not send it twice
  let root;
  let msgBox = $state(null);
  let design = $state(null);
  let tasksSec = $state(null);
  // Notion (FR-505): what the core says, and whether Publish can work at all.
  let notion = $state(null); // notion_status
  let notionReady = $state(null); // null = unknown; else { ok, why }
  let publishing = $state(false);
  let publishError = $state(null);

  const run = $derived(v?.run ?? null);
  const toOptions = $derived((v?.team?.nodes ?? []).map((n) => ({ value: n.id, label: `${n.id}${n.is_lead ? ' (lead)' : ''}` })));
  const paused = $derived(run?.status === 'paused');
  // The run's phase as the core's run record has it (a finished run has none: its status).
  const rr = $derived(run ? runAttention(run.id) : null);
  const phase = $derived(runPhase(run));
  /** Row 2 of the header: the pieces present, joined by ` · ` (the branch piece is always there). */
  const metaText = $derived(
    v
      ? [
          v.team ? `${v.team.name} v${v.team.version}` : null,
          `\u{E725} ${v.feature.integration_worktree ? `orch/${v.feature.slug}/integration` : '—'}`,
          run ? `started ${ago(run.started_at_ms)}` : null,
        ]
          .filter(Boolean)
          .join(' · ')
      : '',
  );
  /** A tooltip param read when the tooltip shows: the full text, only when the element is cut (ellipsis). */
  const ifCut = (text) => (node) => (node.scrollWidth > node.clientWidth ? text : null);

  const events = $derived((v?.timeline ?? []).filter(timelineFilter(tl)).slice(0, 60));
  const lead = $derived(v?.team?.nodes?.find((n) => n.is_lead)?.id ?? 'lead');
  /** An agent's role from the run's own team (app_state may not list a new agent yet). */
  const roleOfAgent = (id) => {
    const node = v?.agents?.find((a) => a.id === id)?.node;
    return v?.team?.nodes?.find((n) => n.id === node)?.role_id ?? null;
  };
  /** The agent id without the `<feature-slug>-` prefix (`back-1`); the full id stays in the tooltip. */
  const shortName = (id) => {
    const prefix = `${v?.feature?.slug}-`;
    return id.startsWith(prefix) ? id.slice(prefix.length) : id;
  };

  async function load() {
    try {
      v = await invoke('feature_view', { feature: spec.featureId });
      error = null;
      // Default the message box to the lead's node, whatever its id.
      if (!to || !v.team?.nodes?.some((n) => n.id === to)) to = v.team?.nodes?.find((n) => n.is_lead)?.id ?? null;
      const names = v.docs ?? [];
      // A doc asked for by name (the Overwatch's source links) wins over the first one.
      const want = requestedDoc();
      if (want) await showDoc(want);
      else if (names.length && (!doc.name || !names.includes(doc.name))) await showDoc(names[0]);
    } catch (e) {
      error = errText(e);
    }
  }

  /** Notion status; the parent page and the token only until both are set
   *  (then Publish can work, and only the status changes). */
  async function loadNotion() {
    if (!v?.feature) return;
    const pid = v.feature.project_id;
    const ready = notionReady?.ok;
    const [st, ps, iv] = await Promise.all([
      invoke('notion_status', { feature: v.feature.id }).catch(() => null),
      ready ? null : invoke('project_settings', { project: pid }).catch(() => null),
      ready ? null : invoke('integrations_view').catch(() => null),
    ]);
    notion = st;
    if (ready) return;
    const page = ps?.settings?.notion_parent_page;
    const conn = iv?.items?.find((i) => i.id === 'notion');
    const name = v.project?.name ?? pid;
    notionReady =
      page && conn?.has_secret
        ? { ok: true }
        : { ok: false, why: `Set a Notion parent page in ${name}'s settings, and connect Notion in Settings › Integrations.` };
  }


  async function publish() {
    if (publishing || !notionReady?.ok) return;
    publishing = true;
    publishError = null;
    try {
      const r = await invoke('notion_publish', { feature: v.feature.id });
      notion = { ...(notion ?? {}), page_url: r.page_url, published_at_ms: r.published_at_ms, error: null };
      toast(`Published ${r.docs.length} docs to Notion`, { action: { label: 'Open', run: () => openUrl(r.page_url) } });
    } catch (e) {
      publishError = errText(e);
    } finally {
      publishing = false;
    }
  }

  async function showDoc(name) {
    try {
      doc = { name, text: await invoke('feature_doc', { feature: spec.featureId, name }) };
    } catch (e) {
      doc = { name, text: `(${errText(e)})` };
    }
  }

  // The doc a source link asked for, once: it is cleared from the pane as it is shown, so a later
  // click on another tab stays and a reload doesn't snap back.
  let anchor = $state(null);
  let anchorAsked = $state(0); // a second click on the same section scrolls back to it
  function requestedDoc() {
    const want = spec?.doc;
    if (!want || !v?.docs) return null;
    anchor = spec.anchor ?? null;
    anchorAsked = untrack(() => anchorAsked) + 1;
    app.panes[paneId] = { ...app.panes[paneId], doc: null, anchor: null }; // one-shot, also when it is not one of the docs
    return v.docs.includes(want) ? want : null;
  }
  // The same pane asked for another doc (a source link while it is open).
  $effect(() => {
    const want = requestedDoc();
    if (want) showDoc(want);
  });

  /** A link to a sibling doc (`api-contract.md#…`) switches the tab. */
  function openSibling(name) {
    anchor = null;
    if (v.docs.includes(name)) return showDoc(name);
    toast(`No doc named ${name} in this feature`, { kind: 'error' });
  }

  async function act(action) {
    try {
      await invoke('run_act', { run: run.id, action });
      toast(`${{ pause: 'Paused', resume: 'Resumed', stop: 'Stopped' }[action]} the team · ${v.feature.slug}`);
      await load();
      await refresh();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }

  function stop() {
    confirm(
      `Stop the team on ${v.feature.slug}?`,
      'Queued work is dropped and running agents are suspended (their sessions and worktrees stay). The run ends as stopped.',
      () => act('stop'),
      'Stop the team',
    );
  }

  async function send() {
    const body = msg.trim();
    if (!body || !run || !to || sending) return;
    sending = true;
    try {
      await invoke('message_send', { run: run.id, to, body });
      msg = '';
      toast(`Sent to ${to}`);
      await load();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    } finally {
      sending = false;
    }
  }

  // The shown doc's repo-relative path (R10), null when it is not in the repo.
  const docPath = $derived(v ? docRepoPath(v.feature, doc.name) : null);

  // The header's `⋯` (R9): Board, Canvas, Publish / Refresh in Notion, Open in Notion; keys that work in the pane.
  const menu = $derived.by(() => {
    if (!v) return [];
    const nr = notionReady;
    return [
      { id: 'work', icon: '\u{F0570}', label: 'Show source in Work', key: 'w', disabled: !canShowInWork, reason: 'No tracker item for this feature', run: showInWork },
      { id: 'canvas', icon: '\u{F1546}', label: 'Canvas', key: 'c', disabled: !run, reason: 'No team run yet', run: () => openCanvas(run.team_id, v.feature.project_id) },
      { separator: true },
      {
        id: 'notion',
        icon: '\u{F0552}',
        label: notion?.page_url ? 'Refresh in Notion' : 'Publish to Notion',
        key: 'N',
        disabled: publishing || !nr?.ok,
        reason: publishing ? 'Publishing…' : (nr?.why ?? 'Checking Notion…'),
        run: publish,
      },
      { id: 'notion-open', icon: '\u{F03CC}', label: 'Open in Notion', disabled: !notion?.page_url, reason: 'Not published yet', run: () => openUrl(notion.page_url) },
    ];
  });

  const canShowInWork = $derived(!!run?.source && !run.source.deleted);
  const showInWork = () => openWork({ projects: [v.feature.project_id], reveal: run?.source?.node ?? null });

  /** `b`: Tasks takes the focus (the first time after the update, the Board's removal toast too). */
  function focusTasks() {
    boardNotice();
    tasksSec?.focus();
  }

  // One-shot landing request (a task link, the run inspector's "Show tasks"): the same pane keeps the
  // request while it loads; Tasks consumes it by its `seq`.
  const land = $derived(spec?.land ?? null);
  let landedTasks = 0;
  $effect(() => {
    if (land?.tasks && !land.task && v && land.seq !== landedTasks) {
      landedTasks = land.seq;
      untrack(() => tasksSec?.focus());
    }
  });

  function openDoc() {
    const wt = v?.feature?.integration_worktree;
    if (!wt || !docPath) return;
    openEditor(`${wt}/${docPath}`, null, { projectId: v.feature.project_id });
  }

  function onKey(e) {
    if (isTypingTarget(e.target)) {
      if (e.key === 'Escape' && !e.defaultPrevented) (root?.focus(), e.preventDefault());
      return;
    }
    // Tasks has focus: its keys (handled there) shadow the pane's e / m / p only.
    if (e.target.closest?.('[data-tasks]') && ['e', 'm', 'p'].includes(e.key)) return;
    if (e.metaKey || e.ctrlKey || e.altKey || !v) return;
    const k = e.key;
    if (k === 'e') openDoc();
    else if (k === 'b') focusTasks();
    else if (k === 'w' && canShowInWork) showInWork();
    else if (k === 'c' && run) openCanvas(run.team_id, v.feature.project_id);
    else if (k === 'p' && run && !runFinished(run.status)) act(paused ? 'resume' : 'pause');
    else if (k === 'm') msgBox?.focus();
    else if (k === 'd') design?.focus();
    else if (k === 'N') publish();
    else return;
    e.preventDefault();
  }

  // Notion: once the feature is known (whichever load brings it), then on its topics.
  const featureKnown = $derived(v?.feature?.id ?? null);
  $effect(() => {
    if (featureKnown) untrack(loadNotion);
  });

  // Refetched when the core says the feature or its team changed (no poll);
  // the Notion status when an integration or the project's settings (its
  // parent page) changed.
  onMount(() => {
    const id = spec.featureId;
    const stop = onTopics([`feature:${id}`, 'features', 'teams'], load);
    const notionTopic = (t) => t === 'integrations' || t === `feature:${id}` || (!!v?.feature && t === `project:${v.feature.project_id}`);
    const stopNotion = onTopics(notionTopic, () => featureKnown && !publishing && loadNotion(), { focus: false });
    return () => (stop(), stopNotion());
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="feature" tabindex="0" data-pane-focus bind:this={root} onkeydown={onKey} role="region" aria-label="Feature">
  {#if error}
    <p class="muted pad">Could not load the feature: {error}</p>
  {:else if !v}
    <p class="muted pad">Loading…</p>
  {:else}
    <header class="fh">
      <div class="row1">
        <span class="slug" use:tooltip={ifCut(v.feature.slug)}>{v.feature.slug}</span>
        {#if phase}<span class="pill" style:color={runStateColor(phase)} style:border-color={runStateColor(phase)} use:tooltip={rr ? `since ${ago(rr.phase_since_ms)}` : null}>{runStateLabel(phase)}</span>{/if}
        <span class="grow"></span>
        {#if publishing}
          {@const pubText = `Publishing ${plural(v.docs.length, 'doc')}…`}
          <span class="muted pub"><span class="glyph" aria-hidden="true">◷</span><span class="pubtxt" use:tooltip={ifCut(pubText)}>{pubText}</span></span>
        {/if}
        <span class="head-acts">
          {#if run && !runFinished(run.status)}
            <button type="button" class="icon-btn" aria-label={paused ? 'Resume team' : 'Pause team'} use:tooltip={{ label: paused ? 'Resume team' : 'Pause team', key: 'p' }} onclick={() => act(paused ? 'resume' : 'pause')}>{paused ? '\u{F040A}' : '\u{F03E4}'}</button>
            <button type="button" class="icon-btn" aria-label="Stop the team…" use:tooltip={'Stop the team…'} onclick={stop}>{'\u{F04DB}'}</button>
          {/if}
          <button type="button" class="icon-btn danger" aria-label="Delete feature…" use:tooltip={'Delete feature…'} onclick={() => (app.overlay = { kind: 'delete-feature', featureId: v.feature.id })}>{'\u{F01B4}'}</button>
          <Menu items={menu} label="More actions" />
        </span>
      </div>
      <div class="row2 muted" use:tooltip={ifCut(metaText)}>{metaText}</div>
    </header>

    <SourceHeader {run} brief={v.run?.brief ?? v.feature?.brief ?? null} project={v.feature.project_id} reload={load} />

    {#if notionReady && !notionReady.ok && !notion?.page_url}<p class="nline muted">{notionReady.why}</p>{/if}
    {#if publishError || notion?.error}
      <p class="nerr">✕ Publish failed: {publishError ?? notion.error}</p>
    {:else if notion?.page_url}
      <p class="nline"><span aria-hidden="true">{'\u{F0219}'}</span> Published to Notion {notion.published_at_ms ? dayHm(notion.published_at_ms) : ''}{notion.comments_seen ? ` · ${notion.comments_seen} comment${notion.comments_seen === 1 ? '' : 's'}` : ''}</p>
    {/if}

    <div class="cols">
      <div class="left">
        <section class="card">
          <div class="cardh"><b>Docs</b><span class="muted">on the integration branch</span></div>
          {#if v.docs.length}
            <div class="tabs" role="tablist">
              {#each v.docs as d (d)}<button class="tab" class:on={doc.name === d} role="tab" aria-selected={doc.name === d} onclick={() => ((anchor = null), showDoc(d))}>{d}</button>{/each}
            </div>
            {#if docPath}
              <div class="docpath" data-docpath>
                <span class="mono muted ell" use:tooltip={docPath}>{docPath}</span>
                <button type="button" class="icon-btn" aria-label="Open in nvim" use:tooltip={{ label: 'Open in nvim', key: 'e' }} onclick={openDoc}>{'\u{F11E7}'}</button>
              </div>
            {/if}
            <div class="rendered"><Markdown text={doc.text} feature={spec.featureId} ondoc={openSibling} {anchor} {anchorAsked} /></div>
          {:else}
            <p class="muted">No docs yet: the lead writes them in the spec step.</p>
          {/if}
        </section>

        <DesignSection bind:this={design} {v} />

      </div>

      <div class="right">
        <section class="card">
          <div class="cardh"><b>Team</b><span class="muted">{v.agents.length} agent{v.agents.length === 1 ? '' : 's'}</span></div>
          {#if v.agents.length}
            <div class="agents">
              {#each v.agents as a (a.id)}
                <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                <div class="trow" onclick={() => openAgent(a.id)} use:tooltip={`Open ${a.id}`}>
                  <AgentChip id={a.id} role={roleOfAgent(a.id)} compact />
                  <span class="name ell">{shortName(a.id)}</span>
                  <span class="act muted ell">{a.tool || (a.task ? `task T-${a.task}` : '')}{a.flag ? ` · ${a.flag}` : ''}</span>
                  <span class="st"><StatusBadge state={statusOf(a.id)} trailing /></span>
                </div>
              {/each}
            </div>
          {:else}<p class="muted">No agent yet.</p>{/if}
        </section>

        <section class="card tl">
          <div class="cardh"><b>Timeline</b><span class="muted">messages · tasks · merges · gates · governor</span></div>
          <div class="chips">{#each TL_FILTERS as f (f.id)}<button class="chip" class:on={tl === f.id} onclick={() => (tl = f.id)}>{f.label}</button>{/each}</div>
          <div class="events">
            {#each events as e (e.id)}
              {@const m = deliveryMark(e)}
              <div class="ev"><span class="evtime mono muted">{new Date(e.ts_ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>{#if e.agent_id}<AgentChip id={e.agent_id} role={roleOfAgent(e.agent_id)} compact />{/if}<span class="evt">{timelineText(e)}</span>{#if m}<span class="dmark" class:failed={e.delivery.state === 'failed'} title={m.title} aria-label={m.title}>{m.glyph}{m.redelivered}</span>{/if}</div>
            {:else}<span class="muted">Nothing yet.</span>{/each}
          </div>
          {#if run && !runFinished(run.status)}
            <div class="msg">
              <Select width="hug" bind:value={to} options={toOptions} aria-label="To" noun="node" />
              <input class="field grow" bind:this={msgBox} bind:value={msg} placeholder="Message {to === lead ? 'the lead' : to}…" onkeydown={(e) => e.key === 'Enter' && (e.preventDefault(), send())} />
              <button type="button" class="icon-btn" aria-label="Send" disabled={!msg.trim() || sending} use:tooltip={{ label: 'Send', key: '⏎' }} onclick={send}>{'\u{F048A}'}</button>
            </div>
          {/if}
        </section>

        <section class="card">
          <div class="cardh"><b>PR and artifacts</b></div>
          {#if run?.pr_url}<PrLine {run} feature={spec.featureId} onrun={(r) => (v = { ...v, run: r })} />
          {:else}<p class="muted">No PR yet: the lead asks with request_pr after integration (PR gate).</p>{/if}
          {#each v.artifacts.filter((a) => !isDesign(a)) as a (a.id)}
            <div class="art"><span class="muted">{a.kind}</span> <span class="ell">{a.uri}</span> <span class="muted">({a.from_node}{a.to_node ? ` → ${a.to_node}` : ''})</span></div>
          {/each}
          {#if v.artifacts.some(isDesign)}<p class="muted nline">Design artifacts moved to the Design section.</p>{/if}
        </section>
      </div>
    </div>

    <TasksSection bind:this={tasksSec} {v} reload={load} {land} onleave={() => root?.focus()} />
  {/if}
</div>

<style>
  .dmark {
    flex: none;
    margin-left: var(--space-xs);
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .dmark.failed {
    color: var(--color-status-error);
  }

  .docpath {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    min-width: 0;
    font: var(--text-caption);
  }
  .docpath .ell {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .nline,
  .nerr {
    margin: 0;
    font: var(--text-caption);
  }
  .nerr {
    padding: var(--space-sm) var(--space-md);
    border: var(--border-width-default) solid var(--color-status-error);
    border-radius: var(--radius-sm);
  }
  .feature {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    padding: var(--space-lg);
    outline: none;
    font: var(--text-label);
  }
  .cardh,
  .ev,
  .msg {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .fh {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
  }
  .row1 {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: var(--size-control);
  }
  /* One line, cut with an ellipsis (the tooltip shows the full text when it is cut). */
  .row2,
  .slug,
  .pubtxt {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .row2 {
    font: var(--text-caption);
  }
  .slug {
    font: var(--text-heading);
    color: var(--color-text-primary);
    /* Shrinks first: the Publishing text only gives way once the slug is down to its minimum. */
    flex: 0 100 auto;
    min-width: 0;
  }
  .pill,
  .head-acts {
    flex: none;
  }
  .pub {
    display: flex;
    gap: var(--space-xs);
    /* Always room for the ◷ glyph; only the text after it ellipsizes. */
    min-width: 1.5em;
    flex: 0 1 auto;
  }
  .glyph {
    flex: none;
  }
  .pubtxt {
    flex: 0 1 auto;
    min-width: 0;
  }
  .pill {
    white-space: nowrap;
    padding: var(--space-xxs) var(--space-sm);
    border: var(--border-width-default) solid;
    border-radius: var(--radius-sm);
  }
  .cols {
    display: flex;
    gap: var(--space-md);
    align-items: flex-start;
  }
  .left {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
  }
  .right {
    width: 440px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-lg);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .cardh b {
    font: var(--text-title);
  }
  .tabs {
    display: flex;
  }
  .tab,
  .chip {
    padding: var(--space-xxs) var(--space-sm);
    background: none;
    border: var(--border-width-default) solid var(--color-border-subtle);
    color: var(--color-text-secondary);
    font: var(--text-label);
    cursor: pointer;
  }
  .tab + .tab {
    border-left: 0;
  }
  .tab.on,
  .chip.on {
    background: var(--color-surface-selected);
    color: var(--color-text-primary);
    border-color: var(--color-focus);
  }
  .rendered {
    padding: var(--space-md) var(--space-lg);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    max-height: 50vh;
    overflow: auto;
  }
  /* Team card: one grid, a subgrid per row, so the columns line up. */
  .agents {
    display: grid;
    grid-template-columns: auto minmax(0, max-content) minmax(0, 1fr) max-content;
    column-gap: var(--space-sm);
  }
  .trow {
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: subgrid;
    align-items: center;
    height: var(--size-row);
    padding: 0 var(--space-xs);
    margin: 0 calc(-1 * var(--space-xs));
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .trow:hover {
    background: var(--color-surface-raised);
  }
  .trow .name {
    max-width: 24ch;
  }
  /* Added to the column gap: 16px name → activity and before the status, 8px mark → name. */
  .trow .act,
  .trow .st {
    margin-left: var(--space-sm);
  }
  .ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }
  .events {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    max-height: 320px;
    overflow: auto;
  }
  .ev {
    align-items: flex-start;
    font: var(--text-caption);
  }
  .evtime {
    flex: none;
    white-space: nowrap;
  }
  .evt {
    flex: 1;
    min-width: 0;
    color: var(--color-text-secondary);
    overflow-wrap: anywhere;
  }
  .art {
    font: var(--text-caption);
  }
  .pad {
    padding: var(--space-xl);
  }
  @container (max-width: 900px) {
    .cols {
      flex-direction: column;
    }
    .right {
      width: auto;
    }
  }
</style>
