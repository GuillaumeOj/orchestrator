<script>
  // The chat column (FR-1304/1305, P4.3 spec §3.3, notes §1–§10; Penpot 04.3):
  // a fixed column of the shell, not a pane. Header (Decisions · Today ▾ ·
  // search) · level filter · today's thread (morning line, cards, lines, the
  // fold) · the outcome digest · footer (D1: no input). Renders `app.now`.
  // Keys with the chat focused: j/k open cards, a r ⏎ o x u e on the focused
  // card, c clear the digest, f filter, / search, Esc back to the centre pane.
  import { tick, untrack } from 'svelte';
  import {
    app,
    confirm,
    coreAct,
    decide,
    featureBySlug,
    focusPane,
    openAgent,
    openBoard,
    openEditor,
    openFeature,
    openMission,
    openReport,
    openUrl,
    projects,
    toast,
    undoItem,
    undoLast,
  } from '../../lib/app.svelte.js';
  import { alreadyText, digestText, FOLD_AT, filterStrip, foldText, nextFilter, thread, verbText } from '../../lib/chat.js';
  import { clock } from '../../lib/format.js';
  import { actionByKey, agentOf, answerText, FILTERS, intentFor, keyIntent } from '../../lib/inbox.js';
  import { errText } from '../../lib/ipc.js';
  import { clearDigest, counts, digest } from '../../lib/now.svelte.js';
  import Kbd from '../Kbd.svelte';
  import OutcomeRow from '../OutcomeRow.svelte';
  import ChatLine from './ChatLine.svelte';
  import DecisionCard from './DecisionCard.svelte';

  // The header's title: P4.4 swaps it for the Overwatch (notes §1).
  const TITLE = 'Decisions';
  const DIGEST_ROWS = 5;
  const PULSE_MS = 1500;

  let foldOpen = $state(false);
  let sel = $state(null); // 'c:<decision id>' | 'fold' | 'digest'
  let marked = $state(new Set());
  let reply = $state(null); // { id, action }
  let choices = $state({}); // decision id → the options block's choices
  let expanded = $state(new Set()); // line item keys
  // A card's collapse waits while the pointer is over it or a reply is typed (notes §2.1).
  let hovered = $state(null);
  let typing = $state(new Set());
  let now = $state(Date.now());
  let outcome = $state(null); // the selected digest row's id
  let moreOutcomes = $state(false);
  let searchOpen = $state(false);
  let query = $state('');
  let pulse = $state(null); // the item key with the focus highlight
  let announce = $state('');
  const seen = new Map(); // decision id → when its effect was first seen
  const memo = new Map(); // the thread's items, reused while unchanged
  let feed;
  let searchField = $state();

  const filter = $derived(app.chatFilter);
  const t = $derived(
    thread({
      chat: app.now.chat,
      decisions: app.now.decisions,
      pending: app.pending,
      filter,
      foldOpen,
      seen,
      held: new Set([hovered, ...typing]),
      now,
      memo,
    }),
  );
  const fold = $derived(t.items.find((i) => i.type === 'fold') ?? null);
  // Its decision in the undo window, or sent and not yet answered by the core
  // (`sent`): either way the card takes no second answer.
  const pendingOf = (id) => app.pending.find((p) => p.item === id) ?? null;
  // j/k stops: the cards, the fold header, the digest header (notes §10.2).
  const stops = $derived([...t.items.filter((i) => i.type === 'card' || i.type === 'fold').map(stopKey), 'digest']);
  function stopKey(i) {
    return i.type === 'card' ? `c:${i.id}` : i.key;
  }
  // The selection, or where it was: a card that collapsed hands it to the next open card.
  const cur = $derived.by(() => {
    if (sel && stops.includes(sel)) return sel;
    if (sel?.startsWith('c:')) {
      const at = t.items.findIndex((i) => i.id === sel.slice(2));
      const next = t.items.slice(at + 1).find((i) => i.type === 'card') ?? t.items.slice(0, Math.max(at, 0)).findLast((i) => i.type === 'card');
      if (next) return stopKey(next);
    }
    // Nothing chosen yet: the oldest open card, as ⌘⇧I would.
    return stops[0];
  });
  const selCard = $derived(cur?.startsWith('c:') ? (t.cards.find((c) => c.id === cur.slice(2)) ?? null) : null);

  const outcomes = $derived(digest());
  const doneToday = $derived(counts().done_today);
  const dg = $derived(digestText(outcomes, doneToday));
  const strip = $derived(filterStrip(filter, t.counts));
  const showFilters = $derived(t.open >= 2 || filter !== 'all');
  const lastSettled = $derived(t.items.findLast((i) => i.type === 'line')?.line.at_ms ?? null);

  // A user's effect starts its collapse clock when it is first seen (no effect_at_ms).
  $effect(() => {
    for (const d of Object.values(app.now.decisions))
      if (d.lifecycle?.state === 'effect' && !seen.has(d.id)) seen.set(d.id, Date.now());
  });
  // The next collapse re-renders the thread at its time.
  $effect(() => {
    const due = t.nextDue;
    if (due == null) return;
    const timer = setTimeout(() => (now = Date.now()), Math.max(0, due - Date.now()) + 5);
    return () => clearTimeout(timer);
  });
  // Folded cards stay shown until open cards drop to 5 or fewer (notes §4).
  $effect(() => {
    if (t.shown <= FOLD_AT && untrack(() => foldOpen)) foldOpen = false;
  });
  // A new day starts with the filter on All (notes §5).
  let day = untrack(() => app.now.chat?.day);
  $effect(() => {
    const d = app.now.chat?.day;
    if (d !== day) {
      day = d;
      app.chatFilter = 'all';
    }
  });

  // New level 1–2 cards are announced, throttled (spec §5 accessibility).
  // Every open card counts, folded or filtered out too (else it is announced late).
  const announced = new Set(untrack(() => t.all.map((c) => c.id)));
  let announcedAt = 0;
  $effect(() => {
    const fresh = t.all.filter((c) => c.d.level <= 2 && !announced.has(c.id) && c.d.lifecycle?.state === 'open');
    if (!fresh.length) return;
    for (const c of fresh) announced.add(c.id);
    if (Date.now() - announcedAt < 3000) return;
    announcedAt = Date.now();
    const c = fresh[0].d;
    announce = fresh.length > 1 ? `${fresh.length} new cards need you` : `${agentOf(c) ?? 'Orchestrator'} asks: ${c.title}`;
  });

  // ⌘⇧I, the badge, a chip, Mission control `d` (focusCard), ⌘L (searchDecisions).
  $effect(() => {
    const req = app.chatFocus;
    if (!req) return;
    app.chatFocus = null;
    untrack(() => (req.search ? openSearch() : focusOn(req.id, req.filter)));
  });

  /** Focus a decision's card (or its line), else the oldest open card, else the digest.
   *  The filter goes to All (or to `filter`: the palette's "Filter decisions: …"). */
  async function focusOn(id, filter = 'all') {
    searchOpen = false;
    app.chatFilter = filter;
    await tick();
    let card = id ? t.cards.find((c) => c.id === id) : null;
    if (id && !card && fold?.n && !foldOpen) {
      foldOpen = true;
      await tick();
      card = t.cards.find((c) => c.id === id);
    }
    // Already closed (notes §8.2): say what happened to it, then show the next open card,
    // or its line when none is open. Unknown: say so, and show the oldest open card.
    const line = id && !card ? t.items.findLast((i) => i.type === 'line' && i.id === id) : null;
    if (line && !t.cards.length) {
      toast(alreadyText(line.line));
      expanded = new Set([...expanded, line.key]);
      sel = null;
      await tick();
      flash(line.key);
      feed?.querySelector(`[data-line-toggle="${line.key}"]`)?.focus();
      return;
    }
    if (line) toast(`${alreadyText(line.line)}. Showing the next card.`);
    else if (id && !card) toast('That decision is no longer listed.');
    const target = card ?? t.cards[0] ?? null;
    sel = target ? `c:${target.id}` : 'digest';
    feed?.focus();
    if (card) flash(card.key);
    reveal();
  }

  function flash(key) {
    pulse = key;
    setTimeout(() => pulse === key && (pulse = null), PULSE_MS);
  }

  async function openSearch() {
    searchOpen = true;
    await tick();
    searchField?.focus();
  }

  function reveal() {
    tick().then(() => feed?.querySelector('.stop.sel')?.scrollIntoView({ block: 'nearest' }));
  }

  function moveSel(delta) {
    const i = stops.indexOf(cur);
    const n = i < 0 ? (delta > 0 ? 0 : stops.length - 1) : Math.min(Math.max(i + delta, 0), stops.length - 1);
    sel = stops[n];
    outcome = null;
    reveal();
  }

  /** Where `o` on a digest row goes: the PR, the report, the task, the feature or the agent. */
  function openOutcome(o) {
    if (o.url) return openUrl(o.url);
    if (o.kind === 'report_ready') return openReport(o.report ?? 'current');
    if (o.task != null && o.project) return openBoard(o.project, o.task);
    if (o.run && o.feature) {
      const f = featureBySlug(o.project, o.feature);
      if (f) return openFeature(f.id);
    }
    if (o.agent) openAgent(o.agent);
  }

  /** Runs what a key or an action id means on decision `d` (the Inbox's set). */
  function apply(intent, d) {
    if (!intent) return false;
    switch (intent.kind) {
      case 'move':
        moveSel(intent.delta);
        return true;
      case 'mark': {
        const m = new Set(marked);
        m.has(d.id) ? m.delete(d.id) : m.add(d.id);
        marked = m;
        return true;
      }
      case 'undo':
        if (!undoLast()) toast('Nothing to undo');
        return true;
      case 'decide': {
        // x-marked cards take a/r together when each has that action; a keyless
        // button (Always allow, Restart fresh, Dismiss) is for its own card.
        const key = d.actions?.find((x) => x.id === intent.action)?.key;
        const targets =
          key && marked.size
            ? t.cards.filter((c) => marked.has(c.id) && actionByKey(c.d, key)?.id === intent.action).map((c) => c.d)
            : [d];
        for (const x of targets) send(x, intent.action);
        marked = new Set();
        return true;
      }
      case 'core':
        coreAct(d, intent.action);
        return true;
      case 'answer': {
        const opts = d.context?.find((b) => b.type === 'options');
        if (opts) {
          const c = choices[d.id] ?? [];
          if (opts.questions.every((_, i) => c[i]?.length)) send(d, 'answer', answerText(c));
          else toast('Pick an answer for every question');
        } else reply = { id: d.id, action: 'answer' };
        return true;
      }
      case 'reply':
        reply = { id: d.id, action: 'reply' };
        return true;
      case 'pane':
        if (agentOf(d)) openAgent(agentOf(d));
        return true;
      case 'editor':
        openEditor(null, null, { agentId: agentOf(d) });
        return true;
    }
    return false;
  }

  /** Decide (5 s undo window, on the card); focus goes to the next card still waiting. */
  function send(d, action, message = null) {
    const at = t.cards.findIndex((c) => c.id === d.id);
    const next = [...t.cards.slice(at + 1), ...t.cards.slice(0, Math.max(at, 0))].find(
      (c) => c.id !== d.id && !pendingOf(c.id) && !marked.has(c.id),
    );
    if (cur === `c:${d.id}` && next) sel = `c:${next.id}`;
    decide(d, action, message);
  }

  // A keyless action (Always allow, Restart fresh, Delete, Dismiss…). Delete asks first.
  function act(d, id) {
    sel = `c:${d.id}`;
    if (id === 'delete')
      return confirm(`Delete ${agentOf(d)}?`, 'Kills it and removes it from the app; its worktree stays.', () => send(d, id), 'Delete');
    apply(intentFor(d, id), d);
  }

  function cardKey(d, key) {
    sel = `c:${d.id}`;
    // A card's own Undo button undoes that card, not the last decision taken.
    if (key === 'u' && pendingOf(d.id) && !pendingOf(d.id).sent) {
      undoItem(d.id);
      toast('Undone');
      return;
    }
    apply(keyIntent(key, d), d);
  }

  function sendReply(d, text) {
    const r = reply;
    reply = null;
    feed?.focus();
    if (text == null || !text.trim() || !r) return;
    send(d, r.action, text);
  }

  function toggleLine(key) {
    const s = new Set(expanded);
    s.has(key) ? s.delete(key) : s.add(key);
    expanded = s;
  }

  /** "↳ after": scroll to the earlier decision's line, expand it, focus it (notes §3). */
  async function jump(ref) {
    expanded = new Set([...expanded, ref.key]);
    await tick();
    const el = feed?.querySelector(`[data-line-toggle="${ref.key}"]`);
    el?.scrollIntoView({ block: 'nearest' });
    el?.focus();
    flash(ref.key);
  }

  function hover(id, on) {
    if (on) hovered = id;
    else if (hovered === id) {
      hovered = null;
      now = Date.now();
    }
  }

  function typed(id, on) {
    if (on === typing.has(id)) return;
    const s = new Set(typing);
    on ? s.add(id) : s.delete(id);
    typing = s;
    if (!on) now = Date.now();
  }

  const clear = () => {
    outcome = null;
    return clearDigest(outcomes).catch((err) => toast(errText(err), { kind: 'error' }));
  };

  /** Esc: back to the centre pane that had the focus (Mission control if none). */
  function leave() {
    sel = null;
    if (app.focused && app.panes[app.focused]) focusPane(app.focused);
    else openMission();
  }

  function onKey(e) {
    if (e.defaultPrevented || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.target.closest?.('input, textarea')) return;
    const key = e.key === 'Enter' ? 'enter' : e.key.length === 1 ? e.key : e.key.toLowerCase();
    const take = () => e.preventDefault();
    if (key === 'escape') {
      take();
      if (searchOpen) return (searchOpen = false);
      return leave();
    }
    // A focused line, link or button takes its own ⏎ (Tab order, notes §10.2).
    if (key === 'enter' && e.target !== feed && e.target.closest?.('button')) return;
    if (key === '/') return take(), openSearch();
    if (key === 'f') return take(), (app.chatFilter = nextFilter(filter, t.counts));
    if (key === 'c') {
      if (outcomes.length) take(), clear();
      return;
    }
    const o = outcome ? outcomes.find((x) => x.id === outcome) : null;
    if (o && key === 'o') return take(), openOutcome(o);
    if (key === 'enter' && cur === 'fold') return take(), (foldOpen = !foldOpen);
    const intent = keyIntent(key, selCard?.d ?? null);
    if (!intent) return;
    // Not on a card (the fold or digest header, a digest row still listed, a
    // focused line or its read-only card): only move and undo.
    const onLine = !!e.target.closest?.('article.line');
    if ((o || !selCard || onLine) && intent.kind !== 'move' && intent.kind !== 'undo') return take();
    if (selCard && pendingOf(selCard.id) && intent.kind !== 'move' && intent.kind !== 'undo') return take();
    if (apply(intent, selCard?.d)) take();
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<aside class="chat" aria-label={TITLE} onkeydown={onKey}>
  <header class="head">
    {#if searchOpen}
      <div class="search">
        <span class="icon muted" aria-hidden="true">{'\u{F0349}'}</span>
        <input class="field" bind:this={searchField} bind:value={query} placeholder="Search decisions and lines" aria-label="Search decisions and lines" onkeydown={(e) => e.key === 'Escape' && (e.preventDefault(), e.stopPropagation(), query ? (query = '') : ((searchOpen = false), feed?.focus()))} />
        {#if query}<button class="iconbtn" aria-label="Clear the search" onclick={() => (query = '')}>×</button>{:else}<Kbd keys="esc" />{/if}
      </div>
    {:else}
      <h2 class="title">{TITLE}</h2>
      <span class="grow"></span>
      <button class="day" aria-haspopup="dialog" title="Older days">Today ▾</button>
      <button class="iconbtn" aria-label="Search decisions · ⌘L" title="Search decisions · ⌘L" onclick={openSearch}><span aria-hidden="true">{'\u{F0349}'}</span></button>
    {/if}
  </header>
  {#if searchOpen && query.trim().length < 2}<p class="strip muted">Type 2 or more characters.</p>{/if}
  {#if showFilters}
    <div class="filters" role="group" aria-label="Filter cards">
      {#each FILTERS as f (f.id)}
        {#if f.id === 'all' || t.counts[f.id] || filter === f.id}
          <button class="filter-chip" class:on={filter === f.id} aria-pressed={filter === f.id} onclick={() => (app.chatFilter = f.id)}>{f.label} {t.counts[f.id]}</button>
        {/if}
      {/each}
      <span class="grow"></span>
      <Kbd keys="f" />
    </div>
    {#if strip}
      <p class="strip" data-strip>{strip} · <button class="link" onclick={() => (app.chatFilter = 'all')}>Show all</button></p>
    {/if}
  {/if}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="feed" role="feed" aria-label="Decisions, today" tabindex="0" data-chat-focus bind:this={feed}>
    {#each t.items as it (it.type === 'card' ? `card:${it.key}` : it.key)}
      {#if it.type === 'morning'}
        <article class="morning" aria-label="Morning summary">{it.entry.text}</article>
      {:else if it.type === 'fold'}
        <button class="fold stop" class:sel={cur === 'fold'} class:warn={it.blocking > 0} aria-expanded={it.open} aria-label="{it.n} more open cards{it.blocking ? `, ${it.blocking} blocking` : ''}. {it.open ? 'Fold them' : 'Show them'}" onclick={() => ((sel = 'fold'), (foldOpen = !foldOpen))}>
          <span aria-hidden="true">{it.open ? '▾' : '▸'}</span> {foldText(it)} · <Kbd keys={it.open ? '⏎' : 'j'} />
        </button>
      {:else if it.type === 'line'}
        <ChatLine item={it} expanded={expanded.has(it.key)} pulse={pulse === it.key} ontoggle={() => toggleLine(it.key)} onopen={(id) => it.d && apply(intentFor(it.d, id), it.d)} />
      {:else}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <article
          class="stop"
          class:sel={cur === `c:${it.id}`}
          aria-label="{it.d.title}"
          onmouseenter={() => hover(it.id, true)}
          onmouseleave={() => hover(it.id, false)}
          onmousedown={() => ((sel = `c:${it.id}`), (outcome = null))}
        >
          <DecisionCard
            d={it.d}
            selected={cur === `c:${it.id}`}
            marked={marked.has(it.id)}
            pending={pendingOf(it.id)}
            ref={it.ref ? { what: `${verbText(it.ref.line.verb)} ${it.ref.line.what}`, at: clock(it.ref.line.at_ms) } : null}
            replying={reply?.id === it.id ? reply.action : null}
            pulse={pulse === it.key}
            bind:choices={() => choices[it.id] ?? [], (v) => (choices[it.id] = v)}
            onkey={(k) => cardKey(it.d, k)}
            onact={(id) => act(it.d, id)}
            onreply={(text) => sendReply(it.d, text)}
            onjump={() => jump(it.ref)}
            onhold={(on) => typed(it.id, on)}
          />
        </article>
      {/if}
    {/each}
    {#if !t.open}
      <p class="empty">
        <b>Nothing needs you</b>{lastSettled ? ` · last settled ${clock(lastSettled)}` : '.'}
        {#if !projects().length}<br /><span class="muted">Add a project and start a session; its cards appear here.</span>
        {:else if !t.items.some((i) => i.type === 'line')}<br /><span class="muted">Cards appear here when an agent needs you; the digest at the end keeps what got done today.</span>{/if}
      </p>
    {/if}
    <section class="digest" aria-label="Today's outcomes">
      <div class="dh stop" class:sel={cur === 'digest'}>
        <b>{dg.head}{#if dg.clear}{' · '}<button class="link" onclick={clear}>Clear</button>{/if}</b>
        {#if dg.more}<span class="muted">· {dg.more}</span>{/if}
        <span class="grow"></span>
        {#if dg.clear}<Kbd keys="c" />{/if}
      </div>
      {#each moreOutcomes ? outcomes : outcomes.slice(0, DIGEST_ROWS) as o (o.id)}
        <OutcomeRow outcome={o} selected={outcome === o.id} onselect={() => (outcome = o.id)} />
      {/each}
      {#if outcomes.length > DIGEST_ROWS && !moreOutcomes}
        <button class="link more" onclick={() => (moreOutcomes = true)}>Show {outcomes.length - DIGEST_ROWS} more outcome{outcomes.length - DIGEST_ROWS === 1 ? '' : 's'}</button>
      {/if}
      {#if dg.body}<p class="muted body">{dg.body}</p>{/if}
    </section>
  </div>
  <footer class="foot muted">
    <span>Cards work here. The Overwatch joins in a later release.</span>
    <span class="keys"><Kbd keys="⌘+L" /> search</span>
  </footer>
  <div class="sr" aria-live="polite">{announce}</div>
</aside>

<style>
  .chat {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    min-height: 0;
    background: var(--color-surface-chrome);
    border-left: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-label);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    height: 40px;
    flex: none;
    padding: 0 var(--space-md);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  .title {
    margin: 0;
    font: var(--text-body-strong);
  }
  .day,
  .iconbtn {
    height: 24px;
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    background: var(--color-surface-pane);
    color: var(--color-text-primary);
    font: var(--text-label);
  }
  .iconbtn {
    width: 24px;
    padding: 0;
  }
  .search {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .search .field {
    flex: 1;
    min-width: 0;
  }
  .filters {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    height: 32px;
    flex: none;
    padding: 0 var(--space-md);
    overflow: hidden;
  }
  .strip {
    margin: 0;
    flex: none;
    padding: var(--space-xs) var(--space-md);
    background: var(--color-surface-pane);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  .feed {
    flex: 1;
    min-height: 0;
    overflow: hidden auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-md);
    outline: none;
  }
  /* 8 px between cards, 4 px after a line (Penpot 04.3). */
  .feed > :global(article.line) {
    margin-bottom: calc(var(--space-xs) - var(--space-sm));
  }
  .morning {
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-pane);
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
  }
  .stop {
    flex: none;
    border-radius: var(--radius-md);
  }
  .fold {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    height: var(--size-row);
    padding: 0 var(--space-sm);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
    font: var(--text-label);
    text-align: left;
  }
  .fold.warn {
    border-color: var(--color-status-waiting-for-you);
    color: var(--color-status-waiting-for-you);
  }
  .fold.sel,
  .dh.sel {
    box-shadow: 0 0 0 var(--border-width-focus) var(--color-focus);
  }
  .empty {
    margin: 0;
    padding: var(--space-sm) var(--space-xs);
    color: var(--color-text-secondary);
  }
  .digest {
    display: flex;
    flex-direction: column;
    margin-top: var(--space-sm);
    padding-top: var(--space-sm);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
  }
  .dh {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    padding: var(--space-xs);
  }
  .digest :global(.row) {
    padding-left: var(--space-xs);
    padding-right: var(--space-xs);
  }
  .more {
    align-self: flex-start;
    padding: var(--space-xs);
  }
  .body {
    margin: 0;
    padding: var(--space-xs);
  }
  .foot {
    display: flex;
    gap: var(--space-sm);
    justify-content: space-between;
    align-items: flex-start;
    min-height: 36px;
    flex: none;
    padding: var(--space-xs) var(--space-md);
    border-top: var(--border-width-default) solid var(--color-border-subtle);
    font: var(--text-caption);
  }
  .keys {
    flex: none;
  }
  .grow {
    flex: 1;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  /* The focus ring shows while the chat has the focus; its header takes the accent (notes §10.3). */
  .chat:not(:focus-within) :global(.card.selected:not(.marked)),
  .chat:not(:focus-within) .sel {
    box-shadow: none;
  }
  /* Marked keeps its accent bar; the ring goes. */
  .chat:not(:focus-within) :global(.card.selected.marked) {
    box-shadow: inset 3px 0 0 var(--color-accent-default);
  }
  .chat:focus-within .head {
    box-shadow: inset 0 -2px 0 var(--color-accent-default);
  }
</style>
