<script>
  // `docs` block (notes §3.3): "Docs · 3 files", one row per file with its line
  // count; a path opens it in nvim (in the feature's integration worktree).
  import { features, openEditor } from '../../lib/app.svelte.js';

  let { block, subject } = $props();
  const s = $derived(subject ?? {});
  const files = $derived(block.files ?? []);
  const wt = $derived(
    features().find((f) => f.project_id === s.project && f.slug === s.feature)?.integration_worktree ?? null,
  );
  const open = (path) => openEditor(wt ? `${wt}/${path}` : path, null, { projectId: s.project, agentId: s.agent });
</script>

<div class="bh">Docs · {files.length} file{files.length === 1 ? '' : 's'}</div>
{#each files as f (f.path)}
  <div class="row">
    <button class="link" title={f.path} onclick={() => open(f.path)}>{f.path.split('/').at(-1)}</button>
    <span class="muted">{f.lines} line{f.lines === 1 ? '' : 's'}</span>
  </div>
{/each}

<style>
  .row {
    display: flex;
    justify-content: space-between;
    gap: var(--space-md);
  }
</style>
