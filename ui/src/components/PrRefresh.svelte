<script>
  // The card head's refresh for a per-issue feature (allow-multiple-prs ux-brief
  // §2.2): one button checks every open PR (`pr_refresh`), with `checked 1m ago`
  // from the oldest check of the open ones.
  import { now30, toast } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { checkedText } from '../lib/pr.js';
  import { tooltip } from '../lib/tooltip.js';

  /** @type {{ run: any, feature: string, onrun?: (r: any) => void }} */
  let { run, feature, onrun = () => {} } = $props();
  const now = $derived(now30());
  const open = $derived((run?.prs_view ?? []).filter((p) => p.state === 'open'));
  const checks = $derived(open.map((p) => p.pr?.checked_at_ms).filter((x) => x != null));
  const checked = $derived(checks.length ? checkedText(Math.min(...checks), { now }) : null);
  let busy = $state(false);
  const SPIN = '\u{F0450}';

  async function refresh() {
    if (busy) return;
    busy = true;
    try {
      const r = await invoke('pr_refresh', { feature });
      if (r) onrun(r);
    } catch (e) {
      toast(`Couldn't check the PRs: ${errText(e)}`, { kind: 'error' });
    } finally {
      busy = false;
    }
  }
</script>

{#if open.length}
  <span class="muted cap" use:tooltip={checked?.clock ?? null}>{busy ? 'checking…' : (checked?.text ?? '')}</span>
  <button type="button" class="refresh" class:spin={busy} aria-label="Refresh PR status" aria-busy={busy} disabled={busy} use:tooltip={'Refresh PR status'} onclick={refresh}><span aria-hidden="true">{SPIN}</span></button>
{/if}

<style>
  .cap {
    font: var(--text-caption);
  }
  .refresh {
    flex: none;
    width: 16px;
    height: 16px;
    padding: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: 0;
    color: var(--color-text-muted);
    font: var(--text-icon);
    cursor: pointer;
  }
  .refresh:hover:not(:disabled) {
    color: var(--color-text-primary);
  }
  .refresh.spin span {
    display: inline-block;
    animation: spin 1s linear infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .refresh.spin span {
      animation: none;
    }
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
