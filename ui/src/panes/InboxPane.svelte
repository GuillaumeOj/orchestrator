<script>
  import AgentChip from '../components/AgentChip.svelte';
  import Kbd from '../components/Kbd.svelte';
  import Markdown from '../components/Markdown.svelte';
  import PermissionCall from '../components/PermissionCall.svelte';
  import StatusBadge from '../components/StatusBadge.svelte';
  // Inbox (FR-707, §9.6, Penpot 04 Inbox and 06 J5): one list in attention-ladder
  // order with a who/what/why preview. Keys: j/k move, a approve, r reject,
  // ⏎ reply, o open the agent pane, e open in nvim, x select (batch), u undo, esc back.
  import { agent, alertAct, app, confirm, decide, openAgent, openEditor, toast, undoLast } from '../lib/app.svelte.js';
  import { ago } from '../lib/format.js';
  import {
    actionLabels,
    afterRemoval,
    FILTERS,
    gateKind,
    keyIntent,
    kindLabel,
    LEVELS,
    legacyItem,
    move,
    parked,
    restarted,
    tailLines,
    teamQuestion,
    visibleItems,
  } from '../lib/inbox.js';
  import { invoke } from '../lib/ipc.js';
  import { openDecisions, statusOf } from '../lib/now.svelte.js';
  import { decisionKind } from '../lib/status.js';

  let filter = $state('all');
  let selected = $state(null);
  let marked = $state(new Set());
  let reply = $state(null); // { item, text }
  let root;

  // P4.1: Decisions from the core's `now`, read as P4.0 items until F2 renders them.
  const all = $derived(openDecisions().map(legacyItem));
  const items = $derived(visibleItems(all, filter, app.pending));
  const sel = $derived(items.find((i) => i.id === selected) ?? items[0] ?? null);
  const levels = $derived([1, 2, 3, 4].map((l) => ({ l, rows: items.filter((i) => i.level === l) })).filter((g) => g.rows.length));
  const labels = $derived(actionLabels(sel));
  const a = $derived(sel?.agent_id ? agent(sel.agent_id) : null);

  const KIND = {
    permission: { state: 'waiting_for_you' },
    startup: { state: 'error' },
    gate: { state: 'waiting_gate' },
    question: { state: 'waiting_for_you' },
    trust: { state: 'waiting_for_you' },
    error: { state: 'error' },
    alert: { state: 'error' },
    budget: { state: 'queued' },
    info: { state: 'done' },
  };

  // Actions with no key: buttons in the detail only (api-contract §1).
  const BUTTONS = ['always', 'in_pane', 'restart_fresh', 'dismiss', 'delete'];

  // ⏎'s label: a plain question is answered in the pane, a permission denied with a message.
  const enterLabel = (it) => actionLabels(it).enter ?? (it.kind === 'question' && !teamQuestion(it) ? 'Answer in the pane' : 'Reply');

  // A gate's preview (the spec docs, or the PR's changed files), loaded once per
  // item. Other gates (a hold back after a restart) carry their text.
  let gate = $state(null); // { id, data?, error?, doc }
  $effect(() => {
    const it = sel;
    if (it?.kind !== 'gate' || gateKind(it) === 'other' || gate?.id === it.id) return;
    gate = { id: it.id, data: null, error: null, doc: 0 };
    invoke('gate_detail', { item: it.id })
      .then((d) => {
        if (gate?.id === it.id) gate = { ...gate, data: d };
      })
      .catch((e) => {
        if (gate?.id === it.id) gate = { ...gate, error: String(e) };
      });
  });

  /** A link to a sibling doc in the spec gate switches the doc tab. */
  function openGateDoc(name) {
    const i = gate?.data?.docs?.findIndex((d) => d.name === name) ?? -1;
    if (i >= 0) gate = { ...gate, doc: i };
    else toast(`No doc named ${name} in this gate`, { kind: 'error' });
  }

  function apply(intent, item) {
    if (!intent) return false;
    switch (intent.kind) {
      case 'move':
        selected = move(items, sel?.id, intent.delta);
        queueMicrotask(() => root?.querySelector('.item.sel')?.scrollIntoView({ block: 'nearest' }));
        return true;
      case 'mark': {
        const m = new Set(marked);
        m.has(item.id) ? m.delete(item.id) : m.add(item.id);
        marked = m;
        return true;
      }
      case 'undo':
        if (!undoLast()) toast('Nothing to undo');
        return true;
      case 'decide': {
        const targets = marked.size ? items.filter((i) => marked.has(i.id) && keyIntent(intent.action === 'approve' ? 'a' : 'r', i)) : [item];
        for (const t of targets) {
          selected = afterRemoval(items, t.id);
          decide(t, intent.action);
        }
        marked = new Set();
        return true;
      }
      case 'alert':
        alertAct(item, intent.action);
        return true;
      case 'reply':
        if ((item.kind === 'question' && !teamQuestion(item)) || item.kind === 'trust') {
          if (item.agent_id) openAgent(item.agent_id);
        } else reply = { item, text: '' };
        return true;
      case 'open':
        if (item.agent_id) openAgent(item.agent_id);
        return true;
      case 'editor':
        openEditor(null, null, { agentId: item.agent_id });
        return true;
    }
    return false;
  }

  // A keyless action (Always allow, Restart fresh…): the next item is selected, as for a/r.
  // "Answer in the pane" only opens it: the dialog waits there, the core decides nothing.
  // Delete asks first: it kills the agent and hides it (its worktree stays; the
  // agent menu's Delete… is the one that can remove it).
  function act(item, action) {
    if (action === 'in_pane') return item.agent_id && openAgent(item.agent_id);
    const go = () => {
      selected = afterRemoval(items, item.id);
      decide(item, action);
    };
    if (action === 'delete') confirm(`Delete ${item.agent_id}?`, 'Kills it and removes it from the app; its worktree stays.', go, 'Delete');
    else go();
  }

  function onKey(e) {
    if (reply || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.target.closest('input, textarea')) return;
    const key = e.key === 'Enter' ? 'enter' : e.key.length === 1 ? e.key : e.key.toLowerCase();
    if (apply(keyIntent(key, sel), sel)) e.preventDefault();
  }

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
      selected = afterRemoval(items, r.item.id);
      // A permission: deny with the reply (Claude's "tell Claude what to do differently").
      decide(r.item, r.item.kind === 'permission' ? 'reject' : 'reply', r.text);
      root?.focus();
    }
  }
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
    <div class="list" role="listbox" aria-label="Pending items">
      {#each levels as g (g.l)}
        <div class="lh"><span>{g.l} · {LEVELS[g.l]} {g.rows.length}</span><span class="muted">{g.l === 4 ? 'digest, silent' : g.l === 3 ? 'default applies on timeout' : 'oldest first'}</span></div>
        {#each g.rows as it (it.id)}
          {@const k = decisionKind(it.decision)}
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div class="item" class:sel={sel?.id === it.id} class:marked={marked.has(it.id)} role="option" tabindex="-1" aria-selected={sel?.id === it.id} onclick={() => (selected = it.id)}>
            <div class="top">
              <span class="kind" style:color={k.color} title={k.label} aria-hidden="true">{k.icon}</span>
              <span class="muted">{kindLabel(it)}{parked(it) ? ' · parked' : ''}{restarted(it) ? ' · after restart' : ''}</span>
              {#if it.agent_id}<AgentChip id={it.agent_id} />{:else}<span class="secondary">Governor</span>{/if}
              <span class="grow"></span>
              <span class="muted">{ago(it.created_ms)}</span>
            </div>
            <div class="title">{it.title}</div>
            {#if it.preview}<div class="mono prev">{it.preview.split('\n').slice(-2).join('\n')}</div>{:else}<div class="muted">{it.detail}</div>{/if}
            {#if sel?.id === it.id && it.level < 4}
              <div class="acts">
                {#if labels.a && it.actions.includes('a')}<span><Kbd keys="a" /> {labels.a}</span>{/if}
                {#if labels.r && it.actions.includes('r')}<span><Kbd keys="r" /> {labels.r}</span>{/if}
                {#if it.actions.includes('enter')}<span><Kbd keys="⏎" /> {enterLabel(it)}</span>{/if}
                {#if it.actions.includes('o')}<span><Kbd keys="o" /> {labels.o ?? 'Open'}</span>{/if}
              </div>
            {/if}
          </div>
        {/each}
      {:else}
        <div class="empty">
          <p><b>Inbox clear.</b></p>
          <p class="muted">Agents that need you show up here first; the info digest keeps the history.</p>
        </div>
      {/each}
    </div>
    <div class="detail">
      {#if sel}
        <div class="dh">
          <StatusBadge state={KIND[sel.kind]?.state ?? 'idle'} compact />
          <b>{kindLabel(sel)} · {sel.agent_id ?? 'Governor'}</b>
          {#if a}<StatusBadge state={statusOf(a.id)} />{/if}
          <span class="grow"></span>
          <span class="muted">waiting {ago(sel.created_ms)}</span>
        </div>
        <dl>
          <dt>Who</dt><dd>{sel.agent_id ? `${sel.agent_id}${a?.project_id ? ` · ${a.project_id}` : ''}${a?.branch ? ` · branch ${a.branch}` : ''}` : 'Governor'}</dd>
          <dt>What</dt><dd>{sel.title}</dd>
          <dt>Why</dt><dd>{sel.detail}</dd>
        </dl>
        {#if sel.kind === 'gate'}
          {#if gateKind(sel) === 'other'}
            <div class="rendered"><Markdown text={sel.payload?.detail || sel.preview || sel.title} /></div>
          {:else if gate?.error}
            <p class="muted">Could not load the gate's details: {gate.error}</p>
          {:else if !gate?.data}
            <p class="muted">Loading…</p>
          {:else if gateKind(sel) === 'spec'}
            {#if gate.data.summary}<p class="summary">{gate.data.summary}</p>{/if}
            <div class="doc-tabs" role="tablist">
              {#each gate.data.docs as d, i (d.name)}
                <button class="tab" class:on={gate.doc === i} role="tab" aria-selected={gate.doc === i} onclick={() => (gate = { ...gate, doc: i })}>{d.name}</button>
              {/each}
              <span class="grow"></span>
              <span class="muted">{gate.data.docs.length} file{gate.data.docs.length === 1 ? '' : 's'}</span>
            </div>
            {#if gate.data.docs[gate.doc]}<div class="rendered"><Markdown text={gate.data.docs[gate.doc].text} feature={gate.data.feature_id ?? null} ondoc={openGateDoc} /></div>{/if}
          {:else}
            <dl>
              <dt>Branch</dt><dd class="mono">{gate.data.branch} → {gate.data.base}</dd>
              <dt>Title</dt><dd>{gate.data.title}</dd>
            </dl>
            {#if gate.data.body}<div class="rendered"><Markdown text={gate.data.body} /></div>{/if}
            <div class="files">
              <div class="muted">{gate.data.files.length} changed file{gate.data.files.length === 1 ? '' : 's'}{gate.data.truncated ? ' (diff cut)' : ''}</div>
              {#each gate.data.files as f (f.path)}
                <div class="mono file"><span class="st">{f.status}</span> {f.path}</div>
              {/each}
            </div>
          {/if}
        {:else if teamQuestion(sel)}
          <div class="rendered"><Markdown text={sel.payload.question} /></div>
          {#if sel.payload.options?.length}
            <div class="opts">
              {#each sel.payload.options as o (o)}<button class="btn ghost" onclick={() => decide(sel, 'reply', o)}>{o}</button>{/each}
            </div>
          {/if}
        {:else if sel.kind === 'permission'}
          <PermissionCall item={sel} />
        {:else if sel.kind === 'startup'}
          {@const tail = tailLines(sel.payload?.tail) || sel.preview}
          {#if tail}<pre class="mono preview">{tail}</pre>{/if}
        {:else if sel.preview}<pre class="mono preview">{sel.preview}</pre>{/if}
        {#if restarted(sel)}
          <p class="muted note">After restart: this was still waiting when the core restarted. Your answer reaches {sel.agent_id ?? 'the agent'} as a turn.</p>
        {/if}
        {#if gateKind(sel) === 'spec'}
          <p class="muted note">Approving lets the lead create the tasks and the team start. Send back (or reply) with what to change: the lead revises and asks again.</p>
        {:else if gateKind(sel) === 'pr'}
          <p class="muted note">Approving pushes the branch with your credentials and opens the pull request with gh; agents never push.</p>
        {/if}
        {#if sel.kind === 'permission'}
          <p class="muted note">
            {parked(sel) ? 'Allowing lets this one call run when the agent retries it.' : "Allowing answers Claude Code's permission prompt for this one call."}
            Deny with a message to tell Claude what to do instead.
            {#if sel.actions.includes('always')}"Always allow" adds the rule <span class="mono">{sel.payload.always}</span>.{/if}
            {#if sel.actions.includes('in_pane')}"Answer in the pane" shows its own dialog.{/if}
          </p>
        {:else if sel.kind === 'startup'}
          <p class="muted note">Open the pane to answer what it shows, or restart it fresh: a new session with its brief and task.</p>
        {/if}
        {#if reply}
          <div class="reply">
            <!-- svelte-ignore a11y_autofocus -->
            <textarea class="field" rows="3" autofocus bind:value={reply.text} onkeydown={sendReply} placeholder={reply.item.kind === 'permission' ? 'Tell Claude what to do instead…' : `Message to ${reply.item.agent_id}…`}></textarea>
            <span class="muted"><Kbd keys="⏎" /> send · <Kbd keys="⇧+⏎" /> new line · <Kbd keys="esc" /> cancel</span>
          </div>
        {/if}
        <div class="actions">
          {#if labels.a && sel.actions.includes('a')}<button class="btn primary" onclick={() => apply(keyIntent('a', sel), sel)}>{labels.a} <Kbd keys="a" /></button>{/if}
          {#if labels.r && sel.actions.includes('r')}<button class="btn danger" onclick={() => apply({ kind: 'decide', action: 'reject' }, sel)}>{labels.r} <Kbd keys="r" /></button>{/if}
          {#if sel.actions.includes('enter')}<button class="btn" onclick={() => apply({ kind: 'reply' }, sel)}>{enterLabel(sel)} <Kbd keys="⏎" /></button>{/if}
          {#each BUTTONS.filter((b) => sel.actions.includes(b) && labels[b]) as b (b)}
            <button
              class="btn"
              class:primary={b === 'restart_fresh'}
              class:danger={b === 'delete'}
              class:ghost={b === 'in_pane' || b === 'dismiss'}
              title={b === 'always' ? sel.payload?.always : null}
              onclick={() => act(sel, b)}>{labels[b]}</button
            >
          {/each}
          {#if sel.actions.includes('o')}<button class="btn ghost" onclick={() => apply(keyIntent('o', sel), sel)}>{labels.o ?? 'Open'} <Kbd keys="o" /></button>{/if}
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
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-chrome);
    font: var(--text-body-strong);
  }
  .lh .muted {
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
  .kind {
    width: var(--size-icon);
    text-align: center;
  }
  .top,
  .acts {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .title {
    font: var(--text-body-strong);
  }
  .prev {
    white-space: pre-wrap;
    color: var(--color-text-secondary);
    max-height: 3em;
    overflow: hidden;
  }
  .detail {
    flex: 1;
    min-width: 0;
    overflow: hidden auto;
    display: flex;
    flex-direction: column;
    gap: var(--space-md);
    padding: var(--space-md) var(--space-lg);
  }
  .dh {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    font: var(--text-title);
  }
  dl {
    display: grid;
    grid-template-columns: 96px 1fr;
    gap: var(--space-xs) var(--space-md);
    margin: 0;
  }
  dt {
    color: var(--color-text-muted);
  }
  dd {
    margin: 0;
  }
  .preview {
    margin: 0;
    padding: var(--space-md);
    background: var(--color-surface-chrome);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    white-space: pre-wrap;
    user-select: text;
    -webkit-user-select: text;
    max-height: 50vh;
    overflow: auto;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
    margin-top: auto;
  }
  .summary {
    margin: 0;
    font: var(--text-body);
  }
  .doc-tabs {
    display: flex;
    align-items: center;
    gap: 0;
  }
  .tab {
    padding: var(--space-xxs) var(--space-sm);
    background: none;
    border: var(--border-width-default) solid var(--color-border-subtle);
    color: var(--color-text-secondary);
    font: var(--text-label);
    cursor: pointer;
  }
  .tab + .tab {
    border-left: 0;
  }
  .tab.on {
    background: var(--color-surface-selected);
    color: var(--color-text-primary);
  }
  .rendered {
    padding: var(--space-md) var(--space-lg);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    max-height: 50vh;
    overflow: auto;
  }
  .files {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
  }
  .file .st {
    display: inline-block;
    width: 1.5em;
    color: var(--color-text-muted);
  }
  .opts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }
  .reply {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .empty,
  .pad {
    padding: var(--space-xl);
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
