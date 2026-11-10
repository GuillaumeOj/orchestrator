<script>
  // Settings › Builds (add-build-pr ux-brief): the build you run, Back to main,
  // and the open pull requests with their CI build. Install downloads in the
  // background; nothing restarts until "Restart to run" is confirmed in the row.
  // Keys: j/k rows, ⏎ the row's action, r refresh, esc back to the sections.
  import { onMount, tick } from 'svelte';
  import { app, applyUpdate, backToMainBuild, checkForUpdates, now30, openUrl, restartToUpdate, toast } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { keyIgnored } from '../lib/keyboard.js';
  import { move } from '../lib/select.js';
  import { tooltip } from '../lib/tooltip.js';
  import { buildsHeader, listLine, restartConfirm, rowState } from '../lib/update.js';

  const REPO = 'https://github.com/GuillaumeOj/orchestrator';

  /** `selected`: "pr:14" from the "PR build is ready" toast: that row starts selected. */
  /** @type {{ selected?: string | null }} */
  let { selected = null } = $props();

  const u = $derived(app.data?.update);
  const now = $derived(now30());
  let list = $state(null); // the last pr_builds_list; kept while a refresh runs
  let error = $state(null); // a rejected pr_builds_list (updates off)
  let refreshing = $state(false);
  // The pane remounts this section when `selected` changes ({#key}), so the initial value is the one.
  const fromToast = () => Number(/^pr:(\d+)$/.exec(selected ?? '')?.[1]) || null;
  let selId = $state(fromToast());
  let confirming = $state(null); // PR number whose "Restart to run" asks first
  let root = $state(null);

  const rows = $derived((list?.prs ?? []).map((p) => ({ ...p, id: p.number })));
  const cur = $derived(rows.find((r) => r.id === selId) ?? rows[0] ?? null);
  const runningPr = $derived(u?.current?.channel === 'pr' ? u.current.pr : null);
  const header = $derived(buildsHeader(u, runningPr ? app.prTitles[runningPr] : null));
  const off = $derived(u && !u.enabled);

  async function load() {
    if (refreshing || off) return;
    refreshing = true;
    try {
      list = await invoke('pr_builds_list');
      error = null;
      app.prTitles = Object.fromEntries(list.prs.map((p) => [p.number, p.title]));
    } catch (e) {
      error = errText(e);
    } finally {
      refreshing = false;
    }
  }

  async function install(n) {
    try {
      applyUpdate(await invoke('pr_build_install', { number: n }));
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }

  async function ask(n) {
    confirming = n;
    await tick();
    root?.querySelector('[data-confirm] [data-restart]')?.focus();
  }

  function restart() {
    confirming = null;
    restartToUpdate('pr');
  }

  /** ⏎ / the row button. */
  function act(r) {
    const a = r && rowState(r, u, now).action;
    if (!a || a.disabled) return;
    if (a.run === 'install') install(r.number);
    else ask(r.number);
  }

  function onKey(e) {
    if (e.key === 'Escape' && confirming != null) {
      confirming = null;
      e.preventDefault();
      root?.focus();
      return;
    }
    if (keyIgnored(e, { controls: true })) return;
    const k = e.key;
    if (k === 'j' || k === 'ArrowDown') selId = move(rows, cur?.id, 1);
    else if (k === 'k' || k === 'ArrowUp') selId = move(rows, cur?.id, -1);
    else if (k === 'r') load();
    else if (k === 'Enter') act(cur);
    else return;
    e.preventDefault();
  }

  onMount(load);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<section class="builds" tabindex="0" data-section-focus bind:this={root} onkeydown={onKey} aria-label="Builds">
  <header class="h"><h2>Builds</h2></header>
  {#if header}
    <div class="run" data-running>
      <span class="lbl muted">Running</span>
      <div class="what">
        <div class="l1">
          <span class="mono">{header.version}</span>
          {#if header.sha}· <a class="mono" href="{REPO}/commit/{header.fullSha}" onclick={(e) => (e.preventDefault(), openUrl(`${REPO}/commit/${header.fullSha}`))}>{header.sha}</a>{/if}
          · {header.channel}
        </div>
        <div class="l2 muted">{header.line2}</div>
      </div>
      {#if header.back.show && !off}
        <div class="back">
          {#if header.back.canCheck}<button class="link" onclick={checkForUpdates}>Check</button>{/if}
          <button class="btn" disabled={!!header.back.why} use:tooltip={{ label: 'Back to main', detail: header.back.why ?? 'Restarts into main (agents are suspended and resume)' }} onclick={backToMainBuild}>Back to main</button>
        </div>
      {/if}
    </div>
  {/if}
  {#if off}
    <p class="muted">PR builds are off: {u.disabled_why ?? 'updates unavailable'}.</p>
  {:else}
    <p class="muted listline">{listLine(list, refreshing)}</p>
    {#if error}
      <p class="warn" role="alert">{error} <button class="link" onclick={load}>Retry</button></p>
    {:else if list?.error}
      <p class="warn" role="alert">{list.error} <button class="link" onclick={load}>Retry</button></p>
    {:else if list && !rows.length}
      <p class="muted">No open pull requests.</p>
    {/if}
    {#each rows as r (r.id)}
      {@const st = rowState(r, u, now)}
      {@const asking = confirming === r.number && st.kind === 'ready'}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="row" class:sel={r.id === cur?.id} data-pr={r.number} data-state={st.kind} onclick={() => (selId = r.id)}>
        <div class="top">
          <a class="num mono" href={r.url} onclick={(e) => (e.preventDefault(), openUrl(r.url))}>#{r.number}</a>
          <a class="title ell" href={r.url} onclick={(e) => (e.preventDefault(), openUrl(r.url))}>{r.draft ? 'Draft: ' : ''}{r.title}</a>
          <span class="grow"></span>
          <span class="state" class:warn={st.tone === 'warn'} class:muted={st.tone === 'muted'}>
            {#if st.dot}<span class="dot" aria-hidden="true">●</span>{/if}{st.text}
          </span>
          {#if st.action && !asking}
            <button class="btn" class:primary={st.action.primary} disabled={st.action.disabled} use:tooltip={{ label: st.action.label, key: '⏎' }} onclick={() => act(r)}>{st.action.label}</button>
          {/if}
        </div>
        <div class="sub muted">
          <span class="ell">{r.branch} · {r.author}</span>
          {#if st.note}<span class="note">{st.note}</span>{/if}
        </div>
        {#if asking}
          <div class="confirm" data-confirm>
            <span>{restartConfirm(u, r.number)}</span>
            <button class="btn primary" data-restart onclick={() => restart()}>Restart</button>
            <button class="btn ghost" use:tooltip={{ label: 'Cancel', key: 'esc' }} onclick={() => (confirming = null)}>Cancel</button>
          </div>
        {/if}
      </div>
    {/each}

  {/if}
</section>

<style>
  .builds {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    max-width: 960px;
    outline: none;
  }
  .h {
    margin-bottom: var(--space-sm);
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  .mono {
    font-family: var(--font-family-mono);
  }
  .run {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .lbl {
    width: 5em;
    flex: none;
  }
  .what {
    flex: 1;
    min-width: 0;
  }
  .l1 {
    font: var(--text-body-strong);
  }
  .l2 {
    font: var(--text-caption);
  }
  .back {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .listline {
    margin: var(--space-sm) 0 0;
    font: var(--text-caption);
  }
  .warn {
    color: var(--color-pacing-amber);
  }
  p {
    margin: 0;
  }
  .row {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .row.sel {
    background: var(--color-surface-raised);
  }
  .builds:focus-visible .row.sel,
  .row.sel:focus-within {
    border-color: var(--color-focus);
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    min-height: var(--size-control);
  }
  .num {
    width: 3.5em;
    flex: none;
    color: var(--color-text-secondary);
    text-decoration: none;
  }
  .title {
    min-width: 0;
    font: var(--text-body-strong);
    color: var(--color-text-primary);
    text-decoration: none;
  }
  .title:hover,
  .num:hover,
  .l1 a.mono:hover {
    text-decoration: underline;
  }
  .l1 a.mono {
    color: inherit;
  }
  .ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .state {
    white-space: nowrap;
    color: var(--color-text-secondary);
  }
  .state.muted {
    color: var(--color-text-muted);
  }
  .dot {
    color: var(--color-accent-default);
    margin-right: var(--space-xs);
  }
  .sub {
    display: flex;
    gap: var(--space-md);
    padding-left: calc(3.5em + var(--space-md));
    font: var(--text-caption);
  }
  .note {
    margin-left: auto;
  }
  .confirm {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    margin-top: var(--space-xs);
    padding: var(--space-sm);
    border-left: 3px solid var(--color-accent-default);
    background: var(--color-surface-chrome);
  }
  .confirm span {
    flex: 1;
  }
  .link {
    background: none;
    border: 0;
    padding: 0;
    color: var(--color-accent-default);
    cursor: pointer;
    font: inherit;
  }
</style>
