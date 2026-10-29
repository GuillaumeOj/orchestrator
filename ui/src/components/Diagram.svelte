<script>
  // A ```mermaid fence in a Markdown doc (specs-rendering §3.4): Mermaid
  // (loaded on first use) draws it in the app's theme and redraws when the
  // theme changes. The SVG is sanitized by lib/mermaid.js before insertion:
  // the renderer's one {@html}. An invalid diagram shows its source and the error.
  import { app } from '../lib/app.svelte.js';
  import { errText } from '../lib/ipc.js';
  import { loadMermaid } from '../lib/markdown.js';
  import CodeBlock from './CodeBlock.svelte';

  let { source = '' } = $props();
  /** @type {{ state: 'loading' | 'ok' | 'error', svg?: string, error?: string }} */
  let out = $state({ state: 'loading' });

  $effect(() => {
    const src = source;
    app.themeRev; // redraw when the theme changes
    let live = true;
    loadMermaid()
      .then((m) => m.renderDiagram(src))
      .then((svg) => live && (out = { state: 'ok', svg }))
      .catch((e) => live && (out = { state: 'error', error: errText(e).split('\n')[0] }));
    return () => (live = false);
  });
</script>

{#if out.state === 'error'}
  <div class="bad">
    <CodeBlock code={source} lang="mermaid" />
    <p class="err">Mermaid: {out.error}</p>
  </div>
{:else if out.state === 'ok'}
  <!-- Sanitized SVG (lib/mermaid.js sanitizeSvg), never agent text as HTML. -->
  <div class="diagram">{@html out.svg}</div>
{:else}
  <div class="diagram loading" aria-busy="true">Drawing the diagram…</div>
{/if}

<style>
  .diagram {
    display: flex;
    padding: var(--space-sm);
    overflow-x: auto;
    background: var(--color-surface-pane);
    border-radius: var(--radius-sm);
  }
  /* margin: auto centers without clipping the left side when it scrolls. */
  .diagram :global(svg) {
    flex: none;
    margin: 0 auto;
  }
  .loading {
    font: var(--text-caption);
    color: var(--color-text-muted);
  }
  .bad {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
  }
  /* Color from the global .err. */
  .err {
    margin: 0;
    font: var(--text-caption);
  }
</style>
