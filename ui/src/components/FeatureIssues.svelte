<script>
  // Row 3 of the feature header (allow-multiple-prs ux-brief §2.1, ux-flows
  // decision 1): `Issues #12 · ENG-7 · One PR per issue [Edit]`, and the Edit
  // popover (`feature_set_prs`) with the same chip input and segmented control
  // as the launch form. Locked parts: an issue with a PR (pending, open or
  // merged) keeps its chip without ×; the mode is fixed once the run had a PR.
  import { untrack } from 'svelte';
  import { openUrl } from '../lib/app.svelte.js';
  import { errText, invoke } from '../lib/ipc.js';
  import { chipError, chipOf, chipOfIssue, issueErrorText, issuesRead, issueTexts, lockedIssues, markBad, modeFor, modeLabel, modeLockedBy, parseCoded } from '../lib/issues.js';
  import { runFinished } from '../lib/teams.js';
  import { tooltip } from '../lib/tooltip.js';
  import IssuesField from './IssuesField.svelte';
  import PrModeControl from './PrModeControl.svelte';

  /** @type {{ feature: any, run: any, onsaved?: (feature: any) => void }} */
  let { feature, run, onsaved = () => {} } = $props();

  const issues = $derived(feature.issues ?? []);
  const mode = $derived(feature.pr_mode ?? 'single');
  const done = $derived(!!run && runFinished(run.status));
  const live = $derived(!!run && !done);
  const read = $derived(issuesRead(issues));
  const locked = $derived(lockedIssues(run?.prs_view));
  const modeLocked = $derived(modeLockedBy(run));

  let open = $state(false);
  let chips = $state([]);
  let pick = $state('single');
  let error = $state(null);
  let busy = $state(false);
  let saved = $state(false);
  let wrap = $state(null);
  let editBtn = $state(null);
  let timer;

  // Once the run had a PR the mode is fixed: dropping to one issue keeps it (the core allows that), only before does it fall back to One PR.
  const effMode = $derived(modeLocked ? mode : modeFor(pick, chips.length));
  const invalid = $derived(!!chipError(chips));

  function show() {
    chips = issues.map(chipOfIssue);
    pick = mode;
    error = null;
    saved = false;
    open = true;
  }
  function close(focus = true) {
    clearTimeout(timer);
    open = saved = false;
    if (focus) editBtn?.focus();
  }
  $effect(() => {
    chips;
    pick;
    untrack(() => (error = null));
  });
  $effect(() => () => clearTimeout(timer));

  async function save() {
    if (busy || invalid) return;
    busy = true;
    error = null;
    try {
      const f = await invoke('feature_set_prs', { feature: feature.id, issues: issueTexts(chips), prMode: effMode });
      onsaved(f);
      if (live) {
        saved = true;
        // Focus goes back to Edit only when it is still in the popover: this fires on its own.
        timer = setTimeout(() => close(!!wrap?.contains(document.activeElement)), 4000);
      } else close();
    } catch (e) {
      const c = parseCoded(errText(e));
      if (c.code === 'bad_ref') chips = markBad(chips, c.ref, issueErrorText(c));
      else error = issueErrorText(c);
    } finally {
      busy = false;
    }
  }

  function onkey(e) {
    if (e.key === 'Escape' && !e.defaultPrevented) {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === 'Enter' && e.metaKey) {
      e.preventDefault();
      save();
    }
  }
  function outside(e) {
    if (open && wrap && !wrap.contains(e.target)) close(false);
  }
  const PLUS = '\u{F0415}';
</script>

<svelte:window onpointerdown={outside} />

{#if read || !done}
  <div class="row3" bind:this={wrap}>
    <div class="r3text">
      {#if read}
        <span class="muted">Issues</span>
        <span class="refs">{#each issues as i, n (i.ref)}{#if n}<span class="muted">{' · '}</span>{/if}{#if i.url}<button type="button" class="link mono" use:tooltip={i.title ? `${i.ref} ${i.title}` : i.url} onclick={() => openUrl(i.url)}>{i.ref}</button>{:else}<span class="mono" use:tooltip={i.title ?? null}>{i.ref}</span>{/if}{/each}</span>
      {:else}
        <button type="button" class="link add" onclick={show}><span class="plus" aria-hidden="true">{PLUS}</span> Add issues</button>
      {/if}
    </div>
    {#if read}<span class="mode muted">· {modeLabel(mode)}</span>{/if}
    {#if read && !done}<button type="button" class="link edit" bind:this={editBtn} aria-expanded={open} aria-haspopup="dialog" onclick={() => (open ? close() : show())}>Edit</button>{/if}
    {#if open}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions, a11y_no_noninteractive_tabindex -->
      <div class="pop" role="dialog" aria-label="Issues and pull requests" tabindex="-1" onkeydown={onkey}>
        <div class="fg">
          <label for="feature-issues">Issues</label>
          <IssuesField id="feature-issues" bind:chips project={feature.project_id} {locked} />
        </div>
        <div class="fg">
          <span class="lbl">Pull requests</span>
          <PrModeControl value={effMode} refs={chips.map((c) => c.ref)} count={chips.length} locked={modeLocked} onchange={(m) => (pick = m)} />
        </div>
        {#if error}<p class="err" role="alert">{error}</p>{/if}
        {#if saved}<p class="ok" role="status"><span aria-hidden="true">✓</span> Saved; the lead is told.</p>{/if}
        <div class="foot">
          <button type="button" class="btn ghost" onclick={() => close()}>Cancel</button>
          <button type="button" class="btn primary" disabled={busy || invalid || saved} onclick={save}>{busy ? 'Saving…' : 'Save'}</button>
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .row3 {
    position: relative;
    display: flex;
    align-items: baseline;
    gap: var(--space-xs);
    min-width: 0;
    font: var(--text-caption);
  }
  .r3text {
    display: flex;
    gap: var(--space-xs);
    min-width: 0;
    flex: 0 1 auto;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .refs {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .mode,
  .edit {
    flex: none;
    white-space: nowrap;
  }
  .link {
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
  .link:hover {
    text-decoration: underline;
  }
  .add {
    color: var(--color-text-muted);
  }
  .plus {
    font: var(--text-icon);
  }
  .pop {
    position: absolute;
    top: calc(100% + var(--space-xs));
    left: 0;
    z-index: 25;
    width: min(420px, 100%);
    min-width: 300px;
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    padding: var(--space-md);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
    font: var(--text-label);
    white-space: normal;
    outline: none;
  }
  .fg {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    color: var(--color-text-secondary);
  }
  .err {
    margin: 0;
    color: var(--color-status-error);
    font: var(--text-caption);
  }
  .ok {
    margin: 0;
    color: var(--color-status-done);
    font: var(--text-caption);
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-sm);
  }
</style>
