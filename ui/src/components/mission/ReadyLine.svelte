<script>
  // A non-item line of the Ready to start lane (P4.5 notes §4.1–§4.2): no source,
  // nothing ready, a source error with its Fix, "+ n more", looking, all hidden.
  // Copy is the notes' (the core's `message` only reaches the tooltip, F3).
  import { errorInfo, moreCount, rateLimitedUntil } from '../../lib/intake.js';
  import { tooltip } from '../../lib/tooltip.js';

  /** @type {{ line: any, now: number, overwatch?: string, onplan?: () => void, onsetup?: () => void, onrefresh?: () => void, onfix?: () => void, onopen?: () => void, onshow?: () => void }} */
  let { line, now, overwatch = 'asleep', onplan = () => {}, onsetup = () => {}, onrefresh = () => {}, onfix = () => {}, onopen = () => {}, onshow = () => {} } = $props();

  const info = $derived(line.line === 'error' ? errorInfo(line.src.error, line.src, now) : null);
  const tail = $derived(overwatch === 'off' ? ' (off: your message waits)' : overwatch === 'down' ? ' (down: your message waits)' : '');
  const fixLabel = $derived(info?.fix === 'gh_scope' ? 'Copy command' : info?.fix === 'retry' ? 'Retry' : info?.fix ? 'Fix' : null);
  // A refresh that stopped at the read bound undercounts: "n+", or just "+ more" when nothing was counted.
  const moreText = $derived(line.line === 'more' ? moreCount(line.src.more, line.src.partial) : '');
  const until = $derived(line.line === 'error' ? rateLimitedUntil(line.src, now) : null);
</script>

<div class="ln" data-line={line.line} class:err={line.line === 'error'}>
  {#if line.line === 'nosource'}
    <span>{line.nameText} {line.many ? 'have' : 'has'} no tracker</span>
    <span class="muted">·</span>
    <button class="link" tabindex="-1" use:tooltip={{ label: 'Plan with the Overwatch', key: '⏎' }} onclick={(e) => (e.stopPropagation(), onplan())}>Plan with the Overwatch{tail}</button>
    <span class="muted">·</span>
    <button class="link" tabindex="-1" use:tooltip={{ label: 'Set up tracker', key: 'e' }} onclick={(e) => (e.stopPropagation(), onsetup())}>Set up tracker</button>
  {:else if line.line === 'empty'}
    <span class="muted">{line.merged ? `Nothing ready in ${line.count} projects` : `Nothing ready in ${line.src.label}`}</span>
    <span class="muted">·</span>
    <button class="link" tabindex="-1" use:tooltip={{ label: 'Refresh', key: '⇧R' }} onclick={(e) => (e.stopPropagation(), onrefresh())}>Refresh</button>
  {:else if line.line === 'error'}
    <span title={info.tip}><span aria-hidden="true">{'\u{F0026}'}</span> {line.name} · {info.text}</span>
    {#if fixLabel}
      <span class="muted">·</span>
      <button class="link" tabindex="-1" use:tooltip={info.fix !== 'retry' ? { label: fixLabel, key: '⏎' } : null} onclick={(e) => (e.stopPropagation(), onfix())}>{fixLabel}</button>
    {/if}
  {:else if line.line === 'more'}
    <span class="muted">+ {moreText ? `${moreText} ` : ''}more in {line.src.label}</span>
    {#if line.src.url}<span class="muted">·</span> <button class="link" tabindex="-1" use:tooltip={{ label: 'Open', key: '⏎' }} onclick={(e) => (e.stopPropagation(), onopen())}>Open ↗</button>{/if}
  {:else if line.line === 'looking'}
    <span class="muted">Looking for items in {line.src.label}…</span>
  {:else if line.line === 'allhidden'}
    <span class="muted">All {line.count} items are hidden</span>
    <span class="muted">·</span>
    <button class="link" tabindex="-1" use:tooltip={{ label: 'Show', key: '⏎' }} onclick={(e) => (e.stopPropagation(), onshow())}>Show</button>
  {/if}
</div>

<style>
  .ln {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-xs);
    min-height: var(--size-row-dense);
    padding: 0 var(--space-md);
    color: var(--color-text-secondary);
    font: var(--text-label);
  }
  .ln.err {
    color: var(--color-status-error);
  }
  .ln.err .link {
    color: var(--color-status-error);
    text-decoration: underline;
  }
</style>
