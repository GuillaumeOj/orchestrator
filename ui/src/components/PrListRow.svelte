<script>
  // One row of Project › Pull requests (notes §1): number + title (a link), draft chip, the action
  // (Launch team… / Open feature / disabled with the reason), then head → base, author, CI, review,
  // and the feature line. Keys with the row focused: Enter / l act, o opens the PR (§1.3).
  import { openUrl } from '../lib/app.svelte.js';
  import { authorName, ciText, featureLine, forkReason, reviewText, rowState } from '../lib/prlaunch.js';
  import { tooltip } from '../lib/tooltip.js';

  let { pr, disabled = false, onlaunch, onfeature } = $props();
  const state = $derived(rowState(pr));
  const reason = $derived(pr.launch?.reason ? (state === 'fork' ? forkReason(pr) : pr.launch.reason) : forkReason(pr));
  const ci = $derived(ciText(pr.checks));
  const review = $derived(reviewText(pr));
  const act = $derived(state === 'launch' ? () => onlaunch(pr) : state === 'active' ? () => onfeature(pr.feature) : null);
  const label = $derived(state === 'active' ? 'Open feature' : 'Launch team…');

  function onKey(e) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const onRow = e.target === e.currentTarget;
    if (e.key === 'o') {
      e.preventDefault();
      openUrl(pr.url);
    } else if (e.key === 'l' && state === 'launch' && !disabled) {
      e.preventDefault();
      act();
    } else if (e.key === 'Enter' && onRow) {
      e.preventDefault();
      if (act && !disabled) act();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<li class="prrow" class:dim={disabled} tabindex="0" data-pr={pr.number} data-state={state} onkeydown={onKey}>
  <div class="l1">
    <button type="button" class="title" use:tooltip={`Open PR #${pr.number} on GitHub`} onclick={() => openUrl(pr.url)}><b class="mono">#{pr.number}</b> <span class="t">{pr.title}</span></button>
    {#if pr.is_draft}<span class="chipd">draft</span>{/if}
    <span class="grow"></span>
    {#if state === 'fork'}
      <button type="button" class="btn" aria-disabled="true" aria-describedby="fork-{pr.number}" use:tooltip={reason}>Launch team…</button>
    {:else if state !== 'none'}
      <button type="button" class="btn" disabled={disabled} onclick={act}>{label}</button>
    {/if}
  </div>
  <div class="l2 muted">
    <span class="mono">{pr.head_ref} → {pr.base_ref}</span>
    · {#if pr.author?.is_bot}<span aria-label="bot">{'\u{F06A9}'}</span> {/if}<span>{authorName(pr.author)}</span>
    {#if ci}· <span>CI {#if pr.checks?.failing}<span class="bad">{ci}</span>{:else}{ci}{/if}</span>{/if}
    {#if review}· <span>{review}</span>{/if}
    {#if state === 'fork'}· <span id="fork-{pr.number}">{reason}</span>{/if}
    {#if pr.feature && state === 'active'}· <span data-feature>{featureLine(pr, 'active')}</span>
    {:else if pr.feature}· <span data-feature>Earlier team: <button type="button" class="linkbtn" onclick={() => onfeature(pr.feature)}>{pr.feature.slug}</button>{pr.feature.run_status ? ` · ${pr.feature.run_status}` : ''}</span>{/if}
  </div>
</li>

<style>
  .prrow {
    list-style: none;
    padding: var(--space-sm) 0;
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
  }
  .prrow:focus-visible {
    outline: var(--border-width-focus) solid var(--color-accent-default);
  }
  .dim {
    opacity: 0.6;
  }
  .l1 {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
  }
  .l1 .btn {
    flex: none;
  }
  .title {
    all: unset;
    cursor: pointer;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .title:hover b {
    color: var(--color-accent-default);
  }
  .title:focus-visible {
    outline: var(--border-width-focus) solid var(--color-accent-default);
  }
  .chipd {
    flex: none;
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-full, 999px);
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .l2 {
    font: var(--text-caption);
    padding-left: var(--space-xs);
  }
  .bad {
    color: var(--color-status-error);
  }
  .linkbtn {
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
</style>
