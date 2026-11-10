<script>
  // Status bar (§9.2): git state of the focused project, live/queued agents with
  // the pressure state in words, the 5h guard chip whenever it holds work, memory.
  import { app, checkForUpdates, contextAgent, contextProject, openSettings, restartToUpdate } from '../lib/app.svelte.js';
  import { claudeChip } from '../lib/claude.js';
  import { clock, mb } from '../lib/format.js';
  import { counts, liveText } from '../lib/now.svelte.js';
  import { powerChip } from '../lib/pacing.js';
  import { tooltip } from '../lib/tooltip.js';
  import { prChip, updateChip } from '../lib/update.js';
  import Kbd from './Kbd.svelte';
  import PressurePill from './PressurePill.svelte';

  const a = $derived(contextAgent());
  const p = $derived(contextProject());
  const git = $derived(a?.git ?? p?.git);
  const branch = $derived(a?.branch || git?.branch);
  const s = $derived(app.data?.usage?.snapshot);
  const c = $derived(counts());
  const m = $derived(app.data?.memory);
  const fiveReset = $derived(app.data?.usage?.five_hour?.resets_at);
  const watch = $derived(app.data?.memory_watch);
  const power = $derived(powerChip(app.data?.power));
  const cc = $derived(claudeChip(app.data?.setup?.claude));
  const upd = $derived(updateChip(app.data?.update));
  const pr = $derived(prChip(app.data?.update, app.prTitles[app.data?.update?.current?.pr]));
</script>

<footer class="status">
  {#if branch}
    <span><span class="icon" aria-hidden="true">{'\u{E725}'}</span> {branch}</span>
    {#if git}<span>↑{git.ahead} ↓{git.behind}</span><span>±{git.dirty}</span>{/if}
    <span class="muted">·</span>
  {/if}
  <span class="live"><span style:color="var(--color-attention-running)" aria-hidden="true">▲</span> {liveText(c)}</span>
  <span class="queued"><span style:color="var(--color-attention-queued)" aria-hidden="true">□</span> queued {c.queued}</span>
  {#if s}
    <PressurePill state={s.state} shadow={app.data?.usage?.shadow} />
    {#if s.five_hour_guard}
      <span class="guard"><span style:color="var(--color-status-blocked-by-limit)" aria-hidden="true">⊘</span> <b>5h guard</b> cap 1{fiveReset ? ` until ${clock(fiveReset * 1000)}` : ''}</span>
    {/if}
  {/if}
  {#if power}
    <span class="offh" class:warn={power.warn} use:tooltip={'Off-hours work (FR-816)'}><span aria-hidden="true">☾</span> {power.text}</span>
  {/if}
  <span class="grow"></span>
  {#if watch?.alert}<span class="memwarn" use:tooltip={`WebKit helper above ${watch.warn_mb} MB · logged every minute in ${watch.log}`}><span aria-hidden="true">⚠</span> {watch.alert}</span>{/if}
  {#if cc}
    <button type="button" class="plain chip claude {cc.kind}" onclick={() => openSettings('claude')} use:tooltip={cc.title}>
      {#if cc.kind === 'updating'}<span class="spinning" aria-hidden="true">◐</span> {/if}{cc.text}{#if cc.version}<span class="ver">{' '}{cc.version}</span>{/if}
    </button>
  {/if}
  {#if pr}
    <button type="button" class="plain chip upd ready" onclick={() => openSettings('builds')} use:tooltip={pr.title}>{pr.text}</button>
  {/if}
  {#if upd?.kind === 'downloading'}
    <span class="upd" use:tooltip={upd.title}>{upd.text}</span>
  {:else if upd}
    <button class="plain chip upd" class:ready={upd.kind === 'ready'} class:warn={upd.kind === 'error'} onclick={upd.kind === 'ready' ? () => restartToUpdate() : checkForUpdates} use:tooltip={upd.title}>{upd.text}</button>
  {/if}
  {#if app.leader}<span class="leader" data-leader>LEADER <Kbd keys="⌘+;" /> then</span>{/if}
  {#if m}<span use:tooltip={`app ${mb(m.app_mb)} · agents ${mb(m.agents_mb)} · shells ${mb(m.shells_mb)}`}>RAM {mb(m.total_mb)} / {mb(m.system_mb)}</span>{/if}
</footer>

<style>
  .status {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    height: var(--size-statusbar);
    padding: 0 var(--space-md);
    background: var(--color-surface-chrome);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-label);
    white-space: nowrap;
    overflow: hidden;
  }
  .plain {
    background: none;
    border: 0;
    padding: 0;
  }
  .chip {
    height: 18px;
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-status-done);
    border-radius: var(--radius-full);
    color: var(--color-text-secondary);
    font: var(--text-label);
    cursor: pointer;
  }
  .offh {
    color: var(--color-text-secondary);
  }
  .offh.warn {
    color: var(--color-pacing-amber);
  }
  .chip.upd.ready {
    border-color: var(--color-accent-default);
  }
  .chip.upd.warn {
    color: var(--color-pacing-amber);
  }
  .chip.claude.available {
    border-color: var(--color-accent-default);
    color: var(--color-accent-default);
  }
  .chip.claude.updating {
    border-color: var(--color-border-subtle);
    color: var(--color-text-muted);
  }
  .chip.claude.failed {
    border-color: var(--color-pacing-amber);
    color: var(--color-pacing-amber);
  }
  /* Narrow: the Claude chip drops its version before the app chip truncates. */
  @media (max-width: 900px) {
    .chip.claude .ver {
      display: none;
    }
  }
  .guard {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    height: 18px;
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-full);
  }
  .memwarn {
    color: var(--color-status-error);
    font-weight: var(--font-weight-semibold);
  }
  /* The leader overlay (§7): only while ⌘; is held, an accent chip. */
  .leader {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-accent-default);
    border-radius: var(--radius-sm);
    background: var(--color-accent-subtle);
    color: var(--color-text-secondary);
  }
</style>
