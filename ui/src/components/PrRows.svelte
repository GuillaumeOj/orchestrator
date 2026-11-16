<script>
  // The "PR and artifacts" card's per-issue rows (allow-multiple-prs ux-brief
  // §2.2, ux-flows decision 4), from `run.prs_view`: `#12 Crash on empty brief`
  // (ref bold, title muted, cut first) then the state: none · pending (link to
  // the gate card) · open (PrLine's content, bare) · merged · closed. The refresh
  // and `checked` text are the card head's (one for every PR). Used by the
  // feature pane and by the run inspector.
  import { focusCard, openUrl } from '../lib/app.svelte.js';
  import { issueOf } from '../lib/issues.js';
  import { prRowState, runOfPr } from '../lib/pr.js';
  import { tooltip } from '../lib/tooltip.js';
  import PrLine from './PrLine.svelte';

  /** @type {{ run: any, feature: any, taskCounts?: Record<string, number>, onrun?: (r: any) => void }} */
  let { run, feature, taskCounts = {}, onrun = () => {} } = $props();
  const rows = $derived(run?.prs_view ?? []);
</script>

<div class="rows">
  {#each rows as p (p.issue ?? 'feature')}
    {@const issue = issueOf(feature, p.issue)}
    {@const st = prRowState(p, taskCounts[p.issue])}
    <div class="row" data-issue={p.issue} data-state={p.state}>
      <div class="line">
      <span class="who" use:tooltip={issue?.title ? `${p.issue} ${issue.title}` : null}>
        <b class="ref mono">{p.issue}</b>{#if issue?.title}{' '}<span class="ttl muted">{issue.title}</span>{/if}
      </span>
      <span class="st {st.tone}">
        {#if p.state === 'open'}
          <PrLine run={{ ...runOfPr(run, p), id: `${run.id}-${p.issue}` }} feature={feature.id} {onrun} bare />
        {:else if p.state === 'pending'}
          <span class="txt">{st.text}</span>
          {#if p.gate != null}<button type="button" class="link" onclick={() => focusCard(`gate-${p.gate}`)}>Show card</button>{/if}
        {:else if p.url}
          <button type="button" class="link txt" use:tooltip={p.url} onclick={() => openUrl(p.url)}>{st.text} <span aria-hidden="true">{'\u{F03CC}'}</span></button>
        {:else}
          <span class="txt">{st.text}</span>
        {/if}
      </span>
      </div>
    </div>
  {/each}
</div>

<style>
  .rows {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }
  /* The state goes on its own line, and beside the issue once the card is wide enough. */
  .row {
    container-type: inline-size;
    min-width: 0;
  }
  .line {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    min-width: 0;
  }
  .who {
    display: flex;
    align-items: baseline;
    gap: var(--space-sm);
    min-width: 0;
    flex: 1 1 0;
    white-space: nowrap;
  }
  .ref {
    flex: none;
  }
  .ttl {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .st {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
    font: var(--text-caption);
  }
  .st :global(.prline) {
    flex: 1;
    min-width: 0;
  }
  .st.muted {
    color: var(--color-text-muted);
  }
  .st.waiting {
    color: var(--color-status-waiting-for-you);
  }
  .st.done {
    color: var(--color-status-done);
  }
  .txt {
    white-space: nowrap;
  }
  .link {
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
  .st.done .link,
  .st.muted .link {
    color: inherit;
  }
  .link:hover {
    text-decoration: underline;
  }
  @container (min-width: 480px) {
    .line {
      flex-direction: row;
      align-items: center;
      gap: var(--space-md);
    }
    .who {
      flex: 1 1 0;
    }
    .st {
      flex: 0 1 auto;
    }
  }
</style>
