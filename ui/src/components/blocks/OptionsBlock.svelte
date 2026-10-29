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
      <label class="opt">
        <input
          type={q.multi ? 'checkbox' : 'radio'}
          name="{name}-{qi}"
          checked={(choices[qi] ?? []).includes(o.label)}
          onchange={(e) => pick(qi, o.label, q.multi, e.currentTarget.checked)}
        />
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
  .opt input {
    accent-color: var(--color-accent-default);
  }
</style>
