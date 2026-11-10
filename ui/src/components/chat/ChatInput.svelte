<script>
  // The input at the foot of the chat (P4.4 notes §2, spec §3.9), in place of the
  // P4.3 footer: "Message the Overwatch · ⏎ to send". ⏎ sends, ⇧⏎ a new line, Esc leaves to the
  // thread and keeps the draft, ⌘L focuses it from anywhere. Never disabled by
  // the status: the message waits and says so. The draft lives in `app.chatDraft`.
  import { app, openSettings, projectName } from '../../lib/app.svelte.js';
  import { clearPlan, intakeUi } from '../../lib/intake.svelte.js';
  import { COUNT_FROM, inputHint, MAX_CHARS } from '../../lib/overwatch.js';

  /** @type {{ ow: any, onsend?: (text: string) => void, onleave?: () => void }} */
  let { ow, onsend = () => {}, onleave = () => {} } = $props();
  let field = $state();
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

  function submit() {
    const text = draft.trim();
    if (!text || over) return;
    app.chatDraft = '';
    onsend(text);
  }

  function onkeydown(e) {
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
  {#if intakeUi.plan}
    <span class="plan muted" data-plan-tag>Planning · {projectName(intakeUi.plan)} <button class="x" type="button" aria-label="Stop planning" onclick={() => (clearPlan(), field?.focus())}>✕</button></span>
  {/if}
  <textarea
    class="field"
    rows="1"
    bind:this={field}
    bind:value={app.chatDraft}
    {onkeydown}
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
