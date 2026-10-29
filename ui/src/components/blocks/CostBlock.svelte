<script>
  // `cost` block (notes §3.3): "≈ +2.1 pp of the week · +6 % of the 5h window ·
  // done ≈14:40"; missing parts are left out.
  import { clock, pp, signed } from '../../lib/format.js';

  let { block } = $props();
  const parts = $derived(
    [
      block.week_pp != null ? `≈ ${pp(block.week_pp, 1)} of the week` : null,
      block.five_hour_pp != null ? `${signed(block.five_hour_pp)} % of the 5h window` : null,
      block.eta_ms ? `done ≈${clock(block.eta_ms)}` : null,
    ].filter(Boolean),
  );
</script>

<div class="bh">Cost</div>
<div>{parts.join(' · ') || '—'}</div>
