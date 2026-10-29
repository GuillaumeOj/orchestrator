<script>
  // The app shell (§9.2, Penpot 03): title bar, sidebar or icon rail, tiling
  // workspace over the shell dock, inspector (docked ≥ 1512 px, overlay
  // below), status bar.
  import { onMount } from 'svelte';
  import Banner from './components/Banner.svelte';
  import Dialogs from './components/Dialogs.svelte';
  import Dock from './components/Dock.svelte';
  import IconRail from './components/IconRail.svelte';
  import Inspector from './components/Inspector.svelte';
  import Palette from './components/Palette.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import TitleBar from './components/TitleBar.svelte';
  import Toasts from './components/Toasts.svelte';
  import Workspace from './components/Workspace.svelte';
  import { app, applyTheme, flushDecisions, focusedSpec, isTerminal, onOutcome, refresh, restoreLayout, run, terms, wide } from './lib/app.svelte.js';
  import { startDev } from './lib/dev.js';
  import { loadUiFont, prepareKitty } from './lib/fonts.js';
  import { inTauri, invoke, listen } from './lib/ipc.js';
  import { classify } from './lib/keyboard.js';
  import { connectNow, onTopics } from './lib/now.svelte.js';

  // The `now` topics app_state answers for (its non-attention data): the
  // projects list and the git state of projects and worktrees too.
  const STATE_TOPICS = ['agents', 'queue', 'usage', 'memory', 'power', 'banners', 'setup', 'features', 'teams'];
  const stateTopic = (t) => STATE_TOPICS.includes(t) || t.startsWith('project:') || t.startsWith('worktree:');
  let ready = $state(false);
  let lastKeyCommand = { id: null, at: 0 };
  const isNav = (k) => k === 'palette' || k === 'jump';

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
        // A dialog keeps the chords to itself; the palette and jump give way.
        if (app.overlay && !isNav(r.command)) {
          if (!isNav(app.overlay.kind)) return;
          app.overlay = null;
        }
        return run(r.command);
      case 'pty':
        terms.get(app.focused)?.write(r.bytes);
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
    let decisions;
    let unlisten = () => {};
    let unlistenOutcome = () => {};
    let stopNow = () => {};
    let stopState = () => {};
    (async () => {
      const [kitty] = await Promise.all([invoke('kitty_config').catch(() => null), loadUiFont().catch(() => [])]);
      if (kitty) app.kitty = await prepareKitty(kitty).catch(() => kitty);
      // app_state is read on its topics (spec §3.2), never on a timer (and right below).
      stopState = onTopics(stateTopic, refresh, { now: false });
      // Attention from the `now` push, resynced on a rev gap and when the window shows again.
      const [, stop] = await Promise.all([refresh(), connectNow()]);
      stopNow = stop;
      await restoreLayout();
      applyTheme();
      ready = true;
      startDev();
      // The undo window's flush: sends due decisions, fetches nothing.
      decisions = setInterval(flushDecisions, 1000);
      unlisten = await listen('menu', (ev) => {
        // A chord handled by the webview may also fire its menu item: once is enough.
        if (lastKeyCommand.id === ev.payload && Date.now() - lastKeyCommand.at < 400) return;
        run(ev.payload);
      });
      unlistenOutcome = await listen('inbox_outcome', (ev) => onOutcome(ev.payload));
    })();
    return () => {
      clearInterval(decisions);
      stopState();
      stopNow();
      unlisten();
      unlistenOutcome();
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
    <div class="work">
      {#if ready || !inTauri}<Workspace />{/if}
      {#if app.dock.open && app.dock.tabs.length}<Dock />{/if}
    </div>
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
  .work {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
