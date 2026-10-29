<script>
  // Agent pane (§9.3, Penpot 04 Agent pane + states): the real `claude` in a PTY,
  // with a header (role, status + reason + since from its attention record,
  // model, context, session usage) and a strip for the states that need words
  // (queued or not started, suspended, limit, error, a permission prompt held
  // or parked for the Inbox). The PTY itself is left untouched.
  import { onMount } from 'svelte';
  import AgentChip from '../components/AgentChip.svelte';
  import CtxBar from '../components/CtxBar.svelte';
  import Kbd from '../components/Kbd.svelte';
  import PermissionCall from '../components/PermissionCall.svelte';
  import Reason from '../components/Reason.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import TermHost from '../components/TermHost.svelte';
  import { agent, agentAction, app, cancelQueued, confirm, decide, mountTerm, openEditor, runNow, toast } from '../lib/app.svelte.js';
  import { ago, clock, duration } from '../lib/format.js';
  import { actionLabels, legacyItem, visibleItems } from '../lib/inbox.js';
  import { errText } from '../lib/ipc.js';
  import { attention, openDecisions, statusOf } from '../lib/now.svelte.js';
  import { cacheState, sessionPp as ppOf, runCost, STATUS } from '../lib/status.js';

  let { paneId } = $props();
  let host = $state();
  let t = null;
  let ended = $state(false);
  const spec = $derived(app.panes[paneId]);
  const a = $derived(agent(spec?.agentId));
  const rec = $derived(a ? attention(a.id) : null);
  const ds = $derived(statusOf(spec?.agentId));
  const fresh = $derived(ds === 'not_started');
  // Answered in the PTY (a trust dialog, a prompt in the pane); a gate, a team
  // question or the Governor's ask is answered in the Inbox.
  const inPane = $derived(/^(trust|wait)-/.test(rec?.waiting_on?.ref ?? ''));
  const snap = $derived(app.data?.usage?.snapshot);
  const sessionPp = $derived(ppOf(a, snap));
  // Its permission prompt (held or parked): the Inbox's own item, not a second list.
  // "Answer in the pane" only hides the strip while the dialog is in the PTY below:
  // once the hold parks (no more `in_pane`), the dialog is gone and the strip is back.
  let handedOver = $state(null);
  const perm = $derived(
    visibleItems(
      openDecisions()
        .filter((d) => d.kind === 'permission' && d.subject?.agent === a?.id)
        .map(legacyItem),
      'all',
      app.pending,
    ).find((i) => !(i.id === handedOver && i.actions.includes('in_pane'))),
  );
  const permLabels = $derived(actionLabels(perm));
  const cache = $derived(cacheState(a, clock));

  function attachIfLive() {
    if (!t || !a?.live || t.alive) return;
    ended = false;
    t.start().catch((e) => toast(`${a.id}: ${errText(e)}`, { kind: 'error' }));
  }

  onMount(() => {
    const m = mountTerm(paneId, host, {
      onExit: () => (ended = true),
      onOpenPath: (file, line) => openEditor(file, line, { agentId: spec?.agentId }),
    });
    t = m.t;
    attachIfLive();
    return m.destroy;
  });

  // A resumed agent: attach again (Claude Code redraws its screen).
  $effect(() => {
    if (a?.live && a?.pid) attachIfLive();
  });

  function onStripKey(e) {
    if (e.key === 'Enter' && !a?.live) {
      e.preventDefault();
      agentAction(a.id, 'resume');
    }
  }
</script>

{#if a}
  <div class="ah">
    <AgentChip id={a.id} />
    <StatusBadge state={ds} />
    <Reason {rec} />
    {#if a.off_hours}<span class="offh" title="Work you scheduled off-hours (FR-816)">· off-hours</span>{/if}
    {#if a.model_note}
      <span class="secondary down" title="Switched by the Governor ({a.model_note.replace(/^downgraded /, '')}); back to the role's model at the next task once it clears.">↓ {a.model ?? 'default model'} · {a.model_note}</span>
    {:else}
      <span class="secondary">{a.model ?? 'default model'}{a.effort ? ` · ${a.effort}` : ''}</span>
    {/if}
    <span class="muted">ctx</span><CtxBar value={a.context_pct} />
    <span class="muted">session</span><span>{sessionPp == null ? '—' : `${sessionPp.toFixed(1)} pp`}</span>
    <span class="grow"></span>
    {#if a.live}
      <button class="hint" onclick={() => agentAction(a.id, 'interrupt')}><Kbd keys="esc" /> interrupt</button>
      <button class="hint" onclick={() => agentAction(a.id, 'suspend')}>suspend</button>
    {:else}
      <button class="hint" onclick={() => agentAction(a.id, 'resume')}><Kbd keys="⏎" /> resume</button>
    {/if}
    <button class="hint" onclick={() => (app.overlay = { kind: 'palette' })}><Kbd keys="⌘+K" /> more</button>
  </div>

  {#if perm}
    <div class="strip warn perm">
      <div class="row">
        <span class="icon" style:color="var(--color-status-waiting-for-you)" aria-hidden="true">{'\u{F0005}'}</span>
        <span><b>Waiting for you</b> for {ago(perm.created_ms)} · {perm.title} · also in the Inbox</span>
        <span class="grow"></span>
        <button class="btn primary" onclick={() => decide(perm, 'approve')}>{permLabels.a}</button>
        <button class="btn danger" onclick={() => decide(perm, 'reject')}>{permLabels.r}</button>
        {#if perm.actions.includes('always')}<button class="btn" title={perm.payload?.always} onclick={() => decide(perm, 'always')}>{permLabels.always}</button>{/if}
        {#if perm.actions.includes('in_pane')}<button class="btn ghost" onclick={() => (handedOver = perm.id)} title="Shows Claude Code's own dialog">{permLabels.in_pane}</button>{/if}
      </div>
      <PermissionCall item={perm} />
    </div>
  {:else if rec?.level === 'needs_you'}
    <div class="strip warn"><span class="icon" style:color={STATUS[ds].color}>{STATUS[ds].icon}</span> <b>{STATUS[ds].label}</b> <Reason {rec} /> · {inPane ? 'answer it in the pane below: the app never does' : 'answer it in the Inbox'}</div>
  {:else if rec?.level === 'queued'}
    <div class="strip">
      <span class="icon" style:color={STATUS[ds].color}>{STATUS[ds].icon}</span>
      <b>{STATUS[ds].label}</b> <Reason {rec} />
      <span class="grow"></span>
      {#if a.queue_entry}
        <button class="btn ghost" onclick={() => cancelQueued(a)} title="Drop the queued turn">Cancel</button>
        <button class="btn" onclick={() => runNow(a, fresh)}>
          {fresh ? 'Start now' : 'Run now'} ≈ {runCost(a.queue_entry)} <Kbd keys="⏎" />
        </button>
      {/if}
    </div>
  {:else if ds === 'blocked_by_limit'}
    <div class="strip"><span class="icon" style:color="var(--color-status-blocked-by-limit)">{'\u{F073A}'}</span> <b>Blocked by limit</b> <Reason {rec} /> · resumes at the reset (CLI auto-continue cancelled)</div>
  {:else if ds === 'error'}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div class="strip" tabindex="0" onkeydown={onStripKey} role="group">
      <span class="icon" style:color="var(--color-status-error)">{'\u{F0159}'}</span>
      <span>{rec?.reason ? `Error: ${rec.reason}` : 'Exited with an error'} · session saved</span>
      <span class="grow"></span>
      <button class="btn" onclick={() => agentAction(a.id, 'resume')}>Resume <Kbd keys="⏎" /></button>
      <button class="btn danger" onclick={() => (app.overlay = { kind: 'delete-agent', agentId: a.id })}>Delete…</button>
    </div>
  {:else if !a.live}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div class="strip" tabindex="0" onkeydown={onStripKey} role="group" data-pane-focus>
      <span class="icon" style:color="var(--color-status-suspended)">{'\u{F04B2}'}</span>
      <span>Suspended{a.last_event_at_ms ? ` ${clock(a.last_event_at_ms)}` : ''}{cache ? ` · cache ${cache}` : ''} · the session is saved</span>
      <span class="grow"></span>
      <button class="btn" onclick={() => agentAction(a.id, 'resume')}>Resume <Kbd keys="⏎" /></button>
      <button class="btn danger" onclick={() => confirm(`Kill ${a.id}?`, 'Also stops leftover processes in its worktree.', () => agentAction(a.id, 'kill'), 'Kill')}>Kill</button>
      <button class="btn danger" onclick={() => (app.overlay = { kind: 'delete-agent', agentId: a.id })}>Delete…</button>
    </div>
  {/if}
{:else}
  <div class="strip">This agent no longer exists.</div>
{/if}

<TermHost bind:host>
  {#if a && !a.live && ended}<div class="ended muted">claude exited · {a.label} · {duration(Date.now() - (a.last_event_at_ms ?? Date.now()))} ago</div>{/if}
</TermHost>

<style>

  .offh {
    color: var(--color-status-done);
  }

  .down {
    color: var(--color-pacing-amber);
  }
  .ah {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-xs) var(--space-sm);
    min-height: var(--size-row);
    padding: var(--space-xs) var(--space-sm);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-label);
    white-space: nowrap;
  }
  .strip {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-sm);
    min-height: 36px;
    padding: var(--space-xs) var(--space-md);
    background: var(--color-surface-raised);
    font: var(--text-label);
  }
  .strip.warn {
    box-shadow: inset 3px 0 0 var(--color-status-waiting-for-you);
  }
  .strip.perm {
    flex-direction: column;
    align-items: stretch;
    padding-block: var(--space-sm);
  }
  .strip .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-sm);
  }
  .ended {
    position: absolute;
    left: var(--space-md);
    bottom: var(--space-sm);
    font: var(--text-label);
  }
</style>
