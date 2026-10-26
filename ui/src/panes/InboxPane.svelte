<script>
  // Inbox (FR-707, §9.6, Penpot 04 Inbox and 06 J5): one list in attention-ladder
  // order with a who/what/why preview. Keys: j/k move, a approve, r reject,
  // ⏎ reply, o open the agent pane, e open in nvim, x select (batch), u undo, esc back.
  import { app, agent, openAgent, openEditor, decide, undoLast, toast } from '../lib/app.svelte.js';
  import { FILTERS, LEVELS, visibleItems, move, afterRemoval, keyIntent, actionLabels } from '../lib/inbox.js';
  import { ago } from '../lib/format.js';
  import { displayState } from '../lib/status.js';
  import StatusBadge from '../components/StatusBadge.svelte';
  import AgentChip from '../components/AgentChip.svelte';
  import Kbd from '../components/Kbd.svelte';

  let { paneId } = $props();
  let filter = $state('all');
  let selected = $state(null);
  let marked = $state(new Set());
  let reply = $state(null); // { item, text }
  let root;

  const all = $derived(app.data?.inbox?.items ?? []);
  const items = $derived(visibleItems(all, filter, app.pending));
  const sel = $derived(items.find((i) => i.id === selected) ?? items[0] ?? null);
  const levels = $derived([1, 2, 3, 4].map((l) => ({ l, rows: items.filter((i) => i.level === l) })).filter((g) => g.rows.length));
  const labels = $derived(actionLabels(sel));
  const a = $derived(sel?.agent_id ? agent(sel.agent_id) : null);

  const KIND = {
    permission: { label: 'Permission', state: 'waiting_for_you' },
    question: { label: 'Waiting for you', state: 'waiting_for_you' },
    trust: { label: 'Trust dialog', state: 'waiting_for_you' },
    error: { label: 'Error', state: 'error' },
    alert: { label: 'Alert', state: 'error' },
    budget: { label: 'Budget', state: 'queued' },
    info: { label: 'Info', state: 'done' },
  };

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
      case 'reply':
        if (item.kind === 'question' || item.kind === 'trust') {
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
      {#if f.id === 'all' || n}<button class="chip" class:on={filter === f.id} onclick={() => (filter = f.id)}>{f.label} {n}</button>{/if}
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
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div class="item" class:sel={sel?.id === it.id} class:marked={marked.has(it.id)} role="option" tabindex="-1" aria-selected={sel?.id === it.id} onclick={() => (selected = it.id)}>
            <div class="top">
              <StatusBadge state={KIND[it.kind]?.state ?? 'idle'} compact />
              <span class="muted">{KIND[it.kind]?.label ?? it.kind}</span>
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
                {#if it.actions.includes('enter')}<span><Kbd keys="⏎" /> {it.kind === 'question' ? 'Answer in the pane' : 'Reply'}</span>{/if}
                {#if it.actions.includes('o')}<span><Kbd keys="o" /> Open</span>{/if}
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
          <b>{KIND[sel.kind]?.label} · {sel.agent_id ?? 'Governor'}</b>
          {#if a}<StatusBadge state={displayState(a)} />{/if}
          <span class="grow"></span>
          <span class="muted">waiting {ago(sel.created_ms)}</span>
        </div>
        <dl>
          <dt>Who</dt><dd>{sel.agent_id ? `${sel.agent_id}${a?.project_id ? ` · ${a.project_id}` : ''}${a?.branch ? ` · branch ${a.branch}` : ''}` : 'Governor'}</dd>
          <dt>What</dt><dd>{sel.title}</dd>
          <dt>Why</dt><dd>{sel.detail}</dd>
        </dl>
        {#if sel.preview}<pre class="mono preview">{sel.preview}</pre>{/if}
        {#if sel.kind === 'permission'}
          <p class="muted note">Allowing answers Claude Code's permission prompt for this one call (no "always allow" rule). Deny with a reply to tell Claude what to do instead; "Answer in the pane" shows its own dialog.</p>
        {/if}
        {#if reply}
          <div class="reply">
            <!-- svelte-ignore a11y_autofocus -->
            <textarea class="field" rows="3" autofocus bind:value={reply.text} onkeydown={sendReply} placeholder={reply.item.kind === 'permission' ? 'Tell Claude what to do instead…' : `Message to ${reply.item.agent_id}…`}></textarea>
            <span class="muted"><Kbd keys="⏎" /> send · <Kbd keys="⇧+⏎" /> new line · <Kbd keys="esc" /> cancel</span>
          </div>
        {/if}
        <div class="actions">
          {#if labels.a && sel.actions.includes('a')}<button class="btn primary" onclick={() => apply({ kind: 'decide', action: 'approve' }, sel)}>{labels.a} <Kbd keys="a" /></button>{/if}
          {#if labels.r && sel.actions.includes('r')}<button class="btn danger" onclick={() => apply({ kind: 'decide', action: 'reject' }, sel)}>{labels.r} <Kbd keys="r" /></button>{/if}
          {#if sel.actions.includes('enter')}<button class="btn" onclick={() => apply({ kind: 'reply' }, sel)}>{sel.kind === 'question' ? 'Answer in the pane' : 'Reply'} <Kbd keys="⏎" /></button>{/if}
          {#if sel.kind === 'permission'}<button class="btn ghost" onclick={() => decide(sel, 'in_pane')}>Answer in the pane</button>{/if}
          {#if sel.actions.includes('o')}<button class="btn ghost" onclick={() => apply({ kind: 'open' }, sel)}>Open <Kbd keys="o" /></button>{/if}
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
  .toolbar {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--space-sm);
    flex-wrap: wrap;
    padding: var(--space-sm) var(--space-md);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  .chip {
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-full);
    background: none;
    padding: var(--space-xxs) var(--space-sm);
  }
  .chip.on {
    border-color: var(--color-accent-default);
    background: var(--color-accent-subtle);
  }
  .keys {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
    color: var(--color-text-secondary);
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
