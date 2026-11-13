<script>
  // The graph strip (improve-intake notes §6.2, §6.3, §7.2): numbered circles by order, arrows for the
  // edges, skipped dashed; read-only, 80 px, hidden without edges. `live`: the Program view colours
  // each circle by its feature's state. Hovering a circle highlights its row (`onhover`).
  import { graphOf } from '../../lib/program.js';

  /** @type {{ program: any, live?: boolean, onhover?: (key: string | null) => void }} */
  let { program, live = false, onhover = () => {} } = $props();

  const g = $derived(graphOf(program));
  const STEP = 120;
  const ROW = 34;
  const R = 12;
  const X0 = 20;
  const Y0 = 20;
  const pos = $derived(new Map((g?.nodes ?? []).map((n) => [n.key, { x: X0 + n.level * STEP, y: Y0 + n.row * ROW }])));
  const width = $derived(X0 * 2 + Math.max(0, ...(g?.nodes ?? []).map((n) => n.level)) * STEP);
  const height = $derived(Y0 * 2 + Math.max(0, ...(g?.nodes ?? []).map((n) => n.row)) * ROW);
  const TONE = { running: 'working', pr: 'gate', merged: 'done', queued: 'queued', failed: 'error' };
</script>

{#if g}
  <div class="strip" data-graph>
    <span class="lab muted">Graph</span>
    <svg {width} {height} viewBox="0 0 {width} {height}" role="img" aria-label="Order of the features: {g.edges.map((e) => `${g.nodes.find((n) => n.key === e.to)?.n} after ${g.nodes.find((n) => n.key === e.from)?.n}`).join(', ')}">
      <defs>
        <marker id="gs-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="8" markerHeight="8" orient="auto-start-reverse">
          <path d="M0,0 L8,4 L0,8" fill="none" stroke="currentColor" />
        </marker>
      </defs>
      {#each g.edges as e (`${e.from}>${e.to}`)}
        {@const a = pos.get(e.from)}
        {@const b = pos.get(e.to)}
        {#if a && b}
          {@const dx = b.x - a.x}
          {@const dy = b.y - a.y}
          {@const len = Math.hypot(dx, dy) || 1}
          <line class="edge" x1={a.x + (dx / len) * R} y1={a.y + (dy / len) * R} x2={b.x - (dx / len) * (R + 1)} y2={b.y - (dy / len) * (R + 1)} marker-end="url(#gs-arrow)" />
        {/if}
      {/each}
      {#each g.nodes as n (n.key)}
        {@const p = pos.get(n.key)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <g class="node {live ? (TONE[n.state] ?? '') : ''}" class:skipped={n.skipped} data-node={n.n} onmouseenter={() => onhover(n.key)} onmouseleave={() => onhover(null)}>
          <circle cx={p.x} cy={p.y} r={R} />
          <text x={p.x} y={p.y} dy="0.35em" text-anchor="middle">{n.n}</text>
        </g>
      {/each}
    </svg>
  </div>
{/if}

<style>
  .strip {
    display: flex;
    align-items: center;
    gap: var(--space-lg);
    min-height: 80px;
    overflow-x: auto;
  }
  .lab {
    font: var(--text-caption);
    flex: none;
  }
  svg {
    flex: none;
    color: var(--color-text-muted);
  }
  .edge {
    stroke: var(--color-text-muted);
    stroke-width: 1;
  }
  .node circle {
    fill: var(--color-surface-pane);
    stroke: var(--tone, var(--color-text-muted));
    stroke-width: 1.5;
  }
  .node text {
    fill: var(--color-text-primary);
    font: var(--text-mono-sm);
  }
  .node.skipped circle {
    stroke-dasharray: 3 2;
  }
  .node.skipped text {
    fill: var(--color-text-muted);
  }
  .working {
    --tone: var(--color-status-working);
  }
  .gate {
    --tone: var(--color-status-at-gate);
  }
  .done {
    --tone: var(--color-status-done);
  }
  .queued {
    --tone: var(--color-status-queued);
  }
  .error {
    --tone: var(--color-status-error);
  }
</style>
