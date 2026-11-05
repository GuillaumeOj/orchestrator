<script>
  // A decision laid out in the chat column (P4.3 notes §2, §3, §13; P4.2 notes
  // §8; Penpot 04.3 "cards per kind"): the old pane's detail in a 336 px card. Kind
  // and age · title · subject · context blocks · reply · actions · marker. Live
  // while the decision is listed; read-only (`line` given) when a collapsed
  // line expands: no actions but the ones that open something.
  import { untrack } from 'svelte';
  import { now30, openUrl } from '../../lib/app.svelte.js';
  import { byWords, lineMarker, verbText } from '../../lib/chat.js';
  import { clock, waited } from '../../lib/format.js';
  import { agentOf, blockOf, deliveryPrimary, keyless, parked, restarted, subjectLine } from '../../lib/inbox.js';
  import {
    decidingShare,
    decidingText,
    NOTE_CLAMP,
    recommended,
    recommendsText,
    refKind,
    SOURCES_SHOWN,
    sourceLabel,
  } from '../../lib/overwatch.js';
  import { decisionKind, LIFECYCLE } from '../../lib/status.js';
  import ContextBlocks from '../ContextBlocks.svelte';
  import Kbd from '../Kbd.svelte';
  import LifecycleMarker from '../LifecycleMarker.svelte';

  /** @type {{ d: any, line?: any, selected?: boolean, marked?: boolean, pending?: any, ref?: any, replying?: string | null, pulse?: boolean, deciding?: boolean, handed?: number | null, entries?: any, choices?: string[][], onkey?: (key: string) => void, onact?: (id: string) => void, onreply?: (text: string | null) => void, onjump?: () => void, onhold?: (held: boolean) => void, onref?: (ref: any) => void }} */
  let {
    d,
    line = null,
    selected = false,
    marked = false,
    pending = null,
    ref = null,
    replying = null,
    pulse = false,
    deciding = false,
    handed = null,
    entries = {},
    choices = $bindable([]),
    onkey = () => {},
    onact = () => {},
    onreply = () => {},
    onjump = () => {},
    onhold = () => {},
    onref = () => {},
  } = $props();

  const live = $derived(!line);
  // "Permission · by rule" on an expanded line settled without you.
  const kindBy = $derived(line?.by && line.by !== 'you' ? ` · ${byWords({ ...line, by_detail: undefined })}` : '');
  const k = $derived(decisionKind(d));
  const state = $derived(d.lifecycle?.state ?? 'open');
  const now = $derived(Math.max(now30(), Date.now()));
  // Actions while it waits on the user, failed (again, with Dismiss) or stale (Dismiss).
  const waitsOnUser = $derived(['open', 'parked', 'failed', 'stale'].includes(state));
  const acting = $derived(live && !pending && waitsOnUser);
  const keyed = $derived((d.actions ?? []).filter((x) => x.key));
  const options = $derived(blockOf(d, 'options'));
  const answered = $derived(!!options && options.questions.every((_, i) => choices[i]?.length));
  // What still opens something once it is settled: the pane, the PR, the Notion page.
  const opens = $derived((d.actions ?? []).filter((x) => x.id === 'open' || x.key === 'o'));
  const prUrl = $derived(d.lifecycle?.effect === 'pr_opened' ? d.lifecycle.effect_detail : null);
  const primary = $derived(deliveryPrimary(d));
  // What a delivery or a crash-looped startup card adds to its context blocks,
  // while there is still something to press (not once settled, pending or stale).
  const advice = $derived(
    !acting || state === 'stale'
      ? null
      : d.kind === 'delivery'
        ? 'Retry puts the messages back in its queue now. Open the pane to see what it shows; Restart fresh starts a new session with its brief and task.'
        : d.kind === 'startup' && d.payload?.failed === 'crash'
          ? 'It was restarted automatically and crashed again. Open the pane to see why, or restart it fresh: a new session with its brief and task.'
          : null,
  );
  const open = $derived(state === 'open' || state === 'parked' || !!pending);

  // The Overwatch's note (P4.4 notes §4): its recommendation is highlighted, never pre-selected;
  // an expanded line keeps the note, read-only, without the highlight.
  const note = $derived(d.annotation?.note ? d.annotation : null);
  const recId = $derived(live ? recommended(d) : null);
  let noteOpen = $state(false);
  let srcOpen = $state(false);
  const sources = $derived(note?.sources ?? []);
  const longNote = $derived((note?.note?.length ?? 0) > NOTE_CLAMP);

  // Deciding (notes §3): the ring and the seconds left, once a second; the card is a normal card meanwhile.
  let tick = $state(Date.now());
  $effect(() => {
    if (!deciding) return;
    tick = Date.now();
    const id = setInterval(() => (tick = Date.now()), 1000);
    return () => clearInterval(id);
  });
  // The hand-over: "Waiting for you" from that moment, in the deciding marker's slot.
  const handedMarker = $derived(
    handed != null && state === 'open' && !pending && !line
      ? { state: 'open', text: `Waiting for you · ${waited(handed, now)}` }
      : null,
  );

  let text = $state('');
  $effect(() => {
    if (!replying) text = '';
  });
  // A reply being written never collapses under the cursor (notes §2.1); a
  // card that leaves (filtered out, settled elsewhere) lets go of its hold.
  $effect(() => {
    const held = !!text.trim();
    untrack(() => onhold(held)); // the parent's set of held cards is not this effect's input
  });
  $effect(() => () => onhold(false));

  function replyKey(e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onreply(null);
    } else if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      onreply(text);
    }
  }
</script>

<div class="card {state}" class:open class:selected class:marked class:pulse class:ro={!live} class:muted={state === 'stale'} data-decision={d.id} data-state={pending && !pending.sent ? 'pending' : state}>
  {#if ref && !pending && waitsOnUser}
    {@const what = `${verbText(ref.verb)} ${ref.what}`}
    <button class="ref" onclick={onjump} aria-label="Follows: {what}, {clock(ref.at_ms, now)}. Jump to it">↳ after: {what} · {clock(ref.at_ms, now)}</button>
  {/if}
  <div class="top">
    <span class="kind" style:color={k.color}><span aria-hidden="true">{k.icon}</span> {k.label}{parked(d) ? ' · parked' : ''}{restarted(d) ? ' · after restart' : ''}{kindBy}</span>
    <span class="grow"></span>
    <span class="muted age">{line ? '' : waited(d.created_ms, now)}</span>
  </div>
  <div class="title">{d.title}</div>
  {#if subjectLine(d)}<div class="subject secondary">{subjectLine(d)}</div>{/if}
  {#if d.summary && !d.context?.length}<p class="summary">{d.summary}</p>{/if}
  <ContextBlocks decision={d} bind:choices />
  {#if live && restarted(d)}
    <p class="muted note">This was still waiting when the core restarted. Your answer reaches {agentOf(d) ?? 'the agent'} as a turn.</p>
  {/if}
  {#if advice}<p class="muted note">{advice}</p>{/if}
  {#if replying}
    <div class="reply">
      <!-- svelte-ignore a11y_autofocus -->
      <textarea class="field" rows="3" autofocus bind:value={text} onkeydown={replyKey} aria-label="Reply" placeholder={d.kind === 'permission' ? 'Tell Claude what to do instead…' : `Message to ${agentOf(d) ?? 'the agent'}…`}></textarea>
      <span class="muted"><Kbd keys="⏎" /> send · <Kbd keys="⇧+⏎" /> new line · <Kbd keys="esc" /> cancel</span>
    </div>
  {/if}
  {#if note}
    <div class="note" class:ro={!live} data-annotation>
      <div class="nh"><b>Overwatch</b>{#if recId || (!live && recommended(d))}{' '}<span class="muted">· {recommendsText(d)}</span>{/if}</div>
      <p class="nt" class:clamp={longNote && !noteOpen}>{note.note}</p>
      {#if longNote}<button class="link" aria-expanded={noteOpen} onclick={() => (noteOpen = !noteOpen)}>{noteOpen ? 'Show less' : 'Show more'}</button>{/if}
      {#if sources.length}
        <div class="srcs">
          {#each srcOpen ? sources : sources.slice(0, SOURCES_SHOWN) as s, i (s)}
            {@const label = sourceLabel(s, entries)}
            {#if i}<span class="muted" aria-hidden="true">·</span>{/if}
            {#if label == null}<span class="muted" title="No longer available">{s}</span>
            {:else if refKind(s)}<button class="link" data-source onclick={() => onref(refKind(s))}>{label} ↗</button>
            {:else}<span class="muted">{label}</span>{/if}
          {/each}
          {#if sources.length > SOURCES_SHOWN && !srcOpen}<button class="link" onclick={() => (srcOpen = true)}>+ {sources.length - SOURCES_SHOWN} more</button>{/if}
        </div>
      {/if}
    </div>
  {/if}
  {#if acting}
    <div class="actions">
      {#each keyed as x (x.id)}
        <button
          class="btn"
          class:primary={x.key === 'a' || (x.id === 'answer' && answered) || (d.kind === 'error' && x.id === 'resume') || primary === x.id}
          class:ghost={x.key === 'o' && primary !== x.id}
          class:rec={x.id === recId}
          aria-label={x.id === recId ? `${x.label}, recommended by the Overwatch` : undefined}
          disabled={x.id === 'answer' && options && !answered}
          onclick={() => onkey(x.key)}>{x.label}{#if x.id === recId}{' '}<span class="tag">· recommended</span>{/if} <Kbd keys={x.key === 'enter' ? '⏎' : x.key} /></button
        >
      {/each}
      {#each keyless(d) as x (x.id)}
        <button
          class="btn"
          class:rec={x.id === recId}
          class:primary={primary ? primary === x.id : x.id === 'restart_fresh'}
          class:danger={x.id === 'delete'}
          class:ghost={primary ? primary !== x.id : x.id !== 'restart_fresh' && x.id !== 'delete'}
          title={x.id === 'always' ? d.payload?.always : null}
          onclick={() => onact(x.id)}>{x.label}{#if x.id === recId}{' '}<span class="tag">· recommended</span>{/if}</button
        >
      {/each}
    </div>
  {:else if live && pending}
    {#if !pending.sent}<div class="actions"><button class="btn ghost" onclick={() => onkey('u')}>Undo <Kbd keys="u" /></button></div>{/if}
  {:else if prUrl || (!live && opens.length)}
    <div class="actions">
      {#if prUrl}<button class="btn ghost" onclick={() => openUrl(prUrl)}>Open PR <Kbd keys="o" /></button>
      {:else}{#each opens as x (x.id)}<button class="btn ghost" onclick={() => onact(x.id)}>{x.label}{#if x.key}{' '}<Kbd keys={x.key} />{/if}</button>{/each}{/if}
    </div>
  {/if}
  {#if deciding && live && !pending}
    <div class="lc deciding" data-deciding role="group" aria-label="Overwatch is deciding, up to 1 minute">
      <span class="ring" style:background="conic-gradient(var(--color-accent-default) {Math.round(decidingShare(d, tick) * 360)}deg, var(--color-border-subtle) 0)" aria-hidden="true"></span>
      <span aria-hidden="true">{decidingText(d, tick)}</span>
    </div>
  {:else}
    <LifecycleMarker decision={d} {pending} given={handedMarker ?? (line ? { state: LIFECYCLE[line.state] ? line.state : 'effect', text: lineMarker(line, now) } : null)} />
  {/if}
</div>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-md);
    min-width: 0;
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    font: var(--text-label);
    overflow-wrap: anywhere;
  }
  .card.open {
    border-color: var(--color-status-waiting-for-you);
  }
  .card.failed {
    border-color: var(--color-lifecycle-failed);
  }
  .card.selected {
    box-shadow: 0 0 0 var(--border-width-focus) var(--color-focus);
  }
  .card.marked {
    box-shadow: inset 3px 0 0 var(--color-accent-default);
  }
  .card.selected.marked {
    box-shadow:
      inset 3px 0 0 var(--color-accent-default),
      0 0 0 var(--border-width-focus) var(--color-focus);
  }
  .card.muted {
    color: var(--color-text-muted);
  }
  .card.ro {
    background: var(--color-surface-pane);
  }
  .card.pulse {
    animation: pulse 1.5s ease-out;
  }
  @keyframes pulse {
    from {
      background: color-mix(in srgb, var(--color-accent-default) 12%, var(--color-surface-raised));
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .card.pulse {
      animation: none;
      background: color-mix(in srgb, var(--color-accent-default) 12%, var(--color-surface-raised));
    }
  }
  .ref {
    align-self: flex-start;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0;
    border: 0;
    background: none;
    color: var(--color-text-muted);
    font: var(--text-caption);
    text-align: left;
  }
  .ref:hover,
  .ref:focus-visible {
    color: var(--color-accent-default);
  }
  .top {
    display: flex;
    align-items: baseline;
    gap: var(--space-sm);
  }
  .kind {
    font: var(--text-caption);
    font-weight: var(--font-weight-semibold);
  }
  .age {
    flex: none;
  }
  .title {
    font: var(--text-body-strong);
  }
  .subject {
    margin-top: calc(-1 * var(--space-xs));
    font: var(--text-caption);
  }
  .summary,
  .note {
    margin: 0;
  }
  .reply {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs) var(--space-sm);
  }
  .actions .btn {
    white-space: nowrap;
  }
  /* The Overwatch's recommendation: an accent outline and a tag, never pre-selected (notes §4.1). */
  .actions .btn.rec {
    outline: 2px solid var(--color-accent-default);
    outline-offset: -1px;
    white-space: normal;
  }
  .tag {
    color: var(--color-accent-default);
    font: var(--text-caption);
  }
  .note {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-pane);
    border-left: 2px solid var(--color-accent-default);
    border-radius: var(--radius-sm);
  }
  .note.ro {
    border-left-color: var(--color-border-strong);
    color: var(--color-text-secondary);
  }
  .nt {
    margin: 0;
    white-space: pre-wrap;
  }
  .nt.clamp {
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .srcs {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0 var(--space-xs);
    font: var(--text-caption);
  }
  .note .link {
    align-self: flex-start;
  }
  .deciding {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    color: var(--color-text-secondary);
  }
  .ring {
    flex: none;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    mask: radial-gradient(circle, transparent 3px, black 3.5px);
  }
  @media (prefers-reduced-motion: reduce) {
    .ring {
      display: none;
    }
  }
</style>
