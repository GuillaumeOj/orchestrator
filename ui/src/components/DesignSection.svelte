<script>
  // Feature pane › Design (FR-602, FR-603; p3-design-notes §5.6–5.7, J12 · 3–7):
  // design artifacts grouped by from → to, a preview of each exported image
  // (from the integration branch, or the designer's branch before the merge)
  // and a tool tile for Penpot / Figma links. Keys (the section focused, `d`
  // from the pane): h j k l / arrows move, ⏎ open, Space large preview, x ask
  // the designer to export a link-only frame (D9). In the preview: Space/esc
  // close, ←/→ previous / next, 0 fit, 1 100 %, ⏎ open.
  import { untrack } from 'svelte';
  import { toast } from '../lib/app.svelte.js';
  import { artifactTitle, artifactTool, designGroups, linkOnly, notMerged, previewable, previewError, size, TOOL } from '../lib/ecosystem.js';
  import { clock } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';
  import AgentChip from './AgentChip.svelte';
  import Kbd from './Kbd.svelte';

  /** @type {{ v: any }} */
  let { v } = $props();

  let previews = $state({}); // artifact id → { state: 'loading' | 'ok' | 'error', src?, mime?, bytes?, source?, error? }
  let sel = $state(0);
  let big = $state(null); // { id, fit: boolean, w?, h? }
  let root = $state(null);

  const groups = $derived(designGroups(v.artifacts));
  const tiles = $derived(groups.flatMap((g) => g.items));
  const cur = $derived(tiles[sel] ?? null);
  const featureId = $derived(v.feature.id);
  /** A node's agent id in this run ("ui" → "onboarding-ui-1"), else the node. */
  const who = (node) => v.agents?.find((a) => a.node === node)?.id ?? node;
  const roleOfNode = (node) => v.team?.nodes?.find((n) => n.id === node)?.role_id ?? node;

  const emptyText = $derived.by(() => {
    const design = v.team?.nodes?.find((n) => n.role_id === 'ui') ?? v.team?.nodes?.find((n) => n.role_id === 'ux');
    if (!design) return 'No designs in this feature.';
    const task = v.tasks?.find((t) => t.node === design.id && t.status !== 'done');
    return task
      ? `No designs yet. ${who(design.id)} is on '${task.title}'. Exported frames show here when they're handed off.`
      : "No designs yet. Exported frames show here when they're handed off.";
  });

  async function loadPreview(a) {
    if (!previewable(a) || previews[a.id]?.state === 'ok' || previews[a.id]?.state === 'loading') return;
    previews = { ...previews, [a.id]: { state: 'loading' } };
    try {
      const r = await invoke('artifact_preview', { feature: featureId, artifact: a.id });
      // SVG too: only ever as an <img> data URL, never inlined into the DOM.
      previews = { ...previews, [a.id]: { state: 'ok', src: `data:${r.mime};base64,${r.data_b64}`, mime: r.mime, bytes: r.bytes, source: r.source } };
    } catch (e) {
      previews = { ...previews, [a.id]: { state: 'error', error: errText(e) } };
    }
  }

  // Each tile's preview once (the pane's poll replaces `v` every few seconds).
  $effect(() => {
    const list = tiles;
    untrack(() => {
      for (const a of list) loadPreview(a);
    });
  });

  async function openArtifact(a) {
    if (!a) return;
    try {
      await invoke('artifact_open', { feature: featureId, artifact: a.id });
    } catch (e) {
      toast(`Could not open ${artifactTitle(a)}: ${errText(e)}`, { kind: 'error' });
    }
  }

  /** D9: a message from you to the designer, through the run (FR-409). */
  async function askExport(a) {
    if (!a || !linkOnly(a) || !v.run) return;
    const tool = TOOL[artifactTool(a)].label;
    const body = `Please export "${artifactTitle(a)}" as an image under .orchestrator/features/${v.feature.slug}/design/, commit it and hand it off again: ${a.to_node ? who(a.to_node) : 'your teammates'} can't open ${tool} links.`;
    try {
      await invoke('message_send', { run: v.run.id, to: a.from_node, body });
      toast(`Asked ${who(a.from_node)} to export ${artifactTitle(a)}`);
    } catch (e) {
      toast(errText(e), { kind: 'error' });
    }
  }

  function showBig(a) {
    if (a && previewable(a)) big = { id: a.id, fit: true };
  }

  /** The caption's "1512 × 982". */
  function measure(e) {
    const img = /** @type {HTMLImageElement} */ (e.currentTarget);
    big = { ...big, w: img.naturalWidth, h: img.naturalHeight };
  }

  function step(d) {
    const imgs = tiles.filter(previewable);
    if (!imgs.length || !big) return;
    const i = imgs.findIndex((a) => a.id === big.id);
    big = { id: imgs[(i + d + imgs.length) % imgs.length].id, fit: true };
  }

  function onKey(e) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const k = e.key;
    if (big) {
      const a = tiles.find((x) => x.id === big.id);
      if (k === ' ' || k === 'Escape') big = null;
      else if (k === 'ArrowLeft') step(-1);
      else if (k === 'ArrowRight') step(1);
      else if (k === '0') big = { ...big, fit: true };
      else if (k === '1') big = { ...big, fit: false };
      else if (k === 'Enter') openArtifact(a);
      else return;
      e.preventDefault();
      e.stopPropagation();
      return;
    }
    if (!tiles.length) return;
    if (k === 'l' || k === 'ArrowRight' || k === 'j' || k === 'ArrowDown') sel = Math.min(tiles.length - 1, sel + 1);
    else if (k === 'h' || k === 'ArrowLeft' || k === 'k' || k === 'ArrowUp') sel = Math.max(0, sel - 1);
    else if (k === 'Enter') openArtifact(cur);
    else if (k === ' ') showBig(cur);
    else if (k === 'x') askExport(cur);
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  /** Focus the section (`d` in the Feature pane). */
  export function focus() {
    root?.focus();
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<section class="card design" tabindex="0" bind:this={root} onkeydown={onKey} aria-label="Design">
  <div class="cardh"><b>Design</b><span class="muted">{tiles.length ? `${tiles.length} handed off` : ''}</span><span class="grow"></span><span class="muted"><Kbd keys="d" /> focus · <Kbd keys="Space" /> preview · <Kbd keys="⏎" /> open</span></div>
  {#if !tiles.length}
    <p class="muted">{emptyText}</p>
  {:else}
    {#each groups as g (g.key)}
      <div class="gh">
        <AgentChip id={who(g.from)} role={roleOfNode(g.from)} />
        <span class="muted">→</span>
        {#if g.to}<AgentChip id={who(g.to)} role={roleOfNode(g.to)} />{:else}<span class="muted">—</span>{/if}
        <span class="muted">· {g.items.length}</span>
      </div>
      <div class="grid">
        {#each g.items as a (a.id)}
          {@const i = tiles.indexOf(a)}
          {@const p = previews[a.id]}
          {@const tool = TOOL[artifactTool(a)]}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="tile" class:sel={i === sel} data-artifact={a.id} onclick={() => (sel = i)} ondblclick={() => showBig(a)}>
            <div class="thumb">
              {#if previewable(a)}
                {#if p?.state === 'ok'}<img src={p.src} alt={artifactTitle(a)} loading="lazy" />
                {:else if p?.state === 'error'}<span class="msg">{previewError(p.error, a)}</span>
                {:else}<span class="skel" aria-busy="true"></span>{/if}
              {:else if linkOnly(a)}
                <span class="tool"><span class="ticon" aria-hidden="true">{tool.icon}</span>{tool.label}</span>
              {:else}
                <span class="msg">{artifactTitle(a)}: no preview</span>
              {/if}
            </div>
            <div class="meta">
              <span class="title" title={a.uri}>{artifactTitle(a)}</span>
              {#if notMerged(a, p)}<span class="chip">On {who(a.from_node)}'s branch · not merged</span>{/if}
              {#if a.note}<span class="note">{a.note}</span>{/if}
              {#if linkOnly(a)}<span class="hint">Link only: {a.to_node ? who(a.to_node) : 'agents'} can't open it. <button class="link" onclick={() => askExport(a)}>Ask {who(a.from_node)} to export</button>{#if i === sel} <Kbd keys="x" />{/if}</span>{/if}
              <span class="foot"><span class="muted">{clock(a.created_at_ms)}</span><span class="grow"></span><button class="link" onclick={() => openArtifact(a)}>Open</button>{#if i === sel}<Kbd keys="⏎" />{/if}</span>
            </div>
          </div>
        {/each}
      </div>
    {/each}
  {/if}

  {#if big}
    {@const a = tiles.find((x) => x.id === big.id)}
    {@const p = previews[big.id]}
    <div class="overlay" role="dialog" aria-modal="true" aria-label="Preview">
      <div class="stage" class:fit={big.fit}>
        {#if p?.state === 'ok'}
          <img src={p.src} alt={artifactTitle(a)} onload={measure} />
        {:else if p?.state === 'error'}
          <p class="msg">Could not load this preview: {p.error}. Open ⏎ to try Quick Look.</p>
        {:else}
          <span class="skel big" aria-busy="true"></span>
        {/if}
      </div>
      <div class="cap">
        <span>{[artifactTitle(a), big.w ? `${big.w} × ${big.h}` : null, p?.bytes != null ? size(p.bytes) : null, p?.source ? (p.source === 'branch' ? `${who(a.from_node)}'s branch` : 'integration branch') : null].filter(Boolean).join(' · ')}{#if p?.mime === 'image/svg+xml'} · SVG shown as an image{/if}</span>
        <span class="grow"></span>
        <span class="muted"><Kbd keys="0" /> fit · <Kbd keys="1" /> 100 % · <Kbd keys="←" /><Kbd keys="→" /> · <Kbd keys="⏎" /> open · <Kbd keys="esc" /> close</span>
      </div>
    </div>
  {/if}
</section>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-lg);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    outline: none;
  }
  .card:focus-visible {
    border-color: var(--color-focus);
  }
  .cardh,
  .gh,
  .foot {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .cardh b {
    font: var(--text-title);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: var(--space-sm);
  }
  .tile {
    display: flex;
    flex-direction: column;
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    overflow: hidden;
    cursor: pointer;
  }
  .tile.sel {
    border-color: var(--color-border-strong);
  }
  .design:focus .tile.sel {
    border: var(--border-width-focus) solid var(--color-focus);
  }
  .thumb {
    aspect-ratio: 3 / 2;
    max-height: 320px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--color-surface-chrome);
    overflow: hidden;
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .skel {
    width: 100%;
    height: 100%;
    background: var(--color-surface-selected);
  }
  .tool {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-xs);
    color: var(--color-text-secondary);
  }
  .ticon {
    font-size: var(--font-size-2xl);
  }
  .msg {
    padding: var(--space-md);
    color: var(--color-text-secondary);
    font: var(--text-caption);
    text-align: center;
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    padding: var(--space-sm);
    font: var(--text-caption);
  }
  .title {
    font: var(--text-body-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .note {
    color: var(--color-text-secondary);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .chip {
    align-self: flex-start;
    padding: 0 var(--space-xs);
    border: var(--border-width-default) solid var(--color-status-queued);
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
  }
  .hint {
    color: var(--color-status-waiting-for-you);
  }
  .link {
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    flex-direction: column;
    background: var(--color-surface-chrome);
  }
  .stage {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .stage.fit img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  .stage:not(.fit) {
    align-items: flex-start;
    justify-content: flex-start;
  }
  .skel.big {
    width: 60%;
    height: 60%;
  }
  .cap {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    padding: var(--space-sm) var(--space-lg);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-caption);
  }
</style>
