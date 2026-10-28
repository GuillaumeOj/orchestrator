<script>
  // Settings › MCP servers (FR-1202, FR-604, p3-design-notes §5.4): the
  // registry (global, with this project's overrides) and an edit drawer by
  // kind. A `keychain:<account>` env value is passed to agents only with the
  // per-entry opt-in (FR-110 tone). Keys: j/k rows, n new, ⏎ edit, t test,
  // x delete; in the drawer ⌘⏎ save, esc close.
  import { onMount } from 'svelte';
  import { confirm, now30, openCanvas, openUrl, runInPane, toast } from '../lib/app.svelte.js';
  import {
    argsToText,
    blankServer,
    checkedAt,
    effectiveServers,
    envToText,
    KINDS,
    keychainAccount,
    kindLabel,
    needsUser,
    reconnectHint,
    secretEnv,
    serverForSave,
    serverProblem,
    textToArgs,
    textToEnv,
    usedByRoles,
    usedByText,
    usedByTitle,
  } from '../lib/ecosystem.js';
  import { errText, invoke } from '../lib/ipc.js';
  import ConnBadge from './ConnBadge.svelte';
  import Kbd from './Kbd.svelte';
  import SecretField from './SecretField.svelte';

  /** `initial`: the Settings pane's mcp_registry_view, taken once (no second fetch on open). */
  /** @type {{ project?: string | null, selected?: string | null, initial?: (() => Promise<any> | null) | null }} */
  let { project = null, selected = null, initial = null } = $props();

  const CONNECTOR_URL = 'https://claude.ai/settings/connectors';

  let view = $state(null);
  let error = $state(null);
  let sel = $state(0);
  let draft = $state(null); // { server, isNew, override, envText, argsText }
  let problem = $state(null);
  let refused = $state(null); // { id, used_by }
  let busy = $state({});
  let stored = $state({}); // Keychain account → stored (mcp_secrets_status); never the value
  const now = $derived(now30());
  let root = $state(null);

  const servers = $derived(effectiveServers(view, project));
  const cur = $derived(servers[sel] ?? null);
  const env = $derived(draft ? textToEnv(draft.envText) : {});
  const secrets = $derived(secretEnv(env));

  async function load(first = null) {
    try {
      view = await (first ?? invoke('mcp_registry_view', { project }));
      error = null;
    } catch (e) {
      error = errText(e);
      view ??= { servers: [], project_overrides: {} };
    }
  }

  /** Banners, the Inbox and Integrations open a server ("design:penpot" = that tool's). */
  function select(want) {
    if (!want) return;
    const tool = want.startsWith('design:') ? want.slice(7) : null;
    const i = servers.findIndex((s) => (tool ? s.design_tool === tool : s.id === want));
    if (i >= 0) {
      sel = i;
      edit(servers[i]);
    } else if (tool) {
      startNew({ id: tool, name: tool === 'penpot' ? 'Penpot' : 'Figma', kind: 'connector', design_tool: tool });
    }
  }

  function edit(s) {
    refused = null;
    problem = null;
    draft = {
      server: { ...blankServer(), ...$state.snapshot(s) },
      isNew: false,
      override: !!(project && s.scope),
      envText: envToText(s.env),
      argsText: argsToText(s.args),
    };
  }

  function startNew(over = {}) {
    refused = null;
    problem = null;
    draft = { server: { ...blankServer(), ...over }, isNew: true, override: false, envText: '', argsText: '' };
  }

  const scopeOf = (override) => (project && override ? { kind: 'project', id: project } : { kind: 'global' });

  async function save() {
    if (!draft) return;
    const s = { ...draft.server, env, args: textToArgs(draft.argsText) };
    problem = serverProblem(s, draft.isNew ? servers.map((x) => x.id) : []);
    if (problem) return;
    try {
      const r = await invoke('mcp_server_save', { server: serverForSave(s), scope: scopeOf(draft.override) });
      toast(`Saved ${r.name}`);
      await load();
      sel = Math.max(0, servers.findIndex((x) => x.id === r.id));
      edit(servers[sel] ?? r);
    } catch (e) {
      problem = errText(e);
    }
  }

  async function test(s) {
    if (!s || busy[s.id]) return;
    busy = { ...busy, [s.id]: true };
    try {
      const r = await invoke('mcp_server_test', { id: s.id, project });
      await load();
      if (draft?.server.id === r.id && !draft.isNew) draft.server = { ...draft.server, status: r.status, detail: r.detail, checked_at_ms: r.checked_at_ms };
      if (r.status === 'connected') toast(`${r.name} connected`);
    } catch (e) {
      toast(`${s.name}: ${errText(e)}`, { kind: 'error' });
    } finally {
      busy = { ...busy, [s.id]: false };
    }
  }

  function remove(s) {
    if (!s) return;
    const scope = scopeOf(s.scope);
    confirm(
      `Delete ${s.name} from the MCP registry?`,
      scope.kind === 'project' ? `The global entry applies to ${project} again.` : 'Roles can no longer reference it.',
      async () => {
        try {
          const r = await invoke('mcp_server_delete', { id: s.id, scope });
          if (r.deleted === false || r.used_by?.length) {
            refused = { id: s.id, name: s.name, used_by: r.used_by };
            return;
          }
          toast(`Deleted ${s.name}`);
          if (draft?.server.id === s.id) draft = null;
          await load();
          sel = Math.min(sel, Math.max(0, servers.length - 1));
        } catch (e) {
          toast(`${s.name}: ${errText(e)}`, { kind: 'error' });
        }
      },
      'Delete',
    );
  }

  /** Reconnect per kind (D3): a pane with claude /mcp, or the connector settings. */
  async function reconnect(s) {
    if (s.kind === 'http' || s.kind === 'sse') return runInPane('claude /mcp', `claude /mcp · ${s.name}`, project);
    if (s.kind === 'connector') return openUrl(CONNECTOR_URL);
  }

  // ---- Keychain values for `keychain:<account>` env refs (their own
  // namespace, apart from the integrations' tokens). Only stored / empty
  // comes back from the core.
  const accounts = $derived(Object.values(env).map(keychainAccount).filter((a) => a != null));
  const accountsKey = $derived([...new Set(accounts)].sort().join('\n'));

  async function loadSecrets(list) {
    if (!list.length) return;
    try {
      const r = await invoke('mcp_secrets_status', { accounts: list });
      stored = { ...stored, ...r };
    } catch {
      // Unknown: the fields read as empty; Save still works.
    }
  }

  $effect(() => {
    const list = accountsKey ? accountsKey.split('\n') : [];
    const t = setTimeout(() => loadSecrets(list), 300);
    return () => clearTimeout(t);
  });

  async function storeSecret(account, value) {
    try {
      await invoke('mcp_secret_set', { account, value });
      stored = { ...stored, [account]: true };
      toast(`${account} stored in Keychain`);
    } catch (e) {
      toast(`${account}: ${errText(e)}. Nothing was stored.`, { kind: 'error' });
    }
  }

  function removeSecret(account) {
    confirm(
      `Remove ${account} from the Keychain?`,
      'Servers that use it fail to start until you store a value again.',
      async () => {
        try {
          await invoke('mcp_secret_delete', { account });
          stored = { ...stored, [account]: false };
          toast(`${account} removed from Keychain`);
        } catch (e) {
          toast(`${account}: ${errText(e)}`, { kind: 'error' });
        }
      },
      'Remove',
    );
  }

  function onKey(e) {
    if (e.metaKey && e.key === 'Enter' && draft) return (e.preventDefault(), save());
    if (e.key === 'Escape' && draft && !e.defaultPrevented) {
      e.preventDefault();
      draft = null;
      root?.focus();
      return;
    }
    if (e.target.closest('input, textarea, select, button') || e.metaKey || e.ctrlKey || e.altKey) return;
    const k = e.key;
    if (k === 'j' || k === 'ArrowDown') sel = Math.min(servers.length - 1, sel + 1);
    else if (k === 'k' || k === 'ArrowUp') sel = Math.max(0, sel - 1);
    else if (k === 'n') startNew();
    else if (k === 'Enter' && cur) edit(cur);
    else if (k === 't') test(draft && !draft.isNew ? draft.server : cur);
    else if (k === 'x') remove(draft && !draft.isNew ? draft.server : cur);
    else return;
    e.preventDefault();
  }

  onMount(() => {
    load(initial?.()).then(() => select(selected));
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<section class="mcp" tabindex="0" data-section-focus bind:this={root} onkeydown={onKey} aria-label="MCP servers">
  <header class="h">
    <h2>MCP servers</h2>
    <span class="muted">Registry of servers that roles pick from. {project ? `Global, with ${project}'s overrides.` : 'Global, with per-project overrides.'}</span>
    <span class="grow"></span>
    <button class="btn" onclick={() => startNew()}>New server <Kbd keys="n" /></button>
  </header>
  {#if error}<p class="err">Could not read the registry: {error}</p>{/if}

  <div class="split">
    <div class="list">
      {#if view && !servers.length}
        <p class="muted empty">No MCP servers yet. Add one with <Kbd keys="n" />, or start a design preset: it adds Penpot.</p>
      {:else if view}
        <table>
          <thead><tr><th>Name</th><th>Kind</th><th>Design tool</th><th>Used by</th><th>Status</th><th>Scope</th></tr></thead>
          <tbody>
            {#each servers as s, i (s.id)}
              <tr class:sel={i === sel} class:editing={draft && !draft.isNew && draft.server.id === s.id} onclick={() => ((sel = i), edit(s))}>
                <td><b>{s.name}</b> <span class="muted mono">{s.id}</span></td>
                <td>{kindLabel(s.kind)}</td>
                <td>{s.design_tool === 'penpot' ? 'Penpot' : s.design_tool === 'figma' ? 'Figma' : '—'}</td>
                <td title={usedByTitle(s.used_by)}>{usedByText(s.used_by)}</td>
                <td>
                  <ConnBadge status={s.status} connector={s.kind === 'connector'} busy={busy[s.id]} />
                  {#if s.status === 'needs_auth' && s.kind !== 'stdio'}<button class="link" onclick={(e) => (e.stopPropagation(), reconnect(s))}>Reconnect</button>{/if}
                </td>
                <td class="muted">{s.scope ? `${s.scope} override` : 'Global'}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        <p class="muted hints"><Kbd keys="j" /><Kbd keys="k" /> move · <Kbd keys="⏎" /> edit · <Kbd keys="t" /> test · <Kbd keys="x" /> delete</p>
      {:else}
        <p class="muted">Loading…</p>
      {/if}
      {#if refused}
        <div class="box errbox" role="alert">
          <span aria-hidden="true">{'\u{F0159}'}</span> {refused.name} is used by {usedByRoles(refused.used_by)}. Remove it from those roles first.
          <div class="roles">
            {#each refused.used_by as u (`${u.team}/${u.node}`)}<button class="link" onclick={() => openCanvas(u.team)}>{u.team} › {u.node}</button>{/each}
          </div>
        </div>
      {/if}
    </div>

    {#if draft}
      {@const s = draft.server}
      <aside class="drawer" aria-label="Edit MCP server">
        <div class="dh">
          <b>{draft.isNew ? 'New MCP server' : s.name}</b>
          {#if !draft.isNew}<ConnBadge status={s.status} connector={s.kind === 'connector'} busy={busy[s.id]} /><span class="muted when">{s.status === 'needs_auth' && s.detail ? `${kindLabel(s.kind)} · ${s.detail}` : checkedAt(s.checked_at_ms, now)}</span>{/if}
          <span class="grow"></span>
          <button class="x" aria-label="Close" onclick={() => (draft = null)}><Kbd keys="esc" /></button>
        </div>
        <div class="grid">
          <label><span>id</span><input class="field mono" bind:value={draft.server.id} disabled={!draft.isNew} spellcheck="false" placeholder="penpot" /></label>
          <label><span>name</span><input class="field" bind:value={draft.server.name} placeholder="Penpot" /></label>
          <div class="kinds" role="radiogroup" aria-label="Kind">
            <span>kind</span>
            <span class="seg">{#each KINDS as k (k.id)}<button role="radio" aria-checked={draft.server.kind === k.id} class:on={draft.server.kind === k.id} onclick={() => (draft.server.kind = k.id)}>{k.label}</button>{/each}</span>
          </div>
          <label><span>design tool</span>
            <select class="field" bind:value={draft.server.design_tool}>
              <option value={null}>—</option><option value="penpot">Penpot</option><option value="figma">Figma</option>
            </select>
          </label>
          {#if s.kind === 'stdio'}
            <label><span>command</span><input class="field mono" bind:value={draft.server.command} spellcheck="false" placeholder="npx" /></label>
            <label class="top"><span>args</span><textarea class="field mono" rows="3" bind:value={draft.argsText} spellcheck="false" placeholder="one per line"></textarea></label>
          {:else if s.kind === 'http' || s.kind === 'sse'}
            <label><span>URL</span><input class="field mono" bind:value={draft.server.url} spellcheck="false" placeholder="https://…" /></label>
            <p class="note muted">OAuth tokens are kept by Claude Code, not by the app.</p>
          {:else}
            <label><span>connector</span><input class="field" bind:value={draft.server.connector_name} placeholder="claude.ai Penpot" /></label>
          {/if}
          {#if s.kind !== 'connector'}
            <label class="top"><span>env</span><textarea class="field mono" rows="3" bind:value={draft.envText} spellcheck="false" placeholder="KEY=value · KEY=keychain:account"></textarea></label>
          {/if}
        </div>

        {#if s.kind !== 'connector'}
          {#each Object.entries(env) as [k, v] (k)}
            {@const account = keychainAccount(v)}
            {#if account != null}
              <div class="kc">
                <span class="chip"><span aria-hidden="true">{'\u{F033E}'}</span> {k} from Keychain: {account}</span>
                <SecretField stored={!!stored[account]} placeholder="Paste the value to store as {account}" onsave={(val) => storeSecret(account, val)} onremove={() => removeSecret(account)} />
              </div>
            {/if}
          {/each}
        {/if}

        {#if s.kind === 'connector'}
          <div class="box info">A claude.ai connector comes from your claude.ai login, not from this app. It can't be passed in --mcp-config, so roles that use it run without --strict-mcp-config: they also see your other claude.ai connectors. Its status reads 'Not seen yet' until an agent uses it.</div>
        {/if}

        {#if secrets.length && s.kind !== 'connector'}
          <div class="box warnbox">
            <p><b>Pass this secret to the agent.</b> Agents using {s.id || 'this server'} get {secrets.join(', ')} in their MCP config. The agent can read this token and send it to the model, and a prompt injection could make it leak. Prefer an OAuth (http) server, whose tokens Claude Code keeps itself.</p>
            <label class="ck"><input type="checkbox" bind:checked={draft.server.allow_secret_env} /> I understand, pass it to agents</label>
          </div>
          {#if !draft.server.allow_secret_env}<p class="box errbox"><span aria-hidden="true">{'\u{F0159}'}</span> {s.id || s.name} uses a Keychain secret in its env, but "pass it to agents" is off. Save works; Test and launch fail until you opt in.</p>{/if}
        {/if}

        {#if !draft.isNew && needsUser(s.status)}
          {@const h = reconnectHint(s)}
          <div class="box warnbox">
            <p>{h.text}</p>
            {#if h.action}<button class="btn" onclick={() => reconnect(s)}>{h.action}</button>{/if}
          </div>
        {/if}

        {#if project}
          <label class="ck"><input type="checkbox" bind:checked={draft.override} disabled={!draft.isNew && !!s.scope} /> Override for {project}</label>
        {/if}
        {#if problem}<p class="err">{problem}</p>{/if}
        <div class="foot">
          {#if !draft.isNew}
            <button class="btn ghost" onclick={() => test(s)} disabled={busy[s.id]}>{busy[s.id] ? 'Testing…' : 'Test'} <Kbd keys="t" /></button>
            <button class="btn danger" onclick={() => remove(s)}>Delete <Kbd keys="x" /></button>
          {/if}
          <span class="grow"></span>
          <button class="btn primary" onclick={save}>Save <Kbd keys="⌘+⏎" /></button>
        </div>
      </aside>
    {/if}
  </div>
</section>

<style>

  .kinds {
    display: grid;
    grid-template-columns: 88px 1fr;
    align-items: center;
    gap: var(--space-sm);
  }
  .kinds > span:first-child {
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .seg {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-xs);
  }
  .seg button {
    padding: 0 var(--space-sm);
    background: none;
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
    font: var(--text-label);
    cursor: pointer;
  }
  .seg button.on {
    border-color: var(--color-focus);
    background: var(--color-surface-selected);
    color: var(--color-text-primary);
  }
  .errbox {
    border-color: var(--color-status-error);
  }
  .mcp {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    outline: none;
  }
  .h {
    display: flex;
    align-items: baseline;
    gap: var(--space-md);
    margin-bottom: var(--space-sm);
  }
  h2 {
    margin: 0;
    font: var(--text-heading);
  }
  .split {
    display: flex;
    gap: var(--space-md);
    align-items: flex-start;
  }
  .list {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th {
    text-align: left;
    color: var(--color-text-muted);
    font: var(--text-caption);
    padding: var(--space-xs) var(--space-sm);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
  }
  td {
    padding: var(--space-xs) var(--space-sm);
    border-bottom: var(--border-width-default) solid var(--color-border-subtle);
    white-space: nowrap;
  }
  tr {
    cursor: pointer;
  }
  tr.sel {
    background: var(--color-surface-raised);
  }
  .mcp:focus-visible tr.sel,
  tr.editing {
    background: var(--color-surface-selected);
    box-shadow: inset var(--border-width-focus) 0 0 var(--color-focus);
  }
  .hints {
    font: var(--text-caption);
  }
  .empty {
    padding: var(--space-lg) 0;
  }
  .drawer {
    width: 400px;
    flex: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-md);
    background: var(--color-surface-raised);
    border: var(--border-width-default) solid var(--color-border-strong);
    border-radius: var(--radius-md);
  }
  .dh,
  .foot {
    display: flex;
    align-items: center;
    gap: var(--space-sm);
  }
  .dh b {
    font: var(--text-title);
  }
  .when {
    font: var(--text-caption);
  }
  .grid {
    display: flex;
    flex-direction: column;
    gap: var(--space-xs);
  }
  .grid label {
    display: grid;
    grid-template-columns: 88px 1fr;
    align-items: center;
    gap: var(--space-sm);
  }
  .grid label.top {
    align-items: start;
  }
  .grid label > span {
    color: var(--color-text-muted);
    font: var(--text-caption);
  }
  .note {
    margin: 0 0 0 calc(88px + var(--space-sm));
    font: var(--text-caption);
  }
  .kc {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
  }
  .chip {
    align-self: flex-start;
    padding: 0 var(--space-sm);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
    font: var(--text-caption);
  }
  .box {
    padding: var(--space-sm) var(--space-md);
    border-radius: var(--radius-sm);
    border-left: var(--border-width-focus) solid;
    background: var(--color-surface-pane);
    font: var(--text-caption);
  }
  .box p {
    margin: 0 0 var(--space-xs);
  }
  .info {
    border-color: var(--color-accent-default);
    color: var(--color-text-secondary);
  }
  .warnbox {
    border-color: var(--color-status-waiting-for-you);
  }
  .ck {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .roles {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-sm);
    margin-top: var(--space-xs);
  }
  .link {
    padding: 0;
    margin-left: var(--space-xs);
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
  .x {
    background: none;
    border: 0;
    color: var(--color-text-muted);
    font: var(--text-title);
    cursor: pointer;
  }
  .err {
    margin: 0;
  }
</style>
