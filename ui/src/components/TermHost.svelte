<script>
  // The box a terminal pane's xterm lives in: kitty's background and padding.
  // `watch` (D5): a dimmed ring and a cover that turns a click into take-over.
  import { app } from '../lib/app.svelte.js';

  /** @type {{ host?: HTMLElement, watch?: boolean, ontakeover?: () => void, children?: import('svelte').Snippet }} */
  let { host = $bindable(), watch = false, ontakeover = () => {}, children } = $props();
  const pad = $derived(app.kitty?.padding ?? [0, 0, 0, 0]);
</script>

<div class="term" class:watch style:background={app.kitty?.options?.theme?.background} style:padding="{pad[0]}px {pad[1]}px {pad[2]}px {pad[3]}px">
  <div class="host" bind:this={host}></div>
  {@render children?.()}
  {#if watch}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="cover" title="Click to take over" onmousedown={(e) => (e.preventDefault(), ontakeover())}></div>
  {/if}
</div>

<style>
  .term {
    position: relative;
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .watch {
    box-shadow: inset 0 0 0 var(--border-width-default) var(--color-border-strong);
  }
  .cover {
    position: absolute;
    inset: 0;
    cursor: text;
  }
  .host {
    flex: 1;
    min-width: 0;
    min-height: 0;
  }
</style>
