<script>
  // The input at the foot of the chat (P4.4 notes §2, spec §3.9), in place of the
  // P4.3 footer: "Ask the Overwatch…". ⏎ sends, ⇧⏎ a new line, Esc leaves to the
  // thread and keeps the draft, ⌘L focuses it from anywhere. Never disabled by
  // the status: the message waits and says so. The draft lives in `app.chatDraft`.
  import { app, openSettings } from '../../lib/app.svelte.js';
  import { COUNT_FROM, inputHint, MAX_CHARS } from '../../lib/overwatch.js';
  import Kbd from '../Kbd.svelte';

  /** @type {{ ow: any, onsend?: (text: string) => void, onleave?: () => void }} */
  let { ow, onsend = () => {}, onleave = () => {} } = $props();
  let field = $state();
  let focused = $state(false);
  const draft = $derived(app.chatDraft);
  const n = $derived(draft.length);
  const over = $derived(n > MAX_CHARS);
  const hint = $derived(inputHint(ow));

  const spaced = (x) => x.toLocaleString('en-US').replace(',', ' ');

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

  function submit() {
    const text = draft.trim();
    if (!text || over) return;
    app.chatDraft = '';
    onsend(text);
  }

  function onkeydown(e) {
    if (e.key === 'Enter' && !e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && !e.isComposing) {
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
  <textarea
    class="field"
    rows="1"
    bind:this={field}
    bind:value={app.chatDraft}
    {onkeydown}
    onfocus={() => (focused = true)}
    onblur={() => (focused = false)}
    aria-label="Message the Overwatch"
    aria-describedby="ow-input-hint"
    aria-invalid={over || undefined}
    placeholder="Ask the Overwatch…"
  ></textarea>
  <div class="hints muted" id="ow-input-hint">
    <span class="state">
      {#if ow.status === 'off'}Off · <button class="link" onclick={() => openSettings('overwatch')}>turn it on in Settings</button>{:else}{hint}{/if}
      {#if n >= COUNT_FROM}<span class:over data-count>{spaced(n)} / {spaced(MAX_CHARS)}</span>{/if}
    </span>
    <span class="keys">
      {#if focused || draft}<Kbd keys="⏎" /> send · <Kbd keys="⇧+⏎" /> new line · <Kbd keys="esc" /> leave{:else}<Kbd keys="⌘+L" />{/if}
    </span>
  </div>
  {#if over}<p class="warn" role="alert">Too long for a message. Put it in a file and ask the Overwatch to read it.</p>{/if}
</div>

<style>
  .input {
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
  .keys {
    flex: none;
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
