// P4.1 exit criterion (spec §1 b): no pane or store module polls; state comes
// from the `now` push and views refetch on their topics. Allowed timers: the
// 30 s relative-time clock and the undo window's flush (it sends due
// decisions, fetches nothing). Debounces, toasts and the terminal pump use
// setTimeout once, not as a loop.

import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const SRC = join(import.meta.dirname, '../src');
const files = (dir) =>
  readdirSync(join(SRC, dir))
    .filter((f) => /\.(js|svelte)$/.test(f))
    .map((f) => [`${dir}/${f}`, readFileSync(join(SRC, dir, f), 'utf8')]);
const ALL = [
  ['App.svelte', readFileSync(join(SRC, 'App.svelte'), 'utf8')],
  ...['lib', 'panes', 'components'].flatMap(files),
];

// file → the setInterval calls it may make (by their callee).
const ALLOWED = {
  'lib/app.svelte.js': ['setInterval(() => (tick.now = Date.now()), 30000)'],
  'App.svelte': ['setInterval(flushDecisions, 1000)'],
};

describe('no polling (spec §1, §3.5)', () => {
  it('reads every pane, component and store module', () => {
    expect(ALL.some(([f]) => f === 'panes/FeaturePane.svelte')).toBe(true);
    expect(ALL.some(([f]) => f === 'lib/now.svelte.js')).toBe(true);
  });

  it('no setInterval but the clock and the undo flush', () => {
    const found = ALL.flatMap(([f, src]) =>
      [...src.matchAll(/setInterval\([^;\n]*/g)]
        .map((m) => m[0])
        .filter((call) => !(ALLOWED[f] ?? []).some((ok) => call.startsWith(ok)))
        .map((call) => `${f}: ${call}`),
    );
    expect(found).toEqual([]);
  });

  it('no self-rescheduling setTimeout loop', () => {
    // A function that schedules itself (`const tick = () => { …; setTimeout(tick, n) }`):
    // from its definition to the next one, a setTimeout naming it.
    const DEF = /(?:function\s+(\w+)|(?:const|let)\s+(\w+)\s*=\s*(?:async\s*)?\([^)]*\)\s*=>)/g;
    const found = ALL.flatMap(([f, src]) => {
      const defs = [...src.matchAll(DEF)];
      return defs
        .map((m, i) => ({ name: m[1] ?? m[2], body: src.slice(m.index, defs[i + 1]?.index) }))
        .filter(({ name, body }) => new RegExp(`setTimeout\\(\\s*${name}\\b`).test(body))
        .map(({ name }) => `${f}: ${name}`);
    });
    expect(found).toEqual([]);
  });

  it('no displayState or notStarted left (status comes from the core)', () => {
    const found = ALL.filter(([, src]) => /\b(displayState|notStarted)\b/.test(src)).map(([f]) => f);
    expect(found).toEqual([]);
  });
});
