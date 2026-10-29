<script>
  // An image in a Markdown doc (specs-rendering §3.5). A relative path loads
  // from the feature's docs folder through feature_asset, shown as a data: URL
  // (an SVG can't run script in an <img>); click opens it large, esc closes.
  // Remote images are never fetched (D2): a chip opening the browser. Without
  // a feature behind the doc, relative images are "not available here".
  import { openUrl } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { imageTarget } from '../lib/markdown.js';

  let { url = '', alt = '', feature = null, inLink = false } = $props();
  const target = $derived(imageTarget(url));
  /** @type {{ state: 'idle' | 'loading' | 'ok' | 'error', src?: string, error?: string }} */
  let img = $state({ state: 'idle' });
  let big = $state(false);

  $effect(() => {
    const t = target;
    const f = feature;
    if (t.kind !== 'asset' || !f) return;
    let live = true;
    img = { state: 'loading' };
    invoke('feature_asset', { feature: f, path: t.path })
      .then((r) => live && (img = { state: 'ok', src: `data:${r.mime};base64,${r.data_b64}` }))
      .catch((e) => live && (img = { state: 'error', error: errText(e) }));
    return () => (live = false);
  });

  // While it is open, keys stop here: the pane's own shortcuts (esc, a gate's
  // approve…) don't act on what the overlay hides.
  function onKey(e) {
    if (!big) return;
    e.stopPropagation();
    if (e.key !== 'Escape') return;
    e.preventDefault();
    big = false;
  }
</script>

<svelte:window onkeydowncapture={onKey} />

{#if target.kind === 'remote'}
  {#if inLink}<span class="chip">🖼 {alt || 'image'}</span>{:else}<button class="chip" title={target.url} onclick={() => openUrl(target.url)}>🖼 {alt || 'image'} ↗</button>{/if}
{:else if target.kind === 'data'}
  <img class="pic" src={target.url} {alt} />
{:else if target.kind === 'asset' && feature}
  {#if img.state === 'ok'}
    {#if inLink}<img class="pic" src={img.src} {alt} />{:else}<button class="open" aria-label="Open {alt || 'the image'} large" onclick={() => (big = true)}><img class="pic" src={img.src} {alt} /></button>{/if}
  {:else if img.state === 'error'}
    <span class="imgerr">image: {alt || target.path} ({img.error})</span>
  {:else}
    <span class="holder" aria-busy="true">{alt || target.path}</span>
  {/if}
{:else if target.kind !== 'dropped'}
  <span class="imgerr">image: {alt || url} (not available here)</span>
{/if}

{#if big && img.state === 'ok'}
  <!-- The whole overlay closes it: one control, esc as well (onKey). -->
  <button class="overlay" aria-label="Close the large {alt || 'image'}" onclick={() => (big = false)}>
    <img src={img.src} {alt} />
    <span class="hint">{alt}{alt ? ' · ' : ''}esc or click to close</span>
  </button>
{/if}

<style>
  .pic {
    max-width: 100%;
    vertical-align: middle;
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
  }
  .open {
    display: inline-block;
    max-width: 100%;
    padding: 0;
    background: none;
    border: 0;
    cursor: zoom-in;
  }
  .holder {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 160px;
    min-height: 90px;
    padding: var(--space-sm);
    font: var(--text-caption);
    color: var(--color-text-muted);
    background: var(--color-surface-chrome);
    border-radius: var(--radius-sm);
  }
  .chip {
    padding: 0 var(--space-sm);
    font: var(--text-caption);
    color: var(--color-accent-default);
    background: var(--color-surface-raised);
    border: 0;
    border-radius: var(--radius-full);
    cursor: pointer;
  }
  .imgerr {
    font: var(--text-caption);
    color: var(--color-text-muted);
  }
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-sm);
    padding: var(--space-xl);
    background: var(--color-surface-chrome);
    border: 0;
    cursor: zoom-out;
  }
  .overlay img {
    max-width: 100%;
    min-height: 0;
    flex: 0 1 auto;
    object-fit: contain;
  }
  .hint {
    font: var(--text-caption);
    color: var(--color-text-muted);
  }
</style>
