<script>
  // The app shell (§9.2, Penpot 03, P4.3 notes §9): title bar, sidebar or icon
  // rail, tiling workspace over the dock, the chat column (the Overwatch: 360 px
  // by default, 340 below 1512, or the width the user dragged by its left edge,
  // ChatResize), the Inspector (docked 320 at ≥ 1728 px, else an overlay over
  // the chat's slot), status bar.
  import { onMount } from 'svelte';
  import Banner from './components/Banner.svelte';
  import ChatColumn from './components/chat/ChatColumn.svelte';
  import ChatResize from './components/chat/ChatResize.svelte';
  import Dialogs from './components/Dialogs.svelte';
  import Dock from './components/Dock.svelte';
  import IconRail from './components/IconRail.svelte';
  import Inspector from './components/Inspector.svelte';
  import Palette from './components/Palette.svelte';
  import PlanChooser from './components/PlanChooser.svelte';
  import PlannerWindow from './components/PlannerWindow.svelte';
  import ProgramPlanDialog from './components/ProgramPlanDialog.svelte';
  import ProjectSettingsWindow from './components/ProjectSettingsWindow.svelte';
  import SettingsWindow from './components/SettingsWindow.svelte';
  import ShortcutsSheet from './components/ShortcutsSheet.svelte';
  import Sidebar from './components/Sidebar.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import TitleBar from './components/TitleBar.svelte';
  import Toasts from './components/Toasts.svelte';
  import Workspace from './components/Workspace.svelte';
  import { app, applyTheme, applyUpdate, chatWidthNow, dockFills, flushDecisions, inspectorDocked, inspectorOverlay, keyContext, refresh, restoreLayout, run, terms, wide } from './lib/app.svelte.js';
  import { startDev } from './lib/dev.js';
  import { loadUiFont, prepareKitty } from './lib/fonts.js';
  import { inTauri, invoke, listen } from './lib/ipc.js';
  import { classify, isTypingTarget } from './lib/keyboard.js';
  import { connectNow, onTopics } from './lib/now.svelte.js';

  // The `now` topics app_state answers for (its non-attention data): the
  // projects list and the git state of projects and worktrees too.
  const STATE_TOPICS = ['agents', 'queue', 'usage', 'memory', 'power', 'banners', 'setup', 'features', 'teams'];
  // Git state (project:, worktree:) changes with every file a watched worktree
  // writes (T-33): read for it at most every 10 s, the rest at once.
  const gitTopic = (t) => t.startsWith('project:') || t.startsWith('worktree:');
  const GIT_EVERY = 10000;
  let ready = $state(false);
  let lastKeyCommand = { id: null, at: 0 };
  const isNav = (k) => k === 'palette' || k === 'jump';
  // An open Select or Menu takes the Esc first (it closes itself, not the inspector overlay).
  const OPEN_POPOVER = '[role="combobox"][aria-expanded="true"], [aria-haspopup="menu"][aria-expanded="true"]';
  // With a window open (design-notes §3.3): the palette and jump open above it,
  // ⌘T / ⌘N their dialog above it, ⌘W closes it, ⌘, / ⌘/ toggle or replace it;
  // every other chord and leader key waits until it closes.
  const OVER_WINDOW = new Set(['palette', 'jump', 'session.new', 'project.add', 'pane.close', 'settings', 'shortcuts']);

  // Keyboard contract §9.7, in the capture phase: ⌘-chords and the leader never
  // reach xterm.js; everything else goes on to the focused element.
  function onKeyDown(e) {
    if (e.isComposing) return;
    const inXterm = !!e.target.closest?.('.xterm');
    const inField = !inXterm && isTypingTarget(e.target);
    const r = classify(e, keyContext(e.target));
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
        if (app.window && !OVER_WINDOW.has(r.command)) return;
        return run(r.command);
      case 'pty':
        terms.get(app.focused)?.write(r.bytes);
        return stop();
      default:
        // The watch-mode strip's Esc goes back to Mission control; Esc anywhere else closes the overlay.
        if (e.key === 'Escape' && inspectorOverlay() && !app.window && !inXterm && !document.querySelector(OPEN_POPOVER) && !e.target.closest?.('[data-watch]')) {
          app.inspector = false;
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
    let unlistenUpdate = () => {};
    let stopNow = () => {};
    let stopState = () => {};
    (async () => {
      const [kitty] = await Promise.all([invoke('kitty_config').catch(() => null), loadUiFont().catch(() => [])]);
      if (kitty) app.kitty = await prepareKitty(kitty).catch(() => kitty);
      // app_state is read on its topics (spec §3.2), never on a timer (and right below).
      const stopTopics = onTopics(STATE_TOPICS, refresh, { now: false });
      const stopGit = onTopics(gitTopic, refresh, { now: false, focus: false, every: GIT_EVERY });
      stopState = () => (stopTopics(), stopGit());
      // Attention from the `now` push, resynced on a rev gap and when the window shows again.
      const [, stop] = await Promise.all([refresh(), connectNow()]);
      stopNow = stop;
      await restoreLayout();
      applyTheme();
      ready = true;
      startDev();
      // The undo window's flush: sends due decisions, fetches nothing.
      decisions = setInterval(flushDecisions, 1000);
      unlistenUpdate = await listen('update', (ev) => applyUpdate(ev.payload));
      applyUpdate(app.data?.update);
      unlisten = await listen('menu', (ev) => {
        // A chord handled by the webview may also fire its menu item: once is enough.
        if (lastKeyCommand.id === ev.payload && Date.now() - lastKeyCommand.at < 400) return;
        run(ev.payload);
      });
    })();
    return () => {
      clearInterval(decisions);
      stopState();
      stopNow();
      unlisten();
      unlistenUpdate();
      window.removeEventListener('resize', onResize);
      window.removeEventListener('keydown', onKeyDown, true);
    };
  });

  const isWide = $derived(wide());
  const docked = $derived(inspectorDocked());
  const overlaid = $derived(inspectorOverlay());
</script>

<div class="shell">
  <TitleBar />
  <Banner />
  <div class="mid">
    {#if app.sidebar}
      {#if isWide}<Sidebar />{:else}<IconRail />{/if}
    {/if}
    <div class="work">
      {#if (ready || !inTauri) && !dockFills()}<Workspace />{/if}
      {#if app.dock.open && app.dock.tabs.length}<Dock />{/if}
    </div>
    <div class="chatslot" style:width="{chatWidthNow()}px">
      <ChatResize />
      <!-- Under the Inspector's overlay the chat is hidden: out of the Tab order too. -->
      <div class="chatwrap" inert={overlaid}><ChatColumn /></div>
      {#if overlaid}<Inspector overlay />{/if}
    </div>
    {#if docked && app.inspector}<Inspector />{/if}
  </div>
  <StatusBar />
  <!-- One floating window at a time, above the app and under the palette and the dialogs (design-notes §2.4). -->
  {#if app.window?.kind === 'settings'}<SettingsWindow />
  {:else if app.window?.kind === 'project-settings'}{#key app.window.projectId}<ProjectSettingsWindow />{/key}
  {:else if app.window?.kind === 'planner'}<PlannerWindow />
  {:else if app.window?.kind === 'shortcuts'}<ShortcutsSheet />{/if}
  {#if app.overlay?.kind === 'palette' || app.overlay?.kind === 'jump'}{#key app.overlay.kind}<Palette mode={app.overlay.kind} />{/key}{/if}
  {#if app.overlay?.kind === 'plan-project'}<PlanChooser />{/if}
  {#if app.overlay?.kind === 'program-plan'}{#key app.overlay}<ProgramPlanDialog />{/key}{/if}
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
  .chatslot {
    position: relative;
    flex: none;
    min-height: 0;
    display: flex;
  }
  .chatwrap {
    flex: 1;
    min-width: 0;
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
