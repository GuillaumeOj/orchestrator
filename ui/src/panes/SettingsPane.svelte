<script>
  // Settings (§9.3, p3-design-notes §5.1): a section list on the left, the
  // section on the right. P3 builds Integrations and MCP servers; the other
  // sections say where their values are edited today. Keys: j/k sections,
  // ⏎ into the section, esc back to the list.
  import { onMount, untrack } from 'svelte';
  import IntegrationsSettings from '../components/IntegrationsSettings.svelte';
  import Kbd from '../components/Kbd.svelte';
  import McpSettings from '../components/McpSettings.svelte';
  import { app, saveLayout } from '../lib/app.svelte.js';
  import { attentionCount } from '../lib/ecosystem.js';
  import { invoke } from '../lib/ipc.js';
  import { move } from '../lib/select.js';

  let { paneId } = $props();
  const spec = $derived(app.panes[paneId]);

  const SECTIONS = [
    { id: 'claude', label: 'Claude Code' },
    { id: 'integrations', label: 'Integrations' },
    { id: 'mcp', label: 'MCP servers' },
    { id: 'terminal', label: 'Terminal & editor' },
    { id: 'pacing', label: 'Pacing policy' },
    { id: 'keyboard', label: 'Keyboard' },
    { id: 'memory', label: 'Memory' },
  ];

  const section = $derived(SECTIONS.find((s) => s.id === spec?.section) ?? SECTIONS[1]);
  // The nav's attention count, refreshed by the Integrations section and on open.
  let needs = $state(0);
  let mcpNeeds = $state(0); // servers that need auth or fail
  let nav = $state(null);
  let body = $state(null);

  function pick(id, into = false) {
    app.panes[paneId] = { ...app.panes[paneId], section: id, server: null, title: SECTIONS.find((s) => s.id === id)?.label ?? 'Settings' };
    saveLayout();
    if (into) queueMicrotask(() => body?.querySelector('[data-section-focus]')?.focus());
  }

  function onNavKey(e) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === 'j' || e.key === 'ArrowDown') pick(move(SECTIONS, section.id, 1));
    else if (e.key === 'k' || e.key === 'ArrowUp') pick(move(SECTIONS, section.id, -1));
    else if (e.key === 'Enter') pick(section.id, true);
    else return;
    e.preventDefault();
  }

  function onBodyKey(e) {
    if (e.key === 'Escape' && !e.defaultPrevented && !e.target.closest('input, textarea, select')) {
      e.preventDefault();
      nav?.focus();
    }
  }

  // One fetch of each view on open: the nav counts read them, and so does the
  // section shown first (handed over once; later loads are the section's own).
  const once = (p) => () => {
    const x = p;
    p = null;
    return x;
  };
  const intsP = invoke('integrations_view');
  const mcpP = invoke('mcp_registry_view', { project: untrack(() => spec?.projectId ?? null) });
  const firstInts = once(intsP);
  const firstMcp = once(mcpP);

  onMount(async () => {
    // The first section has mounted (children mount first) and took its view;
    // a section opened later fetches its own, fresh.
    firstInts();
    firstMcp();
    const [v, m] = await Promise.all([intsP.catch(() => null), mcpP.catch(() => null)]);
    if (v) needs = attentionCount(v.items);
    if (m) mcpNeeds = attentionCount(m.servers);
    if (app.panes[paneId] && app.panes[paneId].title !== section.label) app.panes[paneId] = { ...app.panes[paneId], title: section.label };
  });
</script>

<div class="settings">
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <nav class="nav" tabindex="0" data-pane-focus bind:this={nav} onkeydown={onNavKey} aria-label="Settings sections">
    <div class="navh"><b>Settings</b><Kbd keys="⌘+," /></div>
    {#each SECTIONS as s (s.id)}
      <button class="sec" class:on={s.id === section.id} aria-current={s.id === section.id ? 'page' : undefined} onclick={() => pick(s.id)} tabindex="-1">
        <span class="grow">{s.label}</span>
        {#if s.id === 'integrations' && needs}<span class="warn err" title="{needs} need you"><span aria-hidden="true">{'\u{F0159}'}</span> {needs}</span>{/if}
        {#if s.id === 'mcp' && mcpNeeds}<span class="warn" title="{mcpNeeds} need you"><span aria-hidden="true">{'\u{F1983}'}</span> {mcpNeeds}</span>{/if}
      </button>
    {/each}
    <span class="grow"></span>
    <span class="muted hints"><Kbd keys="j" /><Kbd keys="k" /> sections · <Kbd keys="⏎" /> enter</span>
  </nav>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="body" bind:this={body} onkeydown={onBodyKey}>
    {#if section.id === 'integrations'}
      <IntegrationsSettings initial={firstInts} onstatus={(items) => (needs = attentionCount(items))} />
    {:else if section.id === 'mcp'}
      {#key spec?.server}<McpSettings project={spec?.projectId ?? null} selected={spec?.server ?? null} initial={firstMcp} />{/key}
    {:else}
      <section class="later">
        <h2>{section.label}</h2>
        <p class="muted">Not designed yet. The current values are editable from <Kbd keys="⌘+K" />.</p>
      </section>
    {/if}
  </div>
</div>

<style>
  .settings {
    flex: 1;
    min-height: 0;
    display: flex;
    font: var(--text-label);
  }
  .nav {
    width: 200px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    padding: var(--space-md) var(--space-sm);
    background: var(--color-surface-chrome);
    border-right: var(--border-width-default) solid var(--color-border-subtle);
    outline: none;
  }
  .nav:focus-visible .sec.on {
    outline: var(--border-width-focus) solid var(--color-focus);
  }
  .navh {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 var(--space-sm) var(--space-sm);
  }
  .navh b {
    font: var(--text-title);
  }
  .sec {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    height: var(--size-row);
    padding: 0 var(--space-sm);
    background: none;
    border: 0;
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
    font: var(--text-body);
    text-align: left;
    cursor: pointer;
  }
  .sec.on {
    background: var(--color-surface-selected);
    color: var(--color-text-primary);
  }
  .warn {
    color: var(--color-status-waiting-for-you);
    font: var(--text-caption);
  }
  .hints {
    font: var(--text-caption);
    padding: 0 var(--space-sm);
  }
  .body {
    flex: 1;
    min-width: 0;
    overflow: auto;
    padding: var(--space-lg);
  }
  .later h2 {
    margin: 0 0 var(--space-sm);
    font: var(--text-heading);
  }
</style>
