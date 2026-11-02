<script>
  // A settled card collapsed to one line (P4.3 notes §2.2–2.4, P4.2 notes §8.4):
  // `▸ allowed back-1: cargo add serde_json · rule: installs   14:02`. Verb and
  // agent secondary, what primary (truncates, full text in the tooltip), tail
  // and time muted. ⏎ or a click expands the read-only card from the snapshot.
  import { now30 } from '../../lib/app.svelte.js';
  import { lineLabel, lineText, tailText, verbText, warnVerb } from '../../lib/chat.js';
  import { clock } from '../../lib/format.js';
  import DecisionCard from './DecisionCard.svelte';

  /** @type {{ item: any, expanded?: boolean, pulse?: boolean, ontoggle?: () => void, onkey?: (key: string) => void }} */
  let { item, expanded = false, pulse = false, ontoggle = () => {}, onkey = () => {} } = $props();
  const line = $derived(item.line);
  const now = $derived(Math.max(now30(), Date.now()));
  const tail = $derived(tailText(line));
</script>

<article class="line" class:pulse aria-label={lineLabel(line, now)} data-line={item.id} data-verb={line.verb}>
  <button class="row" aria-expanded={expanded} title={lineText(line)} onclick={ontoggle} data-line-toggle={item.key}>
    <span class="caret" aria-hidden="true">{expanded ? '▾' : '▸'}</span>
    <span class="head"><span class="verb" class:warn={warnVerb(line.verb)}>{verbText(line.verb)}</span>{line.agent ? ` ${line.agent}` : ''}:</span>
    <span class="what">{line.what ?? ''}</span>
    {#if tail}<span class="tail muted">· {tail}</span>{/if}
    <span class="at muted">{clock(line.at_ms, now)}</span>
  </button>
  {#if expanded && item.d}
    <DecisionCard d={item.d} {line} {onkey} />
  {/if}
</article>

<style>
  .line {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    min-width: 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    height: var(--size-row);
    min-width: 0;
    padding: 0 var(--space-xs);
    border: var(--border-width-default) solid transparent;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--color-text-primary);
    font: var(--text-label);
    text-align: left;
    white-space: nowrap;
  }
  .row:hover {
    background: var(--color-surface-raised);
  }
  .row:focus-visible {
    outline: none;
    border-color: var(--color-focus);
  }
  .row[aria-expanded='true'] {
    background: var(--color-accent-subtle);
  }
  .caret,
  .head,
  .tail,
  .at {
    flex: none;
  }
  .head,
  .caret {
    color: var(--color-text-secondary);
  }
  .verb.warn {
    color: var(--color-status-waiting-for-you);
  }
  .what {
    flex: 0 1 auto;
    min-width: 2ch;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tail {
    max-width: 45%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .at {
    margin-left: auto;
    padding-left: var(--space-sm);
  }
  .line.pulse .row {
    animation: pulse 1.5s ease-out;
  }
  @keyframes pulse {
    from {
      background: color-mix(in srgb, var(--color-accent-default) 12%, transparent);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .line.pulse .row {
      animation: none;
      background: color-mix(in srgb, var(--color-accent-default) 12%, transparent);
    }
  }
</style>
