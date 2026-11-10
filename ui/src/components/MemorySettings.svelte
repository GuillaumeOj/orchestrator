<script module>
  const MIN = 256;
  const MAX = 65536;

  /** The message for a threshold the setting can't take, else ''. */
  export function check(v) {
    return Number.isInteger(v) && v >= MIN && v <= MAX ? '' : 'Between 256 and 65 536 MB';
  }
</script>

<script>
  // Settings › Memory (settings-brief §1.8, 04-settings-memory-*; spec §3.1, api-contract §3).
  // Now: the status bar's memory sample as a stacked bar (App / one segment per agent when the
  // sample carries it, else one agents segment / shells) and the pressure level. Warning: the
  // threshold `ui.memory_warn_mb` (default 1600, 256–65 536, validated here); the section warns when
  // the sample's total is above it. Under pressure: read-only text and a link to Pacing policy.
  // "Show in the status bar" and the Renderer card are not built (spec §3.1).
  import { onDestroy, onMount } from 'svelte';
  import { app, openSettings } from '../lib/app.svelte.js';
  import { mb } from '../lib/format.js';
  import { errText, invoke } from '../lib/ipc.js';

  const KEY = 'ui.memory_warn_mb';
  const LEVELS = {
    normal: { label: 'Normal', color: 'var(--color-status-done)' },
    warn: { label: 'Warn', color: 'var(--color-pacing-amber)' },
    critical: { label: 'Critical', color: 'var(--color-status-error)' },
  };

  const m = $derived(app.data?.memory ?? null);
  let warnAt = $state(1600);
  let text = $state('1600');
  let error = $state('');
  let saved = $state(false);
  let timer;
  let pacing = $state(null); // pacing_settings.now: the pressure level and the live cap

  onMount(async () => {
    const [v, p] = await Promise.all([invoke('settings_get', { key: KEY }).catch(() => null), invoke('pacing_settings').catch(() => null)]);
    if (!check(v)) warnAt = v;
    text = String(warnAt);
    pacing = p?.now ?? null;
  });
  onDestroy(() => clearTimeout(timer));

  const above = $derived(!!m && m.total_mb > warnAt);
  const pressure = $derived(LEVELS[pacing?.pressure] ? pacing.pressure : above ? 'warn' : 'normal');
  const level = $derived(LEVELS[pressure]);
  // Segments in order: App, each agent with a sample (else one "agents" segment), shells.
  const parts = $derived.by(() => {
    if (!m) return [];
    const agents = (app.data?.agents ?? []).filter((a) => a.memory_mb > 0);
    const out = [{ key: 'app', label: 'App', mb: m.app_mb, cls: 'app' }];
    if (agents.length) for (const [i, a] of agents.entries()) out.push({ key: a.id, label: a.id, mb: a.memory_mb, cls: `a${i % 4}` });
    else if (m.agents_mb > 0) out.push({ key: 'agents', label: 'Agents', mb: m.agents_mb, cls: 'a0' });
    if (m.shells_mb > 0) out.push({ key: 'shells', label: 'Shells', mb: m.shells_mb, cls: 'sh' });
    return out;
  });
  const scale = $derived(Math.max(m?.system_mb ?? 0, m?.total_mb ?? 1, 1));

  async function commit() {
    const v = Number(text.trim());
    const bad = check(v);
    if (bad) {
      error = bad;
      return;
    }
    error = '';
    if (v === warnAt) return;
    try {
      await invoke('settings_set', { key: KEY, value: v });
      warnAt = v;
      saved = true;
      clearTimeout(timer);
      timer = setTimeout(() => (saved = false), 2000);
    } catch (e) {
      error = `${errText(e)} Still using ${warnAt} MB.`;
      text = String(warnAt);
    }
  }
</script>

<section class="sset" aria-label="Memory settings">
  <h2>Memory</h2>
  <p class="muted lead">What the app and its agents use now, and when it warns.</p>

  <div class="scard" data-group="now">
    {#if m}
      <h3>Now <span class="muted">{mb(m.total_mb)} in use</span></h3>
      <div class="srow">
        <span class="slab">Total</span>
        <div class="sctl bar-wrap">
          <div class="bar" role="img" aria-label="Memory in use: {parts.map((p) => `${p.label} ${mb(p.mb)}`).join(', ')}" class:warn={above}>
            {#each parts as p (p.key)}<span data-seg={p.key} class="mseg {p.cls}" style:width="{(p.mb / scale) * 100}%"></span>{/each}
          </div>
          <div class="legend">
            {#each parts as p (p.key)}<span class="muted">{p.label} {Math.round(p.mb)} MB</span>{/each}
          </div>
        </div>
      </div>
      <div class="srow">
        <span class="slab">Pressure</span>
        <span class="sctl">
          <span class="badge" data-pressure={pressure} style:color={level.color} style:border-color={level.color}><span aria-hidden="true">●</span> {level.label}</span>
          {#if pacing}<span class="muted">live cap {pacing.governor_cap} · {pacing.live_autonomous} live</span>{/if}
        </span>
      </div>
      {#if above}
        <p class="muted shelp wide" role="status" data-warn>The total ({mb(m.total_mb)}) is above the warning ({mb(warnAt)}).</p>
      {/if}
    {:else}
      <h3>Now</h3>
      <p class="muted" data-unavailable>Not available on this system.</p>
    {/if}
  </div>

  <div class="scard" data-group="warning">
    <h3>Warning</h3>
    <div class="srow">
      <label class="slab" for="mem-warn">Warn above</label>
      <span class="sctl">
        <span class="unit">
          <input id="mem-warn" class="field num" type="text" inputmode="numeric" autocomplete="off" bind:value={text} aria-invalid={error ? 'true' : undefined} aria-describedby="mem-warn-help" onblur={commit} onkeydown={(e) => e.key === 'Enter' && commit()} />
          <span class="muted">MB</span>
        </span>
        {#if saved}<span class="sok" role="status">✓ Saved</span>{/if}
      </span>
      {#if error}<p class="serr shelp" role="alert">{error}</p>{/if}
      <p class="shelp" id="mem-warn-help">Compared with the total above: the app, its agents and its shells.</p>
    </div>
  </div>

  <div class="scard" data-group="pressure">
    <h3>Under pressure</h3>
    <p class="muted wide">warn: one live agent less (min. 1) · critical: 1 live agent · back up after 120 s of normal.</p>
    <div class="srow">
      <span class="slab">Live agents cap</span>
      <span class="sctl"><button type="button" class="link" onclick={() => openSettings('pacing')}>Pacing policy › Live agents</button></span>
    </div>
  </div>
</section>

<style>
  .bar-wrap {
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-xs);
  }
  .bar {
    display: flex;
    height: 12px;
    overflow: hidden;
    border-radius: var(--radius-sm);
    background: var(--color-surface-raised);
  }
  .bar.warn {
    outline: var(--border-width-default) solid var(--color-pacing-amber);
  }
  .mseg {
    height: 100%;
  }
  .mseg.app {
    background: var(--color-accent-default);
  }
  .mseg.a0 {
    background: var(--color-role-2, var(--color-status-queued));
  }
  .mseg.a1 {
    background: var(--color-role-3, var(--color-status-working));
  }
  .mseg.a2 {
    background: var(--color-role-4, var(--color-status-done));
  }
  .mseg.a3 {
    background: var(--color-role-5, var(--color-pacing-amber));
  }
  .mseg.sh {
    background: var(--color-text-muted);
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-md);
    font: var(--text-caption);
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    height: 20px;
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid;
    border-radius: var(--radius-full);
    font: var(--text-caption);
  }
  .unit {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .num {
    width: 96px;
    text-align: right;
  }
  .wide {
    margin: 0;
  }
  p.shelp.wide {
    grid-column: 1 / -1;
  }
</style>
