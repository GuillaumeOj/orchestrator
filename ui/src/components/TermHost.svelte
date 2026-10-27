<script>
  // The box a terminal pane's xterm lives in: kitty's background and padding.
  import { app } from '../lib/app.svelte.js';

  /** @type {{ host?: HTMLElement, children?: import('svelte').Snippet }} */
  let { host = $bindable(), children } = $props();
  const pad = $derived(app.kitty?.padding ?? [0, 0, 0, 0]);
</script>

<div class="term" style:background={app.kitty?.options?.theme?.background} style:padding="{pad[0]}px {pad[1]}px {pad[2]}px {pad[3]}px">
  <div class="host" bind:this={host}></div>
  {@render children?.()}
</div>

<style>
  .term {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .host {
    flex: 1;
    min-width: 0;
    min-height: 0;
  }
</style>
