<script>
  // The PR line (ux-brief §1, Penpot 04 Feature · PR line · states): one line in
  // the "PR and artifacts" card. `PR #n`, the CI dot and count with a popover of
  // every job, the position badge, unpushed / rebased, checked-ago and refresh.
  // Under width pressure the items give way in the brief's order (§1.6).
  import { tick, untrack } from 'svelte';
  import { now30, openUrl, toast } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import {
    checkedText,
    ciColor,
    ciCount,
    ciFooter,
    ciHeader,
    ciLabel,
    jobColor,
    jobLabel,
    jobName,
    jobWord,
    jobWordColor,
    lineState,
    positionBadge,
    REBASED_TIP,
    refreshError,
    toPush,
  } from '../lib/pr.js';

  let { run, feature, onrun = () => {} } = $props();
  const now = $derived(now30());

  const pr = $derived(run.pr);
  const st = $derived(lineState(run));
  const open = $derived(st === 'open');
  const stale = $derived(open && !!pr?.error);
  const number = $derived(run.pr_number);
  // The PR link: the number, else (a URL the core could not parse) the URL itself.
  const linkText = $derived(number ? `PR #${number}` : run.pr_url);
  const pos = $derived(open ? positionBadge(pr.position, pr.base) : null);
  const push = $derived(open ? toPush(pr.unpushed) : null);
  const checked = $derived(pr ? checkedText(pr.checked_at_ms, { now, stale }) : null);
  const watched = $derived(st === 'checking' || open);

  let busy = $state(false);
  async function refresh() {
    if (busy) return;
    busy = true;
    try {
      const r = await invoke('pr_refresh', { feature });
      if (r) onrun(r);
    } catch (e) {
      toast(refreshError(run, errText(e)), { kind: 'error' });
    } finally {
      busy = false;
    }
  }

  // --- CI popover: opens on hover (300 ms), pinned by click / Enter / Space.
  let shown = $state(false);
  let pinned = $state(false);
  let timer;
  let ciBtn = $state(null);
  let pop = $state(null);
  let wrap = $state(null);
  const clear = () => clearTimeout(timer);
  const hoverIn = () => {
    clear();
    if (!shown) timer = setTimeout(() => (shown = true), 300);
  };
  const hoverOut = () => {
    clear();
    if (!pinned) timer = setTimeout(() => (shown = false), 150);
  };
  async function toggle(e) {
    clear();
    pinned = !pinned;
    shown = pinned;
    // Opened from the keyboard (detail 0): focus goes to the first row.
    if (shown && e.detail === 0) {
      await tick();
      (pop?.querySelector('a') ?? pop)?.focus();
    }
  }
  function close(focus = true) {
    clear();
    shown = pinned = false;
    if (focus) ciBtn?.focus();
  }
  // Keys act from the button as well as from inside the popover (a click leaves focus on the button).
  function popKey(e) {
    if (!shown || !pop) return;
    const rows = [...pop.querySelectorAll('a')];
    const i = rows.indexOf(document.activeElement);
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      rows[Math.max(0, Math.min(rows.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1)))]?.focus();
    }
  }
  // A pinned popover closes on a press outside it.
  function outside(e) {
    if (pinned && wrap && !wrap.contains(e.target)) close(false);
  }
  function popFocusOut(e) {
    if (pinned && !e.currentTarget.contains(e.relatedTarget) && e.relatedTarget !== ciBtn) close(false);
  }

  // --- Truncation (§1.6): 0 full · 1 `2m ago` · 2 no checked · 3 `↓3 behind` · 4 `↑2` · 5 `↻`.
  let line = $state(null);
  let level = $state(0);
  const sig = $derived(
    [st, pr?.ci?.done, pr?.ci?.total, pr?.position?.status, pr?.position?.behind, pr?.unpushed, pr?.rewritten, pr?.error, checked?.text, number, busy].join('|'),
  );
  let fitting = 0;
  async function fit() {
    const id = ++fitting;
    level = 0;
    for (let l = 0; l <= 5; l++) {
      level = l;
      await tick();
      if (id !== fitting || !line || line.scrollWidth <= line.clientWidth) return;
    }
  }
  $effect(() => {
    sig;
    if (!line) return;
    untrack(fit);
    const ro = new ResizeObserver(() => untrack(fit));
    ro.observe(line);
    return () => ro.disconnect();
  });

  const SPIN = '\u{F0450}';
  const ERRG = '\u{F0026}';
</script>

<svelte:window onpointerdown={outside} />

{#snippet jobCells(j)}
  <span class="dot" style:background={jobColor(j.state)} aria-hidden="true"></span>
  <span class="jn ell">{#if j.workflow}<span class="muted">{j.workflow} / </span>{/if}{j.name}</span>
  <span class="cap jw" style:color={jobWordColor(j.state)}>{jobWord(j.state)}</span>
{/snippet}

{#if st === 'none'}
  <p class="muted nline">No PR yet: the lead asks with request_pr after integration (PR gate).</p>
{:else}
  <div class="prline" class:stale bind:this={line}>
    <button class="link pr" title={run.pr_url} aria-label="Pull request {number ? `#${number}` : ''}, opens in the browser" onclick={() => openUrl(run.pr_url)}>{linkText}</button>

    {#if st === 'checking'}
      <span class="muted cap">checking…</span>
    {:else if open}
      <span class="ci" role="presentation" bind:this={wrap} onmouseenter={hoverIn} onmouseleave={hoverOut} onkeydown={popKey}>
        <button
          class="cibtn"
          bind:this={ciBtn}
          aria-label={ciLabel(pr.ci)}
          aria-expanded={shown}
          aria-controls="ci-{run.id}"
          onclick={toggle}
        >
          <span class="dot" style:background={ciColor(pr.ci?.overall)} aria-hidden="true"></span>
          <span class="mono cap count" class:dim={stale}>{ciCount(pr.ci)}</span>
        </button>
        {#if shown}
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <div class="pop" id="ci-{run.id}" role="dialog" aria-label="CI jobs for PR #{number}" tabindex="-1" bind:this={pop} onfocusout={popFocusOut}>
            <div class="ph cap">{ciHeader(pr.ci)}</div>
            {#if pr.ci?.jobs?.length}
              <div class="rows">
                {#each pr.ci.jobs as j, i (i)}
                  {#if j.url}
                    <a class="row" href={j.url} aria-label={jobLabel(j)} title={jobName(j)} onclick={(e) => (e.preventDefault(), openUrl(j.url))}>
                      {@render jobCells(j)}
                    </a>
                  {:else}
                    <div class="row plain" aria-label={jobLabel(j)} title={jobName(j)}>
                      {@render jobCells(j)}
                    </div>
                  {/if}
                {/each}
              </div>
            {/if}
            <a class="pf" href="{run.pr_url}/checks" onclick={(e) => (e.preventDefault(), openUrl(`${run.pr_url}/checks`))}>{ciFooter(pr.ci)}</a>
          </div>
        {/if}
      </span>

      <span class="badge cap" style:color={stale ? null : pos.color} title={pos.tip}>{level >= 3 ? pos.short : pos.text}</span>
      {#if push}
        <span class="cap muted" title={push.tip}>{level >= 4 ? push.short : push.text}</span>
      {/if}
      {#if pr.rewritten}
        <span class="cap" style:color={stale ? 'var(--color-text-muted)' : 'var(--color-status-waiting-for-you)'} title={REBASED_TIP}>{level >= 5 ? '↻' : 'rebased'}</span>
      {/if}
    {:else if st === 'merged'}
      <span class="cap" style:color="var(--color-status-done)">merged</span>
    {:else if st === 'closed'}
      <span class="cap muted">closed</span>
    {/if}

    <span class="grow"></span>
    {#if stale}
      <span class="err" aria-hidden="true">{ERRG}</span>
    {/if}
    {#if checked && st !== 'checking'}
      {#if level < 2}
        <span class="cap muted" title={stale ? `Last check failed: ${pr.error}. Showing the last known status.` : checked.clock}>
          {busy ? 'checking…' : level >= 1 ? checked.short : checked.text}
        </span>
      {/if}
    {/if}
    {#if watched}
      <button
        class="refresh"
        class:spin={busy || st === 'checking'}
        aria-label="Refresh PR status"
        aria-busy={busy}
        disabled={busy || st === 'checking'}
        title={level >= 1 && checked ? `Refresh PR status · ${checked.clock}` : 'Refresh PR status'}
        onclick={refresh}><span aria-hidden="true">{SPIN}</span></button
      >
    {/if}
  </div>
{/if}

<style>
  .nline {
    margin: 0;
    font: var(--text-caption);
  }
  .prline {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    white-space: nowrap;
    overflow: hidden;
    position: relative;
  }
  .prline:has(.pop) {
    overflow: visible;
  }
  .cap {
    font: var(--text-caption);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .link {
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: var(--text-label);
    cursor: pointer;
  }
  .link:hover {
    text-decoration: underline;
  }
  .ci {
    position: relative;
    display: inline-flex;
  }
  .cibtn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    padding: 0;
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
  }
  .dot {
    width: var(--size-status-dot, 8px);
    height: var(--size-status-dot, 8px);
    border-radius: 50%;
    flex: none;
  }
  .count.dim,
  .stale .badge {
    color: var(--color-text-muted);
  }
  .err {
    color: var(--color-status-error);
    font: var(--text-icon);
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
      animation: pulse 1.2s ease-in-out infinite;
    }
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes pulse {
    50% {
      opacity: 0.4;
    }
  }
  .pop {
    position: absolute;
    top: calc(100% + var(--space-xs));
    left: 0;
    z-index: 20;
    min-width: 320px;
    max-width: 400px;
    width: max-content;
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
    display: flex;
    flex-direction: column;
    outline: none;
  }
  .ph,
  .pf {
    padding: var(--space-sm) var(--space-md);
  }
  .ph {
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-caption);
  }
  .pf {
    border-top: var(--border-width-default) solid var(--color-border-subtle);
    color: var(--color-accent-default);
    font: var(--text-caption);
    text-decoration: none;
  }
  .rows {
    max-height: 288px;
    overflow-y: auto;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    height: 24px;
    padding: 0 var(--space-md);
    color: var(--color-text-primary);
    font: var(--text-label);
    text-decoration: none;
  }
  a.row:hover,
  a.row:focus-visible {
    background: var(--color-surface-selected);
  }
  .jn {
    flex: 1;
    min-width: 0;
  }
  .ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .jw {
    flex: none;
  }
</style>
