<script>
  // Project settings › Allocation (FR-109, FR-802): this project's share of the
  // weekly plan — weight, tier, hard cap — against the rest of the plan, which
  // is read only here. Nothing is saved before Save (⌘⏎); esc discards.
  // Remove is a weight of 0 until Save. Single keys act only when no field has
  // focus (the pane forwards its key events to onKey).
  import { onMount, tick } from 'svelte';
  import {
    addDraft,
    capBar,
    changedFields,
    draftOf,
    isDirty,
    keysFor,
    num,
    ownShare,
    pendingRemoval,
    pp,
    savedToast,
    TIERS,
    tableRows,
    toForm,
    validate,
  } from '../lib/allocation.js';
  import { app, toast } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import Kbd from './Kbd.svelte';

  let { project, name } = $props();
  let view = $state(null); // ProjectAllocationView
  let draft = $state(null); // { weight, tier, cap } text fields; null = nothing being edited
  let lastWeight = '1'; // what "Keep in the plan" restores
  let loadError = $state(null);
  let saveError = $state(null);
  let busy = $state(false);
  let root = $state(null);

  const confirmed = $derived(!!view?.plan_confirmed);
  const errors = $derived(draft ? validate(draft) : {});
  const invalid = $derived(Object.keys(errors).length > 0);
  const pending = $derived(confirmed && pendingRemoval(draft));
  const dirty = $derived(confirmed && !!view && isDirty(view, draft));
  const changed = $derived(view && draft ? changedFields(view, draft) : {});
  const rows = $derived(view ? tableRows(view, draft, name) : []);
  const mine = $derived(ownShare(view, draft, name));
  const bar = $derived(view?.allocation && confirmed ? capBar(view.allocation.used_pct, num(draft?.cap ?? ''), name) : null);
  const keys = $derived(keysFor(view, draft, dirty && !invalid, loadError && !view));

  function take(v) {
    view = v;
    draft = v.plan_confirmed && v.allocation ? draftOf(v.allocation) : null;
    lastWeight = v.allocation ? String(v.allocation.weight) : String(v.proposed_weight ?? 1);
  }

  async function load() {
    loadError = null;
    try {
      take(await invoke('project_allocation', { project }));
    } catch (e) {
      loadError = errText(e);
    }
  }

  async function save() {
    if (busy || !dirty || invalid) return;
    busy = true;
    saveError = null;
    const form = toForm(draft);
    try {
      take(await invoke('project_allocation_set', { project, form }));
      toast(savedToast(name, form));
    } catch (e) {
      saveError = errText(e); // the form keeps the edits
    } finally {
      busy = false;
    }
  }

  async function add() {
    draft = addDraft(view);
    await tick();
    const w = root?.querySelector('[data-field="weight"]');
    w?.focus();
    w?.select();
  }

  function remove() {
    if (!view.allocation) draft = null; // an unsaved Add: nothing pending
    else draft = { ...draft, weight: '0' };
  }

  function keep() {
    draft = { ...draft, weight: lastWeight };
  }

  function cancel() {
    saveError = null;
    if (!view.allocation) draft = null;
    else if (pending) keep();
    else draft = draftOf(view.allocation);
  }

  function setWeight(text) {
    draft = { ...draft, weight: text };
    if (num(text) > 0) lastWeight = text;
  }

  const planner = () => (app.overlay = { kind: 'plan' });

  /** Called by the pane for every key event while this section is shown. */
  export function onKey(e) {
    if (e.metaKey && e.key === 'Enter') return (e.preventDefault(), save());
    const typing = e.target.closest('input, textarea, select');
    if (e.key === 'Escape' && typing && (dirty || draft)) {
      // In a field esc first blurs it; the next one cancels.
      typing.blur();
      return e.preventDefault();
    }
    if (typing || e.metaKey || e.ctrlKey || e.altKey) return;
    const k = e.key;
    if ((k === 'Enter' || k === ' ') && e.target.closest('button')) return;
    if (loadError && !view) {
      if (k === 'r') load();
      else return;
    } else if (!view) return;
    else if (k === 'Escape' && (dirty || (!view.allocation && draft))) cancel();
    else if (!confirmed) {
      if (k === 'g') planner();
      else return;
    } else if (k === 'a' && !view.allocation && !draft) add();
    else if (k === 'x' && draft && !pending) remove();
    else if (k === 'g' && !dirty) planner();
    else return;
    e.preventDefault();
  }

  onMount(load);
</script>

<div class="alloc" bind:this={root}>
  <h2>Allocation</h2>
  <p class="muted lead">{name}'s share of the week. Agents of {name} are charged to it; the rest of the plan is in the Week planner.</p>

  {#if loadError && !view}
    <p class="box errbox" role="alert">
      <span>✕ Couldn't load {name}'s allocation: {loadError}</span><span class="grow"></span>
      <button class="btn ghost" onclick={load}>Retry <Kbd keys="r" /></button>
    </p>
  {:else if !view}
    <div class="skel" aria-busy="true"><i></i><i class="short"></i><i></i></div>
    <h3>Rest of the plan</h3>
    <div class="skel wide" aria-busy="true"><i></i><i></i><i></i><i></i></div>
    <div class="foot"><button class="btn primary" disabled>Save <Kbd keys="⌘+⏎" /></button><button class="btn ghost" disabled>Cancel <Kbd keys="esc" /></button></div>
  {:else}
    {#if !confirmed}
      <p class="box info">ⓘ No weekly plan yet. The core proposes an even split; nothing is applied until you set it up.</p>
      <h3>Proposed split <span class="muted">even · read only · nothing applied</span></h3>
    {:else if !view.allocation && !draft}
      <p class="box info">ⓘ {name} isn't in this week's plan. Its agents are charged to other sessions.</p>
      <div><button class="btn add" onclick={add}>Add to the weekly plan <Kbd keys="a" /></button></div>
    {:else}
      <div class="line">
        <label class="lab" for="alloc-weight">Weight</label>
        <input id="alloc-weight" data-field="weight" class="field mono num" class:bad={errors.weight} type="number" min="0" step="1" value={draft.weight} oninput={(e) => setWeight(e.currentTarget.value)} aria-invalid={!!errors.weight} aria-describedby={errors.weight ? 'alloc-weight-err' : undefined} />
        {#if changed.weight}<span class="chg">• changed</span>{/if}
        {#if mine != null}<span class="muted" class:strike={pending}>{mine}% of the plan</span>{:else if !errors.weight}<span class="muted strike">0% of the plan</span>{/if}
      </div>
      {#if errors.weight}<p class="ferr" id="alloc-weight-err">{errors.weight}</p>{/if}
      <div class="line" class:off={pending}>
        <label class="lab" for="alloc-tier">Tier</label>
        <select id="alloc-tier" class="field auto" disabled={pending} bind:value={draft.tier}>{#each TIERS as t (t)}<option value={t}>{t}</option>{/each}</select>
        {#if changed.tier && !pending}<span class="chg">• changed</span>{/if}
      </div>
      <div class="line" class:off={pending}>
        <label class="lab" for="alloc-cap">Hard cap (%)</label>
        <input id="alloc-cap" data-field="cap" class="field mono num" class:bad={errors.cap} type="number" min="0" max="100" step="1" placeholder="none" disabled={pending} bind:value={draft.cap} aria-invalid={!!errors.cap} aria-describedby={errors.cap ? 'alloc-cap-err' : undefined} />
        {#if changed.cap && !pending}<span class="chg">• changed</span>{/if}
        <span class="muted">stops {name}'s agents for the week at this % of the week</span>
      </div>
      {#if errors.cap}<p class="ferr" id="alloc-cap-err">{errors.cap}</p>{/if}
      {#if view.allocation}
        <div class="line top">
          <span class="lab">This week</span>
          <div class="used">
            <span>used {Number(view.allocation.used_pct).toFixed(1)} pp this week</span>
            {#if bar}
              <div class="cap">
                <div class="track" aria-hidden="true"><i class="fill {bar.state}" style="width: {bar.fill}%"></i><b class="tick"></b></div>
                <span class="words {bar.state}">{bar.words}</span>
              </div>
            {/if}
          </div>
        </div>
      {/if}
      {#if pending}
        <p class="box warnbox">
          <span>▲ From the save on, {name}'s agents are charged to other sessions.</span><span class="grow"></span>
          <button class="btn ghost" onclick={keep}>Keep in the plan <Kbd keys="esc" /></button>
        </p>
      {/if}
    {/if}

    {#if confirmed && (view.allocation || draft)}<h3>Rest of the plan <span class="muted">{pending ? 'after the save' : 'follows the typed weight'}</span></h3>
    {:else if confirmed}<h3>Rest of the plan <span class="muted">read only · plan order</span></h3>{/if}
    <div class="tbl" role="table" aria-label={confirmed ? 'Rest of the plan' : 'Proposed split'}>
      <div class="tr th" role="row"><span>Allocation</span><span>Weight</span><span>Share</span><span>Tier</span><span>Used</span></div>
      {#each rows as r (r.id + (r.own ? '·own' : ''))}
        <div class="tr" class:own={r.highlight} role="row">
          <span class="nm"><span class="trunc">{r.label}</span>{#if r.tag}<span class="muted tag">{r.tag}</span>{/if}</span>
          <span>{r.weight}</span><span>{r.share}%</span><span>{r.tier}</span><span>{confirmed ? pp(r.used_pct) : '—'}</span>
        </div>
      {/each}
    </div>

    {#if !confirmed}
      <div><button class="btn add" onclick={planner}>Set up the weekly plan <Kbd keys="g" /></button></div>
      <p class="muted note">No fields, no Save: the section confirms nothing by itself.</p>
    {:else}
      <button class="link" onclick={planner} disabled={dirty} title={dirty ? 'Save or cancel first' : ''}>Edit the plan in the Week planner <Kbd keys="g" /></button>
      {#if draft && view.allocation && !pending}<button class="link danger" onclick={remove}>Remove from the plan <Kbd keys="x" /></button>{/if}
      {#if saveError}<p class="box errbox" role="alert">✕ Couldn't save: {saveError}. Nothing was changed.</p>{/if}
      <div class="foot">
        <button class="btn primary" disabled={busy || !dirty || invalid} onclick={save}>Save <Kbd keys="⌘+⏎" /></button>
        <button class="btn ghost" disabled={!dirty && !(draft && !view.allocation)} onclick={cancel}>Cancel <Kbd keys="esc" /></button>
      </div>
    {/if}
    <p class="muted hint">{#each keys as [k, label], i (k)}{#if i}{' · '}{/if}<Kbd keys={k} /> {label}{/each}</p>
  {/if}
</div>

<style>
  .alloc {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  h3 {
    margin: var(--space-md) 0 0;
    font: var(--text-title);
  }
  h3 .muted {
    font: var(--text-caption);
  }
  .lead,
  .note,
  .hint {
    margin: 0;
    font: var(--text-caption);
  }
  .line {
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }
  .line.top {
    align-items: flex-start;
  }
  .line.off .lab,
  .line.off .field {
    opacity: 0.5;
  }
  .lab {
    width: 120px;
    flex: none;
    color: var(--color-text-muted);
  }
  .field.auto {
    width: auto;
  }
  .num {
    width: 72px;
    border-color: var(--color-border-strong);
  }
  .num:focus {
    outline: none;
    border: var(--border-width-focus) solid var(--color-focus);
  }
  .num.bad,
  .num.bad:focus {
    border: var(--border-width-focus) solid var(--color-status-error);
  }
  .num:disabled {
    text-decoration: line-through;
  }
  .chg {
    color: var(--color-accent-default);
  }
  .strike {
    text-decoration: line-through;
  }
  .ferr {
    margin: 0;
    padding-left: calc(120px + var(--space-md));
    color: var(--color-status-error);
  }
  .used {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .cap {
    display: flex;
    align-items: center;
    gap: var(--space-md);
  }
  .track {
    position: relative;
    width: 240px;
    height: 6px;
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
  }
  .fill {
    display: block;
    height: 100%;
    border-radius: var(--radius-sm);
  }
  .fill.green {
    background: var(--color-pacing-green);
  }
  .fill.amber {
    background: var(--color-pacing-amber);
  }
  .fill.red {
    background: var(--color-pacing-red);
  }
  .tick {
    position: absolute;
    left: 80%;
    top: -2px;
    width: 2px;
    height: 10px;
    background: var(--color-text-primary);
  }
  .words.green {
    color: var(--color-text-muted);
  }
  .words.amber {
    color: var(--color-pacing-amber);
  }
  .words.red {
    color: var(--color-pacing-red);
  }
  .box {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    margin: 0;
    padding: var(--space-sm) var(--space-md);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
  }
  .info {
    border-color: var(--color-accent-default);
  }
  .errbox {
    border-color: var(--color-status-error);
  }
  .warnbox {
    border-color: var(--color-pacing-amber);
  }
  .add {
    border-color: var(--color-accent-default);
  }
  .tbl {
    display: flex;
    flex-direction: column;
    width: 632px;
    max-width: 100%;
    padding: var(--space-xs);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .tr {
    display: grid;
    grid-template-columns: 260px 80px 80px 100px 112px;
    align-items: center;
    min-height: var(--size-row);
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid transparent;
    border-radius: var(--radius-sm);
  }
  .tr.th {
    color: var(--color-text-muted);
  }
  .tr.own {
    background: var(--color-surface-selected);
    border-color: var(--color-focus);
  }
  .nm {
    display: flex;
    gap: var(--space-sm);
    min-width: 0;
  }
  .trunc {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tag {
    flex: none;
  }
  .link {
    align-self: flex-start;
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
  .link:disabled {
    color: var(--color-text-muted);
    cursor: default;
  }
  .link.danger {
    color: var(--color-status-error);
    text-decoration: none;
  }
  .foot {
    display: flex;
    gap: var(--space-sm);
    margin-top: var(--space-sm);
  }
  .skel {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    max-width: 240px;
  }
  .skel.wide {
    max-width: 632px;
  }
  .skel i {
    height: 10px;
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
  }
  .skel i.short {
    width: 70%;
  }
</style>
