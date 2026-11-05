<script>
  // `links` block (P4.4 notes §7.1): the decisions a proposal comes from, each a
  // link to its entry in the thread ("Mon 14:02 · front-1 · allowed ↗"). An entry
  // no longer listed is plain text.
  let { block, onjump = () => {}, jumpok = () => true } = $props();
  const links = $derived(block.links ?? []);
</script>

{#each links as l (l.chat ?? l.decision)}
  {#if l.chat && jumpok(l.chat)}
    <button class="link" data-link-chat={l.chat} onclick={() => onjump(l.chat)}>{l.label} ↗</button>
  {:else}
    <span class="muted" title="No longer available">{l.label}</span>
  {/if}
{/each}
