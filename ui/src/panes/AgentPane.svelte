<script>
  // Agent pane (§9.3, Penpot 04 Agent pane + states): the real `claude` in a PTY,
  // with a header (role, status + reason + since from its attention record,
  // model, context, session usage) and a strip for the states that need words
  // (queued or not started, suspended, limit, error, a permission prompt held
  // or parked for its card). The PTY itself is left untouched. Opened over
  // Mission control it starts in watch mode (D5): the terminal renders but
  // the strip has the keys, ⏎ or a click takes over, Esc goes back.
  import { onMount } from 'svelte';
  import AgentChip from '../components/AgentChip.svelte';
  import AgentMenu from '../components/agent/AgentMenu.svelte';
  import CtxBar from '../components/CtxBar.svelte';
  import LifecycleMarker from '../components/LifecycleMarker.svelte';
  import PermissionCall from '../components/PermissionCall.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import TermHost from '../components/TermHost.svelte';
  import UndeliveredChip from '../components/UndeliveredChip.svelte';
  import WaitingChip from '../components/WaitingChip.svelte';
  import { agent, agentAction, app, cancelQueued, decide, focusCard, leaveWatch, mountTerm, openEditor, runNow, takeOver, toast } from '../lib/app.svelte.js';
  import { ago, clock, duration } from '../lib/format.js';
  import { agentOf, blockOf, parked, visibleItems } from '../lib/inbox.js';
  import { errText } from '../lib/ipc.js';
  import { runNowCost } from '../lib/mission.js';
  import { attention, openDecisions, statusOf } from '../lib/now.svelte.js';
  import { sessionPp as ppOf, STATUS } from '../lib/status.js';
  import { tooltip } from '../lib/tooltip.js';

  let { paneId } = $props();
  let host = $state();
  let t = null;
  let ended = $state(false);
  const spec = $derived(app.panes[paneId]);
  const a = $derived(agent(spec?.agentId));
  const rec = $derived(a ? attention(a.id) : null);
  const ds = $derived(statusOf(spec?.agentId));
  const fresh = $derived(ds === 'not_started');
  const snap = $derived(app.data?.usage?.snapshot);
  const sessionPp = $derived(ppOf(a, snap));
  // Its permission prompt (held or parked): its card's own decision, not a second list.
  // "Answer in the pane" only hides the strip while the dialog is in the PTY below:
  // once the hold parks (no more `in_pane`), the dialog is gone and the strip is back.
  let handedOver = $state(null);
  const perm = $derived(
    visibleItems(
      openDecisions().filter((d) => d.kind === 'permission' && agentOf(d) === a?.id),
      'all',
      app.pending,
    ).find((d) => !(d.id === handedOver && d.actions.some((x) => x.id === 'in_pane'))),
  );
  // The strip's buttons: the decision's own actions, Deny with message and Open aside (the pane is here);
  // "Answer in the pane" needs a live session: a not-live strip offers Resume instead (§1.3.1).
  const permActions = $derived((perm?.actions ?? []).filter((x) => x.id !== 'reply' && x.id !== 'open' && (x.id !== 'in_pane' || a?.live)));
  const watching = $derived(!!spec?.watch);
  // Opened over Mission control and taken over: the strip says how to go back (notes F4).
  const overBoard = $derived(!!spec?.over && !!app.panes[spec.over]);
  const stopped = $derived(!a || rec?.status === 'done' || (ended && !a.live));
  const name = $derived(spec?.agentId ?? '');
  // The one strip shown (design-notes §1.3.1): it owns Resume and the waiting reason.
  const strip = $derived.by(() => {
    if (!a) return null;
    if (perm) return 'perm';
    if (rec?.level === 'needs_you') return 'needs';
    if (rec?.level === 'queued') return 'queued';
    if (ds === 'blocked_by_limit') return 'blocked';
    if (ds === 'error') return 'error';
    return a.live ? null : 'suspended';
  });
  // The limit that blocks it and when it resets (ms), for the disabled Resume's reason.
  const limit = $derived.by(() => {
    const u = app.data?.usage;
    const weekly = u?.weekly?.used_pct >= 100;
    const w = weekly ? u?.weekly : u?.five_hour;
    return { name: weekly ? 'weekly' : '5-hour', at: w?.resets_at ? w.resets_at * 1000 : null };
  });

  function onWatchKey(e) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === 'Enter' && !stopped) {
      e.preventDefault();
      e.stopPropagation();
      takeOver(paneId);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      leaveWatch(paneId);
    }
  }

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
    if (e.key === 'Enter' && !a?.live && e.target === e.currentTarget) {
      e.preventDefault();
      agentAction(a.id, 'resume');
    }
  }
</script>

{#if watching}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div class="session watch" tabindex="0" data-pane-focus data-watch role="group" aria-label="Watching {name}" onkeydown={onWatchKey}>
    <span class="icon" aria-hidden="true">{'\u{F06D0}'}</span>
    {#if stopped}
      <span><b class="mono">{name}</b> has stopped</span>
    {:else}
      <span>Watching <b class="mono">{name}</b>{rec?.status === 'paused' ? ' (paused)' : ''} · your keys go to Mission control</span>
    {/if}
    <span class="grow"></span>
    {#if !stopped}<button type="button" class="btn primary" use:tooltip={{ label: 'Take over', key: '⏎' }} onclick={() => takeOver(paneId)}>Take over</button>{/if}
    <button type="button" class="btn" use:tooltip={{ label: 'Back', key: 'esc' }} onclick={() => leaveWatch(paneId)}>Back to Mission control</button>
  </div>
{:else if overBoard}
  <div class="session muted">
    <span class="icon" aria-hidden="true">{'\u{F030C}'}</span>
    {#if stopped}<span><b class="mono">{name}</b> has stopped</span>{:else}<span>Typing in <b class="mono">{name}</b></span>{/if} ·
    <button type="button" class="link" use:tooltip={{ label: 'Mission control', key: '⌘; o' }} onclick={() => leaveWatch(paneId)}>Mission control</button>
  </div>
{/if}

{#if a}
  <div class="ah">
    <AgentChip id={a.id} />
    <StatusBadge state={ds} />
    {#if !strip}<WaitingChip {rec} />{/if}
    {#if a.off_hours}<span class="offh" use:tooltip={'Work you scheduled off-hours (FR-816)'}>· off-hours</span>{/if}
    {#if a.model_note}
      <span class="secondary down" use:tooltip={`Switched by the Governor (${a.model_note.replace(/^downgraded /, '')}); back to the role's model at the next task once it clears.`}>↓ {a.model ?? 'default model'} · {a.model_note}</span>
    {:else}
      <span class="secondary">{a.model ?? 'default model'}{a.effort ? ` · ${a.effort}` : ''}</span>
    {/if}
    <span class="muted">ctx</span><CtxBar value={a.context_pct} />
    <span class="muted">session</span><span>{sessionPp == null ? '—' : `${sessionPp.toFixed(1)} pp`}</span>
    <UndeliveredChip {rec} />
    <span class="grow"></span>
    <span class="acts">
      {#if a.live}
        <button type="button" class="icon-btn" aria-label="Interrupt" use:tooltip={{ label: 'Interrupt', key: 'esc' }} onclick={() => agentAction(a.id, 'interrupt')}>{'\u{F0765}'}</button>
        <button type="button" class="icon-btn" aria-label="Suspend" use:tooltip={'Suspend'} onclick={() => agentAction(a.id, 'suspend')}>{'\u{F04B2}'}</button>
      {/if}
      <AgentMenu agent={a} />
    </span>
  </div>

  {#if strip === 'perm'}
    <div class="strip warn perm" data-strip="perm">
      <div class="row">
        <span class="icon" style:color="var(--color-status-waiting-for-you)" aria-hidden="true">{'\u{F0005}'}</span>
        <span>Permission · {perm.title} · <button type="button" class="link" onclick={() => focusCard(perm.id)}>also in Decisions</button></span>
        <span class="grow"></span>
        {#each permActions as x (x.id)}
          {#if x.id === 'in_pane'}
            <button type="button" class="btn" use:tooltip={"Shows Claude Code's own dialog"} onclick={() => (handedOver = perm.id)}>{x.label}</button>
          {:else}
            <button type="button" class="btn" class:primary={x.id === 'approve'} use:tooltip={x.id === 'always' ? perm.payload?.always : null} onclick={() => decide(perm, x.id)}>{x.label}</button>
          {/if}
        {/each}
        {#if !a.live}{@render resume(false)}{/if}
      </div>
      {#if parked(perm)}<LifecycleMarker decision={perm} />{/if}
      {#key perm.id}<PermissionCall block={blockOf(perm, 'tool_call')} />{/key}
    </div>
  {:else if strip === 'needs'}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div class="strip warn" data-strip="needs" tabindex="0" onkeydown={onStripKey} role="group">
      <span class="icon" style:color={STATUS[ds].color} aria-hidden="true">{STATUS[ds].icon}</span>
      <span>{STATUS[ds].label}</span>
      <WaitingChip {rec} />
      <span class="grow"></span>
      {#if !a.live}{@render resume()}{/if}
    </div>
  {:else if strip === 'queued'}
    <div class="strip queued" data-strip="queued">
      <span class="icon" style:color={STATUS[ds].color} aria-hidden="true">{STATUS[ds].icon}</span>
      <span>{STATUS[ds].label}</span>
      <WaitingChip {rec} />
      <span class="grow"></span>
      {#if rec.queue}
        <button type="button" class="btn" onclick={() => runNow(rec)}>{fresh ? 'Start now' : 'Run now'}{runNowCost(rec.queue) ? ` ${runNowCost(rec.queue)}` : ''}</button>
        <button type="button" class="btn" use:tooltip={'Removes it from the queue; the agent stays as it is'} onclick={() => cancelQueued(rec)}>Cancel</button>
      {/if}
    </div>
  {:else if strip === 'blocked'}
    <div class="strip limit" data-strip="blocked">
      <span class="icon" style:color="var(--color-status-blocked-by-limit)" aria-hidden="true">{'\u{F073A}'}</span>
      <span>Blocked by the {limit.name} limit{limit.at ? ` · resets ${clock(limit.at)}` : ''}</span>
      {#if rec?.waiting_on}<WaitingChip {rec} />{/if}
      <span class="grow"></span>
      <span class="disabled-tip" use:tooltip={limit.at ? `Blocked until the reset · ${clock(limit.at)}` : 'Blocked until the reset'}>
        <button type="button" class="btn" disabled>Resume</button>
      </span>
    </div>
  {:else if strip === 'error'}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div class="strip error" data-strip="error" tabindex="0" onkeydown={onStripKey} role="group">
      <span class="icon" style:color="var(--color-status-error)" aria-hidden="true">{'\u{F0159}'}</span>
      <span>{rec?.reason ? `Exited with an error · ${rec.reason}` : 'Exited with an error'}</span>
      {#if rec?.waiting_on}<WaitingChip {rec} />{/if}
      <span class="grow"></span>
      {@render resume()}
    </div>
  {:else if strip === 'suspended'}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div class="strip" data-strip="suspended" tabindex="0" onkeydown={onStripKey} role="group" data-pane-focus>
      <span class="icon" style:color="var(--color-status-suspended)" aria-hidden="true">{'\u{F04B2}'}</span>
      <span>Suspended{a.last_event_at_ms ? ` ${ago(a.last_event_at_ms)} ago` : ''} · session kept{a.context_pct == null ? '' : ` · context ${Math.round(a.context_pct)} %`}</span>
      {#if rec?.waiting_on}<WaitingChip {rec} />{/if}
      <span class="grow"></span>
      {@render resume()}
    </div>
  {/if}
{:else}
  <div class="strip">This agent no longer exists.</div>
{/if}

{#snippet resume(primary = true)}
  <button type="button" class="btn" class:primary use:tooltip={{ label: 'Resume', key: '⏎' }} onclick={() => agentAction(a.id, 'resume')}>Resume</button>
{/snippet}

<TermHost bind:host watch={watching} ontakeover={() => !stopped && takeOver(paneId)}>
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
  /* The state's bar, and a tint for the warm and red states (04-agent-strips, as the banner rows). */
  .strip {
    --strip: var(--color-status-suspended);
    box-shadow: inset 3px 0 0 var(--strip);
  }
  .strip.warn,
  .strip.limit,
  .strip.error {
    background: color-mix(in srgb, var(--strip) 12%, var(--color-surface-pane));
  }
  .strip.warn {
    --strip: var(--color-status-waiting-for-you);
  }
  .strip.queued {
    --strip: var(--color-status-queued);
  }
  .strip.limit {
    --strip: var(--color-status-blocked-by-limit);
  }
  .strip.error {
    --strip: var(--color-status-error);
  }
  .acts {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xxs);
  }
  .disabled-tip {
    display: inline-flex;
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
  .session {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    height: var(--size-strip);
    padding: 0 var(--space-sm);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-label);
    white-space: nowrap;
    outline: none;
  }
  .session.watch {
    min-height: 36px;
    height: auto;
    padding: var(--space-xs) var(--space-md);
    background: var(--color-surface-selected);
    color: var(--color-text-secondary);
  }
  .session.watch b {
    color: var(--color-text-primary);
  }
  .ended {
    position: absolute;
    left: var(--space-md);
    bottom: var(--space-sm);
    font: var(--text-label);
  }
</style>
