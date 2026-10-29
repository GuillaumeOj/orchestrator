<script>
  // `diff_summary` block (notes §3.3, the PR gate): "7 files changed · +142 −9
  // lines", per file a 10-segment bar (scaled to the largest file) and its
  // counts ("−0" left out), "Show n more files" past 12, and the patch on
  // demand (`decision_patch`): "Loading patch…", or why it failed. No spinner.
  import { errText, invoke } from '../../lib/ipc.js';

  let { block, id } = $props();
  const FILES = 12;
  const SEGMENTS = 10;
  let all = $state(false);
  let patch = $state(null); // null | { loading } | { text } | { error }

  const files = $derived(block.files ?? []);
  const shown = $derived(all ? files : files.slice(0, FILES));
  const max = $derived(Math.max(1, ...files.map((f) => f.added + f.removed)));
  const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;

  /** The file's bar: added then removed segments, scaled; the rest empty. */
  function bar(f) {
    const n = Math.max(1, Math.round(((f.added + f.removed) / max) * SEGMENTS));
    const added = f.added + f.removed ? Math.round((f.added / (f.added + f.removed)) * n) : 0;
    return Array.from({ length: SEGMENTS }, (_, i) => (i < added ? 'add' : i < n ? 'del' : 'none'));
  }

  async function showPatch() {
    patch = { loading: true };
    try {
      patch = { text: await invoke('decision_patch', { id, path: null }) };
    } catch (e) {
      patch = { error: errText(e) };
    }
  }
  const lineClass = (l) => (l.startsWith('+') && !l.startsWith('+++') ? 'add' : l.startsWith('-') && !l.startsWith('---') ? 'del' : '');
</script>

<div class="bh">{plural(files.length, 'file')} changed · +{block.added} −{block.removed} lines</div>
{#each shown as f (f.path)}
  <div class="row">
    <span class="mono path" title={f.path}>{f.path}</span>
    <span class="bar" aria-hidden="true">{#each bar(f) as s, i (i)}<i class={s}></i>{/each}</span>
    <span class="n"><span class="added">+{f.added}</span>{#if f.removed}&nbsp;<span class="removed">−{f.removed}</span>{/if}</span>
  </div>
{/each}
{#if !all && files.length > FILES}<button class="link" onclick={() => (all = true)}>Show {plural(files.length - FILES, 'more file')}</button>{/if}
{#if patch?.text != null}
  <pre class="mono patch">{#each patch.text.split('\n') as l, i (i)}<span class={lineClass(l)}>{l}</span>{'\n'}{/each}</pre>
  <button class="link" onclick={() => (patch = null)}>Hide patch</button>
{:else if patch?.loading}
  <span class="muted">Loading patch…</span>
{:else if patch?.error}
  <span class="err">Could not load the patch: {patch.error}</span>
{:else}
  <button class="link" onclick={showPatch}>Show patch</button>
{/if}

<style>
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: var(--text-mono-sm);
  }
  .bar {
    display: inline-flex;
    gap: 1px;
  }
  .bar i {
    width: 6px;
    height: 8px;
    border-radius: 1px;
    background: var(--color-border-subtle);
  }
  .bar i.add {
    background: var(--color-diff-added);
  }
  .bar i.del {
    background: var(--color-diff-removed);
  }
  .n {
    min-width: 7ch;
    text-align: right;
    white-space: nowrap;
  }
  .added,
  .patch .add {
    color: var(--color-diff-added);
  }
  .removed,
  .patch .del {
    color: var(--color-diff-removed);
  }
  .patch {
    margin: 0;
    max-height: 50vh;
    overflow: auto;
    white-space: pre;
    font: var(--text-mono-sm);
    user-select: text;
    -webkit-user-select: text;
  }
</style>
