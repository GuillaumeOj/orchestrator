<script>
  // `markdown` block (notes §3.3): rendered; "From <source>" when a source is given
  // (a Notion page URL is a link).
  import { openUrl } from '../../lib/app.svelte.js';
  import Markdown from '../Markdown.svelte';

  let { block } = $props();
  const url = $derived(/^https?:\/\//.test(block.source ?? '') ? block.source : null);
</script>

{#if block.source}
  <div class="bh">
    From {#if url}<button class="link" onclick={() => openUrl(url)}>{url.replace(/^https?:\/\//, '')} ↗</button>{:else}{block.source}{/if}
  </div>
{/if}
<Markdown text={block.text} />
