<script>
  // A decision laid out in the chat column (P4.3 notes §2, §3, §13; P4.2 notes
  // §8; Penpot 04.3 "cards per kind"): the old pane's detail in a 336 px card. Kind
  // and age · title · subject · context blocks · reply · actions · marker. Live
  // while the decision is listed; read-only (`line` given) when a collapsed
  // line expands: no actions but the ones that open something.
  import { untrack } from 'svelte';
  import { app, now30, openUrl, projectName } from '../../lib/app.svelte.js';
  import { byWords, lineMarker, verbText } from '../../lib/chat.js';
  import { clock, dayTime, waited } from '../../lib/format.js';
  import { agentOf, blockOf, deliveryPrimary, isProgramCard, keyless, parked, restarted, subjectLine } from '../../lib/inbox.js';
  import {
    decidingName,
    decidingShare,
    decidingText,
    overwatchOf,
    recommended,
    recommendsText,
    refKind,
    refLabel,
    SOURCES_SHOWN,
  } from '../../lib/overwatch.js';
  import { programTag } from '../../lib/program.js';
  import { decisionKind, LIFECYCLE } from '../../lib/status.js';
  import { tooltip } from '../../lib/tooltip.js';
  import ContextBlocks from '../ContextBlocks.svelte';
  import LifecycleMarker from '../LifecycleMarker.svelte';
  import BriefBody from './BriefBody.svelte';

  /** @type {{ d: any, line?: any, selected?: boolean, marked?: boolean, pending?: any, ref?: any, replying?: string | null, pulse?: boolean, deciding?: boolean, refok?: (ref: any) => boolean, handed?: number | null, entries?: any, choices?: string[][], onkey?: (key: string) => void, onact?: (id: string) => void, onreply?: (text: string | null) => void, onjump?: () => void, onhold?: (held: boolean) => void, onref?: (ref: any) => void }} */
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
    refok = () => true,
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
  const brief = $derived(d.kind === 'brief');
  // A brief's Launch / Edit opened the dialog: the card stays open meanwhile, its actions off (notes §8.4).
  const opening = $derived(live && brief && app.overlay?.intake?.brief_decision === d.id);
  const cardTitle = $derived(brief ? [d.title ?? d.payload?.title, projectName(d.project ?? d.payload?.project) ?? d.payload?.project_name ?? d.project].filter(Boolean).join(' · ') : d.title);
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

  // Why you are asked (notes §4.5, F3): the core's or a rule's reason, or the Overwatch passing it on.
  const askText = $derived.by(() => {
    const a = d.asks_because ?? d.payload?.asks_because; // the core sends it in the payload
    if (!a || d.kind !== 'permission') return '';
    if (a.kind === 'overwatch_unsure') return 'The Overwatch did not decide this.';
    if (a.kind !== 'core' && a.kind !== 'policy') return '';
    const why = a.kind === 'policy' && a.detail ? `policy ${a.detail.replace(/^policy\s+/, '')}` : (a.detail ?? '');
    const line = why ? `Only you can allow this: ${why}.` : 'Only you can allow this.';
    return line.length > 70 ? `${line.slice(0, 69)}…` : line;
  });

  // The Overwatch's note (P4.4 notes §4): its recommendation is highlighted, never pre-selected;
  // an expanded line keeps the note, read-only, without the highlight.
  const note = $derived(d.annotation?.note ? d.annotation : null);
  const rec = $derived(recommended(d));
  const recId = $derived(live ? rec : null);
  let noteOpen = $state(false);
  let srcOpen = $state(false);
  const sources = $derived([...new Set(note?.sources ?? [])]); // a repeated source would break the keyed each
  // Measured, not guessed: the clamp hides something only when the text overflows 3 lines.
  let noteEl = $state();
  let noteOver = $state(false);
  $effect(() => {
    note?.note;
    if (!noteEl) return;
    const measure = () => {
      if (!noteOpen) noteOver = noteEl.scrollHeight > noteEl.clientHeight + 1;
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(noteEl);
    return () => ro.disconnect();
  });
  const longNote = $derived(noteOver || noteOpen);
  /** A source's click: the card's project goes with it, so a doc is found in the right one. */
  const openSource = (r) => onref({ ...r, project: d.subject?.project ?? null });

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
  <div class="title">{cardTitle}</div>
  {#if programTag(d)}<div class="ptag" data-program-tag><span aria-hidden="true">{'\u{F0328}'}</span> {programTag(d)}</div>{/if}
  {#if subjectLine(d)}<div class="subject secondary">{subjectLine(d)}</div>{/if}
  {#if askText}<div class="why muted" data-asks>{askText}</div>{/if}
  {#if d.summary && !d.context?.length}<p class="summary">{d.summary}</p>{/if}
  {#if brief}
    <BriefBody {d} ow={overwatchOf(app.now)} {live} />
  {:else}
    <ContextBlocks decision={d} bind:choices onjump={(id) => onref({ kind: 'chat', id })} />
  {/if}
  {#if d.kind === 'rule_proposal' && d.payload?.declined_before && live}
    <p class="muted note" data-second-ask>Not now on {dayTime(d.payload.declined_at_ms)}. It won't be asked again if you decline.</p>
  {/if}
  {#if live && restarted(d)}
    <p class="muted note">This was still waiting when the core restarted. Your answer reaches {agentOf(d) ?? 'the agent'} as a turn.</p>
  {/if}
  {#if advice}<p class="muted note">{advice}</p>{/if}
  {#if replying}
    <div class="reply">
      <!-- svelte-ignore a11y_autofocus -->
      <textarea class="field" rows="3" autofocus bind:value={text} onkeydown={replyKey} aria-label="Reply" placeholder={d.kind === 'permission' ? 'Tell Claude what to do instead · ⏎ to send' : `Reply to ${agentOf(d) ?? 'the agent'} · ⏎ to send`}></textarea>
    </div>
  {/if}
  {#if note}
    <div class="ann" class:ro={!live} data-annotation>
      <div class="nh"><b>Overwatch</b>{#if rec}{' '}<span class="muted">· {recommendsText(d)}</span>{/if}</div>
      <p class="nt" class:clamp={!noteOpen} bind:this={noteEl}>{note.note}</p>
      {#if longNote}<button class="link" aria-expanded={noteOpen} onclick={() => (noteOpen = !noteOpen)}>{noteOpen ? 'Show less' : 'Show more'}</button>{/if}
      {#if sources.length}
        <div class="srcs">
          {#each srcOpen ? sources : sources.slice(0, SOURCES_SHOWN) as s, i (i)}
            {@const r = refKind(s)}
            {@const label = r ? refLabel(r, entries) : s}
            {#if i}<span class="muted" aria-hidden="true">·</span>{/if}
            {#if label == null || (r && !refok(r))}<span class="muted" title="No longer available">{s}</span>
            {:else if r}<button class="link" data-source onclick={() => openSource(r)}>{label} ↗</button>
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
          class:primary={x.key === 'a' || x.id === 'launch' || (x.id === 'answer' && answered) || (d.kind === 'error' && x.id === 'resume') || primary === x.id || (isProgramCard(d) && x.key === '⏎')}
          class:ghost={x.key === 'o' && primary !== x.id}
          class:rec={x.id === recId}
          aria-label={x.id === recId ? `${x.label}, recommended by the Overwatch` : undefined}
          disabled={(x.id === 'answer' && options && !answered) || opening}
          use:tooltip={{ label: x.label, key: x.key === 'enter' ? '⏎' : x.key }}
          onclick={() => onkey(x.key)}>{x.label}{#if x.id === recId}{' '}<span class="tag">· recommended</span>{/if}</button
        >
      {/each}
      {#each keyless(d) as x (x.id)}
        <button
          class="btn"
          class:rec={x.id === recId}
          aria-label={x.id === recId ? `${x.label}, recommended by the Overwatch` : undefined}
          class:primary={primary ? primary === x.id : x.id === 'restart_fresh'}
          class:danger={x.id === 'delete'}
          class:ghost={primary ? primary !== x.id : x.id !== 'restart_fresh' && x.id !== 'delete'}
          use:tooltip={x.id === 'always' && d.payload?.always ? { label: x.label, detail: d.payload.always } : null}
          onclick={() => onact(x.id)}>{x.label}{#if x.id === recId}{' '}<span class="tag">· recommended</span>{/if}</button
        >
      {/each}
    </div>
  {:else if live && pending}
    {#if !pending.sent}<div class="actions"><button class="btn ghost" use:tooltip={{ label: 'Undo', key: 'u' }} onclick={() => onkey('u')}>Undo</button></div>{/if}
  {:else if prUrl || (!live && opens.length)}
    <div class="actions">
      {#if prUrl}<button class="btn ghost" use:tooltip={{ label: 'Open PR', key: 'o' }} onclick={() => openUrl(prUrl)}>Open PR</button>
      {:else}{#each opens as x (x.id)}<button class="btn ghost" use:tooltip={{ label: x.label, key: x.key }} onclick={() => onact(x.id)}>{x.label}</button>{/each}{/if}
    </div>
  {/if}
  {#if deciding && live && !pending}
    <div class="lc deciding" data-deciding role="group" aria-label={decidingName(d)}>
      <span class="ring" style:background="conic-gradient(var(--color-accent-default) {Math.round(decidingShare(d, tick) * 360)}deg, var(--color-border-subtle) 0)" aria-hidden="true"></span>
      <span aria-hidden="true">{decidingText(d, tick)}</span>
    </div>
  {:else}
    <LifecycleMarker decision={d} {pending} given={opening ? { state: 'open', text: 'Opening dialog…' } : (handedMarker ?? (line ? { state: LIFECYCLE[line.state] ? line.state : 'effect', text: lineMarker(line, now) } : null))} />
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
  .why {
    margin-top: calc(-1 * var(--space-xs));
    font: var(--text-caption);
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
    outline: 1.5px dashed var(--color-accent-default);
    outline-offset: -1px;
    white-space: normal;
  }
  .tag {
    color: var(--color-accent-default);
    font: var(--text-caption);
  }
  .ann {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-pane);
    border-left: 2px solid var(--color-accent-default);
    border-radius: var(--radius-sm);
  }
  .ann.ro {
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
  .ann .link {
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
  .ptag {
    color: var(--color-accent-default);
    font: var(--text-caption);
  }
</style>
