<script>
  // `options` block (notes §3.3, §7 rows 3–4): every question of an
  // AskUserQuestion, "Question 2 of 3 · Auth", options as radios (single) or
  // checkboxes (multi: "Pick any"), each "label — description". `choices` holds
  // the chosen labels per question (Answer ⏎ sends them).
  let { block, choices = $bindable([]), name = 'q' } = $props();
  const questions = $derived(block.questions ?? []);

  function pick(qi, label, multi, on) {
    const next = questions.map((_, i) => [...(choices[i] ?? [])]);
    next[qi] = multi ? (on ? [...next[qi], label] : next[qi].filter((l) => l !== label)) : [label];
    choices = next;
  }
</script>

{#each questions as q, qi (qi)}
  <fieldset class="q">
    <legend class="bh">
      {questions.length > 1 ? `Question ${qi + 1} of ${questions.length}` : 'Question'}{q.multi ? ' · Pick any' : q.header ? ` · ${q.header}` : ''}
    </legend>
    <div class="text"><b>{q.question}</b></div>
    {#each q.options as o (o.label)}
      {@const on = (choices[qi] ?? []).includes(o.label)}
      <label class="opt">
        <input
          class="sr"
          type={q.multi ? 'checkbox' : 'radio'}
          name="{name}-{qi}"
          checked={on}
          onchange={(e) => pick(qi, o.label, q.multi, e.currentTarget.checked)}
        />
        <!-- The board's radiobox / checkbox glyphs; the native input stays for keyboard and a11y. -->
        <span class="glyph" class:on aria-hidden="true">{q.multi ? (on ? '\u{F0132}' : '\u{F0131}') : on ? '\u{F043E}' : '\u{F043D}'}</span>
        <span>{o.label}{#if o.description}<span class="muted"> — {o.description}</span>{/if}</span>
      </label>
    {/each}
  </fieldset>
{/each}

<style>
  .q {
    margin: 0;
    padding: 0;
    border: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .q + .q {
    margin-top: var(--space-md);
  }
  legend {
    padding: 0;
  }
  .opt {
    display: flex;
    align-items: baseline;
    gap: var(--space-sm);
    cursor: pointer;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    opacity: 0;
    pointer-events: none;
  }
  .glyph {
    color: var(--color-text-muted);
  }
  .glyph.on {
    color: var(--color-accent-default);
  }
  .opt:has(.sr:focus-visible) .glyph {
    outline: var(--border-width-focus) solid var(--color-accent-default);
    border-radius: var(--radius-sm);
  }
</style>
