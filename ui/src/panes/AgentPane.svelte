<script>
  // Agent pane (§9.3, Penpot 04 Agent pane + states): the real `claude` in a PTY,
  // with a header (role, state, model, context, session usage, governor) and a
  // strip for the states that need words (queued, suspended, limit, error, a
  // permission prompt held for the Inbox). The PTY itself is left untouched.
  import { onMount } from 'svelte';
  import AgentChip from '../components/AgentChip.svelte';
  import CtxBar from '../components/CtxBar.svelte';
  import Kbd from '../components/Kbd.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  import TermHost from '../components/TermHost.svelte';
  import { agent, agentAction, app, cancelQueued, confirm, mountTerm, openEditor, runNow, toast } from '../lib/app.svelte.js';
  import { ago, clock, duration } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { cacheState, displayState, notStarted, sessionPp as ppOf, queueReason, runCost } from '../lib/status.js';

  let { paneId } = $props();
  let host = $state();
  let t = null;
  let ended = $state(false);
  const spec = $derived(app.panes[paneId]);
  const a = $derived(agent(spec?.agentId));
  const ds = $derived(a ? displayState(a) : 'idle');
  const snap = $derived(app.data?.usage?.snapshot);
  const sessionPp = $derived(ppOf(a, snap));
  const perm = $derived((app.data?.permissions ?? []).find((p) => p.agent_id === a?.id));
  const liveCount = $derived((app.data?.agents ?? []).filter((x) => x.live).length);
  const queuePos = $derived.by(() => {
    const q = app.data?.queue ?? [];
    const i = q.findIndex((e) => e.agent === a?.id);
    return i < 0 ? null : i + 1;
  });
  const governorText = $derived.by(() => {
    if (!a) return '—';
    if (a.paused && !a.queue_entry) return 'paused · ⌘; p resumes';
    if (a.queue_entry) return `queued ${ordinal(queuePos)} (${a.queue_entry.tier})`;
    if (ds === 'waiting_for_you') return 'interactive · not counted against cap';
    if (a.live) return `admitted · ${liveCount}/${snap?.live_cap ?? '?'} live`;
    return '—';
  });
  const cache = $derived(cacheState(a, clock));

  function ordinal(n) {
    if (!n) return '';
    return `${n}${['th', 'st', 'nd', 'rd'][n % 100 > 10 && n % 100 < 14 ? 0 : n % 10] ?? 'th'}`;
  }

  async function answer(action) {
    if (!perm) return;
    try {
      await invoke('inbox_act', { item: perm.id, action, message: null });
      toast(action === 'approve' ? `Allowed once · ${a.id} · ${perm.summary.slice(0, 40)}` : action === 'reject' ? `Denied · ${a.id}` : `Answer in the pane · ${a.id}`);
    } catch (e) {
      toast(errText(e), { kind: 'error' });
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
    if (e.key === 'Enter' && !a?.live) {
      e.preventDefault();
      agentAction(a.id, 'resume');
    }
  }
</script>

{#if a}
  <div class="ah">
    <AgentChip id={a.id} />
    <StatusBadge state={ds} label={notStarted(a) ? 'Not started yet' : null} />
    {#if a.model_note}
      <span class="secondary down" title="Switched by the Governor ({a.model_note.replace(/^downgraded /, '')}); back to the role's model at the next task once it clears.">↓ {a.model ?? 'default model'} · {a.model_note}</span>
    {:else}
      <span class="secondary">{a.model ?? 'default model'}{a.effort ? ` · ${a.effort}` : ''}</span>
    {/if}
    <span class="muted">ctx</span><CtxBar value={a.context_pct} />
    <span class="muted">session</span><span>{sessionPp == null ? '—' : `${sessionPp.toFixed(1)} pp`}</span>
    <span class="muted">governor</span><span>{governorText}</span>
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
    <div class="strip warn">
      <span class="icon" style:color="var(--color-status-waiting-for-you)" aria-hidden="true">{'\u{F0005}'}</span>
      <span><b>Waiting for you</b> for {ago(perm.created_ms)} · {perm.tool}: <span class="mono">{perm.summary}</span> · also in the Inbox</span>
      <span class="grow"></span>
      <button class="btn primary" onclick={() => answer('approve')}>Allow once</button>
      <button class="btn danger" onclick={() => answer('reject')}>Deny</button>
      <button class="btn ghost" onclick={() => answer('in_pane')} title="Shows Claude Code's own dialog (for 'don't ask again')">Answer in the pane</button>
    </div>
  {:else if a.trust_dialog}
    <div class="strip warn"><span class="icon" style:color="var(--color-status-waiting-for-you)">{'\u{F0005}'}</span> Claude Code asks to trust this folder. Answer it below: the app never does.</div>
  {:else if ds === 'waiting_for_you'}
    <div class="strip warn"><span class="icon" style:color="var(--color-status-waiting-for-you)">{'\u{F0005}'}</span> Waiting for you for {ago(a.last_event_at_ms)} · answer in the pane</div>
  {:else if a.queue_entry}
    <div class="strip">
      <span class="icon" style:color="var(--color-status-queued)">{'\u{F051F}'}</span>
      <span>{notStarted(a) ? 'Not started yet' : 'Queued'} · {queueReason(a.queue_entry)}{a.queue_entry.estimated_start ? ` · starts ≈ ${clock(Date.parse(a.queue_entry.estimated_start))}` : ''}</span>
      <span class="grow"></span>
      <button class="btn ghost" onclick={() => cancelQueued(a)} title="Drop the queued turn">Cancel</button>
      <button class="btn" onclick={() => runNow(a, notStarted(a))}>
        {notStarted(a) ? 'Start now' : 'Run now'} ≈ {runCost(a.queue_entry)} <Kbd keys="⏎" />
      </button>
    </div>
  {:else if ds === 'blocked_by_limit'}
    <div class="strip"><span class="icon" style:color="var(--color-status-blocked-by-limit)">{'\u{F073A}'}</span> {a.last_error === 'rate_limit' ? 'Plan limit reached' : 'Blocked by limit'} · resumes at the reset (CLI auto-continue cancelled)</div>
  {:else if ds === 'error'}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div class="strip" tabindex="0" onkeydown={onStripKey} role="group">
      <span class="icon" style:color="var(--color-status-error)">{'\u{F0159}'}</span>
      <span>{a.last_error ? `Error: ${a.last_error}` : 'exited with an error'} · session saved</span>
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
  .ended {
    position: absolute;
    left: var(--space-md);
    bottom: var(--space-sm);
    font: var(--text-label);
  }
</style>
