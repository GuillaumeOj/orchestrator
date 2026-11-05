<script>
  // Settings → Overwatch (P4.4 notes §9.1, api-contract §3.3): on or off, the model,
  // the batch window, the permission time, the idle grace and the rule threshold.
  // Each row saves on change (⏎ or blur) with `set_overwatch_settings`; the core
  // checks the values too, and its refusal shows under the field. The old value
  // stays applied until a save lands.
  import { app, restartOverwatch } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { overwatchOf, statusTip } from '../lib/overwatch.js';

  /** The number rows: key, label, unit, range, help (notes §9.1). */
  const ROWS = [
    { key: 'batch_secs', label: 'Batch window', unit: 's', min: 5, max: 300, help: 'How long it waits to collect non-urgent events before a turn.' },
    { key: 'permission_secs', label: 'Permission time', unit: 's', min: 10, max: 300, help: 'How long a permission can wait for it before it comes to you.' },
    { key: 'idle_secs', label: 'Idle grace', unit: 's', min: 30, max: 1800, help: 'How long it stays awake after its last event before it sleeps.' },
    { key: 'rule_threshold', label: 'Rule threshold', unit: 'times', min: 2, max: 10, help: 'Proposes a rule after you give the same answer this many times.' },
  ];
  const HOLD_SECS = 300; // the permission hold's default; the core is the judge of the real one
  const MODELS = [
    { id: 'haiku', label: 'Haiku' },
    { id: 'sonnet', label: 'Sonnet' },
    { id: 'opus', label: 'Opus' },
  ];

  let s = $state(null);
  const cfg = $derived(/** @type {any} */ (s));
  let loadError = $state('');
  let errors = $state(/** @type {Record<string, string>} */ ({}));
  let note = $state('');
  const ow = $derived(overwatchOf(app.now));
  const unwell = $derived(ow.status === 'down' || ow.status === 'paused');

  $effect(() => {
    invoke('overwatch_settings')
      .then((v) => (s = v))
      .catch((e) => (loadError = errText(e)));
  });

  /** The message for a value outside its row's range, else ''. */
  function check(row, v) {
    if (!Number.isInteger(v)) return 'A whole number.';
    if (v < row.min || v > row.max) return `${row.min}–${row.max} ${row.unit}.`;
    if (row.key === 'permission_secs' && v >= HOLD_SECS) return `Must not exceed the hold length (${HOLD_SECS} s).`;
    return '';
  }

  async function save(key, value, why = '') {
    errors = { ...errors, [key]: '' };
    try {
      s = await invoke('set_overwatch_settings', { settings: { [key]: value } });
      note = why;
    } catch (e) {
      errors = { ...errors, [key]: errText(e) };
    }
  }

  function onNumber(row, e) {
    const v = Number(e.currentTarget.value);
    const bad = check(row, v);
    if (bad) {
      errors = { ...errors, [row.key]: bad };
      return; // the old value stays applied
    }
    save(row.key, v);
  }

  const toggle = (e) =>
    save(
      'enabled',
      e.currentTarget.checked,
      e.currentTarget.checked ? 'On. It wakes on the next event.' : 'Off. Open permissions come to you. Its session ends now.',
    );
</script>

<section class="ow" aria-label="Overwatch settings">
  <div class="h">
    <h2>Overwatch</h2>
    {#if unwell}
      <span class="warn" data-ow-state>{statusTip(ow).replace(/^Overwatch is /, 'Overwatch · ').replace(/\. Cards still work\.$/, '')}</span>
      {#if ow.status === 'down'}<button class="btn" onclick={() => restartOverwatch().catch(() => {})}>Restart</button>{/if}
    {/if}
  </div>
  {#if loadError}
    <p class="err">Could not read the settings: {loadError}</p>
  {:else if !s}
    <p class="muted" role="status">Loading…</p>
  {:else}
    <div class="row" data-row="enabled">
      <label class="lab" for="ow-enabled">Overwatch</label>
      <input id="ow-enabled" type="checkbox" role="switch" aria-checked={cfg.enabled} checked={cfg.enabled} onchange={toggle} />
      <p class="muted help">An agent that settles routine prompts, answers questions from the docs and keeps agents moving. You see a line for everything it does.</p>
      {#if note}<p class="muted help" role="status" data-note>{note}</p>{/if}
      {#if errors.enabled}<p class="err help">{errors.enabled}</p>{/if}
    </div>
    <div class="row" data-row="model">
      <label class="lab" for="ow-model">Model</label>
      <select id="ow-model" class="field" value={cfg.model} onchange={(e) => save('model', e.currentTarget.value)}>
        {#each MODELS as m (m.id)}<option value={m.id}>{m.label}</option>{/each}
        {#if !MODELS.some((m) => m.id === cfg.model)}<option value={cfg.model}>{cfg.model}</option>{/if}
      </select>
      <p class="muted help">The model for its session. Takes effect at the next session.</p>
      {#if errors.model}<p class="err help">{errors.model}</p>{/if}
    </div>
    {#each ROWS as row (row.key)}
      <div class="row" data-row={row.key}>
        <label class="lab" for="ow-{row.key}">{row.label}</label>
        <span class="num">
          <input
            id="ow-{row.key}"
            class="field"
            type="number"
            min={row.min}
            max={row.max}
            value={cfg[row.key]}
            aria-invalid={errors[row.key] ? 'true' : undefined}
            aria-describedby="ow-{row.key}-help"
            onchange={(e) => onNumber(row, e)}
          />
          <span class="muted">{row.unit}</span>
        </span>
        <p class="muted help" id="ow-{row.key}-help">{row.help}</p>
        {#if errors[row.key]}<p class="err help" role="alert">{errors[row.key]}</p>{/if}
      </div>
    {/each}
  {/if}
</section>

<style>
  .ow {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    max-width: 720px;
  }
  .h {
    display: flex;
    align-items: baseline;
    gap: var(--space-md);
    margin-bottom: var(--space-sm);
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  .warn {
    color: var(--color-status-waiting-for-you);
    font: var(--text-caption);
  }
  .row {
    display: grid;
    grid-template-columns: 160px minmax(0, 1fr);
    gap: var(--space-xxs) var(--space-md);
    align-items: center;
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .lab {
    font: var(--text-body-strong);
  }
  .help {
    grid-column: 2;
    margin: 0;
    font: var(--text-caption);
  }
  .num {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .num .field {
    width: 96px;
  }
  .field[aria-invalid='true'] {
    border-color: var(--color-status-error);
  }
</style>
