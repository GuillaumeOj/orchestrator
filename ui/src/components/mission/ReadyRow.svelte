<script>
  // A Ready to start row (P4.5 notes §2, Penpot 04.2 "Ready row"): source glyph, ref,
  // title, project, labels, the estimate for the suggested mode, the over-room
  // warning, Start team run / Start solo (neutral, opening the prefilled dialogs) and
  // the ⋯ menu (Hide, Open source, Details). Not attention: neutral surface, dashed rule.
  // Under 880 px of row width the project and labels drop to line 2 and the buttons shorten.
  import { estimateText, estimateTip, MODE_LABEL, modeOf, roomWarning, SOURCE_NAME, shownLabels, startLabel } from '../../lib/intake.js';
  import { tooltip } from '../../lib/tooltip.js';
  import Menu from '../Menu.svelte';

  /** @type {{ row: any, selected?: boolean, starting?: boolean, onstart?: (mode: 'team' | 'solo') => void, onhide?: () => void, onunhide?: () => void, onsource?: () => void, ondetails?: () => void }} */
  let { row, selected = false, starting = false, onstart = () => {}, onhide = () => {}, onunhide = () => {}, onsource = () => {}, ondetails = () => {} } = $props();

  const item = $derived(row.item);
  const mode = $derived(modeOf(item));
  const other = $derived(mode === 'team' ? 'solo' : 'team');
  const canTeam = $derived(item.can_team !== false);
  const labels = $derived(shownLabels(item));
  const warn = $derived(roomWarning(item, row.name));
  const GLYPH = { github: '\u{F02A4}', linear: '\u{F0E6D}', repo_docs: '\u{F0214}' };
  const GLYPH_NAME = { github: 'GitHub issue', linear: 'Linear issue', repo_docs: 'Repo doc' };
  // The row's `⋯` (shared Menu, keys right-aligned: they work on the selected row).
  const menuItems = $derived([
    { id: 'details', label: 'Details', key: 'space', run: () => ondetails() },
    { id: 'source', label: 'Open source', key: 'o', run: () => onsource() },
    ...(row.hidden ? [] : [{ id: 'hide', label: 'Hide', key: 'x', run: () => onhide() }]),
  ]);
</script>

<div class="r" class:hidden={row.hidden} class:sel={selected} data-ready={item.id}>
  <div class="l1">
    <span class="glyph" role="img" aria-label={GLYPH_NAME[item.source] ?? SOURCE_NAME[item.source]}>{GLYPH[item.source] ?? ''}</span>
    <span class="ref mono" title={item.ref}>{row.short}</span>
    <span class="title" title={item.title}>{item.title}</span>
    {#if row.hidden}<span class="tag">Hidden</span>{/if}
    {#if starting}<span class="tag">Starting…</span>{/if}
    {#if !row.hidden}
      <span class="est" title={estimateTip(item, mode)}>
        <span class="wide">{estimateText(item, mode)}</span><span class="narrow">{estimateText(item, mode, { compact: true })}</span>
      </span>
      <span class="acts">
        {#each [mode, other] as m, i (m)}
          {@const noTeam = m === 'team' && !canTeam}
          <button class="btn" class:main={i === 0} tabindex="-1" disabled={starting || noTeam} use:tooltip={noTeam ? { label: MODE_LABEL[m].long, detail: `${row.name} has no team yet` } : { label: MODE_LABEL[m].long, key: i === 0 ? '⏎' : MODE_LABEL[m].key }} aria-label={startLabel(item, m)} onclick={(e) => (e.stopPropagation(), onstart(m))}>
            <span class="wide">{MODE_LABEL[m].long}</span><span class="narrow">{MODE_LABEL[m].short}</span>
          </button>
        {/each}
      </span>
    {:else}
      <span class="grow"></span>
      <button class="btn" tabindex="-1" aria-label="Unhide {item.ref} {item.title}" use:tooltip={{ label: 'Unhide', key: 'x' }} onclick={(e) => (e.stopPropagation(), onunhide())}>Unhide</button>
    {/if}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <span class="menuwrap" onclick={(e) => e.stopPropagation()}><Menu items={menuItems} tabindex={-1} label="More for {item.ref}" /></span>
  </div>
  <!-- Line 2 at every width (boards): [warning ·] project · ≤ 2 label chips. -->
  <div class="l2">
    {#if warn}<span class="warn"><span aria-hidden="true">{'\u{F0026}'}</span> {warn}</span><span aria-hidden="true">·</span>{/if}
    <span class="proj">{row.name}</span>
    {#each labels.chips as l (l)}<span class="chip">{l}</span>{/each}{#if labels.more}<span class="chip">+{labels.more}</span>{/if}
  </div>
</div>

<style>
  .r {
    container-type: inline-size;
    min-width: 0;
  }
  .l1,
  .l2 {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
    white-space: nowrap;
  }
  .l2 {
    margin-top: var(--space-xxs);
    padding-left: calc(var(--size-icon) + var(--space-sm));
    color: var(--color-text-muted);
    font: var(--text-caption);
    min-height: 0;
  }
  .glyph {
    flex: none;
    width: var(--size-icon);
    color: var(--color-text-muted);
    text-align: center;
  }
  .ref {
    flex: none;
    max-width: 22ch;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--color-text-muted);
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .hidden .title,
  .hidden .ref {
    opacity: 0.6;
  }
  .tag {
    flex: none;
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .chip {
    padding: 0 var(--space-xs);
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
  }
  .est {
    flex: none;
    color: var(--color-text-secondary);
  }
  .acts {
    flex: none;
    display: inline-flex;
    gap: var(--space-xs);
  }
  .btn {
    background: none;
    border: var(--border-width-default) solid var(--color-border-subtle);
    color: var(--color-text-primary);
  }
  .btn.main {
    border-color: var(--color-border-strong);
  }
  .warn {
    color: var(--color-text-muted);
  }
  .menuwrap {
    position: relative;
    flex: none;
  }
  /* Full rows at 880 px and up (1512 shell), compact below (notes §2.3, D9c): only the long texts shorten. */
  .narrow {
    display: none;
  }
  @container (max-width: 879px) {
    .wide {
      display: none;
    }
    .narrow {
      display: inline;
    }
  }
</style>
