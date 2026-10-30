<script>
  // Inspector · agent (notes §5.1): status, waiting-on, details, actions.
  import { agent, agentAction, app, confirm, openDiff, openInKitty, openSession, openShell, projects } from '../../lib/app.svelte.js';
  import { clock, mb, shortPath, tokens } from '../../lib/format.js';
  import { phaseName } from '../../lib/mission.js';
  import { attention, governorReason, runAttention, statusOf } from '../../lib/now.svelte.js';
  import { cacheState, sessionPp } from '../../lib/status.js';
  import AgentChip from '../AgentChip.svelte';
  import Kbd from '../Kbd.svelte';
  import StatusBadge from '../StatusBadge.svelte';
  import WaitingChip from '../WaitingChip.svelte';

  let { id } = $props();
  const a = $derived(agent(id));
  const p = $derived(projects().find((x) => x.id === a?.project_id));
  const rec = $derived(attention(id));
  // Members only: the run it works in, with its phase.
  const run = $derived(rec?.run ? runAttention(rec.run) : null);
  const pp = $derived(sessionPp(a, app.data?.usage?.snapshot));
  const cache = $derived(cacheState(a, clock));
  const home = $derived(app.data?.core?.user_home ?? '');
  const short = (path) => shortPath(path, home);
</script>

{#if a}
  <div class="who"><AgentChip id={a.id} /><StatusBadge state={statusOf(a.id)} /></div>
  <dl>
    {#if rec?.waiting_on}<dt>Waiting on</dt><dd class="wait"><WaitingChip {rec} /></dd>{/if}
    <dt>Doing</dt><dd>{rec?.activity || (rec?.waiting_on ? '—' : rec?.reason || '—')}</dd>
    {#if run}<dt>Run</dt><dd>{run.project} › {run.feature} · {phaseName(run)}</dd>{/if}
    <dt>Model</dt><dd>{a.model ?? 'default'}{a.effort ? ` · ${a.effort}` : ''}</dd>
    <dt>Context</dt><dd>{a.context_pct == null ? '—' : `${Math.round(a.context_pct)} % ctx`}{cache ? ` · cache ${cache}` : ''}</dd>
    <dt>Usage</dt><dd>{pp == null ? '—' : `+${pp.toFixed(1)} pp this session`} · {a.turns ?? 0} turns</dd>
    {#if a.otel_week}<dt>This week</dt><dd>{tokens(a.otel_week.tokens_in + a.otel_week.cache_read + a.otel_week.cache_write)} in · {tokens(a.otel_week.tokens_out)} out · ≈ ${a.otel_week.cost.toFixed(2)}</dd>{/if}
    <dt>Governor</dt><dd>{governorReason(rec)}</dd>
    <dt>Memory</dt><dd>{a.memory_mb ? mb(a.memory_mb) : '—'}</dd>
    <dt>Worktree</dt><dd class="wrap">{short(a.worktree)}</dd>
    <dt>Project</dt><dd>{p?.name ?? '—'}</dd>
    <dt>Branch</dt><dd>{a.branch || '—'}</dd>
    <dt>Session</dt><dd class="wrap mono">{a.session_id}</dd>
    {#if a.git}<dt>Changes</dt><dd>±{a.git.dirty} · ↑{a.git.ahead} ↓{a.git.behind}</dd>{/if}
  </dl>
  <div class="acts">
    <button class="btn" onclick={() => openSession(a.id)}>Open session <Kbd keys="⏎" /></button>
    {#if a.live}
      <button class="btn" onclick={() => agentAction(a.id, 'interrupt')}>Interrupt <Kbd keys="esc" /></button>
      <button class="btn" onclick={() => agentAction(a.id, 'suspend')}>Suspend <Kbd keys="s" /></button>
    {:else}
      <button class="btn" onclick={() => agentAction(a.id, 'resume')}>Resume <Kbd keys="r" /></button>
    {/if}
    <button class="btn danger" onclick={() => confirm(`Kill ${a.id}?`, 'SIGTERM to its group and to leftover processes in its worktree, SIGKILL after 5 s.', () => agentAction(a.id, 'kill'), 'Kill')}>Kill <Kbd keys="⇧+K" /></button>
    <button class="btn danger" onclick={() => (app.overlay = { kind: 'delete-agent', agentId: a.id })}>Delete…</button>
    <button class="btn ghost" onclick={() => openShell({ agentId: a.id })}>Shell here <Kbd keys="⌘+; t" /></button>
    <button class="btn ghost" onclick={() => openDiff(a.project_id, a.worktree, a.branch)}>Diff</button>
    <button class="btn ghost" onclick={() => openInKitty(a.worktree)}>Open in kitty</button>
  </div>
{:else}
  <p class="muted">{id} no longer exists.</p>
{/if}

<style>
  .who {
    display: flex;
    gap: var(--space-sm);
    align-items: center;
  }
  .wait {
    display: flex;
    min-width: 0;
  }
  dl {
    display: grid;
    grid-template-columns: 84px 1fr;
    gap: var(--space-sm) var(--space-md);
    margin: 0;
    padding-bottom: var(--space-md);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  dt {
    color: var(--color-text-muted);
  }
  dd {
    margin: 0;
  }
  .wrap {
    overflow-wrap: anywhere;
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
  }
</style>
