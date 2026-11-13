<script>
  // Feature pane › source header (notes §9.1; S14 deleted upstream, S15 write-back failed): the tracker item the
  // run started from, two lines, no card border. From `run.source`.
  import { openUrl, toast } from '../../lib/app.svelte.js';
  import { errText, invoke } from '../../lib/ipc.js';
  import { sourceModel } from '../../lib/source.js';
  import { tooltip } from '../../lib/tooltip.js';
  import { openWork } from '../../lib/work.js';

  /** `run` = the feature's run (with `source`), `project` = the project id. */
  let { run, brief = null, project, reload = () => {} } = $props();
  const m = $derived(sourceModel(run, brief));
  let retrying = $state(false);

  async function retry() {
    if (retrying || !m.node) return;
    retrying = true;
    try {
      await invoke('writeback_retry', { node: m.node });
      await reload();
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    } finally {
      retrying = false;
    }
  }

  const showInWork = () => openWork({ projects: [project], reveal: m.node });
</script>

<div class="src" data-source={m.kind} class:deleted={!!m.deleted}>
  <div class="l1">
    {#if m.kind === 'source'}
      <span class="glyph" aria-hidden="true">{m.glyph}</span>
      <span class="ttl">{m.ref} {m.title}</span>
      <span class="sep">·</span>
      {#if m.url}
        <button type="button" class="link" aria-label={`Open ${m.ref} in ${m.tracker}`} use:tooltip={`Open ${m.ref} in ${m.tracker}`} onclick={() => openUrl(m.url)}>{m.tracker} ↗</button>
      {:else}<span>{m.tracker}</span>{/if}
    {:else if m.kind === 'brief' && m.url}
      <button type="button" class="link" onclick={() => openUrl(m.url)}>{m.title} ↗</button>
    {:else}
      <span class="muted">{m.title}</span>
    {/if}
    <span class="grow"></span>
    {#if m.canShowInWork}
      <button type="button" class="btn" use:tooltip={{ label: 'Show in Work', key: 'w' }} onclick={showInWork}>Show in Work</button>
    {/if}
  </div>
  {#if m.deleted}
    <div class="l2 warn">{m.deleted}</div>
  {:else if m.kind === 'source'}
    <div class="l2 muted">
      {#if m.trail}<span>{m.trail}</span>{/if}
      {#if m.closed}<span class="warn" use:tooltip={'The run continues. Stop it from the feature if needed.'}>· {m.closed}</span>{/if}
      {#if m.writeBack?.state === 'failed'}
        <span class="warn" data-writeback="failed">· {m.writeBack.text} ·</span>
        <button type="button" class="link" disabled={retrying} onclick={retry}>Retry</button>
      {:else if m.writeBack}<span data-writeback={m.writeBack.state}>· {m.writeBack.text}</span>{/if}
    </div>
  {/if}
</div>

<style>
  .src {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    min-width: 0;
  }
  .l1,
  .l2 {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
  }
  .l1 {
    min-height: var(--size-control);
  }
  .l2 {
    flex-wrap: wrap;
    font: var(--text-caption);
  }
  .ttl {
    font: var(--text-body-strong);
    color: var(--color-text-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .glyph,
  .sep {
    flex: none;
  }
  .deleted .glyph {
    opacity: 0.5;
  }
  .warn {
    color: var(--color-pacing-amber);
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    color: var(--color-focus);
    font: inherit;
    cursor: pointer;
  }
</style>
