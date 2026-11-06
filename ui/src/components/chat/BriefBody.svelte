<script>
  // The body of a brief card (P4.5 notes §8.3): the brief as markdown clamped at 12 lines,
  // the mode line, the estimate of the proposed mode (size M) and the footer that says how
  // to ask for changes. The card never says the Overwatch started anything.
  import { projectName } from '../../lib/app.svelte.js';
  import { blockOf } from '../../lib/inbox.js';
  import { basisText, overRoom, ppRange, roomWarning } from '../../lib/intake.js';
  import Markdown from '../Markdown.svelte';

  /** @type {{ d: any, ow: any, live?: boolean }} */
  let { d, ow, live = true } = $props();
  const md = $derived(blockOf(d, 'markdown'));
  const cost = $derived(blockOf(d, 'cost'));
  const est = $derived(cost?.estimate ?? cost);
  const mode = $derived(d.mode === 'solo' ? 'solo' : 'team');
  const project = $derived(projectName(d.project) ?? d.project ?? 'project');
  const modeLine = $derived(
    blockOf(d, 'text')?.text ?? [project, mode === 'team' ? 'team run' : 'solo session', mode === 'team' ? d.team : null].filter(Boolean).join(' · '),
  );
  const e = $derived(est?.[mode]);
  const range = $derived(e ? ppRange(e.low_pp, e.high_pp) : '');
  const estLine = $derived(
    range
      ? `${e.basis === 'default' ? 'rough ' : ''}${range.startsWith('<') ? range : `≈ ${range}`} pp week · ${basisText(e, mode, est.size ?? 'M', project)}`
      : '',
  );
  const warn = $derived(est && overRoom(est, mode) ? roomWarning({ estimate: { ...est, over_room: true } }, project) : '');
  const footer = $derived(
    ow?.status === 'off'
      ? 'The Overwatch is off. Turn it on in Settings to ask for changes.'
      : ow?.status === 'down'
        ? 'The Overwatch is down. Messages are kept until it is back.'
        : 'Want changes? Answer in the chat and the Overwatch sends a new brief.',
  );

  let open = $state(false);
  let el = $state();
  let over = $state(false);
  $effect(() => {
    md?.text;
    if (!el) return;
    const measure = () => {
      if (!open) over = el.scrollHeight > el.clientHeight + 1;
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  });
</script>

{#if md}
  <section class="blk" data-block="markdown">
    <div class="bh">Brief</div>
    <div class="md" class:clamp={!open} bind:this={el}><Markdown text={md.text} /></div>
    {#if over || open}<button class="link more" aria-expanded={open} onclick={() => (open = !open)}>{open ? 'Show less' : 'Show more'}</button>{/if}
  </section>
{/if}
<div class="mode" data-brief-mode>{modeLine}</div>
{#if estLine}<div class="est secondary" data-brief-estimate>{estLine}</div>{/if}
{#if warn}<div class="muted warn" data-over-room><span aria-hidden="true">{'\u{F0026}'}</span> {warn}</div>{/if}
{#if live && d.lifecycle?.state === 'open'}<p class="muted foot" data-brief-foot>{footer}</p>{/if}

<style>
  .blk {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-md);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    min-width: 0;
    font: var(--text-body);
  }
  .bh {
    font: var(--text-label);
    color: var(--color-text-muted);
  }
  .md.clamp {
    max-height: 18em;
    overflow: hidden;
  }
  .more {
    align-self: flex-end;
  }
  .est,
  .warn,
  .foot {
    margin: 0;
    font: var(--text-caption);
  }
</style>
