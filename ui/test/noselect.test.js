// T3: every choice of a value is the shared Select (design-notes §2.1.1).
// No native <select> is left in a component, and a Select that sits next to a
// text input has the input's height (R11).
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const SRC = join(import.meta.dirname, '../src');

function files(dir) {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? files(p) : [p];
  });
}
const svelte = files(SRC).filter((p) => p.endsWith('.svelte'));
const read = (rel) => readFileSync(join(SRC, rel), 'utf8');

describe('no native select (R11)', () => {
  it('has no <select in any .svelte', () => {
    expect(svelte.filter((p) => /<select[\s>]/.test(readFileSync(p, 'utf8'))).map((p) => p.slice(SRC.length))).toEqual(
      [],
    );
  });

  it('a regular Select has the text input height; only a strip may be compact', () => {
    const sel = read('components/Select.svelte');
    // The box: --size-control, the same token as input.field's min-height.
    expect(sel).toMatch(/\.box\s*{[^}]*height:\s*var\(--size-control\)/s);
    expect(read('styles/app.css')).toMatch(/select\.field\s*{[^}]*min-height:\s*var\(--size-control\)/s);
    const compact = svelte.filter(
      (p) => /<Select[^>]*\bcompact\b/.test(readFileSync(p, 'utf8')) && !p.endsWith('TabStrip.svelte'),
    );
    expect(compact.map((p) => p.slice(SRC.length))).toEqual([]);
  });
});
