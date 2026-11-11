<script>
  // The input at the foot of the chat (P4.4 notes §2, spec §3.9), in place of the
  // P4.3 footer: "Message the Overwatch · ⏎ to send". ⏎ sends, ⇧⏎ a new line, Esc leaves to the
  // thread and keeps the draft, ⌘L focuses it from anywhere. Never disabled by
  // the status: the message waits and says so. The draft lives in `app.chatDraft`.
  // `@` opens the list of features to mention (add-mention-in-overwatch design-notes): a
  // combobox on this textarea, the keys below, the picks kept in `app.chatMentions`.
  import { tick } from 'svelte';
  import { app, features, openSettings, projectName } from '../../lib/app.svelte.js';
  import { clearPlan, intakeUi } from '../../lib/intake.svelte.js';
  import { candidates, countText, insertToken, mentionsToSend, tokenAtCaret } from '../../lib/mentions.js';
  import { COUNT_FROM, inputHint, MAX_CHARS } from '../../lib/overwatch.js';
  import MentionPopup from './MentionPopup.svelte';

  /** @type {{ ow: any, onsend?: (text: string, mentions: any[]) => void, onleave?: () => void, onannounce?: (msg: string) => void }} */
  let { ow, onsend = () => {}, onleave = () => {}, onannounce = () => {} } = $props();
  let field = $state();
  const LIST_ID = 'ow-mention-list';
  const optionId = (i) => `ow-mention-opt-${i}`;
  const HELP = 'Use up and down arrows to choose, enter to insert, escape to close';
  // The token being completed while the popup is open; `closedFor` is the token a close
  // (Esc, ⏎ / ⇥ with nothing to pick) keeps shut until it is edited.
  let tok = $state(null);
  let active = $state(0);
  let closedFor = null;
  const open = $derived(!!tok);
  const projs = $derived(app.data?.projects ?? []);
  const cands = $derived(tok ? candidates(features(), projs, tok.query) : { current: [], other: [], rows: [] });
  // The highlighted row, kept in range when the features change under the open popup (a poll).
  const cur = $derived(Math.min(active, Math.max(0, cands.rows.length - 1)));
  const noFeatures = $derived(features().every((f) => f.archived));
  const draft = $derived(app.chatDraft);
  const n = $derived([...draft.trim()].length); // code points of the trimmed text, like chat_send
  const over = $derived(n > MAX_CHARS);
  // The hint for the status; its link, when it has one, opens the Overwatch settings.
  const hint = $derived(inputHint(ow));

  const spaced = (x) => x.toLocaleString('en-US').replaceAll(',', ' ');

  export function focus() {
    field?.focus();
  }

  // The field grows with its text to 5 lines (notes §2).
  $effect(() => {
    draft;
    if (!field) return;
    field.style.height = 'auto';
    field.style.height = `${Math.min(field.scrollHeight, 120)}px`;
  });

  // The live region: the count on open and on each query change (debounced), the help once.
  $effect(() => {
    if (!open) return;
    const msg = countText(cands.rows.length, !noFeatures);
    const t = setTimeout(() => {
      onannounce(helped ? msg : `${msg}. ${HELP}`);
      helped = true;
    }, 250);
    return () => clearTimeout(t);
  });
  let helped = false;
  $effect(() => {
    if (!open) helped = false;
  });

  function submit() {
    const text = draft.trim();
    if (!text || over) return;
    const mentions = mentionsToSend(text, app.chatMentions, features(), projs);
    app.chatDraft = '';
    app.chatMentions = [];
    tok = null;
    closedFor = null;
    onsend(text, mentions);
  }

  /** The token at the caret decides the popup. Typing may open it; a caret move only updates or closes it. */
  function sync(typing) {
    const t = field ? tokenAtCaret(field.value, field.selectionStart) : null;
    if (!t) {
      tok = null;
      closedFor = null;
      return;
    }
    const kept = closedFor?.start === t.start && closedFor.query === t.query;
    if (tok && t.start === tok.start) {
      if (t.query !== tok.query) (tok = t), (active = 0);
    } else if (typing && !kept) {
      tok = t;
      active = 0;
      closedFor = null;
    } else tok = null;
  }

  function close(keep) {
    if (keep && tok) closedFor = { ...tok };
    tok = null;
  }

  async function pick(row) {
    const { text, caret } = insertToken(field.value, tok.start, field.selectionStart, row.token);
    app.chatDraft = text;
    app.chatMentions = [...app.chatMentions.filter((m) => m.token !== row.token), { token: row.token, kind: 'feature', id: row.id }];
    tok = null;
    closedFor = null;
    await tick();
    field?.setSelectionRange(caret, caret);
  }

  /** The popup's keys; true when it took the key. */
  function mentionKey(e) {
    const plain = !e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey;
    const last = cands.rows.length - 1;
    if (e.key === 'ArrowDown' || (e.ctrlKey && !e.metaKey && e.key === 'n')) active = Math.max(0, Math.min(cur + 1, last));
    else if (e.key === 'ArrowUp' || (e.ctrlKey && !e.metaKey && e.key === 'p')) active = Math.max(0, cur - 1);
    else if ((e.key === 'Enter' || e.key === 'Tab') && plain) {
      if (cands.rows.length) pick(cands.rows[cur]);
      else close(true);
    } else if (e.key === 'Escape') {
      e.stopPropagation();
      close(true);
    } else return false;
    e.preventDefault();
    return true;
  }

  function onkeydown(e) {
    if (open && !e.isComposing && e.keyCode !== 229 && mentionKey(e)) return;
    if (e.key === 'Enter' && !e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && !e.isComposing && e.keyCode !== 229) {
      e.preventDefault();
      submit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onleave();
    }
  }
</script>

<div class="input" data-overwatch-input>
  {#if open}
    <MentionPopup current={cands.current} other={cands.other} active={cur} query={tok.query} none={noFeatures} id={LIST_ID} {optionId} onpick={pick} onhover={(i) => (active = i)} />
  {/if}
  {#if intakeUi.plan}
    <span class="plan muted" data-plan-tag>Planning · {projectName(intakeUi.plan)} <button class="x" type="button" aria-label="Stop planning" onclick={() => (clearPlan(), field?.focus())}>✕</button></span>
  {/if}
  <textarea
    class="field"
    rows="1"
    bind:this={field}
    bind:value={app.chatDraft}
    {onkeydown}
    oninput={(e) => sync(!/** @type {any} */ (e).inputType?.startsWith('insertFromPaste'))}
    onkeyup={() => sync(false)}
    onclick={() => sync(false)}
    onblur={() => close(false)}
    role="combobox"
    aria-autocomplete="list"
    aria-expanded={open}
    aria-controls={open ? LIST_ID : undefined}
    aria-activedescendant={open && cands.rows.length ? optionId(cur) : undefined}
    aria-label="Message the Overwatch"
    aria-describedby="ow-input-hint"
    aria-invalid={over || undefined}
    placeholder="Message the Overwatch · ⏎ to send"
  ></textarea>
  <div class="hints muted" id="ow-input-hint">
    <span class="state">
      {hint.text}{#if hint.link}<button class="link" onclick={() => openSettings('overwatch')}>{hint.link}</button>{/if}
      {#if n >= COUNT_FROM}<span class:over data-count>{spaced(n)} / {spaced(MAX_CHARS)}</span>{/if}
    </span>
  </div>
  {#if over}<p class="warn" role="alert">Too long for a message. Put it in a file and ask the Overwatch to read it.</p>{/if}
</div>

<style>
  .plan {
    align-self: flex-start;
    font: var(--text-caption);
  }
  .plan .x {
    padding: 0 var(--space-xs);
    border: 0;
    background: none;
    color: inherit;
    cursor: pointer;
  }
  .input {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    flex: none;
    padding: var(--space-sm) var(--space-md);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
  textarea {
    min-height: 40px;
    max-height: 120px;
    resize: none;
    overflow-y: auto;
  }
  .hints {
    display: flex;
    justify-content: space-between;
    gap: var(--space-sm);
    min-height: 20px;
    font: var(--text-caption);
  }
  .state {
    min-width: 0;
  }

  .over {
    color: var(--color-status-error);
  }
  .warn {
    margin: 0;
    color: var(--color-status-error);
    font: var(--text-caption);
  }
</style>
