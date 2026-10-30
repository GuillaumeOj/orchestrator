<script>
  // Row details (`space`, notes §1.8, FR-901 columns kept from the Overview):
  // one line per agent of the row. The record says what it does and how the
  // Governor holds it; app_state gives the raw details (ctx, usage, memory).
  import { agent, app } from '../../lib/app.svelte.js';
  import { mb, waited } from '../../lib/format.js';
  import { attention, waitWhat } from '../../lib/now.svelte.js';
  import { PRESSURE, STATUS, sessionPp } from '../../lib/status.js';

  let { ids, now } = $props();
  const pressure = $derived(PRESSURE[app.data?.usage?.snapshot?.state]?.label);
  const HELD = new Set(['queued', 'not_started']);

  /** "Green · live", "Not started · cap", "Queued · 2nd in line", "—". */
  function governor(rec) {
    if (rec?.waiting_on?.kind === 'governor') {
      const label = rec.status === 'not_started' ? 'Not started' : (STATUS[rec.status]?.label ?? 'Queued');
      const why = String(rec.reason ?? '').split(' · ');
      const what = /cap/.test(why[0]) ? 'cap' : why[0];
      return [label, what].filter(Boolean).join(' · ');
    }
    return rec?.live && pressure ? `${pressure} · live` : '—';
  }

  const rows = $derived(
    ids.map((id) => {
      const a = agent(id);
      const rec = attention(id);
      const held = HELD.has(rec?.status);
      const pp = sessionPp(a, app.data?.usage?.snapshot);
      return {
        id,
        doing: rec?.status === 'not_started' ? 'not started' : (waitWhat(rec) ?? '—'),
        elapsed: held || !a?.created_at_ms ? '—' : waited(a.created_at_ms, now),
        ctx: held || a?.context_pct == null ? '—' : `${Math.round(a.context_pct)} % ctx`,
        usage: held || pp == null ? '—' : `+${pp.toFixed(1)} pp`,
        gov: governor(rec),
        mem: held || !a?.memory_mb || rec?.status === 'suspended' ? '—' : mb(a.memory_mb),
      };
    }),
  );
</script>

<table class="details" aria-label="Row details">
  <thead><tr><th>Agent</th><th>Doing</th><th>Running for</th><th>Context</th><th>Usage this session</th><th>Governor</th><th>Memory</th></tr></thead>
  <tbody>
    {#each rows as r (r.id)}
      <tr><td>{r.id}</td><td class="doing">{r.doing}</td><td>{r.elapsed}</td><td>{r.ctx}</td><td>{r.usage}</td><td>{r.gov}</td><td>{r.mem}</td></tr>
    {/each}
  </tbody>
</table>

<style>
  .details {
    width: 100%;
    margin-top: var(--space-sm);
    border-collapse: collapse;
    border: var(--border-width-default) solid var(--color-border-subtle);
    border-radius: var(--radius-sm);
    font: var(--text-caption);
  }
  th {
    text-align: left;
    font-weight: var(--font-weight-regular);
    color: var(--color-text-muted);
    padding: var(--space-xs) var(--space-sm);
  }
  td {
    padding: var(--space-xxs) var(--space-sm);
    white-space: nowrap;
  }
  .doing {
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
