<script>
  // Write-only secret field (p3-design-notes §5.3). The value leaves the UI
  // on save (the field clears before the core answers) and is never shown
  // back: no reveal, no copy, no length. Stored: "Stored in Keychain".
  // ⌘⏎ saves, esc cancels.
  import Kbd from './Kbd.svelte';

  /** @type {{ stored: boolean, placeholder: string, error?: string | null, onsave: (v: string) => void, onremove: () => void }} */
  let { stored, placeholder, error = null, onsave, onremove } = $props();
  let value = $state('');
  let replacing = $state(false);
  let input = $state(null);

  const editing = $derived(!stored || replacing);
  // Masked; the last 4 show only while typing, to check the paste.
  const masked = $derived(value.length > 4 ? `${'•'.repeat(Math.min(24, value.length - 4))}${value.slice(-4)}` : '•'.repeat(value.length));

  function save() {
    const v = value;
    value = '';
    replacing = false;
    if (v.trim()) onsave(v);
  }

  function cancel() {
    value = '';
    replacing = false;
  }

  function onKey(e) {
    if (e.key === 'Enter' && e.metaKey) return (e.preventDefault(), save());
    if (e.key === 'Escape' && (value || replacing)) {
      e.preventDefault();
      e.stopPropagation();
      cancel();
    }
  }

  /** Focus the input (⏎ Connect on the row). */
  export function focus() {
    if (stored) replacing = true;
    queueMicrotask(() => input?.focus());
  }
</script>

<div class="secret">
  {#if editing}
    <div class="line">
      <input bind:this={input} class="field mono grow" type="password" autocomplete="off" spellcheck="false" aria-label={placeholder} {placeholder} bind:value onkeydown={onKey} />
      {#if value}
        <button class="btn primary" onclick={save}>Save <Kbd keys="⌘+⏎" /></button>
        <button class="btn ghost" onclick={cancel}>Cancel <Kbd keys="esc" /></button>
      {:else if replacing}
        <button class="btn ghost" onclick={cancel}>Cancel <Kbd keys="esc" /></button>
      {:else}
        <span class="muted"><Kbd keys="⌘+⏎" /> save</span>
      {/if}
    </div>
    {#if value}<span class="muted mono typed" aria-hidden="true">{masked}</span>{/if}
  {:else}
    <div class="line stored">
      <span class="lock" aria-hidden="true">{'\u{F033E}'}</span>
      <span>Stored in Keychain</span>
      <span class="muted">·</span>
      <button class="link" onclick={focus}>Replace</button>
      <span class="muted">·</span>
      <button class="link" onclick={onremove}>Remove</button>
    </div>
  {/if}
  {#if error}<p class="err">{error}</p>{/if}
</div>

<style>
  .secret {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
  }
  .line {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .stored {
    color: var(--color-text-secondary);
  }
  .lock {
    color: var(--color-status-done);
  }
  .typed {
    font: var(--text-caption);
  }
  .link {
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
  .link:focus-visible {
    outline: var(--border-width-focus) solid var(--color-focus);
  }
  .err {
    margin: 0;
    font: var(--text-caption);
  }
</style>
