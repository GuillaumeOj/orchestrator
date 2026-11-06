<script>
  // Settings › Claude Code (add-update-claude-code ux-brief §2.1): the installed version, the
  // update block, compatibility and the account. Everything is derived from `setup.claude`.
  // Keys: u update / retry, r check now, ⏎ the block's default button, esc back to the sections.
  import { app, claudeAct, toast } from '../lib/app.svelte.js';
  import { account, compat, hint, installedHeader, liveKeys, updateBlock } from '../lib/claude.js';
  import { shortPath } from '../lib/format.js';
  import ClaudeUpdateBlock from './ClaudeUpdateBlock.svelte';
  import Kbd from './Kbd.svelte';

  const c = $derived(app.data?.setup?.claude ?? null);
  const home = $derived(app.data?.core?.user_home ?? '');
  const head = $derived(installedHeader(c));
  const keys = $derived(liveKeys(c));
  const note = $derived(hint(c));
  const cp = $derived(compat(c));
  const acct = $derived(account(c));

  async function copyPath() {
    try {
      await navigator.clipboard.writeText(head.path);
      toast('Copied the path');
    } catch {
      toast('Could not copy the path', { kind: 'error' });
    }
  }

  function onKey(e) {
    if (e.target.closest('input, textarea, select, button, a') || e.metaKey || e.ctrlKey || e.altKey) return;
    const def = updateBlock(c).buttons.find((b) => b.def);
    if (e.key === 'u' && keys.u) claudeAct('update');
    else if (e.key === 'r' && keys.r) claudeAct('check');
    else if (e.key === 'Enter' && def) claudeAct(def.id);
    else return;
    e.preventDefault();
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<section class="claude" tabindex="0" data-section-focus onkeydown={onKey} aria-label="Claude Code">
  <header class="h"><h2>Claude Code</h2></header>
  <div class="run" data-installed>
    <span class="lbl muted">Installed</span>
    <div class="what">
      <div class="l1"><b class="mono">{head.version}</b>{#if head.meta} · {head.meta}{/if}</div>
      {#if head.path}
        <button class="l2 muted mono path" title="Copy path" onclick={copyPath}>{shortPath(head.path, home)}</button>
      {:else if head.missing}
        <div class="l2 muted">{head.missing}</div>
      {/if}
    </div>
  </div>
  <ClaudeUpdateBlock {c} />
  {#if note}<p class="muted note">{note}</p>{/if}
  {#if cp.supported}
    <h3>Compatibility</h3>
    <div class="kv"><span class="muted">Supported</span><span>{cp.supported}</span></div>
    {#each cp.notes as n (n.text)}
      <div class="kv"><span></span><span class={n.tone}><span aria-hidden="true">{n.tone === 'red' ? '●' : '▲'}</span> {n.text}</span></div>
    {/each}
  {/if}
  {#if acct}
    <h3>Account</h3>
    <div class="kv"><span class="muted">Login</span><span>{acct.login}</span></div>
    {#if acct.note}<div class="kv"><span></span><span class="muted">{acct.note}</span></div>{/if}
  {/if}
  <p class="muted foot">Agents run with auto-update off. The app updates Claude Code only when you ask, and never writes to ~/.claude.</p>
  <p class="muted hints">
    {#if keys.u}<Kbd keys="u" /> update · {/if}{#if keys.r}<Kbd keys="r" /> check now · {/if}<Kbd keys="esc" /> back to sections
  </p>
</section>

<style>
  .claude {
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
  h3 {
    margin: var(--space-md) 0 0;
    font: var(--text-body-strong);
  }
  p {
    margin: 0;
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
  .path {
    display: block;
    padding: 0;
    background: none;
    border: 0;
    text-align: left;
    cursor: copy;
  }
  .note,
  .foot,
  .hints {
    font: var(--text-caption);
  }
  .foot {
    margin-top: var(--space-sm);
  }
  .kv {
    display: grid;
    grid-template-columns: 7em 1fr;
    gap: var(--space-md);
    padding: 0 var(--space-md);
  }
  .amber {
    color: var(--color-pacing-amber);
  }
  .red {
    color: var(--color-pacing-red);
  }
</style>
