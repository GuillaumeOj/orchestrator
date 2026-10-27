<script>
  // The weekly plan (FR-802, J0 step 4, J8): working days and hours, the
  // interactive reserve, the off-hours allowance (FR-816), and one weighted
  // allocation per project with an optional hard cap (FR-811). Without a
  // confirmed plan the core proposes an even split. Saving keeps the week's
  // usage (the Governor replays it into the new plan).
  import { onMount } from 'svelte';
  import { refresh, toast } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import Kbd from './Kbd.svelte';

  let { onSaved = () => {}, onCancel = null } = $props();
  const DAYS = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'];
  let form = $state(null);
  let error = $state(null);
  let busy = $state(false);

  onMount(async () => {
    try {
      form = await invoke('plan_form');
    } catch (e) {
      error = errText(e);
    }
  });

  const total = $derived(form ? form.allocations.reduce((s, a) => s + (Number(a.weight) || 0), 0) : 0);

  function toggleDay(d) {
    form.days = form.days.includes(d) ? form.days.filter((x) => x !== d) : DAYS.filter((x) => x === d || form.days.includes(x));
  }

  async function save() {
    if (busy || !form) return;
    busy = true;
    error = null;
    try {
      const f = $state.snapshot(form);
      f.reserve_pct = Number(f.reserve_pct);
      f.off_hours_allowance_pct = Number(f.off_hours_allowance_pct) || 0;
      for (const a of f.allocations) {
        a.weight = Number(a.weight);
        // Empty means no cap.
        a.hard_cap_pct = a.hard_cap_pct === '' || a.hard_cap_pct == null ? null : Number(a.hard_cap_pct);
      }
      await invoke('plan_set', { form: f });
      toast('Weekly plan saved · this week’s usage kept');
      await refresh();
      onSaved();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }
</script>

{#if !form}
  <p class="muted">{error ?? 'Loading the plan…'}</p>
{:else}
  <form class="plan" onsubmit={(e) => (e.preventDefault(), save())}>
    <div class="days" role="group" aria-label="Working days">
      {#each DAYS as d (d)}
        <button type="button" class="filter-chip day" class:on={form.days.includes(d)} aria-pressed={form.days.includes(d)} onclick={() => toggleDay(d)}>{d}</button>
      {/each}
    </div>
    <div class="row">
      <label>From <input class="field mono" bind:value={form.start} size="5" /></label>
      <label>To <input class="field mono" bind:value={form.end} size="5" /></label>
      <label>Reserve for your own sessions (%) <input class="field mono" type="number" min="0" max="50" bind:value={form.reserve_pct} /></label>
      <label title="Share of the week agents may use outside working hours; 0 keeps them stopped at night">Off-hours allowance (%) <input class="field mono" type="number" min="0" max="100" bind:value={form.off_hours_allowance_pct} /></label>
    </div>
    <table>
      <thead><tr><th>Allocation</th><th>Weight</th><th>Share</th><th>Tier</th><th title="Stops this allocation's agents for the week at this % of the week">Hard cap (%)</th></tr></thead>
      <tbody>
        {#each form.allocations as a (a.id)}
          <tr>
            <td>{a.target || a.id}</td>
            <td><input class="field mono" type="number" min="0" step="1" bind:value={a.weight} aria-label="Weight of {a.id}" /></td>
            <td class="muted">{total ? Math.round(((Number(a.weight) || 0) / total) * 100) : 0}%</td>
            <td>
              <select class="field" bind:value={a.tier} aria-label="Tier of {a.id}">
                <option value="critical">critical</option><option value="high">high</option><option value="normal">normal</option><option value="low">low</option>
              </select>
            </td>
            <td><input class="field mono" type="number" min="0" max="100" step="1" placeholder="none" bind:value={a.hard_cap_pct} aria-label="Hard cap of {a.id}" /></td>
          </tr>
        {/each}
      </tbody>
    </table>
    <p class="muted">Weights are relative; a weight of 0 drops the allocation. A hard cap stops the allocation for the week once it used that % of the week. Sessions outside these projects are charged to “other sessions”.</p>
    {#if error}<p class="err">{error}</p>{/if}
    <div class="foot">
      {#if onCancel}<button type="button" class="btn ghost" onclick={onCancel}>Cancel <Kbd keys="esc" /></button>{/if}
      <button type="submit" class="btn primary" disabled={busy || !form.days.length}>{busy ? 'Saving…' : 'Save plan'}</button>
    </div>
  </form>
{/if}

<style>
  .plan {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }
  .days {
    display: flex;
    gap: var(--space-xs);
  }
  .day {
    min-width: 40px;
  }
  .row {
    display: flex;
    gap: var(--space-md);
    flex-wrap: wrap;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    color: var(--color-text-secondary);
  }
  table {
    border-collapse: collapse;
  }
  th {
    text-align: left;
    color: var(--color-text-secondary);
    font-weight: var(--font-weight-regular);
  }
  td,
  th {
    padding: var(--space-xxs) var(--space-sm) var(--space-xxs) 0;
  }
  td input {
    width: 72px;
  }
  p {
    margin: 0;
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-sm);
  }
</style>
