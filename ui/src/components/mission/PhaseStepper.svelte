<script>
  // Phase stepper (notes §1.2, Penpot P4.2 Phase stepper): six steps, done
  // filled, todo hollow, the current one in its color.phase.* glyph with its
  // label in stepper.mark and its mark (at gate, paused, deferred, failed) as
  // text. `compact` (Inspector): dots only and the current label.
  import { stepper } from '../../lib/mission.js';

  let { run, compact = false, title = null } = $props();
  const s = $derived(stepper(run));
</script>

<span class="stepper" class:compact role="img" aria-label={s.aria} title={title ?? s.aria}>
  {#each s.steps as st, i (st.id)}
    {#if i}<span class="link" class:done={st.state !== 'todo'}></span>{/if}
    <span class="step {st.state}" class:failed={st.mark === 'failed'}>
      {#if st.state === 'current'}
        <span class="glyph" style:color={st.color} aria-hidden="true">{st.icon}</span><span class="label">{st.label}</span>{#if st.mark}<span class="mark" class:failed={st.mark === 'failed'} style:border-color={st.color} style:color={st.color}>{st.mark}</span>{/if}
      {:else}
        <span class="dot" aria-hidden="true"></span>{#if !compact}<span class="label">{st.label}</span>{/if}
      {/if}
    </span>
  {/each}
</span>

<style>
  .stepper {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    font: var(--text-caption);
    white-space: nowrap;
    min-width: 0;
  }
  .step {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .dot {
    width: var(--size-stepper-step);
    height: var(--size-stepper-step);
    border-radius: var(--radius-full);
    border: var(--border-width-default) solid var(--color-stepper-todo);
    flex: none;
  }
  .done .dot {
    background: var(--color-stepper-done);
    border-color: var(--color-stepper-done);
  }
  .done .label {
    color: var(--color-text-secondary);
  }
  .todo .label {
    color: var(--color-text-muted);
  }
  .current .label {
    color: var(--color-stepper-mark);
    font-weight: var(--font-weight-semibold);
  }
  .current.failed .label {
    color: var(--color-phase-failed);
  }
  .link {
    width: var(--size-stepper-link);
    height: var(--border-width-default);
    background: var(--color-stepper-link);
    flex: none;
  }
  .link.done {
    background: var(--color-stepper-done);
  }
  .mark {
    padding: 0 var(--space-xs);
    border: var(--border-width-default) solid;
    border-radius: var(--radius-sm);
    font: var(--text-caption);
  }
</style>
