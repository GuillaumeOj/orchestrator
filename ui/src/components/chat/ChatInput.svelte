<script>
  // The input at the foot of the chat (P4.4 notes §2, spec §3.9), in place of the
  // P4.3 footer: "Message the Overwatch · ⏎ to send". ⏎ sends, ⇧⏎ a new line, Esc leaves to the
  // thread and keeps the draft, ⌘L focuses it from anywhere. Never disabled by
  // the status: the message waits and says so. The draft lives in `app.chatDraft`.
  // `@` opens the list of features to mention (add-mention-in-overwatch design-notes): a
  // combobox on this textarea, the keys below, the picks kept in `app.chatMentions`.
  // A long plain-text paste becomes a placeholder `[Pasted text #1 · 5 000 chars]` (allow-long-text
  // spec §3.2); the text waits in `app.chatPastes` and goes with the send (lib/pastes.js).
  import { tick } from 'svelte';
  import { app, features, openSettings, projectName } from '../../lib/app.svelte.js';
  import { clearPlan, intakeUi } from '../../lib/intake.svelte.js';
  import { candidates, countText, insertToken, mentionsToSend, tokenAtCaret } from '../../lib/mentions.js';
  import { COUNT_FROM, inputHint, MAX_CHARS } from '../../lib/overwatch.js';
  import { atInPlaceholder, countChars, countLines, diffEdit, isLong, keep, nextN, normalize, pasteError, pastesToSend, placeholder, spaced, wholeAt } from '../../lib/pastes.js';
  import MentionPopup from './MentionPopup.svelte';

  /** @type {{ ow: any, onsend?: (text: string, mentions: any[], pastes: { n: number, text: string, chars: number, lines: number }[]) => void, onleave?: () => void, onannounce?: (msg: string) => void }} */
  let { ow, onsend = () => {}, onleave = () => {}, onannounce = () => {} } = $props();
  let field = $state();
  const LIST_ID = 'ow-mention-list';
  const optionId = (i) => `ow-mention-opt-${i}`;
  const HELP = 'Use up and down arrows to choose, enter to insert, escape to close';
  // The token being completed while the popup is open; `closedFor` is the token a close
  // (Esc, ⏎ / ⇥ with nothing to pick) keeps shut until it is edited.
  let tok = $state(null);
  let activeId = $state(null); // the highlighted feature (an id survives a refresh that reorders the rows)
  let closedFor = null;
  const open = $derived(!!tok);
  const projs = $derived(app.data?.projects ?? []);
  const cands = $derived(tok ? candidates(features(), projs, tok.query) : { current: [], other: [], rows: [] });
  // The highlighted row: the id's, else the first (it left the list or none yet).
  const cur = $derived(Math.max(0, cands.rows.findIndex((r) => r.id === activeId)));
  const count = $derived(cands.rows.length);
  const noFeatures = $derived(!!app.data && features().every((f) => f.archived));
  const draft = $derived(app.chatDraft);
  const n = $derived([...draft.trim()].length); // code points of the trimmed text, like chat_send
  const over = $derived(n > MAX_CHARS);
  // The hint for the status; its link, when it has one, opens the Overwatch settings.
  const hint = $derived(inputHint(ow));

  // A refused paste says why until the next keystroke, paste or send (design-notes §1).
  let pasteErr = $state('');

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
    const msg = countText(count, !noFeatures);
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
    const sent = pastesToSend(text, app.chatPastes);
    app.chatDraft = '';
    app.chatMentions = [];
    app.chatPastes = [];
    pasteErr = '';
    tok = null;
    closedFor = null;
    onsend(text, mentions, sent);
  }

  /** Replace `from..to` of the field by `s` through the textarea's own editing, so ⌘Z undoes it
   *  (no webview editing command: the same edit by hand, announced as typing). */
  function edit(from, to, s) {
    field.focus();
    field.setSelectionRange(from, to);
    const done = typeof document.execCommand === 'function' && (s ? document.execCommand('insertText', false, s) : document.execCommand('delete'));
    if (done) return;
    field.setRangeText(s, from, to, 'end');
    field.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: s ? 'insertText' : 'deleteContentBackward' }));
  }

  /** The field becomes `text` with the caret at `caret`, by the smallest edit (the native undo stack stays). */
  function replaceDraft(text, caret) {
    const { from, to, s } = diffEdit(field.value, text);
    if (from !== to || s) edit(from, to, s);
    field.setSelectionRange(caret, caret);
  }

  /** A long plain-text paste: a placeholder at the caret (or over the selection), the text kept aside. */
  function onpaste(e) {
    pasteErr = '';
    const text = normalize(e.clipboardData?.getData('text/plain') ?? '');
    if (!isLong(text)) return; // short or not text: pasted as today
    e.preventDefault();
    const chars = countChars(text);
    // The selection this paste replaces does not count: its placeholders go away with it.
    const rest = field.value.slice(0, field.selectionStart) + field.value.slice(field.selectionEnd);
    const err = pasteError(rest, chars, app.chatPastes);
    if (err) {
      pasteErr = err;
      return;
    }
    const n = nextN(app.chatPastes);
    app.chatPastes = keep(rest, app.chatPastes, { n, text, chars, lines: countLines(text) });
    edit(field.selectionStart, field.selectionEnd, placeholder(n, chars));
    onannounce(`Pasted ${spaced(chars)} characters as pasted text ${n}`);
  }

  /** ⌫ just after a placeholder, ⌦ just before it (⌥⌫ too): the whole placeholder goes, ⌘Z brings it back. */
  function removeWhole(e) {
    const back = e.key === 'Backspace';
    if ((!back && e.key !== 'Delete') || e.metaKey || e.ctrlKey || field.selectionStart !== field.selectionEnd) return false;
    const t = wholeAt(field.value, field.selectionStart, back ? 'back' : 'forward', app.chatPastes);
    if (!t) return false;
    e.preventDefault();
    edit(t.start, t.end, '');
    onannounce(`Removed pasted text ${t.n}`);
    return true;
  }

  /** The token at the caret decides the popup. Typing may open it; a caret move only updates or closes it. */
  function sync(typing) {
    if (picking) return; // the pick's own edit fires an input event: it must not reopen the popup
    let t = field && app.data ? tokenAtCaret(field.value, field.selectionStart) : null;
    if (t && atInPlaceholder(field.value, t.start, field.selectionStart, app.chatPastes)) t = null; // no `@` completes inside a placeholder
    if (!t) {
      tok = null;
      closedFor = null;
      return;
    }
    const kept = closedFor?.start === t.start && closedFor.query === t.query;
    if (tok && t.start === tok.start) {
      if (t.query !== tok.query) (tok = t), (activeId = null);
    } else if (typing && !kept) {
      tok = t;
      activeId = null;
      closedFor = null;
    } else tok = null;
  }

  // Only typing opens the popup: not undo / redo, a drop, a paste or an autocorrect (design §5).
  const typed = (type) => !type || type === 'insertText' || type === 'insertCompositionText' || type.startsWith('delete');

  function close(keep) {
    if (keep && tok) closedFor = { ...tok };
    tok = null;
  }

  let picking = false;
  async function pick(row) {
    const { text, caret } = insertToken(field.value, tok.start, field.selectionStart, row.token);
    app.chatMentions = [...app.chatMentions.filter((m) => m.token !== row.token), { token: row.token, kind: 'feature', id: row.id }];
    tok = null;
    closedFor = null;
    picking = true;
    try {
      replaceDraft(text, caret); // through the field's own editing: ⌘Z still restores a removed paste
    } finally {
      picking = false;
    }
    if (app.chatDraft !== field.value) app.chatDraft = field.value; // an editing command that fired no input event
    await tick();
    field?.setSelectionRange(caret, caret);
  }

  /** The popup's keys; true when it took the key. */
  function mentionKey(e) {
    const plain = !e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey;
    const rows = cands.rows;
    const down = e.key === 'ArrowDown' || (e.ctrlKey && !e.metaKey && e.key === 'n');
    const up = e.key === 'ArrowUp' || (e.ctrlKey && !e.metaKey && e.key === 'p');
    if ((down || up) && !rows.length) return false; // nothing to move in: the caret moves
    if (down) activeId = rows[Math.min(cur + 1, rows.length - 1)].id;
    else if (up) activeId = rows[Math.max(0, cur - 1)].id;
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
    if (pasteErr && !['Shift', 'Meta', 'Control', 'Alt'].includes(e.key)) pasteErr = '';
    if (open && !e.isComposing && e.keyCode !== 229 && mentionKey(e)) return;
    if (!e.isComposing && e.keyCode !== 229 && removeWhole(e)) return;
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
    <MentionPopup current={cands.current} other={cands.other} active={cur} query={tok.query} none={noFeatures} id={LIST_ID} {optionId} onpick={pick} onhover={(i) => (activeId = cands.rows[i]?.id ?? null)} />
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
    {onpaste}
    oninput={(e) => sync(typed(/** @type {any} */ (e).inputType))}
    onkeyup={() => sync(false)}
    onclick={() => sync(false)}
    onblur={() => close(false)}
    role="combobox"
    aria-autocomplete="list"
    aria-expanded={open}
    aria-controls={open && count ? LIST_ID : undefined}
    aria-activedescendant={open && count ? optionId(cur) : undefined}
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
  {#if pasteErr}<p class="warn" role="alert">{pasteErr}</p>
  {:else if over}<p class="warn" role="alert">Too long for a message. A paste of over 800 characters or 10 lines is sent as a file; shorter text stays in the message.</p>{/if}
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
