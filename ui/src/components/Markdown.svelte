<script>
  // Renders feature docs from lib/markdown.js blocks as elements (never
  // {@html}): agent-written text can't inject markup.
  import { parse } from '../lib/markdown.js';

  let { text = '' } = $props();
  const blocks = $derived(parse(text));
</script>

{#snippet spans(list)}
  {#each list as s, i (i)}
    {#if s.t === 'b'}<b>{s.s}</b>{:else if s.t === 'i'}<i>{s.s}</i>{:else if s.t === 'code'}<code>{s.s}</code>{:else}{s.s}{/if}
  {/each}
{/snippet}

<div class="md">
  {#each blocks as b, i (i)}
    {#if b.type === 'h'}
      <div class="h h{b.level}" role="heading" aria-level={b.level}>{@render spans(b.spans)}</div>
    {:else if b.type === 'p'}
      <p>{@render spans(b.spans)}</p>
    {:else if b.type === 'quote'}
      <blockquote>{@render spans(b.spans)}</blockquote>
    {:else if b.type === 'code'}
      <pre class="mono">{b.text}</pre>
    {:else if b.type === 'hr'}
      <hr />
    {:else}
      <svelte:element this={b.type} class:tasks={b.items.some((it) => it.checked !== null)}>
        {#each b.items as it, j (j)}
          <li>{#if it.checked !== null}<span class="box" aria-hidden="true">{it.checked ? '☑' : '☐'}</span> {/if}{@render spans(it.spans)}</li>
        {/each}
      </svelte:element>
    {/if}
  {/each}
</div>

<style>
  .md {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    font: var(--text-body);
    color: var(--color-text-secondary);
    user-select: text;
    -webkit-user-select: text;
  }
  .h {
    color: var(--color-text-primary);
    margin-top: var(--space-xs);
  }
  .h1 {
    font: var(--text-heading);
  }
  .h2 {
    font: var(--text-title);
  }
  .h3,
  .h4,
  .h5,
  .h6 {
    font: var(--text-body-strong);
  }
  p,
  blockquote,
  pre {
    margin: 0;
  }
  blockquote {
    padding-left: var(--space-md);
    border-left: var(--border-width-focus) solid var(--color-border-strong);
  }
  pre {
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-chrome);
    border-radius: var(--radius-sm);
    white-space: pre-wrap;
    overflow: auto;
  }
  code {
    font: var(--text-mono-sm);
    color: var(--color-text-primary);
  }
  ul,
  ol {
    margin: 0;
    padding-left: var(--space-xl);
  }
  ul.tasks {
    list-style: none;
    padding-left: var(--space-sm);
  }
  hr {
    border: 0;
    border-top: var(--border-width-default) solid var(--color-border-subtle);
    width: 100%;
  }
  b,
  i {
    color: var(--color-text-primary);
  }
</style>
