<script>
  // One line of the Work tree (notes §2.2 Tree row, §2.7 fold, §11 lines; page 02): 32 px, slots left → right
  // indent · disclosure · kind glyph · ref · issue type · title · upstream state · labels · dependency ·
  // children progress · app-state chip (fixed column). Lines (loading, error, more, source status) share the
  // indent. Buttons inside are not Tab stops: the pane's keys reach them.
  import { app } from '../../lib/app.svelte.js';
  import { liveCapText } from '../../lib/program.js';
  import { tooltip } from '../../lib/tooltip.js';
  import { closedMark, depMark, GLYPH, isClosed, isContainer, labelsOf, loadErrorText, progressText, refOf, rightOf, selectable, tailOf, taskLineOf, titleOf, trackerName, upstreamText } from '../../lib/workmodel.js';
  import AppChip from './AppChip.svelte';
  import KindGlyph from './KindGlyph.svelte';

  /**
   * `ariaBase`: added to `row.level` for aria-level (1 with project rows at level 0: aria-level starts at 1).
   * @type {{ row: any, ariaBase?: number, focused?: boolean, inDetails?: boolean, selected?: boolean, selecting?: boolean,
   *          narrow?: boolean, readyTip?: string, projectTail?: any, onrow?: () => void, ondisclose?: () => void,
   *          onchip?: (c: any) => void, onaction?: (what: string) => void }}
   */
  let {
    row,
    ariaBase = 0,
    focused = false,
    inDetails = false,
    selected = false,
    selecting = false,
    narrow = false,
    readyTip = '',
    projectTail = null,
    onrow = () => {},
    ondisclose = () => {},
    onchip = () => {},
    onaction = () => {},
  } = $props();

  const liveCap = $derived(liveCapText(app.now.counts?.global));
  const n = $derived(row.node);
  /** The slots of a tree item, each computed once (other row kinds have none). */
  const slots = $derived.by(() => {
    if (row.kind !== 'node' || !n) return null;
    const closed = isClosed(n);
    return {
      right: rightOf(n, { short: narrow, readyTip, liveCap }),
      dep: depMark(n),
      tail: tailOf(n),
      mark: closedMark(n),
      labels: narrow ? { chips: [], more: 0 } : labelsOf(n),
      closed,
      strong: !closed && (isContainer(n) || ['Epic', 'Feature'].includes(n.upstream?.issue_type)),
      canSel: selectable(n).ok,
      ref: refOf(n),
      up: upstreamText(n),
      prog: progressText(n),
    };
  });
  const lineLevel = $derived(row.level + ariaBase);
</script>

{#snippet guides(k)}
  {#each { length: k } as _, i (i)}<span class="guide" aria-hidden="true"></span>{/each}
{/snippet}

{#snippet disclosure(open, has)}
  {#if has}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <span class="disc" aria-hidden="true" onclick={(e) => (e.stopPropagation(), ondisclose())}>{open ? '⌄' : '›'}</span>
  {:else}<span class="disc" aria-hidden="true"></span>{/if}
{/snippet}

{#if row.kind === 'node'}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="row node"
    class:focused
    class:open={inDetails}
    class:selected
    class:hidden={n.hidden}
    class:ancestor={n.ancestor_only}
    class:closed={slots.closed}
    role="treeitem"
    tabindex="-1"
    aria-level={lineLevel}
    aria-expanded={row.hasKids ? row.open : undefined}
    aria-selected={focused}
    data-key={row.key}
    onclick={onrow}
  >
    {@render guides(row.indent)}
    {#if selecting}<span class="selbox" aria-hidden="true">{slots.canSel ? (selected ? '\u{F0135}' : '\u{F0131}') : ''}</span>{/if}
    {@render disclosure(row.open, row.hasKids)}
    {#if row.level >= 5}<span class="ltag">L{row.level}</span>{/if}
    <KindGlyph node={n} dim={!!n.upstream?.deleted} />
    {#if slots.ref}<span class="ref">{slots.ref}</span>{/if}
    {#if n.upstream?.issue_type}<span class="type">{n.upstream.issue_type}</span>{/if}
    <span class="title" class:strong={slots.strong}>{titleOf(n)}</span>
    {#if n.hidden}<span class="type">Hidden</span>{/if}
    <span class="grow"></span>
    {#if slots.up}<span class="up">{slots.up}</span>{/if}
    {#each slots.labels.chips as l (l)}<span class="label">{l}</span>{/each}
    {#if slots.labels.more}<span class="up">+{slots.labels.more}</span>{/if}
    {#if slots.dep}<span class="dep" use:tooltip={slots.dep.tip}><span aria-hidden="true">{'\u{F0337}'}</span>{#if !narrow} {slots.dep.text}{:else}<span class="sr">{slots.dep.text}</span>{/if}</span>{/if}
    {#if slots.prog}<span class="prog">{slots.prog}</span>{/if}
    {#if slots.tail}<span class="tail" class:warn={slots.tail.warn}>{slots.tail.text}</span>{/if}
    {#if slots.mark}<span class="mark" use:tooltip={slots.mark.tip}>⚠ {slots.mark.text}</span>{/if}
    <span class="right" class:narrow>
      {#if slots.right?.chip}<AppChip chip={slots.right.chip} onopen={onchip} />{:else if slots.right?.rollup}<span class="rollup" use:tooltip={slots.right.tip}>{slots.right.rollup}</span>{/if}
    </span>
  </div>
{:else if row.kind === 'project'}
  {@const p = row.project}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="row project" class:focused class:open={inDetails} role="treeitem" tabindex="-1" aria-level={lineLevel} aria-expanded={row.open} aria-selected={focused} data-key={row.key} onclick={onrow}>
    {@render disclosure(row.open, true)}
    <KindGlyph glyph={GLYPH.project} label="Project {p.name}" />
    <span class="title strong">{p.name}</span>
    <span class="up">· {projectTail?.error ?? projectTail?.tracker ?? ''}</span>
    <span class="grow"></span>
    {#if projectTail?.fix}
      <button type="button" class="linkbtn err" tabindex="-1" onclick={(e) => (e.stopPropagation(), onaction('fix'))}>{projectTail.fix}</button>
    {:else if projectTail?.right}<span class="up">{projectTail.right}</span>{/if}
  </div>
{:else if row.kind === 'fold'}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="row fold" class:focused role="treeitem" tabindex="-1" aria-level={lineLevel} aria-expanded={row.open} aria-selected={focused} data-key={row.key} onclick={onrow}>
    {@render guides(row.indent)}
    {@render disclosure(row.open, true)}
    <KindGlyph glyph={GLYPH.fold} label="Team tasks" />
    <span class="up" use:tooltip={`Tasks the team of ${n.app?.feature ?? 'this feature'} wrote to ${trackerName(n)}. Manage them in the feature.`}>Team tasks · {row.count}{n.app?.feature ? ` · feature ${n.app.feature}` : ''}</span>
  </div>
{:else if row.kind === 'task'}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="row task" class:focused role="treeitem" tabindex="-1" aria-level={lineLevel} aria-selected={focused} data-key={row.key} onclick={onrow}>
    {@render guides(row.indent)}
    <span class="disc" aria-hidden="true"></span>
    <span class="up">{taskLineOf(n)}</span>
  </div>
{:else}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="row line {row.kind} {row.tone ?? ''}" class:focused role="treeitem" tabindex="-1" aria-level={lineLevel} aria-selected={focused} data-key={row.key} onclick={onrow}>
    {@render guides(row.indent)}
    <span class="disc" aria-hidden="true"></span>
    {#if row.kind === 'loading'}
      <span class="up"><span aria-hidden="true">{'\u{F0450}'}</span> Loading…</span>
    {:else if row.kind === 'error'}
      <span class="errtext">{loadErrorText(row.message)}</span>
      <button type="button" class="btn sm" tabindex="-1" onclick={(e) => (e.stopPropagation(), onaction('retry'))}>Retry</button>
    {:else if row.kind === 'more'}
      {#if row.roots}
        <span class="lnk">Showing {row.shown} of {row.shown + row.left} · or search with /</span>
        <button type="button" class="btn sm" tabindex="-1" onclick={(e) => (e.stopPropagation(), onaction('more'))}>Load {Math.min(100, row.left)} more</button>
      {:else}
        <span class="lnk">+ {row.left} more sub-issues</span>
        <button type="button" class="btn sm" tabindex="-1" onclick={(e) => (e.stopPropagation(), onaction('more'))}>Load {Math.min(50, row.left)} more</button>
      {/if}
    {:else if row.kind === 'limit'}
      <span class="up">GitHub shows at most 100 sub-issues per issue.</span>
    {:else if row.kind === 'srcline'}
      <span class="tone" aria-hidden="true">{row.tone === 'info' ? '\u{F02FC}' : '⚠'}</span>
      <span class="srctext">{row.text}</span>
      {#if row.fix && row.fixLabel}<button type="button" class="btn sm" tabindex="-1" onclick={(e) => (e.stopPropagation(), onaction('fix'))}>{row.fixLabel}</button>{/if}
    {/if}
  </div>
  {#if row.kind === 'srcline' && row.command}<div class="row line cmd" style="--indent: {row.indent}"><span class="mono">{row.command}</span></div>{/if}
{/if}

<style>
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    height: 32px;
    padding: 0 var(--space-md) 0 var(--space-sm);
    min-width: 0;
    font: var(--text-body);
    color: var(--color-text-primary);
    border: var(--border-width-default) solid transparent;
    border-radius: var(--radius-sm);
    cursor: default;
    outline: none;
  }
  .row:hover {
    background: color-mix(in srgb, var(--color-surface-raised) 60%, transparent);
  }
  .row.open {
    background: var(--color-surface-raised);
  }
  .row.selected {
    background: var(--color-surface-selected);
  }
  .row.focused {
    border-color: var(--color-focus);
  }
  .row.hidden > :not(.right) {
    opacity: 0.6;
  }
  .row.ancestor .title,
  .row.closed .title {
    color: var(--color-text-muted);
  }
  .row.fold,
  .row.task,
  .row.line {
    height: 28px;
  }
  .guide {
    flex: none;
    align-self: stretch;
    width: 16px;
    margin-right: calc(-1 * var(--space-sm));
    background: linear-gradient(var(--color-border-subtle), var(--color-border-subtle)) 7px 0 / 1px 100% no-repeat;
  }
  .disc {
    flex: none;
    width: 12px;
    text-align: center;
    color: var(--color-text-muted);
    cursor: pointer;
  }
  .selbox {
    flex: none;
    width: 16px;
    color: var(--color-focus);
    font: var(--text-icon);
  }
  .ltag,
  .ref,
  .prog {
    flex: none;
    font: var(--text-mono-sm);
    color: var(--color-text-muted);
  }
  .ltag {
    font: var(--text-caption);
  }
  .type,
  .label {
    flex: none;
    padding: 0 var(--space-xs);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    font: var(--text-caption);
    color: var(--color-text-muted);
  }
  .label {
    border-radius: var(--radius-full);
    padding: 0 var(--space-sm);
  }
  .title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 0 1 auto;
  }
  .title.strong {
    font: var(--text-body-strong);
  }
  .grow {
    flex: 1;
    min-width: var(--space-sm);
  }
  .up,
  .dep,
  .rollup,
  .mark {
    flex: none;
    font: var(--text-label);
    color: var(--color-text-muted);
    white-space: nowrap;
  }
  .row.fold .up,
  .row.task .up,
  .row.line .up {
    flex: 0 1 auto;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tail {
    flex: none;
    font: var(--text-label);
    color: var(--color-text-muted);
  }
  .tail.warn,
  .mark {
    color: var(--color-pacing-amber);
  }
  .right {
    flex: none;
    width: 200px;
    display: flex;
    justify-content: flex-end;
    min-width: 0;
  }
  .right.narrow {
    width: 160px;
  }
  .right:empty {
    width: 0;
  }
  .errtext,
  .err {
    color: var(--color-status-error);
    font: var(--text-label);
  }
  .lnk {
    color: var(--color-accent-default);
    font: var(--text-label);
  }
  .srctext {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: var(--text-label);
  }
  .line.info .srctext,
  .line.info .tone {
    color: var(--color-text-muted);
  }
  .line.warn .srctext,
  .line.warn .tone {
    color: var(--color-pacing-amber);
  }
  .line.error .srctext,
  .line.error .tone {
    color: var(--color-status-error);
  }
  .cmd {
    padding-left: calc(var(--indent, 0) * 16px + 32px);
  }
  .btn.sm {
    height: var(--size-control-sm);
    flex: none;
  }
  .linkbtn {
    background: none;
    border: 0;
    padding: 0;
    font: var(--text-label);
    cursor: pointer;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
</style>
