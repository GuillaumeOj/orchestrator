<script module>
  /** The Project settings window's sections, in nav order (General first, design-notes §3.1). */
  export const SECTIONS = [
    { id: 'general', label: 'General' },
    { id: 'files', label: 'Files' },
    { id: 'tracker', label: 'Tracker' },
    { id: 'allocation', label: 'Allocation' },
    { id: 'notion', label: 'Notion' },
    { id: 'mcp', label: 'MCP overrides' },
  ];

</script>

<script>
  // Project settings (design-notes §3, improve-intake notes §10): a project's sections in the
  // floating window. Tracker (where its work lives, replacing Task backend and Intake), Notion
  // (the parent page for Publish, FR-505), the MCP overrides, Files, General and Allocation.
  // ⌘⏎ saves the section. A dirty footer section asks "Discard changes?" before the window
  // closes or the section changes; a backdrop click on it only flashes the footer.
  import { onMount, untrack } from 'svelte';
  import { app, guardWindow, onWindowClose, openSettings, projects, toast, windowSection } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import AllocationSettings from './AllocationSettings.svelte';
  import FilesSettings from './FilesSettings.svelte';
  import FloatingWindow from './FloatingWindow.svelte';
  import McpSettings from './McpSettings.svelte';
  import ProjectGeneral from './ProjectGeneral.svelte';
  import SettingsFooter from './SettingsFooter.svelte';
  import TrackerSettings from './TrackerSettings.svelte';

  const spec = $derived(app.window);
  // The window is for one project: a new project mounts a new window.
  const pid = untrack(() => spec?.projectId);
  const name = $derived(projects().find((p) => p.id === pid)?.name ?? pid);

  const section = $derived(SECTIONS.find((s) => s.id === spec?.section) ?? SECTIONS.find((s) => s.id === 'tracker'));

  let saved = $state(null); // project_settings (Notion's page)
  let error = $state(null);
  let notionPage = $state('');
  let busy = $state(false);
  let alloc = $state(null); // AllocationSettings (forwards the keys)
  let files = $state(null); // FilesSettings (same)
  let tracker = $state(null); // TrackerSettings (⌘⏎ saves)
  let general = $state(null); // ProjectGeneral (name and default branch: its own footer)
  let mcp = $state(null); // McpSettings (the server form is the one with unsaved edits)

  const notionDirty = $derived(!!saved && notionPage !== (saved.settings?.notion_parent_page ?? ''));

  /** The shown section has changes that are not saved (the window's Discard guard). */
  function sectionDirty() {
    if (section.id === 'notion') return notionDirty;
    if (section.id === 'general') return !!general?.hasChanges();
    if (section.id === 'tracker') return !!tracker?.hasChanges();
    if (section.id === 'allocation') return !!alloc?.hasChanges();
    if (section.id === 'mcp') return !!mcp?.hasChanges();
    return false;
  }
  // Discarding (closing or leaving the section): the fields go back to what is saved.
  /** Discarding the shown section's changes. */
  function sectionRevert() {
    if (section.id === 'general') general?.revert();
    else if (section.id === 'tracker') tracker?.revert();
    else if (notionDirty) revertNotion();
  }
  onMount(() => guardWindow(sectionDirty, sectionRevert));

  async function load() {
    try {
      saved = await invoke('project_settings', { project: pid });
      error = null;
      notionPage = saved.settings?.notion_parent_page ?? '';
    } catch (e) {
      error = errText(e);
    }
  }

  /** Notion › Save: only the parent page, the rest of the settings as they are. */
  async function saveNotion() {
    if (busy || !saved) return;
    busy = true;
    try {
      // The backend is the tracker's now, saved by its own section: read the stored one back, or a
      // Notion save would put this window's stale copy over a tracker saved since it opened.
      const now = await invoke('project_settings', { project: pid });
      const settings = { ...now.settings, notion_parent_page: notionPage.trim() || undefined };
      saved = await invoke('project_settings_set', { project: pid, form: { settings, backend: now.backend, track_orchestrator_dir: now.track_orchestrator_dir } });
      toast(`Saved ${name}'s settings`);
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    } finally {
      busy = false;
    }
  }

  /** Notion › Revert: the field goes back to the saved page. */
  function revertNotion() {
    notionPage = saved?.settings?.notion_parent_page ?? '';
  }

  function onKey(e) {
    if (section.id === 'allocation') return alloc?.onKey(e);
    if (section.id === 'files') return files?.onKey(e);
    if (section.id === 'tracker') return tracker?.onKey(e);
    if (e.metaKey && e.key === 'Enter') {
      e.preventDefault();
      if (section.id === 'notion') saveNotion();
      else if (section.id === 'general') general?.save();
    }
  }

  onMount(load);
</script>

<FloatingWindow title="Project settings" subtitle={name} size={{ w: 960, h: 760 }} nav={SECTIONS} section={section.id} onsection={windowSection} onclose={onWindowClose} restoreFocus={false}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- data-pane-focus: where a section puts the focus back when its button goes away (it keeps the keys). -->
  <div class="ps" tabindex="-1" data-pane-focus onkeydown={onKey}>
    {#if section.id === 'general'}
      <ProjectGeneral bind:this={general} project={pid} />
    {:else if section.id === 'allocation'}
      <AllocationSettings bind:this={alloc} project={pid} {name} />
    {:else if section.id === 'files'}
      <FilesSettings bind:this={files} project={pid} {name} />
    {:else if section.id === 'tracker'}
      <TrackerSettings bind:this={tracker} project={pid} {name} block={spec?.block ?? null} />
    {:else if error}
      <p class="err">Could not read {name}'s settings: {error}</p>
    {:else if !saved}
      <p class="muted">Loading…</p>
    {:else if section.id === 'notion'}
      <h2>Notion</h2>
      <p class="muted lead">Publish to Notion (on a feature's page) creates one page per feature under this parent page.</p>
      <label class="line"><span class="lab">Parent page</span><input class="field grow" bind:value={notionPage} placeholder="Notion page URL or id" spellcheck="false" /></label>
      <p class="muted note">Share the page with your Notion integration. The token is in <button class="link" onclick={() => openSettings('integrations')}>Settings › Integrations</button>.</p>
      <SettingsFooter dirty={notionDirty} {busy} onrevert={revertNotion} onsave={() => saveNotion()} />
    {:else if section.id === 'mcp'}
      {#key spec?.server}<McpSettings bind:this={mcp} project={pid} selected={spec?.server ?? null} />{/key}
    {/if}
  </div>
</FloatingWindow>

<style>
  .ps {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    font: var(--text-label);
    outline: none;
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  .lead,
  .note {
    margin: 0;
    font: var(--text-caption);
  }
  .line {
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }
  .lab {
    width: 120px;
    flex: none;
    color: var(--color-text-muted);
  }
  .link {
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
</style>
