<script>
  // The Issues chip input (allow-multiple-prs ux-brief §1.1, §7), shared by the
  // launch form and the feature's Edit popover. A chip per ref; suggestions from
  // the project's intake (`issue_suggest`); Space or comma commits only a text
  // that parses as a ref; Enter picks the highlighted suggestion, else commits
  // what was typed (the server validates it). Backspace on an empty input
  // removes the last chip that is not locked.
  import { invoke } from '../lib/ipc.js';
  import { addChip, chipError, chipOf, lockedLabel, lockedText, MAX_ISSUES, parseRef } from '../lib/issues.js';
  import { tooltip } from '../lib/tooltip.js';

  /** @type {{ chips: any[], project?: string, locked?: Map<string, number | null>, disabled?: boolean, error?: string | null, id?: string }} */
  let { chips = $bindable([]), project = '', locked = new Map(), disabled = false, error = null, id = 'issues-field' } = $props();

  let text = $state('');
  let input = $state(null);
  let found = $state([]);
  let active = $state(-1);
  let open = $state(false);
  let seq = 0;
  let blurred = false; // a suggestion that lands after the input lost focus stays closed
  const full = $derived(chips.length >= MAX_ISSUES);
  const shown = $derived(found.filter((s) => !chips.some((c) => c.ref.toLowerCase() === s.ref.toLowerCase())));
  const message = $derived(error ?? chipError(chips));
  const LOCK = '\u{F033E}';

  async function suggest(t) {
    const mine = ++seq;
    if (!t.trim() || !project) {
      found = [];
      open = false;
      return;
    }
    try {
      const r = await invoke('issue_suggest', { project, text: t.trim() });
      if (mine !== seq) return;
      found = r ?? [];
    } catch {
      if (mine === seq) found = [];
    }
    active = -1;
    open = found.length > 0 && !blurred;
  }
  $effect(() => {
    suggest(text);
  });

  function commit(raw, known = null) {
    const t = raw.trim();
    if (!t) return;
    chips = addChip(chips, chipOf(t, known));
    text = '';
    found = [];
    open = false;
    active = -1;
  }
  const pick = (s) => commit(s.ref, s);

  function remove(c) {
    chips = chips.filter((x) => x !== c);
    input?.focus();
  }
  const isLocked = (c) => locked.has(c.ref);

  function onkey(e) {
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      if (!open || !shown.length) return;
      e.preventDefault();
      active = (active + (e.key === 'ArrowDown' ? 1 : -1) + shown.length) % shown.length;
    } else if (e.key === 'Enter') {
      if (open && active >= 0 && shown[active]) {
        e.preventDefault();
        pick(shown[active]);
      } else if (text.trim()) {
        e.preventDefault();
        commit(text);
      }
    } else if (e.key === ' ' || e.key === ',') {
      // Space and comma commit only a text that parses as a ref (otherwise it stays, to search titles).
      if (parseRef(text)) {
        e.preventDefault();
        commit(text);
      }
    } else if (e.key === 'Backspace' && !text) {
      const last = [...chips].reverse().find((c) => !isLocked(c));
      if (last) {
        e.preventDefault();
        chips = chips.filter((x) => x !== last);
      }
    } else if (e.key === 'Escape' && open) {
      e.preventDefault();
      e.stopPropagation();
      open = false;
    }
  }
</script>

<div class="issues">
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="box" class:off={disabled || full} class:bad={!!message} onclick={() => input?.focus()}>
    {#each chips as c (c.text)}
      {@const lock = isLocked(c)}
      <span class="chip" class:bad={!!c.bad} class:lock use:tooltip={lock ? lockedText(c.ref, locked.get(c.ref)) : null} aria-label={lock ? lockedLabel(c.ref, locked.get(c.ref)) : undefined}>
        <span class="ref mono">{c.ref}</span>
        {#if c.title}<span class="ttl muted">{c.title}</span>{/if}
        {#if lock}<span class="lk" aria-hidden="true">{LOCK}</span>{/if}
        {#if !lock && !disabled}<button type="button" class="x" aria-label="Remove {c.ref}" onclick={(e) => (e.stopPropagation(), remove(c))}>×</button>{/if}
      </span>
    {/each}
    <input
      {id}
      class="in"
      bind:this={input}
      bind:value={text}
      disabled={disabled || full}
      placeholder={full ? `${MAX_ISSUES} issues at most` : chips.length ? '' : '12, #12, ENG-7 or a GitHub issue URL'}
      spellcheck="false"
      autocomplete="off"
      role="combobox"
      aria-expanded={open && shown.length > 0}
      aria-controls="{id}-list"
      aria-autocomplete="list"
      aria-activedescendant={active >= 0 ? `${id}-opt-${active}` : undefined}
      onkeydown={onkey}
      oninput={() => (blurred = false)}
      onfocus={() => (blurred = false)}
      onblur={() => ((blurred = true), (open = false))}
    />
  </div>
  {#if open && shown.length}
    <ul class="list" id="{id}-list" role="listbox" aria-label="Issue suggestions">
      {#each shown as s, i (s.ref)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li id="{id}-opt-{i}" role="option" aria-selected={i === active} class:on={i === active} onmousedown={(e) => (e.preventDefault(), pick(s))}>
          <span class="mono">{s.ref}</span>
          {#if s.title}<span class="muted ell">{s.title}</span>{/if}
        </li>
      {/each}
    </ul>
  {/if}
  {#if message}<p class="msg" role="alert">{message}</p>{:else if full}<p class="muted cap">{MAX_ISSUES} issues at most</p>{/if}
</div>

<style>
  .issues {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .box {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-xs);
    min-height: var(--size-control);
    padding: var(--space-xxs) var(--space-xs);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    cursor: text;
  }
  .box:focus-within {
    outline: var(--border-width-focus) solid var(--color-focus);
    outline-offset: 0;
  }
  .box.off {
    cursor: default;
  }
  .box.bad {
    border-color: var(--color-status-error);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    max-width: 100%;
    min-width: 0;
    padding: 0 var(--space-xs);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    font: var(--text-caption);
  }
  .chip.bad {
    border-color: var(--color-status-error);
    background: color-mix(in srgb, var(--color-status-error) 15%, transparent);
  }
  .ref {
    font-weight: var(--font-weight-semibold);
  }
  .chip.bad {
    color: var(--color-status-error);
  }
  .ttl {
    min-width: 0;
    max-width: 18ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lk {
    color: var(--color-text-muted);
    font: var(--text-icon);
  }
  .x {
    padding: 0 2px;
    background: none;
    border: 0;
    color: var(--color-text-muted);
    font: inherit;
    cursor: pointer;
  }
  .x:hover,
  .x:focus-visible {
    color: var(--color-text-primary);
  }
  .in {
    flex: 1;
    min-width: 8ch;
    padding: 0;
    background: none;
    border: 0;
    outline: none;
    color: var(--color-text-primary);
    font: var(--text-label);
  }
  .in:disabled {
    color: var(--color-text-disabled);
  }
  .list {
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    z-index: 30;
    margin: var(--space-xxs) 0 0;
    padding: 0;
    list-style: none;
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
    max-height: 240px;
    overflow-y: auto;
  }
  li {
    display: flex;
    gap: var(--space-sm);
    align-items: baseline;
    padding: var(--space-xs) var(--space-sm);
    font: var(--text-label);
    cursor: pointer;
  }
  li.on,
  li:hover {
    background: var(--color-surface-selected);
  }
  .ell {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .msg {
    margin: 0;
    color: var(--color-status-error);
    font: var(--text-caption);
  }
  .cap {
    margin: 0;
    font: var(--text-caption);
  }
</style>
