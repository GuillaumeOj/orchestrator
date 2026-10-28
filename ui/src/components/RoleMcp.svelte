<script>
  // Role inspector additions (FR-601, FR-819 field; p3-design-notes §5.5):
  // the MCP servers picker over the registry, with status glyphs and the
  // connector / strict-MCP warning, and the model downgrade line. Both save
  // the role through `onsave` (role_save).
  import { openSettings } from '../lib/app.svelte.js';
  import { downgradeProblem, downgradeWarning, downgradeWhen, family, MODELS, roleMcpNotes, toggleRef } from '../lib/ecosystem.js';
  import ConnBadge from './ConnBadge.svelte';

  /** @type {{ role: any, servers: any[] | null, isLead?: boolean, project?: string | null, onsave: (patch: object) => void }} */
  let { role, servers, isLead = false, project = null, onsave } = $props();

  const refs = $derived(role.mcp_refs ?? []);
  const notes = $derived(roleMcpNotes(role, servers ?? []));
  // The downgrade being edited; saved only when valid (same model is refused).
  let dg = $state(null);
  $effect(() => {
    dg = role.downgrade ? { ...role.downgrade } : null;
  });
  const problem = $derived(dg ? downgradeProblem(role, dg.to) : null);
  const warning = $derived(dg ? downgradeWarning(role, isLead) : null);
  const defaultTo = $derived(MODELS.find((m) => m !== family(role.model) && m !== 'opus') ?? 'sonnet');

  function setDowngrade(next) {
    dg = next;
    if (!next || !downgradeProblem(role, next.to)) onsave({ downgrade: next });
  }
</script>

<div class="lab top"><span>MCP servers</span>
  <div class="picker">
    {#if servers == null}
      <span class="muted">Loading the registry…</span>
    {:else if !servers.length}
      <span class="muted">No servers in the registry. <button class="link" onclick={() => openSettings('mcp', null, project)}>Add one in Settings</button></span>
    {:else}
      {#each servers as s (s.id)}
        <label class="srv">
          <input type="checkbox" checked={refs.includes(s.id)} onchange={() => onsave({ mcp_refs: toggleRef(refs, s.id, servers) })} />
          <span class="sname">{s.name}</span>
          <ConnBadge status={s.status} connector={s.kind === 'connector'} />
          <span class="muted kind">{s.kind === 'connector' ? 'connector · needs your claude.ai login' : s.kind}</span>
        </label>
      {/each}
    {/if}
    {#if servers && !refs.length}<span class="muted note">No MCP servers. The role only has Claude Code's built-in tools.</span>{/if}
    {#each notes.missing as id (id)}
      <p class="warn">{'\u{F0026}'} {id} is not in the MCP registry. Add it in <button class="link" onclick={() => openSettings('mcp', null, project)}>Settings › MCP servers</button> or <button class="link" onclick={() => onsave({ mcp_refs: refs.filter((r) => r !== id) })}>remove it here</button>.</p>
    {/each}
    {#each notes.connectors as id (id)}
      <p class="warn">{'\u{F0026}'} {id} is a claude.ai connector, so this role can't use strict MCP: it will also see your other claude.ai connectors. Strict MCP is turned off for this role.</p>
    {/each}
  </div>
</div>

<div class="lab top"><span>downgrade</span>
  <div class="dg">
    <label class="ck"><input type="checkbox" checked={!!dg} onchange={(e) => setDowngrade(e.currentTarget.checked ? { to: defaultTo, when: 'red' } : null)} /> Use a cheaper model under pressure</label>
    {#if dg}
      <div class="line">
        to
        <select class="field sm" aria-label="Downgrade to" value={dg.to} onchange={(e) => setDowngrade({ ...dg, to: e.currentTarget.value })}>
          {#each MODELS as m (m)}<option value={m}>{m}</option>{/each}
        </select>
        when
        <select class="field sm" aria-label="Downgrade when" value={dg.when} onchange={(e) => setDowngrade({ ...dg, when: e.currentTarget.value })}>
          {#each downgradeWhen(role.model) as w (w.id)}<option value={w.id}>{w.label}</option>{/each}
        </select>
      </div>
      <span class="muted note">Switches at the next task, never mid-task. Loses the prompt cache; the conversation is kept.</span>
      {#if problem}<p class="err">{problem}</p>{/if}
      {#if warning}<p class="warn">{'\u{F0026}'} {warning}</p>{/if}
    {/if}
  </div>
</div>

<style>
  .lab {
    display: grid;
    grid-template-columns: 80px 1fr;
    gap: var(--space-sm);
    align-items: start;
  }
  .lab > span:first-child {
    color: var(--color-text-muted);
    font: var(--text-caption);
    padding-top: var(--space-xxs);
  }
  .picker,
  .dg {
    display: flex;
    flex-direction: column;
    gap: var(--space-xxs);
    min-width: 0;
  }
  .srv {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
    min-width: 0;
  }
  .sname {
    font: var(--text-body-strong);
  }
  .kind,
  .note {
    font: var(--text-caption);
  }
  .kind {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .line {
    display: flex;
    align-items: center;
    gap: var(--space-xs);
  }
  .field.sm {
    width: auto;
  }
  .ck {
    display: inline-flex;
    align-items: center;
    gap: var(--space-xxs);
  }
  .warn,
  .err {
    margin: 0;
    font: var(--text-caption);
  }
  .warn {
    color: var(--color-status-waiting-for-you);
  }
  .link {
    padding: 0;
    background: none;
    border: 0;
    color: var(--color-accent-default);
    font: inherit;
    cursor: pointer;
  }
</style>
