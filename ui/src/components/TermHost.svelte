<script>
  // The box a terminal pane's xterm lives in: kitty's background and padding.
  // `watch` (D5): a dimmed ring; a press takes over (before xterm.js gets
  // focus), the wheel still scrolls the scrollback.
  import { app } from '../lib/app.svelte.js';

  /** @type {{ host?: HTMLElement, watch?: boolean, ontakeover?: () => void, children?: import('svelte').Snippet }} */
  let { host = $bindable(), watch = false, ontakeover = () => {}, children } = $props();
  const pad = $derived(app.kitty?.padding ?? [0, 0, 0, 0]);

  function onPress(e) {
    if (!watch || e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    ontakeover();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="term" class:watch title={watch ? 'Click to take over' : null} style:background={app.kitty?.options?.theme?.background} style:padding="{pad[0]}px {pad[1]}px {pad[2]}px {pad[3]}px" onmousedowncapture={onPress}>
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
  .watch {
    box-shadow: inset 0 0 0 var(--border-width-default) var(--color-border-strong);
    cursor: text;
  }
  .host {
    flex: 1;
    min-width: 0;
    min-height: 0;
  }
</style>
