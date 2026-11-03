<script>
  // Settings › Pacing policy (live-cap feature, design-notes.md): the base live cap and the
  // off-hours live cap, the caps they give per Governor state and under memory pressure (read
  // only, recomputed while typing), and the cap in effect now. ↑/↓ step, ⏎ applies, esc leaves
  // the field, then the section; r retries a failed load.
  import { onMount } from 'svelte';
  import { openSingleton, toast } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { checkCap, derivedCaps } from '../lib/pacing.js';
  import { PRESSURE } from '../lib/status.js';
  import Kbd from './Kbd.svelte';

  let view = $state(null); // the last PacingSettings from the core
  let loadError = $state(null);
  let live = $state(''); // the typed text of each field
  let off = $state('');
  let shown = $state(3); // the base the derived block shows: the last valid Live agents value
  let busy = $state(false);
  let saveError = $state(null);
  let saved = $state(false);
  let root = $state(null);
  let savedTimer = null;

  const liveCheck = $derived(view ? checkCap(live, 'live agents', view.live_cap_min, view.live_cap_max) : 0);
  const offCheck = $derived(view ? checkCap(off, 'off-hours live agents', view.off_hours_live_cap_min, view.off_hours_live_cap_max) : 0);
  const liveErr = $derived(typeof liveCheck === 'object' ? liveCheck.error : null);
  const offErr = $derived(typeof offCheck === 'object' ? offCheck.error : null);
  // Changed = a different value, not different text ("03" is still 3).
  const liveChanged = $derived(view != null && live.trim() !== String(view.live_cap) && liveCheck !== view.live_cap);
  const offChanged = $derived(view != null && off.trim() !== String(view.off_hours_live_cap) && offCheck !== view.off_hours_live_cap);
  const canApply = $derived(!busy && (liveChanged || offChanged) && !liveErr && !offErr);
  const atDefaults = $derived(view != null && live.trim() === String(view.live_cap_default) && off.trim() === String(view.off_hours_live_cap_default));
  const caps = $derived(derivedCaps(shown));
  const lowering = $derived(view != null && !liveErr && liveCheck < view.now.live_autonomous);
  const now = $derived(view?.now ?? null);

  function take(v) {
    view = v;
    live = String(v.live_cap);
    off = String(v.off_hours_live_cap);
    shown = v.live_cap;
  }

  async function load() {
    loadError = null;
    try {
      take(await invoke('pacing_settings'));
    } catch (e) {
      loadError = errText(e);
    }
  }

  async function apply() {
    if (!canApply) return;
    busy = true;
    saveError = null;
    saved = false;
    const both = liveChanged && offChanged;
    const n = liveCheck;
    // Only what changed: an absent field is left alone, so an untouched Live agents (under
    // `orchd --cap`, the flag's value) is not stored as the user's choice.
    const args = {};
    if (liveChanged) args.live_cap = liveCheck;
    if (offChanged) args.off_hours_live_cap = offCheck;
    try {
      take(await invoke('pacing_settings_set', args));
      toast(both ? 'Pacing settings saved' : liveChanged ? `Live agents set to ${n}` : `Off-hours live agents set to ${offCheck}`);
      saved = true;
      clearTimeout(savedTimer);
      savedTimer = setTimeout(() => (saved = false), 3000);
    } catch (e) {
      saveError = errText(e);
    } finally {
      busy = false;
    }
  }

  /** Any edit drops the last outcome ('Saved', 'Could not save'). */
  function touch() {
    saved = false;
    saveError = null;
  }

  function reset() {
    live = String(view.live_cap_default);
    off = String(view.off_hours_live_cap_default);
    shown = view.live_cap_default;
    touch();
  }

  function onLiveInput() {
    touch();
    if (typeof liveCheck === 'number') shown = liveCheck;
  }

  /** ↑/↓ step by 1 from the typed number (else the saved one), stopping at the bounds. */
  function step(e, which) {
    const [lo, hi, from] = which === 'live' ? [view.live_cap_min, view.live_cap_max, live] : [view.off_hours_live_cap_min, view.off_hours_live_cap_max, off];
    const cur = /^-?\d+$/.test(from.trim()) ? Number(from) : which === 'live' ? view.live_cap : view.off_hours_live_cap;
    const next = Math.min(hi, Math.max(lo, cur + (e.key === 'ArrowUp' ? 1 : -1)));
    if (which === 'live') {
      live = String(next);
      shown = next;
    } else off = String(next);
    touch();
  }

  function onFieldKey(e, which) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      e.preventDefault();
      step(e, which);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      apply();
    } else if (e.key === 'Escape') {
      // onBodyKey ignores esc inside an input: leave the field, stay in the section.
      e.preventDefault();
      root?.focus();
    }
  }

  function onKey(e) {
    if (e.metaKey || e.ctrlKey || e.altKey || e.target.closest('input, textarea, select')) return;
    if (e.key === 'r' && loadError) {
      e.preventDefault();
      load();
    } else if (e.key === 'p') {
      e.preventDefault();
      openSingleton('planner');
    }
  }

  onMount(() => {
    load();
    return () => clearTimeout(savedTimer);
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<section class="pacing" tabindex="-1" data-section-focus={view ? undefined : ''} bind:this={root} onkeydown={onKey} aria-label="Pacing policy">
  <h2>Pacing policy</h2>
  <p class="muted intro">How many agents may run at once. The <button class="link" onclick={() => openSingleton('planner')}>Week planner</button> edits the plan: days, hours, reserve, allocations.</p>

  {#if loadError}
    <p class="secondary help"><span class="err" aria-hidden="true">{'\u{F0159}'}</span> Could not read the pacing settings: {loadError}</p>
    <div><button class="btn" onclick={load}>Retry <Kbd keys="r" /></button></div>
    <p class="muted hints"><Kbd keys="r" /> retry · <Kbd keys="esc" /> back to sections</p>
  {:else if !view}
    <p class="muted line">Reading the pacing settings…</p>
    {#each [0, 1, 2] as i (i)}<div class="skel" aria-busy="true"></div>{/each}
  {:else}
    {#if view.shadow}
      <p class="note secondary"><span class="muted" aria-hidden="true">{'\u{F02FD}'}</span> Shadow mode is on: the Governor logs the cap without enforcing it. The memory cap still applies.</p>
    {/if}

    <div class="field-block">
      <label for="live-cap" class="lbl">Live agents</label>
      <div class="inline">
        <input id="live-cap" class="num" class:invalid={liveErr} type="text" inputmode="numeric" autocomplete="off" data-field="live" data-section-focus aria-invalid={liveErr ? 'true' : undefined} aria-describedby={liveErr ? 'live-cap-err live-cap-help' : 'live-cap-help'} bind:value={live} oninput={onLiveInput} onkeydown={(e) => onFieldKey(e, 'live')} readonly={busy} />
        <span class="muted cap">{view.live_cap_min}–{view.live_cap_max}</span>
        {#if liveChanged}<span class="was">● was {view.live_cap}</span>{/if}
      </div>
      <p class="muted help" id="live-cap-help">Autonomous agents running at once when the week is on plan.</p>
      {#if liveErr}
        <p class="secondary help" id="live-cap-err"><span class="err" aria-hidden="true">{'\u{F0159}'}</span> {liveErr}</p>
      {:else if typeof liveCheck === 'number' && liveCheck > 6}
        <p class="secondary help"><span class="warn" aria-hidden="true">{'\u{F0026}'}</span> More than 6 agents can use 190–255 MB each and use the week sooner. The Governor still paces the week.</p>
      {/if}
    </div>

    <div class="gives" role="group" class:dim={liveErr} aria-label="What it gives">
      <div class="states">
        {#each Object.entries(PRESSURE) as [k, p], i (k)}
          {#if i}<span class="muted" aria-hidden="true">·</span>{/if}
          <span class="state" class:muted={view.shadow}>
            <span style:color={view.shadow ? null : p.color} aria-hidden="true">{p.icon}</span>
            {p.label} <b class:muted={view.shadow}>{caps[k]}</b>
          </span>
        {/each}
        {#if view.shadow}<span class="muted">· logged only</span>{/if}
      </div>
      <div class="secondary mem">Memory: normal {caps.memory_normal} · warn {caps.memory_warn} · critical {caps.memory_critical}</div>
      <div class="muted foot">Half-weight hours scale the cap down. Leads and the Overwatch have their own slot and don't count.</div>
    </div>

    <div class="field-block">
      <label for="off-cap" class="lbl">Off-hours live agents</label>
      <div class="inline">
        <input id="off-cap" class="num" class:invalid={offErr} type="text" inputmode="numeric" autocomplete="off" data-field="off" aria-invalid={offErr ? 'true' : undefined} aria-describedby={offErr ? 'off-cap-err off-cap-help' : 'off-cap-help'} bind:value={off} oninput={touch} onkeydown={(e) => onFieldKey(e, 'off')} readonly={busy} />
        <span class="muted cap">{view.off_hours_live_cap_min}–{view.off_hours_live_cap_max}</span>
        {#if offChanged}<span class="was">● was {view.off_hours_live_cap}</span>{/if}
      </div>
      <p class="muted help" id="off-cap-help">Agents running outside working hours while the week is a little over plan, never more than Live agents allows.</p>
      {#if offErr}
        <p class="secondary help" id="off-cap-err"><span class="err" aria-hidden="true">{'\u{F0159}'}</span> {offErr}</p>
      {/if}
      {#if !view.off_hours_allowance_pct}
        <p class="secondary help"><span class="muted" aria-hidden="true">{'\u{F02FD}'}</span> The plan has no off-hours allowance, so agents don't run off-hours. Set one in the <button class="link" onclick={() => openSingleton('planner')}>Week planner</button>.</p>
      {/if}
    </div>

    <p class="muted now">
      Now: <span style:color={PRESSURE[now.state]?.color}><span aria-hidden="true">{PRESSURE[now.state]?.icon}</span> {PRESSURE[now.state]?.label ?? now.state}</span>
      · {now.governor_cap} allowed · {now.live_autonomous} live · memory {now.pressure}, {now.process_cap} {now.process_cap === 1 ? 'process' : 'processes'}
    </p>

    {#if lowering}<p class="muted help">Working agents finish their turn; new turns wait for a slot.</p>{/if}
    {#if saveError}<p class="secondary help"><span class="err" aria-hidden="true">{'\u{F0159}'}</span> Could not save: {saveError}</p>{/if}

    <div class="actions">
      <button class="btn primary" disabled={!canApply} onclick={apply}>{busy ? 'Applying…' : 'Apply'} <Kbd keys="⏎" /></button>
      <button class="btn" disabled={busy || atDefaults} onclick={reset}>Reset to defaults</button>
      {#if saved}<span class="done" role="status">Saved</span>{/if}
    </div>
    <p class="muted hints"><Kbd keys="⏎" /> apply · <Kbd keys="↑↓" /> step · <Kbd keys="tab" /> next · <Kbd keys="esc" /> back to sections</p>
  {/if}
</section>

<style>
  .pacing {
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    max-width: 960px;
    outline: none;
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  p {
    margin: 0;
  }
  .intro,
  .help,
  .hints {
    font: var(--text-caption);
  }
  .line {
    font: var(--text-body);
  }
  .field-block {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .lbl {
    font: var(--text-body-strong);
  }
  .inline {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
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
  .num:focus-visible {
    outline: none;
    border: var(--border-width-focus) solid var(--color-focus);
  }
  .num.invalid {
    border-color: var(--color-status-error);
  }
  .cap {
    font: var(--text-caption);
  }
  .was {
    color: var(--color-accent-default);
    font: var(--text-caption);
  }
  .warn {
    color: var(--color-status-waiting-for-you);
  }
  .done {
    color: var(--color-status-done);
  }
  .note {
    padding: var(--space-xs) var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    font: var(--text-caption);
    align-self: flex-start;
  }
  .gives {
    width: 640px;
    max-width: 100%;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    padding: var(--space-md);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .gives.dim {
    opacity: 0.5;
  }
  .states {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
    color: var(--color-text-secondary);
  }
  .states b {
    color: var(--color-text-primary);
    font: var(--text-body-strong);
  }
  .states b.muted {
    color: var(--color-text-muted);
    font-weight: 400;
  }
  .mem {
    font: var(--text-mono-sm);
  }
  .foot {
    font: var(--text-caption);
  }
  .now {
    font: var(--text-caption);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .skel {
    height: 28px;
    max-width: 640px;
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
  }
</style>
