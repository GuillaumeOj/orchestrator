<script module>
  /** A workspace tab's kind icon (02-tab-strip.png). */
  const ICON = {
    mission: '\u{F0437}',
    project: '\u{F024B}',
    feature: '\u{F062C}',
    diff: '\u{F08AA}',
    canvas: '\u{F1049}',
    editor: '\u{E62B}',
    shell: '\u{F018D}',
    report: '\u{F0219}',
    setup: '\u{F0493}',
    trust: '\u{F0498}',
  };
</script>

<script>
  // The workspace (design-notes §4.3, §5.1, 03-app-*, 03-context-*,
  // 03-empty-workspace-*): one tab strip led by the project context Select,
  // then the active tab's pane. Tabs of other projects are hidden, not closed;
  // a hidden pane does not render, its terminal keeps running (TermPane keeps
  // the buffer).
  import {
    activeTab,
    app,
    closePaneByUser,
    contextOptions,
    contextWaiting,
    focusPane,
    openMission,
    openProject,
    projectName,
    setContext,
    visibleTabs,
  } from '../lib/app.svelte.js';
  import PaneHost from './PaneHost.svelte';
  import Select from './Select.svelte';
  import TabStrip from './TabStrip.svelte';

  const PREFIX = 'ws';
  const PANEL = 'ws-panel';

  /** A tab's label: a project pane says what it is ("orch · project"). */
  const tabTitle = (spec) => (spec.type === 'project' ? `${spec.title} · project` : spec.title);

  const tabs = $derived(
    visibleTabs().map((id) => {
      const s = app.panes[id];
      return { id, title: s ? tabTitle(s) : id, icon: ICON[s?.type] };
    }),
  );
  const active = $derived(activeTab());
  const ctx = $derived(app.workspace.project);

  /** @type {any} */
  let ctxSelect = $state();
  // The palette's "Set the project context…" opens the Select.
  $effect(() => {
    if (!app.contextOpen || !ctxSelect) return;
    app.contextOpen = false;
    ctxSelect.show();
  });
</script>

<main class="ws">
  <TabStrip {tabs} {active} label="Workspace tabs" idPrefix={PREFIX} panelId={PANEL} onselect={focusPane} onclose={closePaneByUser}>
    {#snippet leading()}
      <span class="ctx">
        <Select
          bind:this={ctxSelect}
          compact
          bare
          width="hug"
          aria-label="Project context"
          noun="project"
          value={ctx}
          options={contextOptions()}
          dot={contextWaiting() ?? false}
          onchange={setContext}
        />
      </span>
    {/snippet}
  </TabStrip>
  {#if active}
    {#key active}<PaneHost paneId={active} id={PANEL} labelledby="{PREFIX}-tab-{active}" />{/key}
  {:else}
    <div class="empty">
      <p class="muted">⌘K to open anything</p>
      <p class="acts">
        {#if ctx === 'all'}
          <button class="btn" onclick={openMission}>Mission control</button>
        {:else}
          <button class="btn" onclick={() => openProject(ctx)}>Open {projectName(ctx)}</button>
        {/if}
        <button class="btn" onclick={() => (app.overlay = { kind: 'new-session' })}>New session</button>
      </p>
    </div>
  {/if}
</main>

<style>
  .ws {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--color-surface-pane);
  }
  .ctx {
    display: flex;
    max-width: 220px;
    min-width: 0;
  }
  .empty {
    margin: auto;
    text-align: center;
  }
  .acts {
    display: flex;
    justify-content: center;
    gap: var(--space-sm);
  }
</style>
