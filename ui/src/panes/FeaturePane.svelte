<script>
  // Feature pane (§9.3, Penpot 04 Feature · 1512): the feature's latest team
  // run: docs from the integration branch (rendered), tasks, the team with
  // live states, the timeline (FR-408) with a message box (FR-409), artifacts
  // and the PR, and the Design section (P3). Keys: e open the doc in nvim, b
  // board, c canvas, p pause / resume the team, m message, d Design,
  // N publish / refresh the docs in Notion (FR-505).
  import { onMount, untrack } from 'svelte';
  import AgentChip from '../components/AgentChip.svelte';
  import DesignSection from '../components/DesignSection.svelte';
  import Kbd from '../components/Kbd.svelte';
  import Markdown from '../components/Markdown.svelte';
  import PrLine from '../components/PrLine.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import { app, confirm, openAgent, openBoard, openCanvas, openEditor, openUrl, refresh, toast } from '../lib/app.svelte.js';
  import { isDesign } from '../lib/ecosystem.js';
  import { ago, dayHm } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { backendLabel, remoteName } from '../lib/linear.js';
  import { onTopics, runAttention, runPhase, statusOf } from '../lib/now.svelte.js';
  import { handle, runFinished, runStateColor, runStateLabel, TL_FILTERS, timelineFilter, timelineText } from '../lib/teams.js';

  let { paneId } = $props();
  const spec = $derived(app.panes[paneId]);
  let v = $state(null);
  let error = $state(null);
  let doc = $state({ name: null, text: '' });
  let tl = $state('all');
  let to = $state(null); // a node id; null = the lead (whatever its id)
  let msg = $state('');
  let root;
  let msgBox = $state(null);
  let design = $state(null);
  // Notion (FR-505): what the core says, and whether Publish can work at all.
  let notion = $state(null); // notion_status
  let notionReady = $state(null); // null = unknown; else { ok, why }
  let publishing = $state(false);
  let publishError = $state(null);

  const run = $derived(v?.run ?? null);
  const paused = $derived(run?.status === 'paused');
  // The run's phase as the core's run record has it (a finished run has none: its status).
  const rr = $derived(run ? runAttention(run.id) : null);
  const phase = $derived(runPhase(run));

  const byStatus = (s) => (v?.tasks ?? []).filter((t) => t.status === s);
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
      if (names.length && (!doc.name || !names.includes(doc.name))) await showDoc(names[0]);
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
        : { ok: false, why: `Set a Notion parent page in ${name}'s settings, and connect Notion (⌘+,).` };
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

  /** A link to a sibling doc (`api-contract.md#…`) switches the tab. */
  function openSibling(name) {
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
    if (!body || !run || !to) return;
    try {
      await invoke('message_send', { run: run.id, to, body });
      msg = '';
      toast(`Sent to ${to}`);
      await load();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }

  function openDoc() {
    const wt = v?.feature?.integration_worktree;
    if (!wt || !doc.name) return;
    openEditor(`${wt}/.orchestrator/features/${v.feature.slug}/${doc.name}`, null, { projectId: v.feature.project_id });
  }

  function onKey(e) {
    if (e.target.closest('input, textarea, select')) {
      if (e.key === 'Escape') (root?.focus(), e.preventDefault());
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey || !v) return;
    const k = e.key;
    if (k === 'e') openDoc();
    else if (k === 'b') openBoard(v.feature.project_id);
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
      <span class="slug">{v.feature.slug}</span>
      {#if phase}<span class="pill" style:color={runStateColor(phase)} style:border-color={runStateColor(phase)} title={rr ? `since ${ago(rr.phase_since_ms)}` : null}>{runStateLabel(phase)}</span>{/if}
      <span class="muted">{v.team?.name ?? ''}{v.team ? ` v${v.team.version}` : ''} · {'\u{E725}'} {v.feature.integration_worktree ? `orch/${v.feature.slug}/integration` : '—'}{run ? ` · started ${ago(run.started_at_ms)}` : ''}</span>
      <span class="grow"></span>
      {#if publishing}
        <span class="muted">◷ Publishing {v.docs.length} doc{v.docs.length === 1 ? '' : 's'}…</span>
      {:else if notionReady}
        <button class="btn ghost" disabled={!notionReady.ok} title={notionReady.ok ? '' : notionReady.why} onclick={publish}>{notion?.page_url ? 'Refresh in Notion' : 'Publish to Notion'} <Kbd keys="N" /></button>
      {/if}
      <button class="btn ghost" onclick={() => openBoard(v.feature.project_id)}>Board <Kbd keys="b" /></button>
      {#if run}<button class="btn ghost" onclick={() => openCanvas(run.team_id, v.feature.project_id)}>Canvas <Kbd keys="c" /></button>{/if}
      {#if run && !runFinished(run.status)}
        <button class="btn" onclick={() => act(paused ? 'resume' : 'pause')}>{paused ? 'Resume team' : 'Pause team'} <Kbd keys="p" /></button>
        <button class="btn danger" onclick={stop}>Stop</button>
      {/if}
      <button class="btn danger" onclick={() => (app.overlay = { kind: 'delete-feature', featureId: v.feature.id })}>Delete…</button>
    </header>

    {#if notionReady && !notionReady.ok && !notion?.page_url}<p class="nline muted">{notionReady.why}</p>{/if}
    {#if publishError || notion?.error}
      <p class="nerr">✕ Publish failed: {publishError ?? notion.error}</p>
    {:else if notion?.page_url}
      <p class="nline"><span aria-hidden="true">{'\u{F0219}'}</span> Published {notion.published_at_ms ? dayHm(notion.published_at_ms) : ''} · <button class="link" onclick={() => openUrl(notion.page_url)}>Open in Notion ↗</button>{notion.comments_seen ? ` · ${notion.comments_seen} comment${notion.comments_seen === 1 ? '' : 's'}` : ''}</p>
    {/if}

    <div class="cols">
      <div class="left">
        <section class="card">
          <div class="cardh"><b>Docs</b><span class="muted">.orchestrator/features/{v.feature.slug}/ on the integration branch</span><span class="grow"></span>{#if doc.name}<button class="btn ghost" onclick={openDoc}>Open in nvim <Kbd keys="e" /></button>{/if}</div>
          {#if v.docs.length}
            <div class="tabs" role="tablist">
              {#each v.docs as d (d)}<button class="tab" class:on={doc.name === d} role="tab" aria-selected={doc.name === d} onclick={() => showDoc(d)}>{d}</button>{/each}
            </div>
            <div class="rendered"><Markdown text={doc.text} feature={spec.featureId} ondoc={openSibling} /></div>
          {:else}
            <p class="muted">No docs yet: the lead writes them in the spec step.</p>
          {/if}
        </section>

        <DesignSection bind:this={design} {v} />

        <section class="card">
          <div class="cardh"><b>Tasks</b><span class="muted">{backendLabel(v.project?.backend)} · {byStatus('done').length}/{v.tasks.length} done</span><span class="grow"></span><button class="btn ghost" onclick={() => openBoard(v.feature.project_id)}>Open board <Kbd keys="b" /></button></div>
          <div class="taskcols">
            {#each [['in_progress', 'In progress', 'working'], ['review', 'Review', 'waiting_gate'], ['ready', 'Ready', 'idle'], ['done', 'Done', 'done']] as [s, label, st] (s)}
              {@const list = byStatus(s)}
              {#if list.length || s !== 'done'}
                <div class="tcol">
                  <div class="colh"><StatusBadge state={st} compact /> {label} <span class="muted">{list.length}</span></div>
                  {#each list as t (t.id)}
                    <div class="task">
                      <div class="tt"><span class="muted mono">{handle(t)}</span><span class="grow"></span><span class="muted">{remoteName(t.backend)}</span></div>
                      <div class="title">{t.title}</div>
                      <div class="tt">
                        {#if t.node}<AgentChip id={t.node} role={v.team?.nodes?.find((n) => n.id === t.node)?.role_id ?? t.node} />{/if}
                        <span class="grow"></span>
                        {#if s === 'ready' && t.depends_on.some((d) => v.tasks.find((x) => x.id === d)?.status !== 'done')}<span class="blocked">{'\u{F051F}'} blocked by {t.depends_on.map((d) => `T-${d}`).join(', ')}</span>
                        {:else if t.branch}<span class="muted mono">{t.branch.split('/').pop()}</span>{/if}
                      </div>
                    </div>
                  {:else}<span class="muted">—</span>{/each}
                </div>
              {/if}
            {/each}
          </div>
        </section>
      </div>

      <div class="right">
        <section class="card">
          <div class="cardh"><b>Team</b><span class="muted">{v.agents.length} agent{v.agents.length === 1 ? '' : 's'}</span></div>
          {#if v.agents.length}
            <div class="agents">
              {#each v.agents as a (a.id)}
                <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                <div class="trow" onclick={() => openAgent(a.id)} title="Open {a.id}">
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
              <div class="ev"><span class="evtime mono muted">{new Date(e.ts_ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>{#if e.agent_id}<AgentChip id={e.agent_id} role={roleOfAgent(e.agent_id)} compact />{/if}<span class="evt">{timelineText(e)}</span></div>
            {:else}<span class="muted">Nothing yet.</span>{/each}
          </div>
          {#if run && !runFinished(run.status)}
            <div class="msg">
              <select class="field to" bind:value={to} aria-label="To">
                {#each v.team?.nodes ?? [] as n (n.id)}<option value={n.id}>{n.id}{n.is_lead ? ' (lead)' : ''}</option>{/each}
              </select>
              <input class="field grow" bind:this={msgBox} bind:value={msg} placeholder="Message {to === lead ? 'the lead' : to}…" onkeydown={(e) => e.key === 'Enter' && (e.preventDefault(), send())} />
              <Kbd keys="⏎" />
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
          {#if v.artifacts.some(isDesign)}<p class="muted nline">Design artifacts moved to the Design section (d).</p>{/if}
        </section>
      </div>
    </div>
  {/if}
</div>

<style>

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
  .link {
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
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
  .fh,
  .cardh,
  .tt,
  .arow,
  .ev,
  .msg,
  .colh {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .slug {
    font: var(--text-heading);
    color: var(--color-text-primary);
  }
  .pill {
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
  .taskcols {
    display: flex;
    gap: var(--space-sm);
    align-items: flex-start;
  }
  .tcol {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .task {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .title {
    font: var(--text-body-strong);
  }
  .blocked {
    color: var(--color-status-queued);
    font: var(--text-caption);
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
  .to {
    width: 9em;
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
