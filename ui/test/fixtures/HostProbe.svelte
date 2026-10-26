<script>
  import { onMount } from 'svelte';
  import TermHost from '../../src/components/TermHost.svelte';
  let { seen } = $props();
  let host = $state();
  onMount(() => seen(host));
</script>

<TermHost bind:host />
