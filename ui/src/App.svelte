<script>
  // The app shell (§9.2, Penpot 03): title bar, sidebar or icon rail, tiling
  // workspace, inspector (docked ≥ 1512 px, overlay below), status bar.
  import { onMount } from 'svelte';
  import { app, terms, refresh, restoreLayout, run, focusedSpec, isTerminal, flushDecisions, wide, setTheme } from './lib/app.svelte.js';
  import { classify } from './lib/keyboard.js';
  import { invoke, listen, inTauri } from './lib/ipc.js';
  import { loadUiFont, prepareKitty } from './lib/fonts.js';
  import { startDev } from './lib/dev.js';
  import TitleBar from './components/TitleBar.svelte';
  import Banner from './components/Banner.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import IconRail from './components/IconRail.svelte';
  import Workspace from './components/Workspace.svelte';
  import Inspector from './components/Inspector.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import Palette from './components/Palette.svelte';
  import Dialogs from './components/Dialogs.svelte';
  import Toasts from './components/Toasts.svelte';

  let ready = $state(false);
  let lastKeyCommand = { id: null, at: 0 };

  // Keyboard contract §9.7, in the capture phase: ⌘-chords and the leader never
  // reach xterm.js; everything else goes on to the focused element.
  function onKeyDown(e) {
    if (e.isComposing) return;
    const spec = focusedSpec();
    const inXterm = !!e.target.closest?.('.xterm');
    const inField = !inXterm && !!e.target.closest?.('input, textarea, select');
    const r = classify(e, { leader: app.leader, terminal: isTerminal(spec) && inXterm, agent: spec?.type === 'agent', remaps: app.kitty?.key_remaps });
    const stop = () => {
      e.preventDefault();
      e.stopPropagation();
    };
    switch (r.kind) {
      case 'leader':
        app.leader = true;
        return stop();
      case 'cancel':
        app.leader = false;
        return stop();
      case 'swallow':
        if (inField && e.metaKey) return; // ⌘← etc. in text fields
        return stop();
      case 'command':
        app.leader = false;
        lastKeyCommand = { id: r.command, at: Date.now() };
        stop();
        if (app.overlay && !['palette', 'jump'].includes(r.command) && app.overlay.kind !== 'palette' && app.overlay.kind !== 'jump') return;
        if (app.overlay && ['palette', 'jump'].includes(app.overlay.kind) && !['palette', 'jump'].includes(r.command)) app.overlay = null;
        return run(r.command);
      case 'pty':
        terms.get(app.focused)?.send(r.bytes);
        return stop();
      default:
        if (e.key === 'Escape' && app.inspectorOverlay && !inXterm) {
          app.inspectorOverlay = false;
          stop();
        }
    }
  }

  onMount(() => {
    const onResize = () => (app.width = window.innerWidth);
    window.addEventListener('resize', onResize);
    window.addEventListener('keydown', onKeyDown, true);
    let poll;
    let unlisten = () => {};
    (async () => {
      const [kitty] = await Promise.all([invoke('kitty_config').catch(() => null), loadUiFont().catch(() => [])]);
      if (kitty) app.kitty = await prepareKitty(kitty).catch(() => kitty);
      await refresh();
      await restoreLayout();
      if (app.theme) setTheme(app.theme);
      ready = true;
      startDev();
      poll = setInterval(() => {
        refresh();
        flushDecisions();
      }, 1000);
      unlisten = await listen('menu', (ev) => {
        // A chord handled by the webview may also fire its menu item: once is enough.
        if (lastKeyCommand.id === ev.payload && Date.now() - lastKeyCommand.at < 400) return;
        run(ev.payload);
      });
    })();
    return () => {
      clearInterval(poll);
      unlisten();
      window.removeEventListener('resize', onResize);
      window.removeEventListener('keydown', onKeyDown, true);
    };
  });

  const isWide = $derived(wide());
</script>

<div class="shell">
  <TitleBar />
  <Banner />
  <div class="mid">
    {#if app.sidebar}
      {#if isWide}<Sidebar />{:else}<IconRail />{/if}
    {/if}
    {#if ready || !inTauri}<Workspace />{/if}
    {#if isWide && app.inspector}<Inspector />{/if}
  </div>
  <StatusBar />
  {#if !isWide && app.inspectorOverlay}<Inspector overlay />{/if}
  {#if app.overlay?.kind === 'palette' || app.overlay?.kind === 'jump'}{#key app.overlay.kind}<Palette mode={app.overlay.kind} />{/key}{/if}
  <Dialogs />
  <Toasts />
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .mid {
    flex: 1;
    min-height: 0;
    display: flex;
  }
</style>
