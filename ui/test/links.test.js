import { describe, expect, it } from 'vitest';
import { findPaths, linksAcrossRows } from '../src/lib/term.js';

const rows = (...lines) => lines.map((l) => (typeof l === 'string' ? { text: l, wrapped: false } : l));

describe('path links (FR-208)', () => {
  it('finds paths with lines on one row', () => {
    expect(findPaths('Fixing in src/services/checkout.ts:71.').map((p) => [p.file, p.line])).toEqual([['src/services/checkout.ts', 71]]);
  });

  it('joins a soft-wrapped path and reports it on every row it covers', () => {
    const r = rows('Update(src/routes/che', { text: 'ckout.ts) done', wrapped: true });
    const on0 = linksAcrossRows(r, 0);
    expect(on0).toHaveLength(1);
    expect(on0[0].file).toBe('src/routes/checkout.ts');
    expect(on0[0].start).toEqual({ row: 0, col: 7 });
    expect(on0[0].end).toEqual({ row: 1, col: 8 });
    expect(linksAcrossRows(r, 1)[0].file).toBe('src/routes/checkout.ts');
  });

  it("joins Claude Code's own indented continuation of a long path", () => {
    const r = rows(
      '● Write(/Users/me/dev/orchestrator/.claude/worktrees   ',
      '       /cool.worktrees/solo/solo-1/hello.txt)',
      '  └ Error: denied',
    );
    const l = linksAcrossRows(r, 1);
    expect(l).toHaveLength(1);
    expect(l[0].file).toBe('/Users/me/dev/orchestrator/.claude/worktrees/cool.worktrees/solo/solo-1/hello.txt');
    expect(l[0].start).toEqual({ row: 0, col: 8 });
    expect(l[0].end).toEqual({ row: 1, col: 44 });
    expect(linksAcrossRows(r, 2)).toEqual([]);
  });

  it('keeps a joined row from swallowing what follows the path', () => {
    const r = rows('Read the file at src/a.ts', '/help for commands');
    expect(linksAcrossRows(r, 0).map((l) => l.file)).toEqual(['src/a.ts']);
    expect(linksAcrossRows(r, 1)).toEqual([]);
  });
});
