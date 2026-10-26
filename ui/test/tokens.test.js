import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { build, check, resolve, flatten, SOURCE, CSS_OUT } from '../scripts/tokens.mjs';

describe('token pipeline', () => {
  it('generated files match tokens.json and the UI uses semantic tokens only', () => {
    expect(check()).toEqual([]);
  });

  it('resolves references and emits no palette', () => {
    const doc = JSON.parse(readFileSync(SOURCE, 'utf8'));
    const { css, variables } = build(doc);
    expect(variables.has('--color-status-working')).toBe(true);
    expect([...variables].some((v) => v.startsWith('--palette'))).toBe(false);
    expect(css).toContain(":root[data-theme='light']");
    expect(resolve('{a}', flatten({ a: { $value: '{b}' }, b: { $value: '4' } }))).toBe('4');
    expect(() => resolve('{a}', flatten({ a: { $value: '{a}' } }))).toThrow(/cycle/);
  });

  it('detects drift', () => {
    const doc = JSON.parse(readFileSync(SOURCE, 'utf8'));
    doc['theme/dark'].color.status.working.$value = '#ff0000';
    const { css } = build(doc);
    expect(css).not.toBe(readFileSync(CSS_OUT, 'utf8'));
  });
});
