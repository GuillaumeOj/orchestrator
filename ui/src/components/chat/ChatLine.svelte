<script>
  // A settled card collapsed to one line (P4.3 notes §2.2–2.4, P4.2 notes §8.4):
  // `▸ allowed back-1: cargo add serde_json · rule: installs   14:02`. Verb and
  // agent secondary, what primary (truncates, full text in the tooltip), tail
  // and time muted. ⏎ or a click expands the read-only card from the snapshot.
  // P4.4 notes §5–§6: the Overwatch's lines carry their sources and an Undo
  // (`u`, or the button when expanded), "undone 14:03" once used, "Can't be
  // undone: …" when it can't be; "Correct the answer" opens a reply field.
  import { tick } from 'svelte';
  import { now30, toast, undoLine } from '../../lib/app.svelte.js';
  import { lineLabel, lineText, tailText, verbText, warnVerb } from '../../lib/chat.js';
  import { clock } from '../../lib/format.js';
  import { errText } from '../../lib/ipc.js';
  import { undoChip, undoInfo, undoneTail } from '../../lib/overwatch.js';
  import Kbd from '../Kbd.svelte';
  import DecisionCard from './DecisionCard.svelte';

  /** @type {{ item: any, expanded?: boolean, pulse?: boolean, entries?: any, refok?: (ref: any) => boolean, ontoggle?: () => void, onopen?: (id: string) => void, onref?: (ref: any) => void, onexpand?: () => void }} */
  let { item, expanded = false, pulse = false, entries = {}, refok = () => true, ontoggle = () => {}, onopen = () => {}, onref = () => {}, onexpand = () => {} } = $props();
  const line = $derived(item.line);
  const now = $derived(Math.max(now30(), Date.now()));
  const tail = $derived(tailText(line, entries));
  const undone = $derived(undoneTail(line, now));
  const chip = $derived(undoChip(line));
  const info = $derived(undoInfo(line, item.d?.subject?.project ?? null, now));
  const showUndo = $derived(info.state !== 'none' || line.by === 'overwatch' || line.by === 'core' || line.by === 'policy');

  let busy = $state(false);
  let failed = $state('');
  let correcting = $state(false);
  let draft = $state(''); // kept until sent or cancelled, across collapse and expand
  let field = $state();

  async function run(text = null) {
    if (busy || info.state !== 'available') return;
    busy = true;
    failed = '';
    try {
      const r = await undoLine(item.entry, text);
      if (r?.ok === false) failed = r.gone ? `Can't be undone: ${r.gone}` : 'Could not undo.';
      else {
        correcting = false;
        draft = '';
        toast(info.toast);
      }
    } catch (err) {
      failed = `Could not undo: ${errText(err)}`;
    } finally {
      busy = false;
    }
  }
  /** The chip, `u` and the body's button: a correction opens its field, the rest act. */
  async function press() {
    if (!info.input) return run();
    if (!expanded) onexpand();
    correcting = true;
    await tick();
    field?.focus();
  }
  function fieldKey(e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      correcting = false;
    } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey) && draft.trim()) {
      e.preventDefault();
      run(draft.trim());
    }
  }
</script>

<article class="line" class:pulse class:leftopen={line.verb === 'left open'} class:undone={!!undone} aria-label={lineLabel(line, now)} data-line={item.id} data-entry={item.key} data-verb={line.verb}>
  <div class="row" class:open={expanded}>
    <button class="toggle" aria-expanded={expanded} title={lineText(line, entries)} onclick={ontoggle} data-line-toggle={item.key}>
      <span class="caret" aria-hidden="true">{expanded ? '▾' : '▸'}</span>
      <span class="head"><span class="verb" class:warn={warnVerb(line.verb)}>{verbText(line.verb)}</span>{line.agent ? ` ${line.agent}` : ''}:</span>
      <span class="what">{line.what ?? ''}</span>
      {#if tail}<span class="tail muted">· {tail}</span>{/if}
      {#if undone}<span class="tail muted">· {undone}</span>{/if}
    </button>
    {#if chip}<button class="chip link" data-undo-run onclick={press} aria-label={info.label}>{info.short} <Kbd keys="u" /></button>{/if}
    <span class="at muted">{clock(line.at_ms, now)}</span>
  </div>
  {#if expanded}
    {#if item.d}
      <DecisionCard d={item.d} {line} {entries} {refok} {onref} onact={onopen} />
    {:else}
      <p class="body">{lineText(line, entries)}</p>
    {/if}
    {#if showUndo}
      <div class="undo" data-undo-body={info.state}>
        {#if info.state === 'done'}
          <p class="muted" data-undo-done>{info.done}</p>
        {:else if info.state === 'available'}
          {#if correcting}
            <textarea class="field" rows="2" bind:this={field} bind:value={draft} onkeydown={fieldKey} aria-label="Correction" placeholder="What should {line.agent ?? 'the agent'} know instead?"></textarea>
            <div class="acts">
              <button class="btn" disabled={busy || !draft.trim()} onclick={() => run(draft.trim())}>{busy ? info.busy : 'Send correction'} <Kbd keys="⌘+⏎" /></button>
              <button class="btn ghost" onclick={() => (correcting = false)}>Cancel <Kbd keys="esc" /></button>
            </div>
          {:else}
            <button class="btn" data-undo-run disabled={busy} onclick={press}>{busy ? info.busy : info.label}</button>
            <p class="muted">{info.explain}</p>
          {/if}
          {#if failed}<p class="err" role="alert">{failed}</p>{/if}
        {:else if info.why}
          <p class="muted" data-undo-why>{info.why}</p>
        {/if}
      </div>
    {/if}
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
    color: var(--color-text-primary);
    font: var(--text-label);
    white-space: nowrap;
  }
  .toggle {
    display: flex;
    flex: 1;
    align-items: center;
    gap: var(--space-xs);
    min-width: 0;
    height: 100%;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-align: left;
    white-space: nowrap;
  }
  .row:hover {
    background: var(--color-surface-raised);
  }
  .row:has(.toggle:focus-visible) {
    border-color: var(--color-focus);
  }
  .toggle:focus-visible {
    outline: none;
  }
  .row.open {
    background: var(--color-accent-subtle);
  }
  .chip {
    flex: none;
    padding: 0;
    font: var(--text-caption);
  }
  .undone,
  .undone .what {
    color: var(--color-text-secondary);
  }
  .body {
    margin: 0;
    padding: 0 var(--space-md);
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }
  .undo {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    align-items: flex-start;
    padding: 0 var(--space-md) var(--space-xs);
  }
  .undo p {
    margin: 0;
    font: var(--text-caption);
  }
  .undo .field {
    width: 100%;
  }
  .acts {
    display: flex;
    gap: var(--space-xs);
  }
  .err {
    color: var(--color-status-error);
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
  .leftopen .row,
  .leftopen .head,
  .leftopen .caret {
    color: var(--color-text-muted);
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
