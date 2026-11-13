<script>
  // The origin mark (P4.5 notes §6): `from #123`, `from ENG-42`, `from checkout-v2.md`, `from brief`.
  // Muted, mono, no border; hover underlines; not a focus stop. A click opens the source (the
  // browser, the editor for a repo doc) or, for a brief, its line in the chat.
  import { focusCard, openEditor, openUrl, projects } from '../../lib/app.svelte.js';
  import { docPath, originText, originTip } from '../../lib/intake.js';

  /** @type {{ origin: any, project?: string | null, suffix?: string, label?: string | null }} */
  let { origin, project = null, suffix = '', label = null } = $props();

  function open(e) {
    e.stopPropagation();
    if (origin.kind === 'brief') return focusCard(origin.decision);
    if (origin.url) return openUrl(origin.url);
    const p = projects().find((x) => x.id === project || x.name === project);
    if (p && origin.ref) openEditor(docPath(origin), null, { projectId: p.id });
  }
</script>

{#if origin}
  <button class="origin mono" type="button" tabindex="-1" title={originTip(origin)} onclick={open}>{label ?? `${suffix}${originText(origin)}`}</button>
{/if}

<style>
  .origin {
    flex: none;
    padding: 0;
    border: 0;
    background: none;
    color: var(--color-text-muted);
    font-size: 11px;
    cursor: pointer;
  }
  .origin:hover {
    text-decoration: underline;
  }
</style>
