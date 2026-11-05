<script>
  // `rule` block (P4.4 notes §7.1, api-contract §3.1): the proposed rule in mono,
  // exact FR-708 grammar, with its scope and the file it will be written to.
  import { projectName } from '../../lib/app.svelte.js';
  import { midCut } from '../../lib/policy.js';

  let { block } = $props();
  const project = $derived(projectName(block.project) ?? 'project');
  const scope = $derived(
    [block.scope === 'global' ? 'every project' : project, block.role ? `role ${block.role}` : '']
      .filter(Boolean)
      .join(' · '),
  );
</script>

<div class="bh">Rule</div>
<dl class="rule">
  <dt>{block.effect}</dt><dd class="mono" data-rule-match>{block.match}</dd>
  <dt>scope</dt><dd>{scope}</dd>
  <dt>file</dt><dd class="mono" title={block.file}>{midCut(block.file, 40)}</dd>
</dl>

<style>
  .rule {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: var(--space-xs) var(--space-md);
    margin: 0;
  }
  dt {
    color: var(--color-text-muted);
  }
  dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
</style>
