<script>
  // Member chip (notes §1.3, Penpot P4.2 Member chip): role mark, status
  // icon, name and the member's short wait. Selected (h/l target): filled
  // outline. Busy while an action on it waits for the core.
  import { shortWait } from '../../lib/mission.js';
  import { STATUS } from '../../lib/status.js';
  import AgentChip from '../AgentChip.svelte';

  /** @type {{ id: string, rec: any, now: number, selected?: boolean, busy?: boolean, onclick?: (e: MouseEvent) => void }} */
  let { id, rec, now, selected = false, busy = false, onclick = () => {} } = $props();
  const st = $derived(STATUS[rec?.status] ?? STATUS.idle);
  const wait = $derived(shortWait(rec, now));
  const error = $derived(rec?.status === 'error');
</script>

<button class="mchip" class:selected class:busy aria-pressed={selected} title="{id} · {st.label}{rec?.reason ? ` · ${rec.reason}` : ''}" {onclick} tabindex="-1">
  <AgentChip {id} compact />
  <span class="icon" style:color={st.color} aria-hidden="true">{busy ? '\u{F0772}' : st.icon}</span>
  <span class="name">{id}</span>
  {#if wait}<span class="wait" class:error>{wait}</span>{/if}
</button>

<style>
  .mchip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    height: var(--size-chip);
    max-width: 100%;
    padding: 0 var(--space-sm) 0 var(--space-xs);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    background: var(--color-surface-pane);
    color: var(--color-text-primary);
    font: var(--text-label);
    white-space: nowrap;
    cursor: pointer;
  }
  .mchip.selected {
    border-color: var(--color-focus);
    outline: var(--border-width-focus) solid var(--color-focus);
    outline-offset: -1px;
    background: var(--color-surface-selected);
  }
  .mchip.busy {
    opacity: 0.7;
  }
  .icon {
    width: var(--size-icon);
    text-align: center;
  }
  .name {
    font-weight: var(--font-weight-semibold);
  }
  .wait {
    color: var(--color-text-muted);
    font: var(--text-caption);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .wait.error {
    color: var(--color-status-error);
  }
</style>
