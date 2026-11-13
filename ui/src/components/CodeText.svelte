<script>
  // Text with `backticked` code rendered as mono chips (the core's warnings and the compat notes carry them).
  import { codeSpans } from '../lib/claude.js';

  let { text } = $props();
</script>

{#each codeSpans(text) as p}{#if p.code}<span class="code">{p.text}</span>{:else}{p.text}{/if}{/each}

<style>
  .code {
    font-family: var(--font-family-mono);
    padding: 0 var(--space-xs);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
  }
</style>
