import { describe, expect, it } from 'vitest';
import { lineKind, patchByFile } from '../src/lib/diff.js';

const PATCH = [
  'diff --git a/src/a.rs b/src/a.rs',
  'index 111..222 100644',
  '--- a/src/a.rs',
  '+++ b/src/a.rs',
  '@@ -1,2 +1,2 @@',
  '-old',
  '+new',
  ' same',
  'diff --git a/old name.txt b/new name.txt',
  'similarity index 90%',
].join('\n');

describe('diff', () => {
  it('splits a patch per file, keyed by the new path', () => {
    const s = patchByFile(PATCH);
    expect(Object.keys(s)).toEqual(['src/a.rs', 'new name.txt']);
    expect(s['src/a.rs']).toHaveLength(8);
  });

  it('an empty or missing patch has no sections', () => {
    expect(patchByFile('')).toEqual({});
    expect(patchByFile(undefined)).toEqual({});
  });

  it('classifies lines', () => {
    expect(['@@ -1 +1 @@', '+++ b/x', '--- a/x', '+a', '-b', ' c'].map(lineKind)).toEqual([
      'hunk',
      'meta',
      'meta',
      'add',
      'del',
      'ctx',
    ]);
  });
});
