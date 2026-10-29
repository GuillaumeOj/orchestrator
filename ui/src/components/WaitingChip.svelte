<script>
  // Waiting-on chip (notes §1.1, Penpot P4.1 Waiting-on chip): the kind icon
  // (muted), the core's reason, and since. Since shows for needs_you, waiting,
  // queued and error; running shows its activity instead. The ref is a link: a
  // decision opens its Inbox detail, an agent focuses it, a task opens its card.
  // Compact (Overview row, tooltips): the reason truncates, since never does;
  // the full text is on hover. The status badge sits before it, on the surface.
  import { now30, openAgent, openBoard, openInbox } from '../lib/app.svelte.js';
  import { waited } from '../lib/format.js';
  import { since, waitLine } from '../lib/now.svelte.js';
  import { WAITING_ON } from '../lib/status.js';

  let { rec, compact = false } = $props();
  const running = $derived(rec?.level === 'running' && !rec?.waiting_on);
  const text = $derived(running ? rec?.activity : rec?.reason);
  const kind = $derived(running ? 'running' : rec?.waiting_on?.kind);
  const showSince = $derived(['needs_you', 'waiting', 'queued', 'error'].includes(rec?.level));
  const when = $derived(showSince ? waited(since(rec), Math.max(now30(), Date.now())) : '');
  const ref = $derived(rec?.waiting_on?.ref ?? null);
  // What clicking the reason opens (notes §1.1), when its ref names something.
  const go = $derived.by(() => {
    if (!ref) return null;
    switch (rec.waiting_on.kind) {
      case 'user':
        return () => openInbox(ref);
      case 'agent':
        return () => openAgent(ref);
      case 'task':
        return rec.project ? () => openBoard(rec.project, ref.replace(/^T-/, '')) : null;
      default:
        return null;
    }
  });
  const title = $derived(waitLine(rec, Math.max(now30(), Date.now())));
</script>

{#if text}
  <span class="chip" class:compact {title}>
    {#if kind}<span class="icon muted" aria-hidden="true">{WAITING_ON[kind]}</span>{/if}
    {#if go}<button class="link reason" onclick={(e) => (e.stopPropagation(), go())}>{text}</button>{:else}<span class="reason">{text}</span>{/if}
    {#if when}<span class="since muted">{compact ? '' : '· '}{when}</span>{/if}
  </span>
{/if}

<style>
  .chip {
    display: inline-flex;
    align-items: baseline;
    gap: var(--space-xs);
    min-width: 0;
    max-width: 100%;
    color: var(--color-text-secondary);
  }
  .reason {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: inherit;
  }
  button.reason {
    text-decoration: underline;
    color: var(--color-accent-default);
  }
  .since {
    flex: none;
    white-space: nowrap;
  }
  .icon {
    flex: none;
  }
</style>
