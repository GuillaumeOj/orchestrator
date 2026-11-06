<script>
  // A Ready to start row (P4.5 notes §2, Penpot 04.2 "Ready row"): source glyph, ref,
  // title, project, labels, the estimate for the suggested mode, the over-room
  // warning, Start team run / Start solo (neutral, opening the prefilled dialogs) and
  // the ⋯ menu (Hide, Open source, Details). Not attention: neutral surface, dashed rule.
  // Under 880 px of row width the project and labels drop to line 2 and the buttons shorten.
  import { estimateText, estimateTip, modeOf, roomWarning, SOURCE_NAME, shownLabels, startLabel } from '../../lib/intake.js';
  import Kbd from '../Kbd.svelte';

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
  let menu = $state(false);

  const label = (m) => (m === 'team' ? 'Start team run' : 'Start solo');
  const short = (m) => (m === 'team' ? 'Team run' : 'Solo');
</script>

<div class="r" class:hidden={row.hidden} class:sel={selected} data-ready={item.id}>
  <div class="l1">
    <span class="glyph" role="img" aria-label={GLYPH_NAME[item.source] ?? SOURCE_NAME[item.source]}>{GLYPH[item.source] ?? ''}</span>
    <span class="ref mono" title={item.ref}>{row.short}</span>
    <span class="title" title={item.title}>{item.title}</span>
    {#if row.hidden}<span class="tag">Hidden</span>{/if}
    {#if starting}<span class="tag">Starting…</span>{/if}
    <span class="wide meta">
      <span class="proj">{row.name}</span>
      {#each labels.chips as l (l)}<span class="chip">{l}</span>{/each}{#if labels.more}<span class="chip">+{labels.more}</span>{/if}
    </span>
    {#if !row.hidden}
      <span class="est" title={estimateTip(item, mode)}>
        <span class="wide">{estimateText(item, mode)}</span><span class="narrow">{estimateText(item, mode, { compact: true })}</span>
      </span>
      <span class="acts">
        {#each [mode, other] as m, i (m)}
          {#if m === 'team' && !canTeam}
            <button class="btn" disabled tabindex="-1" title="{row.name} has no team yet" aria-label="Start team run for {item.ref} {item.title}">{label(m)}</button>
          {:else}
            <button class="btn" class:main={i === 0} tabindex="-1" disabled={starting} aria-label={startLabel(item, m)} onclick={(e) => (e.stopPropagation(), onstart(m))}>
              <span class="wide">{label(m)}</span><span class="narrow">{short(m)}</span>{#if i === 0}{' '}<Kbd keys="⏎" />{:else if m === 'solo'}{' '}<Kbd keys="a" />{:else}{' '}<Kbd keys="t" />{/if}
            </button>
          {/if}
        {/each}
      </span>
    {:else}
      <span class="grow"></span>
      <button class="btn" tabindex="-1" aria-label="Unhide {item.ref} {item.title}" onclick={(e) => (e.stopPropagation(), onunhide())}>Unhide <Kbd keys="x" /></button>
    {/if}
    <span class="menuwrap">
      <button class="btn ghost more" tabindex="-1" aria-label="More for {item.ref}" aria-expanded={menu} onclick={(e) => (e.stopPropagation(), (menu = !menu))}>⋯</button>
      {#if menu}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div class="menu" role="menu" tabindex="-1" onclick={(e) => (e.stopPropagation(), (menu = false))}>
          <button role="menuitem" tabindex="-1" onclick={() => ondetails()}>Details <Kbd keys="space" /></button>
          <button role="menuitem" tabindex="-1" onclick={() => onsource()}>Open source <Kbd keys="o" /></button>
          {#if !row.hidden}<button role="menuitem" tabindex="-1" onclick={() => onhide()}>Hide <Kbd keys="x" /></button>{/if}
        </div>
      {/if}
    </span>
  </div>
  <div class="l2" class:nowarn={!warn}>
    <span class="meta2">{row.name}{#if labels.chips.length} · {labels.chips.join(', ')}{#if labels.more}, +{labels.more}{/if}{/if}</span>
    {#if warn}<span class="warn"><span aria-hidden="true">{'\u{F0026}'}</span> {warn}</span>{/if}
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
  .l2.nowarn {
    display: none;
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
  .meta {
    flex: none;
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .chip {
    padding: 0 var(--space-xs);
    border: var(--border-width-default) solid var(--color-border-subtle);
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
  .menu {
    position: absolute;
    right: 0;
    top: 100%;
    z-index: 10;
    display: flex;
    flex-direction: column;
    min-width: 160px;
    padding: var(--space-xs);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-overlay);
  }
  .menu button {
    display: flex;
    justify-content: space-between;
    gap: var(--space-md);
    padding: var(--space-xs) var(--space-sm);
    border: 0;
    background: none;
    color: var(--color-text-primary);
    font: var(--text-label);
    text-align: left;
  }
  .menu button:hover {
    background: var(--color-surface-selected);
  }
  /* Full rows at 880 px and up (1512 shell), compact below (notes §2.3, D9c). */
  .narrow,
  .meta2 {
    display: none;
  }
  @container (max-width: 879px) {
    .wide.meta {
      display: none;
    }
    span.wide {
      display: none;
    }
    .narrow {
      display: inline;
    }
    .meta2 {
      display: inline;
    }
    .l2.nowarn {
      display: flex;
    }
  }
</style>
