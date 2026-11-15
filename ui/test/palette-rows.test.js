import { readFileSync } from 'node:fs';
import { render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it } from 'vitest';
import Palette from '../src/components/Palette.svelte';
import { app } from '../src/lib/app.svelte.js';
import { fixture, nowOf } from './fixture.js';

const src = readFileSync('src/components/Palette.svelte', 'utf8');
const rule = (sel) => src.match(new RegExp(`\\n  ${sel} \\{([^}]*)\\}`))?.[1] ?? '';

beforeEach(() => {
  app.data = fixture();
  app.now = nowOf(app.data);
  app.panes = {};
});

describe('palette rows', () => {
  it('grow with wrapped content: min-height, no fixed height, wrappable label', () => {
    const li = rule('li');
    expect(li).toMatch(/min-height:\s*var\(--size-row\)/);
    expect(li).not.toMatch(/(^|[\s;])height:/);
    const label = rule('\\.label');
    expect(label).toMatch(/min-width:\s*0/);
    expect(label).toMatch(/overflow-wrap:\s*anywhere/);
    expect(src).toMatch(/\.kbd\) \{[^}]*flex-shrink:\s*0/);
  });

  it('renders a long label and detail in full inside one row', () => {
    const long = `/dev/${'very-long-directory-name/'.repeat(8)}project`;
    app.data.projects = [{ ...app.data.projects[0], name: 'p'.repeat(90), path: long }];
    const { getAllByRole } = render(Palette, { mode: 'jump' });
    const row = getAllByRole('option').find((r) => r.textContent.includes(long));
    expect(row).toBeTruthy();
    expect(row.textContent).toContain('p'.repeat(90));
  });
});
