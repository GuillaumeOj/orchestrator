<script>
  // `docs` block (notes §3.3): "Docs · 3 files", one row per file with its line
  // count; a repo path opens it in nvim (in the feature's integration worktree), an
  // absolute path (a doc kept outside the repository) opens in the Feature pane.
  import { featureBySlug, openEditor, openFeature } from '../../lib/app.svelte.js';
  import { baseName, plural } from '../../lib/format.js';

  let { block, decision } = $props();
  const s = $derived(decision?.subject ?? {});
  const files = $derived(block.files ?? []);
  const feature = $derived(featureBySlug(s.project, s.feature));
  const wt = $derived(feature?.integration_worktree ?? null);
  const open = (path) => {
    if (path.startsWith('/')) {
      if (feature) openFeature(feature.id, baseName(path));
      return;
    }
    openEditor(wt ? `${wt}/${path}` : path, null, { projectId: s.project, agentId: s.agent });
  };
</script>

<div class="bh">Docs · {plural(files.length, 'file')}</div>
{#each files as f (f.path)}
  <div class="row">
    <button class="link" title={f.path} onclick={() => open(f.path)}>{baseName(f.path)}</button>
    <span class="muted">{plural(f.lines, 'line')}</span>
  </div>
{/each}

<style>
  .row {
    display: flex;
    justify-content: space-between;
    gap: var(--space-md);
  }
</style>
