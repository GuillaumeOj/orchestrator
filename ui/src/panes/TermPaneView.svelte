<script>
  // Shell and editor panes (FR-1001, FR-1102): the login shell or nvim on a PTY
  // the core owns, rendered like kitty. Closing the shell closes the pane, as in kitty.
  import { onMount } from 'svelte';
  import { app, mountTerm, closePane, openEditor } from '../lib/app.svelte.js';
  import TermHost from '../components/TermHost.svelte';

  let { paneId } = $props();
  let host = $state();

  onMount(() => {
    const m = mountTerm(paneId, host, {
      onExit: () => closePane(paneId),
      onOpenPath: (file, line) => openEditor(file, line, { projectId: app.panes[paneId]?.projectId }),
    });
    m.t.start();
    return m.destroy;
  });
</script>

<TermHost bind:host />
