<script>
  // Settings → Overwatch (P4.4 notes §9.1, api-contract §3.3): on or off, the model,
  // the batch window, the permission time, the idle grace and the rule threshold.
  // Each row saves on change (⏎ or blur) with `set_overwatch_settings`; the core
  // checks the values too, and its refusal shows under the field. The old value
  // stays applied until a save lands, and the message says which.
  import { app, restartOverwatch } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { overwatchOf, sectionStatus } from '../lib/overwatch.js';

  /** The number rows: key, label, unit, range, help (notes §9.1). */
  const ROWS = [
    { key: 'batch_secs', label: 'Batch window', unit: 's', min: 5, max: 300, help: 'How long it waits to collect non-urgent events before a turn.' },
    { key: 'permission_secs', label: 'Permission time', unit: 's', min: 10, max: 300, help: 'How long a permission can wait for it before it comes to you.' },
    { key: 'idle_secs', label: 'Idle grace', unit: 's', min: 30, max: 1800, help: 'How long it stays awake after its last event before it sleeps.' },
    { key: 'rule_threshold', label: 'Rule threshold', unit: 'times', min: 2, max: 10, help: 'Proposes a rule after you give the same answer this many times.' },
  ];
  const HOLD_DEFAULT = 300; // used when the setting can't be read; the core is the judge either way
  const MODELS = [
    { id: 'haiku', label: 'Haiku' },
    { id: 'sonnet', label: 'Sonnet' },
    { id: 'opus', label: 'Opus' },
  ];

  /** @type {{ enabled: boolean, model: string, batch_secs: number, permission_secs: number, idle_secs: number, rule_threshold: number } | null} */
  let s = $state(null);
  let hold = $state(HOLD_DEFAULT); // `permissions.hold_secs`: Permission time stays strictly below it
  let loadError = $state('');
  let errors = $state(/** @type {Record<string, string>} */ ({}));
  let note = $state('');
  let restarting = $state(false);
  const ow = $derived(overwatchOf(app.now));
  const status = $derived(sectionStatus(ow));

  $effect(() => {
    invoke('overwatch_settings')
      .then((v) => (s = v))
      .catch((e) => (loadError = errText(e)));
    invoke('settings_get', { key: 'permissions.hold_secs' })
      .then((v) => {
        if (Number.isInteger(v) && v > 0) hold = v;
      })
      .catch(() => {});
  });
  $effect(() => {
    if (ow.status !== 'down') restarting = false;
  });

  const rangeText = (row) => `${row.min}–${row.max} ${row.unit}`;
  /** The message for a value the row can't take, else ''. The old value stays: "Still using 60 s." */
  function check(row, v) {
    const still = ` Still using ${s?.[row.key]} ${row.unit}.`;
    if (!Number.isInteger(v)) return `A whole number.${still}`;
    if (v < row.min || v > row.max) return `${rangeText(row)}.${still}`;
    if (row.key === 'permission_secs' && v >= hold) return `Must be below the hold length (${hold} s).${still}`;
    return '';
  }

  async function save(key, value, why = '') {
    errors = { ...errors, [key]: '' };
    try {
      s = await invoke('set_overwatch_settings', { settings: { [key]: value } });
      note = why;
      return true;
    } catch (e) {
      const row = ROWS.find((r) => r.key === key);
      errors = { ...errors, [key]: `${errText(e)}${row ? ` Still using ${s?.[key]} ${row.unit}.` : ''}` };
      return false;
    }
  }

  // A refused save puts the control back on the value that is still applied.
  async function onNumber(row, e) {
    const box = e.currentTarget;
    const v = Number(box.value);
    const bad = check(row, v);
    if (bad) {
      errors = { ...errors, [row.key]: bad };
      box.value = String(s?.[row.key]);
      return;
    }
    if (!(await save(row.key, v))) box.value = String(s?.[row.key]);
  }

  async function toggle(e) {
    const box = e.currentTarget;
    const on = box.checked;
    const ok = await save('enabled', on, on ? 'On. It wakes on the next event.' : 'Off. Open permissions come to you. Its session ends now.');
    if (!ok) box.checked = !on;
  }

  async function pickModel(e) {
    const box = e.currentTarget;
    const before = s?.model;
    if (!(await save('model', box.value)) && before) box.value = before;
  }

  async function restart() {
    restarting = true;
    try {
      await restartOverwatch();
    } catch {
      restarting = false;
    }
  }
</script>

<section class="ow" aria-label="Overwatch settings">
  <div class="h">
    <div>
      <h2>Overwatch</h2>
      <p class="muted sub">An agent that settles routine prompts for you.</p>
    </div>
    {#if status}
      <span class="state" data-ow-state><span class="dot {ow.status}" aria-hidden="true"></span> {status}</span>
      {#if ow.status === 'down'}<button class="btn" disabled={restarting} onclick={restart}>{restarting ? 'Restarting…' : 'Restart'}</button>{/if}
    {/if}
  </div>
  {#if loadError}
    <p class="err">Could not read the settings: {loadError}</p>
  {:else if !s}
    <p class="muted" role="status">Loading…</p>
  {:else}
    <div class="row" data-row="enabled">
      <div class="what">
        <label class="lab" for="ow-enabled">Overwatch</label>
        <p class="muted help">An agent that settles routine prompts, answers questions from the docs and keeps agents moving. You see a line for everything it does.</p>
        {#if note}<p class="muted help" role="status" data-note>{note}</p>{/if}
        {#if errors.enabled}<p class="err help">{errors.enabled}</p>{/if}
      </div>
      <input id="ow-enabled" type="checkbox" role="switch" aria-checked={s.enabled} checked={s.enabled} onchange={toggle} />
    </div>
    <div class="row" data-row="model">
      <div class="what">
        <label class="lab" for="ow-model">Model</label>
        <p class="muted help">The model for its session. Takes effect at the next session.</p>
        {#if errors.model}<p class="err help">{errors.model}</p>{/if}
      </div>
      <select id="ow-model" class="field" value={s.model} onchange={pickModel}>
        {#each MODELS as m (m.id)}<option value={m.id}>{m.label}</option>{/each}
        {#if !MODELS.some((m) => m.id === s?.model)}<option value={s.model}>{s.model}</option>{/if}
      </select>
    </div>
    {#each ROWS as row (row.key)}
      <div class="row" data-row={row.key}>
        <div class="what">
          <label class="lab" for="ow-{row.key}">{row.label}</label>
          <p class="muted help" id="ow-{row.key}-help">{row.help} ({rangeText(row)})</p>
          {#if errors[row.key]}<p class="err help" role="alert">{errors[row.key]}</p>{/if}
        </div>
        <span class="num">
          <input
            id="ow-{row.key}"
            class="field"
            type="number"
            min={row.min}
            max={row.max}
            value={s[row.key]}
            aria-invalid={errors[row.key] ? 'true' : undefined}
            aria-describedby="ow-{row.key}-help"
            onchange={(e) => onNumber(row, e)}
          />
          <span class="muted">{row.unit}</span>
        </span>
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
    align-items: center;
    gap: var(--space-md);
    margin-bottom: var(--space-sm);
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  .sub {
    margin: 0;
  }
  .state {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    margin-left: auto;
    color: var(--color-status-waiting-for-you);
    font: var(--text-caption);
  }
  .dot {
    box-sizing: border-box;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 1.5px solid var(--color-status-waiting-for-you);
  }
  .dot.down {
    background: var(--color-status-waiting-for-you);
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-lg);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .what {
    min-width: 0;
  }
  .lab {
    font: var(--text-body-strong);
  }
  .help {
    margin: 0;
    font: var(--text-caption);
  }
  .num {
    display: flex;
    flex: none;
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
