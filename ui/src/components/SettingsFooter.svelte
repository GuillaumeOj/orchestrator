<script>
  // The footer of a form section (settings-brief §0.3): "Unsaved changes", Revert, Save; both
  // buttons disabled until dirty. It carries `.foot`, the hook a backdrop click on a dirty
  // window flashes (backdropClose in lib/app.svelte.js).
  import { tooltip } from '../lib/tooltip.js';

  /** @type {{ dirty: boolean, busy?: boolean, onrevert: () => void, onsave: () => void, revertKey?: string }} */
  let { dirty, busy = false, onrevert, onsave, revertKey = '' } = $props();
</script>

<div class="sfoot foot">
  {#if dirty}<span class="dirty">Unsaved changes</span>{/if}
  <button type="button" class="btn" disabled={!dirty} use:tooltip={revertKey ? { label: 'Revert', key: revertKey } : null} onclick={onrevert}>Revert</button>
  <button type="button" class="btn primary" disabled={busy || !dirty} use:tooltip={{ label: 'Save', key: '⌘⏎' }} onclick={onsave}>Save</button>
</div>
