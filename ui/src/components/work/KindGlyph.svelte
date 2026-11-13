<script>
  // Kind glyph (notes §2.3, page 02): 16 px, monochrome; the shape carries the upstream state of issues.
  import { glyphOf } from '../../lib/workmodel.js';

  /** `node` (a WorkNode), or `glyph` + `label` given directly (project row, Team tasks fold). */
  let { node = null, glyph = '', label = '', dim = false } = $props();
  const g = $derived(node ? glyphOf(node) : { g: glyph, label });
</script>

<span class="kg" class:dim role="img" aria-label={g.label}>{g.g}</span>

<style>
  .kg {
    flex: none;
    width: var(--size-icon);
    text-align: center;
    font: var(--text-icon);
    color: var(--color-text-muted);
  }
  .dim {
    opacity: 0.5;
  }
</style>
