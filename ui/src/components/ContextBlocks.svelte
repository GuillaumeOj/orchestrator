<script>
  // A decision's context blocks in their order (api-contract §2.2, Penpot P4.1
  // Context blocks): one component per block type in one shell (surface.pane,
  // border.subtle, radius.md, header in text.label muted). P4.3's cards reuse it.
  import CostBlock from './blocks/CostBlock.svelte';
  import CountdownBlock from './blocks/CountdownBlock.svelte';
  import DiffSummaryBlock from './blocks/DiffSummaryBlock.svelte';
  import DocsBlock from './blocks/DocsBlock.svelte';
  import ErrorBlock from './blocks/ErrorBlock.svelte';
  import LastMessageBlock from './blocks/LastMessageBlock.svelte';
  import MarkdownBlock from './blocks/MarkdownBlock.svelte';
  import OptionsBlock from './blocks/OptionsBlock.svelte';
  import PtyTailBlock from './blocks/PtyTailBlock.svelte';
  import PermissionCall from './PermissionCall.svelte';

  let { decision, choices = $bindable([]) } = $props();
  const BLOCKS = {
    tool_call: PermissionCall,
    markdown: MarkdownBlock,
    docs: DocsBlock,
    diff_summary: DiffSummaryBlock,
    last_message: LastMessageBlock,
    pty_tail: PtyTailBlock,
    cost: CostBlock,
    countdown: CountdownBlock,
    error: ErrorBlock,
  };
</script>

<div class="blocks">
  {#each decision.context ?? [] as block, i (`${decision.id}:${i}`)}
    <section class="blk" data-block={block.type}>
      {#if block.type === 'options'}
        <OptionsBlock {block} name={decision.id} bind:choices />
      {:else if BLOCKS[block.type]}
        {@const Block = BLOCKS[block.type]}
        <Block {block} {decision} />
      {/if}
    </section>
  {/each}
</div>

<style>
  .blocks {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    min-width: 0;
  }
  .blk {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-md);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    min-width: 0;
    font: var(--text-body);
  }
  .blk :global(.bh) {
    font: var(--text-label);
    color: var(--color-text-muted);
  }
</style>
