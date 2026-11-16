<script>
  // The Pull requests segmented control (allow-multiple-prs ux-brief §1.2, §7):
  // One PR · One PR per issue, radio semantics with arrow keys, the helper line
  // under it. "Per issue" is disabled under 2 issues (tooltip `Add at least 2
  // issues`); a locked mode (the feature already has a PR) shows its reason inline.
  import { MODE_LOCKED, modeHelp, NEEDS_TWO, PR_MODES, perIssueAllowed } from '../lib/issues.js';
  import { tooltip } from '../lib/tooltip.js';

  /** @type {{ value: string, refs?: string[], count?: number, locked?: boolean, help?: boolean, onchange?: (v: string) => void }} */
  let { value = $bindable('single'), refs = [], count = refs.length, locked = false, help = true, onchange = () => {} } = $props();

  const LOCK = '\u{F033E}';
  const off = (m) => locked || (m === 'per_issue' && !perIssueAllowed(count));
  const reason = (m) => (locked ? MODE_LOCKED : m === 'per_issue' && !perIssueAllowed(count) ? NEEDS_TWO : null);
  let els = $state([]);

  function set(m) {
    if (off(m) || m === value) return;
    value = m;
    onchange(m);
  }
  function onkey(e, i) {
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) return;
    e.preventDefault();
    const step = e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? -1 : 1;
    for (let k = 1; k <= PR_MODES.length; k++) {
      const j = (i + step * k + PR_MODES.length * k) % PR_MODES.length;
      if (!off(PR_MODES[j].value)) {
        set(PR_MODES[j].value);
        els[j]?.focus();
        return;
      }
    }
  }
</script>

<div class="mode">
  <div class="seg" role="radiogroup" aria-label="Pull requests">
    {#each PR_MODES as m, i (m.value)}
      {@const dis = off(m.value)}
      <!-- A disabled-looking radio stays focusable (aria-disabled) so its reason is reachable on focus. -->
      <button
        type="button"
        role="radio"
        class:on={value === m.value}
        class:dis
        aria-checked={value === m.value}
        aria-disabled={dis}
        tabindex={value === m.value ? 0 : -1}
        bind:this={els[i]}
        use:tooltip={reason(m.value)}
        onclick={() => set(m.value)}
        onkeydown={(e) => onkey(e, i)}>{m.label}</button
      >
    {/each}
  </div>
  {#if locked}<p class="muted cap lockline"><span class="lk" aria-hidden="true">{LOCK}</span> {MODE_LOCKED}</p>{/if}
  {#if help && !locked}<p class="muted cap" data-mode-help>{modeHelp(value, refs)}</p>{/if}
</div>

<style>
  .mode {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .seg {
    display: flex;
    flex-wrap: wrap;
  }
  .seg button {
    padding: var(--space-xxs) var(--space-sm);
    background: none;
    border: var(--border-width-default) solid var(--color-border-subtle);
    color: var(--color-text-secondary);
    font: var(--text-label);
    cursor: pointer;
  }
  .seg button + button {
    border-left: 0;
  }
  .seg button.on {
    background: var(--color-surface-selected);
    color: var(--color-text-primary);
  }
  .seg button.dis {
    color: var(--color-text-disabled);
    cursor: default;
  }
  .seg button:focus-visible {
    outline: var(--border-width-focus) solid var(--color-focus);
    outline-offset: -2px;
  }
  .cap {
    margin: 0;
    font: var(--text-caption);
  }
  .lk {
    font: var(--text-icon);
  }
</style>
