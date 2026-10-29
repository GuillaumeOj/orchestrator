<script>
  // A fenced code block in a Markdown doc (specs-rendering §3.3): syntax
  // colors from lib/highlight.js runs (elements, never HTML), the language
  // label and a Copy button that copies the raw text.
  import { toast } from '../lib/app.svelte.js';
  import { errText } from '../lib/ipc.js';
  import { highlighter, loadHighlight } from '../lib/markdown.js';

  let { code = '', lang = null } = $props();
  // Plain text until lib/highlight.js has loaded (once, on the first block with a language).
  let hl = $state(highlighter());
  $effect(() => {
    if (lang && !hl)
      loadHighlight().then(
        (m) => (hl = m),
        () => {}, // the chunk didn't load: the block stays plain
      );
  });
  const runs = $derived(hl ? hl.highlight(code, lang) : null);

  async function copy() {
    try {
      await navigator.clipboard.writeText(code);
      toast('Copied');
    } catch (e) {
      toast(`Could not copy: ${errText(e)}`, { kind: 'error' });
    }
  }
</script>

<div class="code">
  <div class="bar">
    {#if lang}<span class="lang">{lang}</span>{/if}
    <button class="copy" onclick={copy}>Copy</button>
  </div>
  <!-- One line: whitespace inside <pre> is part of the code. -->
  <pre><code>{#if runs}{#each runs as r, i (i)}{#if r.t}<span class="hl-{r.t}">{r.v}</span>{:else}{r.v}{/if}{/each}{:else}{code}{/if}</code></pre>
</div>

<style>
  .code {
    position: relative;
  }
  .bar {
    position: absolute;
    top: var(--space-xxs);
    right: var(--space-xs);
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    font: var(--text-caption);
    color: var(--color-text-muted);
  }
  .copy {
    opacity: 0;
    padding: 0 var(--space-xs);
    color: var(--color-text-secondary);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }
  .code:hover .copy,
  .copy:focus-visible {
    opacity: 1;
  }
  pre {
    margin: 0;
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-chrome);
    border-radius: var(--radius-sm);
    white-space: pre;
    overflow-x: auto;
  }
  code {
    font: var(--text-mono-sm);
    color: var(--color-text-primary);
  }
  .hl-keyword {
    color: var(--color-syntax-keyword);
  }
  .hl-string {
    color: var(--color-syntax-string);
  }
  .hl-number {
    color: var(--color-syntax-number);
  }
  .hl-comment {
    color: var(--color-syntax-comment);
    font-style: italic;
  }
  .hl-function {
    color: var(--color-syntax-function);
  }
  .hl-type {
    color: var(--color-syntax-type);
  }
  .hl-variable {
    color: var(--color-syntax-variable);
  }
  .hl-constant {
    color: var(--color-syntax-constant);
  }
  .hl-attr {
    color: var(--color-syntax-attr);
  }
  .hl-tag {
    color: var(--color-syntax-tag);
  }
  .hl-meta {
    color: var(--color-syntax-meta);
  }
  .hl-addition {
    color: var(--color-syntax-addition);
    background: var(--color-syntax-addition-bg);
  }
  .hl-deletion {
    color: var(--color-syntax-deletion);
    background: var(--color-syntax-deletion-bg);
  }
  .hl-punctuation {
    color: var(--color-syntax-punctuation);
  }
</style>
