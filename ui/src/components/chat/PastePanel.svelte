<script>
  // The expansion of a paste chip, under the user's bubble (allow-long-text design-notes §3): a header
  // (`#1 · 5 000 chars · 120 lines`, Copy, ✕) and the text, mono and pre-wrapped, 16 rows then it scrolls.
  // A sent entry's text is fetched once with `chat_paste` (lib/paste-store.svelte.js); an unsent one has it.
  import { loadPaste, pasteKey, pasteUi } from '../../lib/paste-store.svelte.js';
  import { panelHead } from '../../lib/pastes.js';
  import { tooltip } from '../../lib/tooltip.js';

  /** @type {{ entry: string, paste: { n: number, chars: number, lines: number, text?: string }, id: string, onclose?: () => void, onannounce?: (msg: string) => void }} */
  let { entry, paste, id, onclose = () => {}, onannounce = () => {} } = $props();
  const st = $derived(paste.text != null ? { status: 'ok', text: paste.text } : (pasteUi.loaded[pasteKey(entry, paste.n)] ?? { status: 'loading' }));
  let copy = $state('Copy'); // 'Copy' | 'Copied' | 'Copy failed', back to Copy after 1.5 s
  let timer;

  async function copyText() {
    try {
      await navigator.clipboard.writeText(st.text);
      copy = 'Copied';
      onannounce(`Copied pasted text ${paste.n}`);
    } catch {
      copy = 'Copy failed';
    }
    clearTimeout(timer);
    timer = setTimeout(() => (copy = 'Copy'), 1500);
  }

  // Esc collapses and gives the focus back to the chip; every other key stays the panel's own.
  function onkeydown(ev) {
    if (ev.key !== 'Escape') return;
    ev.preventDefault();
    ev.stopPropagation();
    onclose();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="paste-panel" {id} data-paste-panel={paste.n} {onkeydown}>
  <div class="head muted">
    <span class="lab">{panelHead(paste)}</span>
    {#if st.status !== 'missing'}<button class="act" type="button" disabled={st.status !== 'ok'} onclick={copyText}>{copy}</button>{/if}
    <button class="act x" type="button" aria-label="Hide pasted text {paste.n}" use:tooltip={'Hide'} onclick={onclose}>✕</button>
  </div>
  {#if st.status === 'ok'}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div class="text" role="region" aria-label="Pasted text {paste.n}" tabindex="0">{st.text}</div>
  {:else if st.status === 'missing'}
    <p class="note muted" role="status">Pasted text no longer available</p>
  {:else if st.status === 'error'}
    <p class="note muted">Could not load pasted text: {st.error} · <button class="link" type="button" onclick={() => loadPaste(entry, paste.n)}>Retry</button></p>
  {:else}
    <p class="note muted" aria-busy="true">Loading pasted text…</p>
  {/if}
</div>

<style>
  .paste-panel {
    min-width: min(240px, 100%);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    text-align: left;
  }
  .head {
    display: flex;
    align-items: flex-start;
    gap: var(--space-xs);
    min-height: 28px;
    padding-left: var(--space-sm);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-caption);
  }
  .lab {
    flex: 1;
    min-width: 0;
    padding-block: 5px;
    overflow-wrap: break-word;
  }
  .act {
    flex: none;
    height: var(--size-control-sm);
    margin-top: 3px;
    padding: 0 var(--space-xs);
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  .act:hover:not(:disabled) {
    color: var(--color-text-primary);
  }
  .act:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .act:focus-visible,
  .text:focus-visible {
    outline: none;
    box-shadow: 0 0 0 var(--border-width-focus) var(--color-focus);
  }
  .x {
    margin-right: var(--space-xs);
  }
  .text {
    padding: var(--space-sm);
    max-height: calc(16 * 1.33em + 2 * var(--space-sm));
    overflow-y: auto;
    overscroll-behavior: contain;
    color: var(--color-text-primary);
    font: var(--text-mono-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }
  .note {
    margin: 0;
    padding: var(--space-sm);
    font: var(--text-caption);
  }
</style>
