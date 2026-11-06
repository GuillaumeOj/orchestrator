<script>
  // Ready row details (`space`, P4.5 notes §3): the item's text (plain, never HTML or links), both
  // modes' estimates with their basis, the room, labels, milestone / priority / assignees,
  // when it changed. Loads the full body with `intake_item` once opened (a user action).
  import { onMount } from 'svelte';
  import { clock, dayTime } from '../../lib/format.js';
  import { ageText, basisText, exactRange, overRoom, SOURCE_NAME } from '../../lib/intake.js';
  import { errText, invoke } from '../../lib/ipc.js';
  import Kbd from '../Kbd.svelte';

  /** @type {{ item: any, name: string, now: number, hidden?: boolean, onstart?: (m: 'team' | 'solo') => void, onhide?: () => void, onunhide?: () => void, onsource?: () => void }} */
  let { item, name, now, hidden = false, onstart = () => {}, onhide = () => {}, onunhide = () => {}, onsource = () => {} } = $props();

  let full = $state(null); // { item, body, truncated, milestone, … } from intake_item
  let failed = $state(null);
  let more = $state(false);

  onMount(() => {
    let alive = true;
    invoke('intake_item', { item: item.id })
      .then((r) => alive && (full = r))
      .catch((e) => alive && (failed = errText(e)));
    return () => {
      alive = false;
    };
  });

  // The front matter block is not shown (its keys appear below).
  const stripFront = (t) => String(t ?? '').replace(/^---\n[\s\S]*?\n---\n?/, '');
  const text = $derived(full ? stripFront(full.body) : item.excerpt);
  const lines = $derived(String(text ?? '').split('\n'));
  const clamped = $derived(!more && lines.length > 12);
  const shown = $derived(clamped ? lines.slice(0, 12).join('\n') : text);
  const info = $derived(full?.item ?? item);
  const est = $derived(item.estimate);
  const room = $derived(est?.room_pp != null ? `${Number(est.room_pp).toFixed(1)} pp left this week${est.room_scope === 'allocation' ? ` for ${name}` : ''}` : '');
  const labels = $derived(item.labels ?? []);
  const rows = $derived(
    [
      ['Milestone', full?.milestone],
      ['Priority', full?.priority],
      ['Points', full?.points],
      ['Assignees', full?.assignees?.length ? full.assignees.join(', ') : null],
    ].filter(([, v]) => v != null && v !== ''),
  );
  const front = $derived(
    item.source === 'repo_docs'
      ? [item.size && `size ${item.size}`, item.mode && `mode ${item.mode}`, item.team && `team ${item.team}`].filter(Boolean).join(' · ')
      : '',
  );
  const modeLine = (m) => (m === 'team' ? 'Team run' : 'Solo');
</script>

<div class="det" data-ready-details={item.id}>
  <div class="hd"><b class="mono">{item.ref}</b> <span class="muted">· {SOURCE_NAME[item.source]}</span></div>
  <div class="body" aria-label="Item text">{shown}{#if full == null && !failed} <span class="muted">Loading…</span>{/if}</div>
  {#if lines.length > 12}<button class="link" tabindex="-1" onclick={(e) => (e.stopPropagation(), (more = !more))}>{more ? 'Show less' : 'Show more'}</button>{/if}
  {#if full?.truncated}<div class="muted">Shortened to 4 000 characters. Open the source for the rest.</div>{/if}
  {#if failed}<div class="error">Could not load the full text: {failed}</div>{/if}
  <hr />
  <dl>
    {#each ['team', 'solo'] as m (m)}
      {#if est?.[m]}
        <dt>{modeLine(m)}</dt>
        <dd>{exactRange(est[m])} <span class="muted">{basisText(est[m], m, item.size, name)}</span>{#if overRoom(est, m)} <span class="warn">over this week's room</span>{/if}</dd>
      {/if}
    {/each}
    {#if room}<dt>Room</dt><dd>{room}</dd>{/if}
    {#if labels.length}<dt>Labels</dt><dd>{labels.join(' · ')}</dd>{/if}
    {#each rows as [k, v] (k)}<dt>{k}</dt><dd>{v}</dd>{/each}
    {#if front}<dt>Front matter</dt><dd>{front}</dd>{/if}
    {#if info.updated_at_ms}<dt>Updated</dt><dd>{ageText(info.updated_at_ms, now)} <span class="muted">({dayTime(info.updated_at_ms)})</span></dd>{/if}
  </dl>
  <div class="acts">
    <button class="btn ghost" tabindex="-1" onclick={(e) => (e.stopPropagation(), onsource())}>Open source ↗ <Kbd keys="o" /></button>
    {#if hidden}
      <button class="btn" tabindex="-1" onclick={(e) => (e.stopPropagation(), onunhide())}>Unhide <Kbd keys="x" /></button>
    {:else}
      {#if item.can_team !== false}<button class="btn" tabindex="-1" onclick={(e) => (e.stopPropagation(), onstart('team'))}>Start team run <Kbd keys="t" /></button>{/if}
      <button class="btn" tabindex="-1" onclick={(e) => (e.stopPropagation(), onstart('solo'))}>Start solo <Kbd keys="a" /></button>
      <button class="btn ghost" tabindex="-1" onclick={(e) => (e.stopPropagation(), onhide())}>Hide <Kbd keys="x" /></button>
    {/if}
  </div>
</div>

<style>
  .det {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
    margin-top: var(--space-sm);
    padding-top: var(--space-sm);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-label);
  }
  .body {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--color-text-secondary);
  }
  hr {
    width: 100%;
    margin: var(--space-xs) 0;
    border: 0;
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-xs) var(--space-md);
    margin: 0;
  }
  dt {
    color: var(--color-text-muted);
  }
  dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .warn {
    color: var(--color-text-muted);
  }
  .error {
    color: var(--color-status-error);
  }
  .acts {
    display: flex;
    gap: var(--space-sm);
    margin-top: var(--space-xs);
  }
</style>
