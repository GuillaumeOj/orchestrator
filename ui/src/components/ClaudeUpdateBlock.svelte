<script>
  // The Claude Code update block (add-update-claude-code ux-brief §2.1.2-3): the state line,
  // its buttons and the output of `claude update`. Shared by Settings › Claude Code and,
  // `compact`, the Setup step 1 (no key caps, a 6-line log shown only on failure).
  import { tick } from 'svelte';
  import { claudeAct, holdSecondClock, now1s, openSettings, openSingleton } from '../lib/app.svelte.js';
  import { updateBlock } from '../lib/claude.js';
  import Kbd from './Kbd.svelte';

  /** @type {{ c: any, compact?: boolean }} */
  let { c, compact = false } = $props();

  const st = $derived(updateBlock(c, now1s()));
  const kind = $derived(st.kind);
  const lines = $derived(c?.update?.log ?? []);
  // Each state's own choice about the output; reset when the state changes (a new run, a check).
  let shown = $state(null); // null = the state's default
  let lastKind = null;
  $effect(() => {
    if (kind !== lastKind) {
      lastKind = kind;
      shown = null;
    }
  });
  const logOpen = $derived(st.log === 'open' ? shown !== false : st.log === 'collapsible' ? shown === true : false);
  const showLog = $derived(compact ? st.kind === 'update-failed' && logOpen : logOpen);

  // The elapsed time ticks once a second while updating. `kind` is a primitive, so this effect
  // does not re-run (and restart the clock) at each tick, when `st` is a new object.
  $effect(() => {
    if (kind === 'updating') return holdSecondClock();
  });

  let logEl = $state(null);
  let stick = true; // follow the tail until the user scrolls up
  const onScroll = () => (stick = !logEl || logEl.scrollHeight - logEl.scrollTop - logEl.clientHeight < 8);
  $effect(() => {
    const n = lines.length;
    if (!n || !showLog || !stick) return;
    tick().then(() => logEl && (logEl.scrollTop = logEl.scrollHeight));
  });

  const GLYPH = { dot: '●', spinner: '◐', ring: '○', warn: '▲' };
  const isErr = (l) => /error|fail|denied|ENOENT|EACCES/i.test(l);
</script>

<div class="blk" class:raised={st.kind === 'available' || st.kind === 'update-failed'} class:compact data-state={st.kind} data-update-block>
  {#if st.before}<div class="before muted">{st.before}</div>{/if}
  <div class="l1">
    <span class="g {st.tone}" class:spin={st.glyph === 'spinner'} aria-hidden="true">{GLYPH[st.glyph]}</span>
    <span class="txt {st.tone}" class:ell={st.kind !== 'not-found'} aria-live="polite" title={st.tip}><b>{st.lead}</b>{#if st.rest}<span class="rest" class:num={st.kind === 'updating'}>{st.kind === 'updating' ? '  ' : ''}{st.rest}</span>{/if}</span>
    <span class="grow"></span>
    {#if st.setup && !compact}<button class="link" onclick={() => openSingleton('setup')}>Set up</button>{/if}
    {#each st.buttons as b (b.id)}
      <button class="btn" class:primary={b.primary} class:def={b.def && !compact} data-act={b.id} onclick={() => claudeAct(b.id)}>{b.label}{#if b.def && !compact} <Kbd keys="⏎" />{/if}</button>
    {/each}
  </div>
  {#if st.line2}<div class="l2 muted">{st.line2}</div>{/if}
  {#if st.line3}<div class="l2 muted" title={c?.update?.error}>{st.line3}</div>{/if}
  {#if st.log === 'collapsible' && !logOpen && !compact}
    <button class="link l2" onclick={() => (shown = true)}>Show output</button>
  {/if}
  {#if showLog}
    <div class="logh muted">
      <span>{st.logLabel ?? 'Output of claude update'}</span>
      <span class="grow"></span>
      {#if st.kind !== 'updating' && !compact}<button class="link" onclick={() => (shown = false)}>Hide output</button>{/if}
    </div>
    <pre class="log" class:short={compact} bind:this={logEl} onscroll={onScroll} tabindex="-1">{#each lines as l, i (i)}<span class:amber={isErr(l)}>{l}</span>
{:else}<span class="muted">Waiting for output…</span>{/each}</pre>
  {/if}
  {#if compact && st.kind === 'update-failed'}
    <button class="link l2" onclick={() => openSettings('claude')}>Open Settings › Claude Code</button>
  {/if}
</div>

<style>
  .blk {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .blk.raised {
    background: var(--color-surface-raised);
  }
  .blk.compact {
    padding: var(--space-xs) var(--space-sm);
  }
  .l1 {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-height: var(--size-control);
  }
  .l1 .txt {
    min-width: 0;
    font: var(--text-label);
  }
  .txt.ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .txt b {
    font-weight: var(--font-weight-semibold);
  }
  .rest {
    color: var(--color-text-secondary);
  }
  .num {
    font-variant-numeric: tabular-nums;
  }
  .g {
    flex: none;
    font-size: 11px;
  }
  .g.spin {
    display: inline-block;
    animation: claude-spin 1s linear infinite;
  }
  @keyframes claude-spin {
    to {
      transform: rotate(360deg);
    }
  }
  .muted {
    color: var(--color-text-muted);
  }
  .green {
    color: var(--color-pacing-green);
  }
  .accent {
    color: var(--color-accent-default);
  }
  .running {
    color: var(--color-attention-running);
  }
  .amber {
    color: var(--color-pacing-amber);
  }
  .red {
    color: var(--color-pacing-red);
  }
  .before,
  .l2 {
    font: var(--text-caption);
  }
  .l2 {
    padding-left: calc(11px + var(--space-sm));
    text-align: left;
  }
  .btn.def {
    outline: var(--border-width-focus) solid var(--color-focus);
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    color: var(--color-accent-default);
    cursor: pointer;
    font: inherit;
    text-decoration: underline;
  }
  .logh {
    display: flex;
    gap: var(--space-sm);
    font: var(--text-caption);
  }
  .log {
    margin: 0;
    padding: var(--space-sm);
    height: 160px; /* ≈ 10 lines of 16 px */
    overflow: auto;
    background: var(--color-surface-chrome);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    font: var(--text-mono);
    font-size: 12px;
    line-height: 16px;
    user-select: text;
    white-space: pre-wrap;
    word-break: break-all;
  }
  .log.short {
    height: 96px; /* 6 lines */
  }
  .log span {
    display: inline;
  }
</style>
