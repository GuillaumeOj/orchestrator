<script>
  // The workspace's active pane, without a header (design-notes §4.3): the tab
  // is the pane's title and the active tab is the cue, no accent border. The
  // body switch by `spec.type` picks the pane.
  import { app, focusPane } from '../lib/app.svelte.js';
  import AgentPane from '../panes/AgentPane.svelte';
  import DiffPane from '../panes/DiffPane.svelte';
  import FeaturePane from '../panes/FeaturePane.svelte';
  import MissionPane from '../panes/MissionPane.svelte';
  import ProgramPane from '../panes/ProgramPane.svelte';
  import ProjectPane from '../panes/ProjectPane.svelte';
  import ReportPane from '../panes/ReportPane.svelte';
  import SetupPane from '../panes/SetupPane.svelte';
  import TeamCanvasPane from '../panes/TeamCanvasPane.svelte';
  import TermPaneView from '../panes/TermPaneView.svelte';
  import WorkPane from '../panes/WorkPane.svelte';

  /** @type {{ paneId: string, id?: string, labelledby?: string }} */
  let { paneId, id, labelledby } = $props();
  const spec = $derived(app.panes[paneId]);
</script>

{#if spec}
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_no_noninteractive_element_interactions -->
  <div class="pane" {id} role="tabpanel" tabindex="-1" aria-labelledby={labelledby} data-pane={paneId} onmousedown={() => app.focused !== paneId && focusPane(paneId)}>
    {#if spec.type === 'agent'}<AgentPane {paneId} />
    {:else if spec.type === 'shell' || spec.type === 'editor'}<TermPaneView {paneId} />
    {:else if spec.type === 'mission'}<MissionPane {paneId} />
    {:else if spec.type === 'project'}<ProjectPane {paneId} />
    {:else if spec.type === 'diff'}<DiffPane {paneId} />
    {:else if spec.type === 'canvas'}<TeamCanvasPane {paneId} />
    {:else if spec.type === 'feature'}<FeaturePane {paneId} />
    {:else if spec.type === 'report'}<ReportPane {paneId} />
    {:else if spec.type === 'work'}<WorkPane {paneId} />
    {:else if spec.type === 'program'}<ProgramPane {paneId} />
    {:else if spec.type === 'setup'}<SetupPane />{/if}
  </div>
{/if}

<style>
  .pane {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
    background: var(--color-surface-pane);
  }
</style>
