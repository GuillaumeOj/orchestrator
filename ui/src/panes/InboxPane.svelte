<script>
  import AgentChip from '../components/AgentChip.svelte';
  import ContextBlocks from '../components/ContextBlocks.svelte';
  import Kbd from '../components/Kbd.svelte';
  import LifecycleMarker from '../components/LifecycleMarker.svelte';
  import OutcomeRow from '../components/OutcomeRow.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  // Inbox (FR-707, §9.6, Penpot 04 Inbox and 06 J5) over the core's Decisions
  // (P4.1, api-contract §2): levels 1–3 oldest first, each with its subject,
  // context blocks, lifecycle marker and actions; level 4 is today's outcome
  // digest with Clear. Keys: j/k move, a/r/⏎/o the decision's action with that
  // key, e open in nvim, x select (batch), u undo, c clear the digest, esc back.
  import {
    app,
    confirm,
    coreAct,
    decide,
    featureBySlug,
    openAgent,
    openBoard,
    openEditor,
    openFeature,
    openReport,
    openUrl,
    toast,
    undoLast,
  } from '../lib/app.svelte.js';
  import { ago } from '../lib/format.js';
  import {
    actionByKey,
    afterRemoval,
    agentOf,
    answerText,
    blockOf,
    FILTERS,
    intentFor,
    keyIntent,
    keyless,
    LEVELS,
    move,
    parked,
    restarted,
    subjectLine,
    visibleItems,
  } from '../lib/inbox.js';
  import { errText } from '../lib/ipc.js';
  import { clearDigest, counts, digest, statusOf } from '../lib/now.svelte.js';
  import { decisionKind } from '../lib/status.js';

  let filter = $state('all');
  let selected = $state(null);
  let marked = $state(new Set());
  let reply = $state(null); // { item, action, text }
  let choices = $state([]); // the options block's chosen labels, per question
  let outcome = $state(null); // the selected digest row's id
  let root;

  // Open and parked decisions, and those that failed or went stale (their
  // marker says why; the core keeps them 10 min). Oldest first per level.
  const SHOWN = new Set(['open', 'parked', 'failed', 'stale']);
  const all = $derived(
    Object.values(app.now.decisions)
      .filter((d) => SHOWN.has(d.lifecycle?.state) && d.level < 4)
      .sort((x, y) => x.level - y.level || x.created_ms - y.created_ms),
  );
  const items = $derived(visibleItems(all, filter, app.pending));
  const sel = $derived(items.find((i) => i.id === selected) ?? items[0] ?? null);
  const levels = $derived([1, 2, 3].map((l) => ({ l, rows: items.filter((i) => i.level === l) })).filter((g) => g.rows.length));
  const outcomes = $derived(digest());
  const doneToday = $derived(counts().done_today);
  const options = $derived(blockOf(sel, 'options'));
  // Answer ⏎ is enabled when every question has a choice (notes §7 row 4).
  const answered = $derived(!!options && options.questions.every((_, i) => choices[i]?.length));
  // A stale or failed decision: Dismiss and, failed, the kind's actions again (notes §7 rows 19–20).
  const live = $derived(sel?.lifecycle?.state !== 'stale');

  // A new selection starts with no choices (by id: a push re-upserting the
  // same decision keeps them); the waiting-on chip's link selects one.
  const selId = $derived(sel?.id ?? null);
  $effect(() => {
    selId;
    choices = [];
  });
  $effect(() => {
    if (app.inboxFocus) {
      selected = app.inboxFocus;
      outcome = null;
      app.inboxFocus = null;
    }
  });

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

  function apply(intent, d) {
    if (!intent) return false;
    switch (intent.kind) {
      case 'move':
        selected = move(items, sel?.id, intent.delta);
        outcome = null;
        queueMicrotask(() => root?.querySelector('.item.sel')?.scrollIntoView({ block: 'nearest' }));
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
        // x-marked decisions take a/r together when each has that action.
        const key = d.actions?.find((x) => x.id === intent.action)?.key;
        const targets = marked.size ? items.filter((i) => marked.has(i.id) && i.lifecycle?.state !== 'stale' && actionByKey(i, key)?.id === intent.action) : [d];
        for (const t of targets) send(t, intent.action);
        marked = new Set();
        return true;
      }
      case 'core':
        coreAct(d, intent.action);
        return true;
      case 'answer':
        if (options) {
          if (answered) send(d, 'answer', answerText(choices));
          else toast('Pick an answer for every question');
        } else reply = { item: d, action: 'answer', text: '' };
        return true;
      case 'reply':
        reply = { item: d, action: 'reply', text: '' };
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

  /** Decide (undo window); the next decision is selected. */
  function send(d, action, message = null) {
    selected = afterRemoval(items, d.id);
    decide(d, action, message);
  }

  // A keyless action (Always allow, Restart fresh, Delete…). Delete asks first:
  // it kills the agent and hides it (its worktree stays).
  function act(d, id) {
    if (id === 'delete')
      return confirm(`Delete ${agentOf(d)}?`, 'Kills it and removes it from the app; its worktree stays.', () => send(d, id), 'Delete');
    apply(intentFor(d, id), d);
  }

  function onKey(e) {
    if (reply || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.target.closest('input, textarea')) return;
    const key = e.key === 'Enter' ? 'enter' : e.key.length === 1 ? e.key : e.key.toLowerCase();
    if (key === 'c' && outcomes.length) {
      e.preventDefault();
      clear();
      return;
    }
    const o = key === 'o' && outcome ? outcomes.find((x) => x.id === outcome) : null;
    if (o) {
      e.preventDefault();
      openOutcome(o);
      return;
    }
    const intent = keyIntent(key, sel);
    // A stale decision takes no action but Dismiss (its buttons say so too).
    if (!live && intent && !NAV.has(intent.kind)) return;
    if (apply(intent, sel)) e.preventDefault();
  }
  const NAV = new Set(['move', 'mark', 'undo', 'editor']);

  const clear = () => {
    outcome = null;
    return clearDigest(outcomes).catch((err) => toast(errText(err), { kind: 'error' }));
  };

  function sendReply(e) {
    if (e.key === 'Escape') {
      reply = null;
      root?.focus();
      e.preventDefault();
    } else if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      const r = reply;
      reply = null;
      if (!r.text.trim()) return;
      send(r.item, r.action, r.text);
      root?.focus();
    }
  }

  const NOTES = {
    permission: 'Allowing answers its permission prompt for this one call. Deny with a message to tell Claude what to do instead.',
    'gate-spec': 'Approving lets the lead create the tasks and the team start. Send back with notes: the lead revises and asks again.',
    'gate-pr': 'Approving pushes the branch with your credentials and opens the pull request with gh; agents never push.',
    startup: 'Open the pane to answer what it shows, or restart it fresh: a new session with its brief and task.',
  };
  const note = (d) => NOTES[d.kind === 'gate' ? `gate-${d.gate}` : d.kind];
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div class="inbox" tabindex="0" data-pane-focus bind:this={root} onkeydown={onKey} role="application" aria-label="Inbox">
  <div class="toolbar">
    {#each FILTERS as f (f.id)}
      {@const n = visibleItems(all, f.id, app.pending).length}
      {#if f.id === 'all' || n}<button class="filter-chip" class:on={filter === f.id} onclick={() => (filter = f.id)}>{f.label} {n}</button>{/if}
    {/each}
    {#if marked.size}<span class="secondary">{marked.size} selected · a/r apply to all</span>{/if}
    <span class="grow"></span>
    <span class="keys"><Kbd keys="j/k" /> move <Kbd keys="a" /> approve <Kbd keys="r" /> reject <Kbd keys="⏎" /> reply <Kbd keys="o" /> open <Kbd keys="x" /> select <Kbd keys="u" /> undo</span>
  </div>
  <div class="split">
    <div class="list" role="listbox" aria-label="Decisions">
      {#each levels as g (g.l)}
        <div class="lh"><span>{g.l} · {LEVELS[g.l]} {g.rows.length}</span><span class="muted">{g.l === 3 ? 'default applies on timeout' : 'oldest first'}</span></div>
        {#each g.rows as it (it.id)}
          {@const k = decisionKind(it)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div class="item" class:sel={sel?.id === it.id} class:marked={marked.has(it.id)} class:stale={it.lifecycle?.state === 'stale'} role="option" tabindex="-1" aria-selected={sel?.id === it.id} onclick={() => ((selected = it.id), (outcome = null))}>
            <div class="top">
              <span class="kind" style:color={k.color} title={k.label} aria-hidden="true">{k.icon}</span>
              <span class="muted">{k.label}{parked(it) ? ' · parked' : ''}{restarted(it) ? ' · after restart' : ''}</span>
              {#if agentOf(it)}<AgentChip id={agentOf(it)} />{:else}<span class="secondary">{it.subject?.project ?? 'Governor'}</span>{/if}
              <span class="grow"></span>
              <span class="muted">{ago(it.created_ms)}</span>
            </div>
            <div class="title">{it.title}</div>
            {#if it.summary}<div class="muted ell">{it.summary}</div>{/if}
            {#if sel?.id === it.id}
              <div class="acts">
                {#each it.actions.filter((x) => x.key) as x (x.id)}<span><Kbd keys={x.key === 'enter' ? '⏎' : x.key} /> {x.label}</span>{/each}
              </div>
            {/if}
          </div>
        {/each}
      {/each}
      {#if !levels.length}
        <div class="empty">
          <p><b>Inbox clear.</b></p>
          <p class="muted">Agents that need you show up here first; the digest below keeps what got done today.</p>
        </div>
      {/if}
      <div class="lh digest">
        <span>4 · Today{outcomes.length ? ` · ${outcomes.length} outcome${outcomes.length === 1 ? '' : 's'}` : ''}</span>
        {#if outcomes.length}<button class="link" onclick={clear}>Clear <Kbd keys="c" /></button>{/if}
      </div>
      {#each outcomes as o (o.id)}
        <OutcomeRow outcome={o} selected={outcome === o.id} onselect={() => (outcome = o.id)} />
      {:else}
        <p class="muted pad-sm">{doneToday ? `All caught up · ${doneToday} done today` : 'Nothing done yet today.'}</p>
      {/each}
    </div>
    <div class="detail" class:muted={sel && !live}>
      {#if sel}
        {@const k = decisionKind(sel)}
        <div class="dh">
          <span class="kind" style:color={k.color} aria-hidden="true">{k.icon}</span>
          <span>{k.label}</span>
          <span class="secondary subject">{subjectLine(sel)}</span>
          <span class="grow"></span>
          {#if agentOf(sel)}<StatusBadge state={statusOf(agentOf(sel))} />{/if}
        </div>
        <div class="dtitle">{sel.title}</div>
        <LifecycleMarker decision={sel} pending={app.pending.find((p) => p.item === sel.id)} />
        {#if sel.summary}<p class="summary">{sel.summary}</p>{/if}
        <ContextBlocks decision={sel} bind:choices />
        {#if restarted(sel)}
          <p class="muted note">After restart: this was still waiting when the core restarted. Your answer reaches {agentOf(sel) ?? 'the agent'} as a turn.</p>
        {/if}
        {#if note(sel)}<p class="muted note">{note(sel)}</p>{/if}
        {#if reply}
          <div class="reply">
            <!-- svelte-ignore a11y_autofocus -->
            <textarea class="field" rows="3" autofocus bind:value={reply.text} onkeydown={sendReply} placeholder={reply.item.kind === 'permission' ? 'Tell Claude what to do instead…' : `Message to ${agentOf(reply.item) ?? 'the agent'}…`}></textarea>
            <span class="muted"><Kbd keys="⏎" /> send · <Kbd keys="⇧+⏎" /> new line · <Kbd keys="esc" /> cancel</span>
          </div>
        {/if}
        <div class="actions">
          {#if live}
            {#each sel.actions.filter((x) => x.key) as x (x.id)}
              <button
                class="btn"
                class:primary={x.key === 'a' || (x.id === 'answer' && answered)}
                class:danger={x.key === 'r' && x.id === 'reject'}
                class:ghost={x.key === 'o'}
                disabled={x.id === 'answer' && options && !answered}
                onclick={() => apply(keyIntent(x.key, sel), sel)}>{x.label} <Kbd keys={x.key === 'enter' ? '⏎' : x.key} /></button
              >
            {/each}
            {#each keyless(sel) as x (x.id)}
              <button
                class="btn"
                class:primary={x.id === 'restart_fresh'}
                class:danger={x.id === 'delete'}
                class:ghost={x.id === 'in_pane' || x.id === 'dismiss'}
                title={x.id === 'always' ? sel.payload?.always : null}
                onclick={() => act(sel, x.id)}>{x.label}</button
              >
            {/each}
          {:else}
            <button class="btn ghost" onclick={() => act(sel, 'dismiss')}>Dismiss</button>
          {/if}
        </div>
      {:else}
        <p class="muted pad">Nothing selected.</p>
      {/if}
    </div>
  </div>
</div>

<style>
  .inbox {
    container-type: inline-size;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    outline: none;
    font: var(--text-label);
  }
  .split {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .list {
    width: 40%;
    min-width: 320px;
    overflow: auto;
    border-right: var(--border-width-default) solid var(--color-border-subtle);
  }
  .lh {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-chrome);
    font: var(--text-body-strong);
  }
  .lh .muted,
  .lh .link {
    font: var(--text-label);
  }
  .item {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    padding: var(--space-sm) var(--space-md);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  .item.sel {
    background: var(--color-surface-selected);
    box-shadow: inset 0 0 0 var(--border-width-focus) var(--color-accent-default);
  }
  .item.marked {
    box-shadow: inset 3px 0 0 var(--color-accent-default);
  }
  .item.stale {
    color: var(--color-text-muted);
  }
  .kind {
    width: var(--size-icon);
    text-align: center;
  }
  .top,
  .acts {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    flex-wrap: wrap;
  }
  .title {
    font: var(--text-body-strong);
  }
  .ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .detail {
    flex: 1;
    min-width: 0;
    overflow: hidden auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-md) var(--space-lg);
  }
  .dh {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    min-width: 0;
  }
  .subject {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dtitle {
    font: var(--text-title);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
    margin-top: auto;
    padding-top: var(--space-sm);
  }
  .summary {
    margin: 0;
    font: var(--text-body);
  }
  .reply {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .digest {
    margin-top: var(--space-md);
  }
  .empty,
  .pad {
    padding: var(--space-xl);
  }
  .pad-sm {
    margin: 0;
    padding: var(--space-sm) var(--space-md);
  }
  .note {
    margin: 0;
  }
  @container (max-width: 760px) {
    .split {
      flex-direction: column;
    }
    .list {
      width: auto;
      min-width: 0;
      max-height: 45%;
      border-right: 0;
      border-bottom: var(--border-width-default) solid var(--color-border-subtle);
    }
  }
</style>
