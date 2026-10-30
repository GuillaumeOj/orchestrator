<script>
  // Inspector · run (notes §5.2, Penpot P4.2 Inspector): feature and project,
  // the stepper with its phase and since, the blocker, members with their
  // status and short wait, the tasks, the PR, when it started.
  import { features, now30, openBoard, openFeature, openUrl, select } from '../../lib/app.svelte.js';
  import { clock, waited } from '../../lib/format.js';
  import { phaseFull, phaseName, prText, shortWait, tasksDetail } from '../../lib/mission.js';
  import { attention, runAttention } from '../../lib/now.svelte.js';
  import { STATUS } from '../../lib/status.js';
  import AgentChip from '../AgentChip.svelte';
  import Kbd from '../Kbd.svelte';
  import PhaseStepper from '../mission/PhaseStepper.svelte';
  import PrLine from '../PrLine.svelte';

  let { id } = $props();
  const r = $derived(runAttention(id));
  const now = $derived(Math.max(now30(), Date.now()));
  const f = $derived(features().find((x) => x.run?.id === id) ?? null);
  const b = $derived(r?.blocker);
</script>

{#if r}
  <div class="title"><span style:color="var(--color-phase-executing)" aria-hidden="true">{'\u{F0D3D}'}</span> <b>{r.feature}</b> <span class="muted">{r.project}</span></div>
  <section>
    <h4>Phase</h4>
    <PhaseStepper run={r} compact />
    <div>{r.phase === 'deferred' ? phaseFull(r, now) : `${phaseName(r)}${r.phase_since_ms ? ` · since ${clock(r.phase_since_ms, now)} (${waited(r.phase_since_ms, now)})` : ''}`}</div>
  </section>
  {#if b}
    <section>
      <h4>{r.level === 'error' ? 'Error' : 'Blocker'}</h4>
      <div><button class="link" onclick={() => select('agent', b.agent)}>{b.agent}</button> · {b.reason}</div>
      {#if b.waiting_on?.since_ms}<div class="muted">{waited(b.waiting_on.since_ms, now)}{b.waiting_on.kind === 'user' ? ' · Open in Inbox d' : ''}</div>{/if}
    </section>
  {/if}
  <section>
    <h4>Members</h4>
    {#each r.members as m (m)}
      {@const rec = attention(m)}
      <button class="member" onclick={() => select('agent', m)}>
        <AgentChip id={m} compact />
        <span style:color={(STATUS[rec?.status] ?? STATUS.idle).color} aria-hidden="true">{(STATUS[rec?.status] ?? STATUS.idle).icon}</span>
        <b>{m}</b> <span class="muted">{shortWait(rec, now)}</span>
      </button>
    {/each}
  </section>
  <section>
    <h4>Tasks</h4>
    <div>{tasksDetail(r.tasks)}</div>
  </section>
  {#if r.pr}
    <section>
      <h4>PR</h4>
      {#if f?.run?.pr_url}<PrLine run={f.run} feature={f.id} />{:else}<button class="link" onclick={() => openUrl(r.pr.url)}>{prText(r.pr)}</button>{/if}
    </section>
  {/if}
  {#if r.started_ms}
    <section>
      <h4>Started</h4>
      <div>Started {clock(r.started_ms, now)} · {waited(r.started_ms, now) === 'just now' ? 'just now' : `${waited(r.started_ms, now)} ago`}</div>
    </section>
  {/if}
  <div class="acts">
    {#if f}<button class="btn" onclick={() => openFeature(f.id)}>Open feature</button>{/if}
    <button class="btn ghost" onclick={() => openBoard(r.project)}>Open board</button>
    {#if r.pr}<button class="btn ghost" onclick={() => openUrl(r.pr.url)}>Open PR ↗ <Kbd keys="o" /></button>{/if}
  </div>
{:else}
  <p class="muted">This run is over: its outcome is in Done today.</p>
{/if}

<style>
  .title {
    display: flex;
    align-items: baseline;
    gap: var(--space-sm);
    font: var(--text-body);
  }
  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  h4 {
    margin: 0;
    font: var(--text-caption);
    color: var(--color-text-muted);
  }
  .member {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    padding: var(--space-xxs) 0;
    border: 0;
    background: none;
    color: var(--color-text-primary);
    font: var(--text-label);
    text-align: left;
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
  .acts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
    padding-top: var(--space-md);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
</style>
