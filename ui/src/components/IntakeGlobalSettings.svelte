<script>
  // Settings › Pacing › Intake (P4.5 notes p4-5 §7.4, F16): the default estimates used until there
  // is history. The refresh interval is per project now (Project settings › Tracker). Plain settings keys.
  import { onMount } from 'svelte';
  import { toast } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { tooltip } from '../lib/tooltip.js';

  const KEYS = { team: 'intake.default_team_pp', solo: 'intake.default_solo_pp' };
  const DEFAULTS = { team: 6, solo: 1.5 };
  const RANGE = 'Use a number between 0.5 and 50.';

  let held = $state({ ...DEFAULTS }); // the saved values
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
  const dirty = $derived(teamPp !== held.team || soloPp !== held.solo);
  const canSave = $derived(!busy && dirty && teamPp != null && soloPp != null);
  /** Any edit drops the last save's outcome. */
  const clearOutcome = () => ((saved = false), (failure = null));

  async function load() {
    const [t, s] = await Promise.all(Object.values(KEYS).map((key) => invoke('settings_get', { key }).catch(() => null)));
    const num = (v, d) => (typeof v === 'number' && Number.isFinite(v) ? v : d);
    held = { team: num(t, DEFAULTS.team), solo: num(s, DEFAULTS.solo) };
    team = String(held.team);
    solo = String(held.solo);
  }

  async function save() {
    if (!canSave) return;
    busy = true;
    failure = null;
    saved = false;
    const next = { team: teamPp, solo: soloPp };
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
<section class="ig" aria-label="Intake" onkeydown={onKey} oninput={clearOutcome}>
  <h3>Intake</h3>
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
    <button class="btn primary" disabled={!canSave} use:tooltip={{ label: 'Save', key: '⏎' }} onclick={save}>{busy ? 'Saving…' : 'Save'}</button>
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
