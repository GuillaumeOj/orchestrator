<script>
  // Feature pane (§9.3, Penpot 04 Feature · 1512): the feature's latest team
  // run: docs from the integration branch (rendered), tasks, the team with
  // live states, the timeline (FR-408) with a message box (FR-409), artifacts
  // and the PR, and the Design section (P3). Keys: e open the doc in nvim, b
  // board, c canvas, p pause / resume the team, m message, d Design.
  import { onMount } from 'svelte';
  import AgentChip from '../components/AgentChip.svelte';
  import DesignSection from '../components/DesignSection.svelte';
  import Kbd from '../components/Kbd.svelte';
  import Markdown from '../components/Markdown.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import { agent, app, confirm, openAgent, openBoard, openCanvas, openEditor, refresh, toast } from '../lib/app.svelte.js';
  import { isDesign } from '../lib/ecosystem.js';
  import { ago } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { displayState } from '../lib/status.js';
  import { runFinished, runStateColor, runStateLabel, TL_FILTERS, timelineFilter, timelineText } from '../lib/teams.js';

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

  const run = $derived(v?.run ?? null);
  const paused = $derived(run?.status === 'paused');
  const byStatus = (s) => (v?.tasks ?? []).filter((t) => t.status === s);
  const events = $derived((v?.timeline ?? []).filter(timelineFilter(tl)).slice(0, 60));
  const lead = $derived(v?.team?.nodes?.find((n) => n.is_lead)?.id ?? 'lead');
  /** An agent's role from the run's own team (the poll may not list a new agent yet). */
  const roleOfAgent = (id) => {
    const node = v?.agents?.find((a) => a.id === id)?.node;
    return v?.team?.nodes?.find((n) => n.id === node)?.role_id ?? null;
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

  async function showDoc(name) {
    try {
      doc = { name, text: await invoke('feature_doc', { feature: spec.featureId, name }) };
    } catch (e) {
      doc = { name, text: `(${errText(e)})` };
    }
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
    else return;
    e.preventDefault();
  }

  onMount(() => {
    load();
    const t = setInterval(load, 4000);
    return () => clearInterval(t);
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
      {#if run}<span class="pill" style:color={runStateColor(run.status)} style:border-color={runStateColor(run.status)}>{runStateLabel(run.status)}</span>{/if}
      <span class="muted">{v.team?.name ?? ''}{v.team ? ` v${v.team.version}` : ''} · {'\u{E725}'} {v.feature.integration_worktree ? `orch/${v.feature.slug}/integration` : '—'}{run ? ` · started ${ago(run.started_at_ms)}` : ''}</span>
      <span class="grow"></span>
      <button class="btn ghost" onclick={() => openBoard(v.feature.project_id)}>Board <Kbd keys="b" /></button>
      {#if run}<button class="btn ghost" onclick={() => openCanvas(run.team_id, v.feature.project_id)}>Canvas <Kbd keys="c" /></button>{/if}
      {#if run && !runFinished(run.status)}
        <button class="btn" onclick={() => act(paused ? 'resume' : 'pause')}>{paused ? 'Resume team' : 'Pause team'} <Kbd keys="p" /></button>
        <button class="btn danger" onclick={stop}>Stop</button>
      {/if}
      <button class="btn danger" onclick={() => (app.overlay = { kind: 'delete-feature', featureId: v.feature.id })}>Delete…</button>
    </header>

    <div class="cols">
      <div class="left">
        <section class="card">
          <div class="cardh"><b>Docs</b><span class="muted">.orchestrator/features/{v.feature.slug}/ on the integration branch</span><span class="grow"></span>{#if doc.name}<button class="btn ghost" onclick={openDoc}>Open in nvim <Kbd keys="e" /></button>{/if}</div>
          {#if v.docs.length}
            <div class="tabs" role="tablist">
              {#each v.docs as d (d)}<button class="tab" class:on={doc.name === d} role="tab" aria-selected={doc.name === d} onclick={() => showDoc(d)}>{d}</button>{/each}
            </div>
            <div class="rendered"><Markdown text={doc.text} /></div>
          {:else}
            <p class="muted">No docs yet: the lead writes them in the spec step.</p>
          {/if}
        </section>

        <DesignSection bind:this={design} {v} />

        <section class="card">
          <div class="cardh"><b>Tasks</b><span class="muted">{v.project?.backend === 'github' ? 'GitHub Issues' : 'Local'} · {byStatus('done').length}/{v.tasks.length} done</span><span class="grow"></span><button class="btn ghost" onclick={() => openBoard(v.feature.project_id)}>Open board <Kbd keys="b" /></button></div>
          <div class="taskcols">
            {#each [['in_progress', 'In progress', 'working'], ['review', 'Review', 'waiting_gate'], ['ready', 'Ready', 'idle'], ['done', 'Done', 'done']] as [s, label, st] (s)}
              {@const list = byStatus(s)}
              {#if list.length || s !== 'done'}
                <div class="tcol">
                  <div class="colh"><StatusBadge state={st} compact /> {label} <span class="muted">{list.length}</span></div>
                  {#each list as t (t.id)}
                    <div class="task">
                      <div class="tt"><span class="muted mono">{t.backend === 'github' && t.external_id ? `#${t.external_id}` : `T-${t.id}`}</span><span class="grow"></span><span class="muted">{t.backend === 'github' ? 'GitHub' : 'Local'}</span></div>
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
          {#each v.agents as a (a.id)}
            {@const live = agent(a.id)}
            <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
            <div class="arow" onclick={() => openAgent(a.id)} title="Open {a.id}">
              <AgentChip id={a.id} role={roleOfAgent(a.id)} />
              <StatusBadge state={live ? displayState(live) : a.state} />
              <span class="muted ell">{a.tool ? `${a.tool}` : a.task ? `task T-${a.task}` : ''}{a.flag ? ` · ${a.flag}` : ''}</span>
            </div>
          {:else}<p class="muted">No agent yet.</p>{/each}
        </section>

        <section class="card tl">
          <div class="cardh"><b>Timeline</b><span class="muted">messages · tasks · merges · gates · governor</span></div>
          <div class="chips">{#each TL_FILTERS as f (f.id)}<button class="chip" class:on={tl === f.id} onclick={() => (tl = f.id)}>{f.label}</button>{/each}</div>
          <div class="events">
            {#each events as e (e.id)}
              <div class="ev"><span class="mono muted">{new Date(e.ts_ms).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}</span>{#if e.agent_id}<AgentChip id={e.agent_id} role={roleOfAgent(e.agent_id)} compact />{/if}<span class="evt">{timelineText(e)}</span></div>
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
          {#if run?.pr_url}<div class="arow"><span class="muted">PR</span> <span class="mono">{run.pr_url}</span></div>
          {:else}<p class="muted">No PR yet: the lead asks with request_pr after integration (PR gate).</p>{/if}
          {#each v.artifacts.filter((a) => !isDesign(a)) as a (a.id)}
            <div class="art"><span class="muted">{a.kind}</span> <span class="ell">{a.uri}</span> <span class="muted">({a.from_node}{a.to_node ? ` → ${a.to_node}` : ''})</span></div>
          {/each}
        </section>
      </div>
    </div>
  {/if}
</div>

<style>
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
    background: var(--color-surface-chrome);
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
  .arow {
    cursor: pointer;
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
  .evt {
    color: var(--color-text-secondary);
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
