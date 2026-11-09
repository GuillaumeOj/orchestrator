<script>
  // Settings › Pacing › Intake (P4.5 notes p4-5 §7.4, F16): how often Ready to start checks its
  // sources and the default estimates used until there is history. Plain settings keys.
  import { onMount } from 'svelte';
  import { toast } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import Kbd from './Kbd.svelte';
  import Select from './Select.svelte';

  const KEYS = { refresh: 'intake.refresh_secs', team: 'intake.default_team_pp', solo: 'intake.default_solo_pp' };
  const DEFAULTS = { refresh: 600, team: 6, solo: 1.5 };
  /** @type {[number, string][]} */
  const EVERY = [
    [300, '5 min'],
    [600, '10 min'],
    [1800, '30 min'],
    [3600, '1 h'],
  ];
  const RANGE = 'Use a number between 0.5 and 50.';

  let held = $state({ ...DEFAULTS }); // the saved values
  let refresh = $state(String(DEFAULTS.refresh));
  let team = $state(String(DEFAULTS.team));
  let solo = $state(String(DEFAULTS.solo));
  let busy = $state(false);
  let saved = $state(false);
  let failure = $state(null);

  /** A pp amount in 0.5–50, or null. */
  const pp = (t) => {
    const v = Number(String(t).trim().replace(',', '.'));
    return String(t).trim() !== '' && Number.isFinite(v) && v >= 0.5 && v <= 50 ? v : null;
  };
  const teamPp = $derived(pp(team));
  const soloPp = $derived(pp(solo));
  const dirty = $derived(Number(refresh) !== held.refresh || teamPp !== held.team || soloPp !== held.solo);
  const canSave = $derived(!busy && dirty && teamPp != null && soloPp != null);
  const refreshOptions = $derived(
    (EVERY.some(([s]) => s === Number(refresh))
      ? EVERY
      : /** @type {[number, string][]} */ ([...EVERY, [Number(refresh), `${Math.round(Number(refresh) / 60)} min`]]).sort((a, b) => a[0] - b[0])
    ).map(([secs, label]) => ({ value: String(secs), label })),
  );

  async function load() {
    const [r, t, s] = await Promise.all(Object.values(KEYS).map((key) => invoke('settings_get', { key }).catch(() => null)));
    const num = (v, d) => (typeof v === 'number' && Number.isFinite(v) ? v : d);
    held = { refresh: num(r, DEFAULTS.refresh), team: num(t, DEFAULTS.team), solo: num(s, DEFAULTS.solo) };
    refresh = String(held.refresh);
    team = String(held.team);
    solo = String(held.solo);
  }

  async function save() {
    if (!canSave) return;
    busy = true;
    failure = null;
    saved = false;
    const next = { refresh: Number(refresh), team: teamPp, solo: soloPp };
    try {
      for (const k of Object.keys(KEYS)) if (next[k] !== held[k]) await invoke('settings_set', { key: KEYS[k], value: next[k] });
      held = next;
      saved = true;
      toast('Intake settings saved');
    } catch (e) {
      failure = errText(e);
    } finally {
      busy = false;
    }
  }

  function onKey(e) {
    if (e.key === 'Enter' && e.target.closest('input')) {
      e.preventDefault();
      e.stopPropagation();
      save();
    }
  }

  onMount(load);
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section class="ig" aria-label="Intake" onkeydown={onKey} oninput={() => ((saved = false), (failure = null))}>
  <h3>Intake</h3>
  <div class="block">
    <label class="lbl" for="ig-refresh">Refresh every</label>
    <Select id="ig-refresh" width="hug" bind:value={refresh} options={refreshOptions} />
    <p class="muted help">How often Ready to start checks sources. Refresh in the lane checks at once.</p>
  </div>
  <div class="block">
    <label class="lbl" for="ig-team">Default estimate, team run</label>
    <input id="ig-team" class="num" class:invalid={teamPp == null} type="text" inputmode="decimal" autocomplete="off" aria-invalid={teamPp == null ? 'true' : undefined} bind:value={team} />
    <p class="muted help">Used until there is history: 6 pp of the week.</p>
    {#if teamPp == null}<p class="ferr" role="alert">{RANGE}</p>{/if}
  </div>
  <div class="block">
    <label class="lbl" for="ig-solo">Default estimate, solo session</label>
    <input id="ig-solo" class="num" class:invalid={soloPp == null} type="text" inputmode="decimal" autocomplete="off" aria-invalid={soloPp == null ? 'true' : undefined} bind:value={solo} />
    <p class="muted help">1.5 pp of the week.</p>
    {#if soloPp == null}<p class="ferr" role="alert">{RANGE}</p>{/if}
  </div>
  {#if failure}<p class="ferr" role="alert">Could not save: {failure}</p>{/if}
  <div class="actions">
    <button class="btn primary" disabled={!canSave} onclick={save}>{busy ? 'Saving…' : 'Save'} <Kbd keys="⏎" /></button>
    {#if saved}<span class="done" role="status">Saved</span>{/if}
  </div>
</section>

<style>
  .ig {
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    max-width: 960px;
    padding-top: var(--space-md);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
  h3 {
    margin: 0;
    font: var(--text-heading);
  }
  p {
    margin: 0;
  }
  .block {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    align-items: flex-start;
  }
  .lbl {
    font: var(--text-body-strong);
  }
  .help {
    font: var(--text-caption);
  }
  .num {
    width: 72px;
    height: var(--size-control);
    padding: 0 var(--space-sm);
    background: var(--color-surface-chrome);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-sm);
    color: var(--color-text-primary);
    font: var(--text-mono);
  }
  .num.invalid {
    border-color: var(--color-status-error);
  }
  .ferr {
    color: var(--color-status-error);
    font: var(--text-caption);
  }
  .done {
    color: var(--color-status-done);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
</style>
