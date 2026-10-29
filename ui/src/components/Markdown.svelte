<script>
  // Renders Markdown (lib/markdown.js mdast tree) as elements, never {@html}:
  // agent-written text can't inject markup (specs-rendering §3.2, D1). Raw
  // HTML shows as muted text, except <br>. `feature` is for relative images
  // (next: feature_asset); `ondoc(name, anchor)` opens a sibling .md doc,
  // without it such links are plain text.
  import { openUrl } from '../lib/app.svelte.js';
  import { frontPairs, linkKind, prepare } from '../lib/markdown.js';
  import CodeBlock from './CodeBlock.svelte';

  // biome-ignore lint/correctness/noUnusedVariables: images (feature_asset) land next.
  let { text = '', feature = null, ondoc = null } = $props();
  const doc = $derived(prepare(text));
  // Heading and footnote ids are prefixed per instance: two docs on screen don't clash.
  const prefix = `md${Math.random().toString(36).slice(2, 8)}-`;
  let root;

  const BR = /^<br\s*\/?>$/i;
  const ALERT_TITLE = { note: 'Note', tip: 'Tip', important: 'Important', warning: 'Warning', caution: 'Caution' };

  function scrollTo(id) {
    if (!id) return;
    const el = document.getElementById(prefix + id) ?? document.getElementById(prefix + String(id).toLowerCase());
    if (el && root?.contains(el)) el.scrollIntoView({ block: 'start', behavior: 'smooth' });
  }

  function follow(e, k) {
    e.preventDefault();
    if (k.kind === 'external') openUrl(k.url);
    else if (k.kind === 'anchor') scrollTo(k.anchor);
    else if (k.kind === 'doc') openDoc(k);
  }

  async function openDoc(k) {
    await ondoc?.(k.name, k.anchor);
    if (!k.anchor) return;
    // The view switches the text on this same instance: scroll once it rendered.
    await Promise.resolve();
    requestAnimationFrame(() => scrollTo(k.anchor));
  }

  function jump(e, id) {
    e.preventDefault();
    scrollTo(id);
  }

  // A remote image inside a link (`[![CI](badge.svg)](…/actions)`) is the link's
  // content: let the click reach the link instead of also opening the image.
  function openChip(e, url) {
    if (!e.currentTarget.closest('a')) openUrl(url);
  }

  const imageUrl = (n) => (n.type === 'imageReference' ? (doc.defs.get(n.identifier)?.url ?? '') : n.url);
</script>

{#snippet blocks(list, tight = false)}
  {#each list as n, i (i)}{@render block(n, tight)}{/each}
{/snippet}

{#snippet block(n, tight)}
  {#if n.type === 'paragraph'}
    {#if tight}{@render inlines(n.children)}{:else}<p>{@render inlines(n.children)}</p>{/if}
  {:else if n.type === 'heading'}
    <svelte:element this={`h${n.depth}`} id={prefix + n.data.slug}>{@render inlines(n.children)}</svelte:element>
  {:else if n.type === 'thematicBreak'}
    <hr />
  {:else if n.type === 'blockquote'}
    <blockquote class={n.data?.alert ? `alert ${n.data.alert}` : undefined}>
      {#if n.data?.alert}<p class="alert-title">{ALERT_TITLE[n.data.alert]}</p>{/if}
      {@render blocks(n.children)}
    </blockquote>
  {:else if n.type === 'list'}
    {@const loose = n.spread || n.children.some((li) => li.spread)}
    <svelte:element
      this={n.ordered ? 'ol' : 'ul'}
      start={n.ordered && n.start != null && n.start !== 1 ? n.start : undefined}
      class:tasks={n.children.some((li) => li.checked != null)}
    >
      {#each n.children as li, j (j)}
        {#if li.checked != null}
          <li class="task">
            <input type="checkbox" checked={li.checked} disabled aria-label={li.checked ? 'done' : 'to do'} />
            <div>{@render blocks(li.children, !loose)}</div>
          </li>
        {:else}
          <li>{@render blocks(li.children, !loose)}</li>
        {/if}
      {/each}
    </svelte:element>
  {:else if n.type === 'code'}
    <!-- `mermaid` fences render as code until the diagram renderer lands. -->
    <CodeBlock code={n.value} lang={n.lang} />
  {:else if n.type === 'html'}
    {#if BR.test(n.value.trim())}<br />{:else}<div class="raw">{n.value}</div>{/if}
  {:else if n.type === 'table'}
    <div class="table">
      <table>
        <thead>
          <tr>
            {#each n.children[0]?.children ?? [] as c, j (j)}<th style:text-align={n.align?.[j] ?? undefined}>{@render inlines(c.children)}</th>{/each}
          </tr>
        </thead>
        {#if n.children.length > 1}
          <tbody>
            {#each n.children.slice(1) as row, r (r)}
              <tr>
                <!-- As GitHub: every row has the header's cells (short rows padded, extra cells dropped). -->
                {#each n.children[0].children as _, j (j)}<td style:text-align={n.align?.[j] ?? undefined}>{@render inlines(row.children[j]?.children ?? [])}</td>{/each}
              </tr>
            {/each}
          </tbody>
        {/if}
      </table>
    </div>
  {:else if n.type === 'yaml'}
    {@const pairs = frontPairs(n.value)}
    {#if pairs}
      <dl class="front">
        {#each pairs as [k, v], j (j)}<dt>{k}</dt><dd>{v}</dd>{/each}
      </dl>
    {:else}
      <CodeBlock code={n.value} lang="yaml" />
    {/if}
  {:else if n.type === 'definition' || n.type === 'footnoteDefinition'}
    <!-- Definitions resolve links; footnotes render at the end. -->
  {:else if n.children}
    {@render blocks(n.children, tight)}
  {:else if n.value}
    <p>{n.value}</p>
  {/if}
{/snippet}

{#snippet inlines(list)}{#each list as n, i (i)}{@render inline(n)}{/each}{/snippet}

{#snippet inline(n)}
  {#if n.type === 'text'}{n.value}{:else if n.type === 'strong'}<strong>{@render inlines(n.children)}</strong>{:else if n.type === 'emphasis'}<em>{@render inlines(n.children)}</em>{:else if n.type === 'delete'}<del>{@render inlines(n.children)}</del>{:else if n.type === 'inlineCode'}<code>{n.value}</code>{:else if n.type === 'break'}<br />{:else if n.type === 'html'}{#if BR.test(n.value.trim())}<br />{:else}<code class="raw">{n.value}</code>{/if}{:else if n.type === 'link'}{@render link(n.url, n.title, n.children)}{:else if n.type === 'linkReference'}{@const d = doc.defs.get(n.identifier)}{#if d}{@render link(d.url, d.title, n.children)}{:else}{@render inlines(n.children)}{/if}{:else if n.type === 'image' || n.type === 'imageReference'}{@render image(imageUrl(n), n.alt ?? '')}{:else if n.type === 'footnoteReference'}{#if n.data?.num}<sup class="fnref"><a href="#{prefix}fn:{n.data.num}" id="{prefix}{n.data.refId}" onclick={(e) => jump(e, `fn:${n.data.num}`)}>{n.data.num}</a></sup>{:else}[^{n.label ?? n.identifier}]{/if}{:else if n.children}{@render inlines(n.children)}{:else if n.value}{n.value}{/if}
{/snippet}

{#snippet link(url, title, children)}
  {@const k = linkKind(url, !!ondoc)}
  {#if k.kind === 'external'}<a class="ext" href={k.url} title={title ?? undefined} onclick={(e) => follow(e, k)}>{@render inlines(children)}<span class="arrow" aria-hidden="true">↗</span></a>{:else if k.kind === 'anchor' || k.kind === 'doc'}<a href={url} title={title ?? undefined} onclick={(e) => follow(e, k)}>{@render inlines(children)}</a>{:else if k.kind === 'repo'}<span class="repo" title={k.path}>{@render inlines(children)}</span>{:else}<span title={title ?? undefined}>{@render inlines(children)}</span>{/if}
{/snippet}

<!-- Images (feature_asset, the large preview) land with the next task: for now the
     remote chip (D2: never fetched) and the "not available" line. -->
{#snippet image(url, alt)}
  {#if /^https?:\/\//i.test(url)}<button class="chip" title={url} onclick={(e) => openChip(e, url)}>🖼 {alt || 'image'} ↗</button>{:else if !/^data:/i.test(url)}<span class="imgerr">image: {alt || url} (not available here)</span>{/if}
{/snippet}

<div class="md" bind:this={root}>
  {@render blocks(doc.root.children)}{#if doc.footnotes.length}
    <section class="footnotes" aria-label="Footnotes">
      <ol>
        {#each doc.footnotes as f (f.num)}
          <li id="{prefix}fn:{f.num}">
            <!-- One paragraph stays inline with ↩; several keep their <p>s, else they run together. -->
            {@render blocks(f.node.children, f.node.children.length === 1)}
            <a class="back" href="#{prefix}fnref:{f.num}" aria-label="Back to the reference" onclick={(e) => jump(e, `fnref:${f.num}`)}>↩</a>
          </li>
        {/each}
      </ol>
    </section>
  {/if}
</div>

<style>
  .md {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    min-width: 0;
    font: var(--text-body);
    color: var(--color-text-secondary);
    user-select: text;
    -webkit-user-select: text;
    overflow-wrap: anywhere;
  }
  h1,
  h2,
  h3,
  h4,
  h5,
  h6 {
    margin: var(--space-xs) 0 0;
    color: var(--color-text-primary);
  }
  h1,
  h2 {
    margin-top: var(--space-md);
    padding-bottom: var(--space-xxs);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  h1:first-child,
  h2:first-child {
    margin-top: 0;
  }
  h1 {
    font: var(--text-heading);
  }
  h2 {
    font: var(--text-title);
  }
  h3,
  h4,
  h5,
  h6 {
    font: var(--text-body-strong);
  }
  p {
    margin: 0;
  }
  a {
    color: var(--color-accent-default);
    text-decoration: none;
  }
  a:hover {
    text-decoration: underline;
  }
  .arrow {
    margin-left: 0.15em;
    font-size: 0.8em;
  }
  .repo {
    color: var(--color-text-muted);
    text-decoration: underline dotted;
    cursor: help;
  }
  strong,
  em {
    color: var(--color-text-primary);
  }
  del {
    color: var(--color-text-muted);
  }
  code {
    padding: 0 var(--space-xxs);
    font: var(--text-mono-sm);
    color: var(--color-text-primary);
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
    overflow-wrap: normal;
  }
  .raw {
    font: var(--text-mono-sm);
    color: var(--color-text-muted);
    white-space: pre-wrap;
  }
  code.raw {
    background: none;
    padding: 0;
  }
  blockquote {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    margin: 0;
    padding-left: var(--space-md);
    color: var(--color-text-muted);
    border-left: var(--border-width-focus) solid var(--color-border-strong);
  }
  .alert-title {
    font: var(--text-body-strong);
  }
  .alert.note {
    border-left-color: var(--color-accent-default);
  }
  .alert.tip {
    border-left-color: var(--color-status-done);
  }
  .alert.important {
    border-left-color: var(--color-status-at-gate);
  }
  .alert.warning {
    border-left-color: var(--color-status-waiting-for-you);
  }
  .alert.caution {
    border-left-color: var(--color-status-error);
  }
  .alert.note .alert-title {
    color: var(--color-accent-default);
  }
  .alert.tip .alert-title {
    color: var(--color-status-done);
  }
  .alert.important .alert-title {
    color: var(--color-status-at-gate);
  }
  .alert.warning .alert-title {
    color: var(--color-status-waiting-for-you);
  }
  .alert.caution .alert-title {
    color: var(--color-status-error);
  }
  ul,
  ol {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    margin: 0;
    padding-left: var(--space-xl);
  }
  ul {
    list-style: disc;
  }
  :global(:is(ul, ol)) ul {
    list-style: circle;
  }
  :global(:is(ul, ol) :is(ul, ol)) ul {
    list-style: square;
  }
  li > :global(:is(ul, ol)) {
    margin-top: var(--space-xxs);
  }
  /* Task items are flex (no marker); a plain item in the same list keeps its bullet. */
  ul.tasks {
    padding-left: var(--space-sm);
    list-style-position: inside;
  }
  li.task {
    display: flex;
    align-items: baseline;
    gap: var(--space-xs);
  }
  li.task > div {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    min-width: 0;
  }
  li.task input {
    margin: 0;
    opacity: 1;
    accent-color: var(--color-accent-default);
  }
  hr {
    width: 100%;
    margin: 0;
    border: 0;
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
  .table {
    max-width: 100%;
    overflow-x: auto;
  }
  table {
    border-collapse: collapse;
    overflow-wrap: normal;
  }
  th,
  td {
    padding: var(--space-xs) var(--space-sm);
    text-align: left;
    vertical-align: top;
    border: var(--border-width-default) solid var(--color-border-subtle);
  }
  th {
    font: var(--text-body-strong);
    color: var(--color-text-primary);
    background: var(--color-surface-raised);
  }
  tbody tr:nth-child(even) {
    background: var(--color-surface-chrome);
  }
  td code,
  th code {
    white-space: nowrap;
  }
  .front {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-xxs) var(--space-md);
    margin: 0;
    font: var(--text-caption);
  }
  .front dt {
    color: var(--color-text-muted);
  }
  .front dd {
    margin: 0;
    color: var(--color-text-secondary);
  }
  sup.fnref {
    font-size: 0.75em;
    line-height: 0;
  }
  .footnotes {
    padding-top: var(--space-sm);
    font: var(--text-caption);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
  .footnotes ol {
    gap: var(--space-xxs);
  }
  .back {
    margin-left: var(--space-xxs);
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
</style>
