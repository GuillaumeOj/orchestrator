<script>
  // Shell and editor panes (FR-1001, FR-1102): the login shell or nvim on a PTY
  // the core owns, rendered like kitty. Closing the shell closes the pane, as in kitty.
  import { onMount } from 'svelte';
  import { app, focusing, termFor, closePane, focusPane, openEditor } from '../lib/app.svelte.js';

  let { paneId } = $props();
  let host;
  const spec = $derived(app.panes[paneId]);
  const pad = $derived(app.kitty?.padding ?? [0, 0, 0, 0]);
  const bg = $derived(app.kitty?.options?.theme?.background);

  onMount(() => {
    const t = termFor(paneId, {
      onKey: (e) => !e.metaKey,
      onExit: () => closePane(paneId),
      onFocus: () => !focusing && app.focused !== paneId && focusPane(paneId),
      onOpenPath: (file, line) => openEditor(file, line, { projectId: spec?.projectId }),
    });
    t.mount(host);
    t.start();
    if (app.focused === paneId) t.focus();
    const ro = new ResizeObserver(() => t.scheduleFit());
    ro.observe(host);
    return () => {
      ro.disconnect();
      t.unmount();
    };
  });
</script>

<div class="term" style:background={bg} style:padding="{pad[0]}px {pad[1]}px {pad[2]}px {pad[3]}px">
  <div class="host" bind:this={host}></div>
</div>

<style>
  .term {
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
