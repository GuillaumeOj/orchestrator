<script module>
  /** The Settings window's sections, in nav order (settings-brief §0; no Keyboard: the sheet has it). */
  export const SECTIONS = [
    { id: 'claude', label: 'Claude Code' },
    { id: 'integrations', label: 'Integrations' },
    { id: 'overwatch', label: 'Overwatch' },
    { id: 'permissions', label: 'Permissions' },
    { id: 'mcp', label: 'MCP servers' },
    { id: 'terminal', label: 'Terminal & editor' },
    { id: 'pacing', label: 'Pacing policy' },
    { id: 'memory', label: 'Memory' },
    { id: 'builds', label: 'Builds' },
  ];
</script>

<script>
  // Settings (⌘,; design-notes §3, 04-settings-*.png): the floating window,
  // its section nav on the left (FloatingWindow: j/k ↑↓ sections, ⏎ into the
  // section, esc back to the nav, then closes). The sections are the
  // components they were in the pane; Keyboard moved to the shortcuts sheet (⌘/).
  import { onMount } from 'svelte';
  import { app, guardWindow, onWindowClose, windowSection } from '../lib/app.svelte.js';
  import { navBadge } from '../lib/claude.js';
  import { attentionCount } from '../lib/ecosystem.js';
  import { invoke } from '../lib/ipc.js';
  import { tooltip } from '../lib/tooltip.js';
  import BuildsSettings from './BuildsSettings.svelte';
  import ClaudeSettings from './ClaudeSettings.svelte';
  import FloatingWindow from './FloatingWindow.svelte';
  import IntegrationsSettings from './IntegrationsSettings.svelte';
  import McpSettings from './McpSettings.svelte';
  import MemorySettings from './MemorySettings.svelte';
  import OverwatchSettings from './OverwatchSettings.svelte';
  import PacingSettings from './PacingSettings.svelte';
  import PermissionsSettings from './PermissionsSettings.svelte';
  import TerminalSettings from './TerminalSettings.svelte';

  const spec = $derived(app.window);
  const section = $derived(SECTIONS.find((s) => s.id === spec?.section) ?? SECTIONS[1]);
  // The nav's attention count, refreshed by the Integrations section and on open.
  let needs = $state(0);
  let mcpNeeds = $state(0); // servers that need auth or fail
  const claudeBadge = $derived(navBadge(app.data?.setup?.claude));

  // One fetch of each view on open: the nav counts read them, and so does the
  // section shown first (handed over once; later loads are the section's own).
  const once = (p) => () => {
    const x = p;
    p = null;
    return x;
  };
  const intsP = invoke('integrations_view');
  const mcpP = invoke('mcp_registry_view', { project: null });
  const firstInts = once(intsP);
  const firstMcp = once(mcpP);

  // MCP servers' server form is the one section with unsaved edits (design-notes §3.3).
  let mcp = $state(null);
  onMount(() => guardWindow(() => section.id === 'mcp' && !!mcp?.hasChanges()));

  onMount(async () => {
    // The first section has mounted (children mount first) and took its view;
    // a section opened later fetches its own, fresh.
    firstInts();
    firstMcp();
    const [v, m] = await Promise.all([intsP.catch(() => null), mcpP.catch(() => null)]);
    if (v) needs = attentionCount(v.items);
    if (m) mcpNeeds = attentionCount(m.servers);
  });
</script>

{#snippet badge(id)}
  {#if id === 'claude' && claudeBadge}
    {#if claudeBadge.kind === 'error'}<span class="status-dot err" use:tooltip={claudeBadge.title} role="img" aria-label="Claude Code needs you">●</span>
    {:else if claudeBadge.kind === 'warn'}<span class="status-dot amber" use:tooltip={claudeBadge.title} role="img" aria-label="Claude Code update failed">●</span>
    {:else if claudeBadge.kind === 'spinner'}<span class="glyph spinning" use:tooltip={claudeBadge.title} role="img" aria-label="Updating Claude Code">◐</span>
    {:else}<span class="status-dot up" use:tooltip={claudeBadge.title} role="img" aria-label="Update available">●</span>{/if}
  {/if}
  {#if id === 'integrations' && needs}<span class="warn" use:tooltip={`${needs} need you`}>{needs}</span>{/if}
  {#if id === 'mcp' && mcpNeeds}<span class="warn" use:tooltip={`${mcpNeeds} need you`}>{mcpNeeds}</span>{/if}
{/snippet}

<FloatingWindow
  title="Settings"
  size={{ w: 960, h: 760 }}
  nav={SECTIONS}
  navTitle="Settings"
  section={section.id}
  onsection={windowSection}
  navBadge={badge}
  onclose={onWindowClose}
  restoreFocus={false}
>
  {#if section.id === 'integrations'}
    <IntegrationsSettings initial={firstInts} onstatus={(items) => (needs = attentionCount(items))} />
  {:else if section.id === 'overwatch'}
    <OverwatchSettings />
  {:else if section.id === 'permissions'}
    <PermissionsSettings />
  {:else if section.id === 'mcp'}
    {#key spec?.server}<McpSettings bind:this={mcp} project={null} selected={spec?.server ?? null} initial={firstMcp} />{/key}
  {:else if section.id === 'claude'}
    <ClaudeSettings />
  {:else if section.id === 'builds'}
    {#key spec?.server}<BuildsSettings selected={spec?.server ?? null} />{/key}
  {:else if section.id === 'pacing'}
    <PacingSettings />
  {:else if section.id === 'terminal'}
    <TerminalSettings />
  {:else if section.id === 'memory'}
    <MemorySettings />
  {/if}
</FloatingWindow>

<style>
  .warn {
    color: var(--color-status-waiting-for-you);
    font: var(--text-caption);
  }
  .status-dot {
    font: var(--text-caption);
  }
  .status-dot.amber {
    color: var(--color-pacing-amber);
  }
  .status-dot.up {
    color: var(--color-accent-default);
  }
  .glyph {
    font: var(--text-icon);
  }
  .glyph.spinning {
    color: var(--color-attention-running);
  }
</style>
