<script>
  // Settings › Integrations (FR-1203, p3-design-notes §5.1–5.3, J13 · 1–2):
  // GitHub (gh), Linear and Notion (a token in the Keychain), Penpot and Figma
  // (from the MCP registry). Rows: j/k move, t test, ⏎ connect / reconnect,
  // x remove the token. A row needing you sorts first.
  import { onMount, tick } from 'svelte';
  import { confirm, openSettings, runInPane, toast } from '../lib/app.svelte.js';
  import { checkedAt, INTEGRATION_COPY, integrationDetail, needsUser, sortIntegrations } from '../lib/ecosystem.js';
  import { errText, invoke } from '../lib/ipc.js';
  import ConnBadge from './ConnBadge.svelte';
  import Kbd from './Kbd.svelte';
  import SecretField from './SecretField.svelte';

  /** @type {{ onstatus?: (items: any[]) => void }} */
  let { onstatus = () => {} } = $props();

  let items = $state(null); // null = loading
  let error = $state(null);
  let selId = $state(null); // by id: rows re-sort when a status changes
  let busy = $state({}); // id → true while a test or a save runs
  let fieldError = $state({}); // id → text under the secret field
  let now = $state(Date.now());
  const fields = {};
  let root = $state(null);

  const rows = $derived(items ? sortIntegrations(items) : []);
  const cur = $derived(rows.find((r) => r.id === selId) ?? rows[0] ?? null);

  async function load() {
    try {
      const v = await invoke('integrations_view');
      items = v.items;
      selId ??= sortIntegrations(items)[0]?.id ?? null;
      error = null;
      onstatus(items);
    } catch (e) {
      error = errText(e);
      items ??= [];
    }
  }

  function put(it) {
    items = items.map((x) => (x.id === it.id ? it : x));
    onstatus(items);
  }

  async function test(it) {
    if (!it || busy[it.id]) return;
    busy = { ...busy, [it.id]: true };
    try {
      const r = await invoke('integration_test', { id: it.id });
      put(r);
      if (r.status === 'connected') toast(`${r.name} connected`);
    } catch (e) {
      toast(`${it.name}: ${errText(e)}`, { kind: 'error' });
    } finally {
      busy = { ...busy, [it.id]: false };
    }
  }

  /** The secret goes to the core once; the field is already cleared. */
  async function saveSecret(it, secret) {
    busy = { ...busy, [it.id]: true };
    fieldError = { ...fieldError, [it.id]: null };
    try {
      const r = await invoke('integration_secret_set', { id: it.id, secret });
      put(r);
      toast(`${it.name} token stored in Keychain`);
    } catch (e) {
      fieldError = { ...fieldError, [it.id]: `${errText(e)}. Nothing was stored.` };
    } finally {
      busy = { ...busy, [it.id]: false };
    }
  }

  function remove(it) {
    if (!it?.needs_secret || !it.has_secret) return;
    const what = INTEGRATION_COPY[it.id]?.secret ?? 'token';
    confirm(
      `Remove the ${it.name} ${what} from the Keychain?`,
      it.id === 'linear' ? 'A project using Linear as its task backend stops syncing until you add a key.' : '',
      async () => {
        try {
          put(await invoke('integration_secret_remove', { id: it.id }));
          toast(`${it.name} ${what} removed from Keychain`);
        } catch (e) {
          toast(`${it.name}: ${errText(e)}`, { kind: 'error' });
        }
      },
      'Remove',
    );
  }

  /** Penpot and Figma come from the MCP registry (their status is their servers'). */
  const designTool = (it) => it.id === 'penpot' || it.id === 'figma';

  /** ⏎: what Connect / Reconnect does per integration (§5.2). */
  async function connect(it) {
    if (!it) return;
    if (it.id === 'github') return runInPane('gh auth login', 'gh auth login');
    if (it.needs_secret) {
      // The secret field shows on the selected row only.
      selId = it.id;
      await tick();
      return fields[it.id]?.focus();
    }
    if (designTool(it)) return openSettings('mcp', `design:${it.id}`);
  }

  const failedOf = (it) => it.has_secret && needsUser(it.status);
  const connectLabel = (it) =>
    designTool(it) ? 'Open MCP servers' : failedOf(it) ? 'Replace' : it.status === 'not_configured' ? 'Connect' : 'Reconnect';
  const lastCheck = $derived(Math.max(0, ...(items ?? []).map((i) => i.checked_at_ms ?? 0)) || null);

  function onKey(e) {
    if (e.target.closest('input, textarea, select, button') || e.metaKey || e.ctrlKey || e.altKey || !rows.length) return;
    const k = e.key;
    const i = Math.max(0, rows.indexOf(cur));
    if (k === 'j' || k === 'ArrowDown') selId = rows[Math.min(rows.length - 1, i + 1)].id;
    else if (k === 'k' || k === 'ArrowUp') selId = rows[Math.max(0, i - 1)].id;
    else if (k === 't') test(cur);
    else if (k === 'Enter') connect(cur);
    else if (k === 'x') remove(cur);
    else return;
    e.preventDefault();
  }

  onMount(() => {
    load();
    const t = setInterval(() => (now = Date.now()), 30000);
    return () => clearInterval(t);
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<section class="ints" tabindex="0" data-section-focus bind:this={root} onkeydown={onKey} aria-label="Integrations">
  <header class="h">
    <h2>Integrations</h2>
    <span class="muted">{items == null ? 'Checking integrations…' : 'Services the core talks to. Tokens live in the macOS Keychain; agents never see them.'}</span>
    <span class="grow"></span>
    {#if lastCheck}<span class="muted when">{checkedAt(lastCheck, now)}</span>{/if}
  </header>
  {#if error}<p class="err">Could not read the integrations: {error}</p>{/if}
  {#if items == null}
    {#each ['GitHub', 'Linear', 'Notion', 'Penpot', 'Figma'] as n (n)}
      <div class="row skel" aria-busy="true"><span class="name">{n}</span><span class="skelbar"></span></div>
    {/each}
  {:else}
    {#each rows as it (it.id)}
      {@const focused = it === cur}
      {@const failed = failedOf(it)}
      <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
      <div class="row" class:sel={focused} data-integration={it.id} onclick={() => (selId = it.id)}>
        <div class="top">
          <span class="name"><span class="ico" aria-hidden="true">{{ github: '\u{F02A4}', linear: '\u{F0453}', notion: '\u{F0219}', penpot: '\u{F0431}', figma: '\u{F0431}' }[it.id] ?? ''}</span> {it.name}</span>
          <ConnBadge status={it.status} busy={busy[it.id]} connector={designTool(it)} />
          <span class="detail ell">{integrationDetail(it)}</span>
          <span class="grow"></span>
          <span class="muted when">{checkedAt(it.checked_at_ms, now)}</span>
          {#if it.status !== 'not_configured'}<button class="btn ghost" disabled={busy[it.id]} onclick={() => test(it)}>{busy[it.id] ? 'Testing…' : 'Test'} <Kbd keys="t" /></button>{/if}
          <button class="btn ghost" onclick={() => connect(it)}>{connectLabel(it)} <Kbd keys="⏎" /></button>
          {#if it.needs_secret && it.has_secret}<button class="btn ghost" onclick={() => remove(it)}>Remove <Kbd keys="x" /></button>{/if}
        </div>
        {#if it.needs_secret && focused}
          <div class="secretline">
            <SecretField bind:this={fields[it.id]} stored={it.has_secret} placeholder={INTEGRATION_COPY[it.id]?.placeholder ?? `Paste a ${it.name} token`} error={fieldError[it.id]} onsave={(v) => saveSecret(it, v)} onremove={() => remove(it)} />
            {#if failed}<p class="err">{'\u{F0159}'} {it.name} rejected the {INTEGRATION_COPY[it.id]?.secret ?? 'token'}{it.detail ? ` (${it.detail})` : ''}. Paste a new {INTEGRATION_COPY[it.id]?.secret ?? 'token'} with Replace.</p>
            {:else if !it.has_secret}<p class="muted help">{INTEGRATION_COPY[it.id]?.help}</p>{/if}
          </div>
        {/if}
      </div>
    {/each}
    <p class="muted hints"><Kbd keys="j" /><Kbd keys="k" /> rows · <Kbd keys="t" /> test · <Kbd keys="⏎" /> connect · <Kbd keys="x" /> remove · <Kbd keys="esc" /> back to sections</p>
  {/if}
</section>

<style>

  .hints {
    margin: var(--space-sm) 0 0;
    font: var(--text-caption);
  }
  .ico {
    color: var(--color-text-muted);
  }
  .ints {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    max-width: 960px;
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
  .row {
    display: flex;
    flex-direction: column;
    gap: var(--space-sm);
    padding: var(--space-sm) var(--space-md);
    background: var(--color-surface-pane);
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-md);
  }
  .ints:focus-visible .row.sel,
  .row.sel:focus-within {
    border-color: var(--color-focus);
  }
  .row.sel {
    background: var(--color-surface-raised);
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--space-md);
    min-height: var(--size-control);
  }
  .name {
    width: 6em;
    flex: none;
    font: var(--text-body-strong);
  }
  .detail {
    color: var(--color-text-secondary);
    min-width: 0;
  }
  .ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .when {
    font: var(--text-caption);
    white-space: nowrap;
  }
  .secretline {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    padding-left: calc(6em + var(--space-md));
  }
  .help,
  .secretline .err {
    margin: 0;
    font: var(--text-caption);
  }
  .skel {
    flex-direction: row;
    align-items: center;
  }
  .skel .skelbar {
    flex: 1;
    height: 8px;
    background: var(--color-surface-raised);
    border-radius: var(--radius-sm);
  }
</style>
