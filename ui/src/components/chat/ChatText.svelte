<script>
  // A text entry of the thread (P4.4 notes §2.1, api-contract §3.2): the user's
  // message (right, accent wash, plain text, the delivery marker under it) or
  // the Overwatch's reply (left, a rule, markdown with its source links, a
  // "↳ re:" reference when the question is not just above, clamped at 12 lines).
  import { app, features, now30, openFeature, projectName } from '../../lib/app.svelte.js';
  import { clock, dayTime } from '../../lib/format.js';
  import { plannedEntries } from '../../lib/intake.svelte.js';
  import { mentionRuns, snapshotOf } from '../../lib/mentions.js';
  import { deliveryText } from '../../lib/overwatch.js';
  import { tooltip } from '../../lib/tooltip.js';
  import Markdown from '../Markdown.svelte';

  /** @type {{ item: any, ow: any, pulse?: boolean, reply?: { id: string, text: string } | null, note?: string, failed?: string | null, past?: boolean, today?: string | null, reflabel?: (ref: any) => string | null, onref?: (ref: any) => void, refok?: (ref: any) => boolean, onreplyto?: (id: string) => void, onremove?: () => void, onretry?: () => void, onsession?: () => void }} */
  let { item, ow, pulse = false, reply = null, note = '', failed = null, past = false, today = null, reflabel, onref, refok, onreplyto = () => {}, onremove = () => {}, onretry = () => {}, onsession } = $props();
  const e = $derived(item.entry);
  const user = $derived(e.author === 'user');
  const planned = $derived.by(() => {
    if (!user) return null;
    const p = e.meta?.plan?.project ?? e.meta?.plan ?? e.context?.plan ?? plannedEntries[e.id] ?? null;
    return typeof p === 'string' ? p : null;
  });
  // The features this user text mentions: the entry's resolved snapshot (`meta.mentions`), else the
  // ids of the context it was sent with (a waking message), resolved from the features loaded.
  const mentioned = $derived.by(() => {
    if (!user) return [];
    if (e.meta?.mentions?.length) return e.meta.mentions;
    if (!e.context?.mentions?.length) return [];
    const byId = new Map(features().map((f) => [f.id, f]));
    return (e.context?.mentions ?? []).flatMap((m) => (byId.has(m.id) ? [snapshotOf(byId.get(m.id), app.data?.projects)] : []));
  });
  const runs = $derived(user ? mentionRuns(e.text ?? '', mentioned) : []);
  // A feature is "removed" only once the features have loaded (an archived one is still listed).
  const gone = (m) => !!app.data && !features().some((f) => f.id === m.id);
  const now = $derived(Math.max(now30(), Date.now()));
  // A past day's entry carries its day ("Mon 2 Nov 18:40") and no live delivery state (notes §2).
  const stamp = $derived(past && today && e.day && e.day !== today ? dayTime(e.at_ms) : clock(e.at_ms, now));
  const marker = $derived(user && !past ? deliveryText(e, ow) : '');
  let more = $state(false);
  let bodyEl = $state();
  // Long = the clamp hides something: measured, not guessed from the characters (notes §2.1).
  let over = $state(false);
  // Measured again when the body changes size (a diagram or an image arrives later).
  $effect(() => {
    e.text;
    if (!bodyEl) return;
    const measure = () => {
      if (!more) over = bodyEl.scrollHeight > bodyEl.clientHeight + 1;
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(bodyEl);
    return () => ro.disconnect();
  });
  const long = $derived(!user && (over || more));

  function linkKey(ev, id) {
    if (ev.key !== 'Enter') return;
    ev.preventDefault();
    ev.stopPropagation();
    openFeature(id);
  }

  function onkeydown(ev) {
    if (ev.key !== 'Enter' || ev.target !== ev.currentTarget || !long) return;
    ev.preventDefault();
    more = !more;
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_tabindex -->
<article
  class="text"
  class:user
  class:ow={!user}
  class:pulse
  aria-label="{user ? 'You' : 'Overwatch'}, {stamp}"
  data-entry={e.id}
  data-author={e.author}
  tabindex={long ? 0 : undefined}
  {onkeydown}
>
  {#if reply}<button class="ref" onclick={() => onreplyto(reply.id)} aria-label="Reply to: {reply.text}">↳ re: {reply.text}</button>{/if}
  <div class="label muted">{user ? 'You' : 'Overwatch'} · {stamp}</div>
  {#if user}
    <!-- No whitespace between the runs: the body is pre-wrap. -->
    <div class="body plain">{#each runs as r}{#if !r.m}{r.t}{:else if gone(r.m)}<span class="mention gone" use:tooltip={'Feature removed'}>{r.t}<span class="vh">{' (feature removed)'}</span></span>{:else}<span class="mention" role="link" tabindex="0" aria-label="Open feature {r.t}" use:tooltip={`${r.m.title} · ${r.m.project}`} onclick={() => openFeature(r.m.id)} onkeydown={(ev) => linkKey(ev, r.m.id)}>{r.t}</span>{/if}{/each}</div>
    {#if planned}<div class="muted" data-planning>planning · {projectName(planned)}</div>{/if}
  {:else}
    <div class="body" class:clamped={!more} class:fits={!over} bind:this={bodyEl}>
      <Markdown text={e.text ?? ''} {onref} {refok} {reflabel} />
    </div>
    {#if long}<button class="link more" aria-expanded={more} onclick={() => (more = !more)}>{more ? 'Show less' : 'Show more'}</button>{/if}
  {/if}
  {#if failed}
    <div class="mark fail" role="alert">Not sent: {failed} · <button class="link" onclick={onretry}>Retry</button></div>
  {:else if marker || (e.delivery === 'pending' && !past)}
    <div class="mark muted">
      {marker}{#if e.delivery === 'pending' && !past}{' · '}<button class="link" onclick={onremove}>Remove</button>{/if}
    </div>
  {/if}
  {#if note}
    <div class="mark muted" data-noreply>
      {#if onsession}No reply. Ask again, or <button class="link" onclick={onsession}>look at its session</button>.{:else}{note}{/if}
    </div>
  {/if}
</article>

<style>
  .text {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    min-width: 0;
    flex: none;
    outline: none;
  }
  .text:focus-visible {
    box-shadow: 0 0 0 var(--border-width-focus) var(--color-focus);
    border-radius: var(--radius-sm);
  }
  .user {
    align-self: flex-end;
    max-width: 85%;
  }
  .user .label {
    text-align: right;
  }
  .user .body {
    padding: var(--space-sm) var(--space-md);
    background: var(--color-accent-subtle);
    border-radius: var(--radius-md);
    text-align: left;
  }
  .plain {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }
  /* A mention in a user text (design-notes §4 and the ui section). */
  .mention {
    padding: 0 var(--space-xs);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--color-accent-default) 22%, var(--color-accent-subtle));
    color: var(--color-text-primary);
    font-weight: var(--font-weight-semibold);
    box-decoration-break: clone;
    -webkit-box-decoration-break: clone;
    overflow-wrap: anywhere;
    cursor: pointer;
  }
  .mention:hover {
    text-decoration: underline;
    text-decoration-color: currentColor;
    text-decoration-thickness: 1px;
    text-underline-offset: 2px;
  }
  .mention:focus-visible {
    outline: none;
    box-shadow: 0 0 0 var(--border-width-focus) var(--color-focus);
  }
  .mention.gone:hover,
  .mention.gone {
    padding: 0;
    background: none;
    color: var(--color-text-secondary);
    font-weight: inherit;
    text-decoration: line-through;
    cursor: default;
  }
  .vh {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  .ow {
    padding-left: var(--space-sm);
    border-left: 2px solid var(--color-border-strong);
  }
  .label {
    font: var(--text-caption);
  }
  .text.pulse {
    animation: pulse 1.5s ease-out;
  }
  @keyframes pulse {
    from {
      background: color-mix(in srgb, var(--color-accent-default) 12%, transparent);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .text.pulse {
      animation: none;
      background: color-mix(in srgb, var(--color-accent-default) 12%, transparent);
    }
  }
  .body.clamped {
    max-height: calc(12 * 1.5em);
    overflow: hidden;
  }
  .body.clamped:not(.fits) {
    mask-image: linear-gradient(to bottom, black calc(100% - 2em), transparent);
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
  .more {
    align-self: flex-start;
  }
  .mark {
    font: var(--text-caption);
  }
  .user .mark {
    text-align: right;
  }
  .fail {
    color: var(--color-status-error);
  }
</style>
