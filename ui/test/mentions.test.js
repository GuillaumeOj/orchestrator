// @mentions: the pure logic (trigger, candidates, token boundaries, what a send carries,
// rendering runs). Against plain data; holds at any date, hour and time zone.

import { describe, expect, it } from 'vitest';
import {
  candidates,
  countText,
  emphasis,
  insertToken,
  matchPositions,
  mentionRuns,
  mentionsToSend,
  projectKey,
  tokenAtCaret,
  tokensIn,
  tokenTable,
} from '../src/lib/mentions.js';

const P = [
  { id: 'p1', name: 'Shop API' },
  { id: 'p2', name: 'blog' },
];
const F = (id, slug, over = {}) => ({
  id,
  project_id: 'p1',
  slug,
  title: slug.replace(/-/g, ' '),
  archived: false,
  run: null,
  ...over,
});
const run = (status) => ({ id: `r-${status}`, status });

describe('tokenAtCaret', () => {
  const at = (t, c = t.length) => tokenAtCaret(t, c);
  it('opens at the start, after whitespace and after ( [ " \'', () => {
    expect(at('@')).toEqual({ start: 0, query: '' });
    expect(at('hi @ad')).toEqual({ start: 3, query: 'ad' });
    expect(at('a\n@x')).toEqual({ start: 2, query: 'x' });
    for (const c of ['(', '[', '"', "'"]) expect(at(`${c}@ab`)).toEqual({ start: 1, query: 'ab' });
  });
  it('not inside a word, not in an inline code span', () => {
    expect(at('me@x.com')).toBeNull();
    expect(at('a@b')).toBeNull();
    expect(at('`code @ab')).toBeNull();
    expect(at('`a` @ab')).toEqual({ start: 4, query: 'ab' });
    expect(at('`x`\n@ab')).toEqual({ start: 4, query: 'ab' }); // the count is per line
  });
  it('the query is the text up to the caret, in the token charset', () => {
    expect(at('@add-mention', 4)).toEqual({ start: 0, query: 'add' });
    expect(at('@shop/ch.x_y')).toEqual({ start: 0, query: 'shop/ch.x_y' });
    expect(at('@ab cd')).toBeNull(); // a space ends it
    expect(at('@ab!')).toBeNull(); // so does any other char
    expect(at('hello')).toBeNull();
  });
});

describe('tokensIn (spec §8)', () => {
  it('trims trailing . / - _ and keeps the whole token', () => {
    expect(tokensIn('see @add-mention-in-overwatch. ok').map((t) => t.token)).toEqual(['@add-mention-in-overwatch']);
    expect(tokensIn('(@a/b-)').map((t) => t.token)).toEqual(['@a/b']);
    expect(tokensIn('@x?').map((t) => t.token)).toEqual(['@x']);
    expect(tokensIn('@. @').length).toBe(0);
  });
  it('skips words, code and gives positions', () => {
    expect(tokensIn('me@x.com `@a` @b')).toEqual([{ start: 14, end: 16, token: '@b' }]);
  });
});

describe('project key and tokens (spec §8, §9)', () => {
  it('lowercases, replaces runs outside [a-z0-9._-] and trims', () => {
    expect(projectKey('Shop API')).toBe('shop-api');
    expect(projectKey('  My  App!! ')).toBe('my-app');
    expect(projectKey('日本')).toBe('');
    expect(projectKey('a.b_c-d')).toBe('a.b_c-d');
  });
  it('@slug, @key/slug when the slug is shared, the project id when the key is empty or shared', () => {
    const fs = [F('a', 'checkout'), F('b', 'checkout', { project_id: 'p2' }), F('c', 'unique', { project_id: 'p2' })];
    const t = tokenTable(fs, P);
    expect([...t.values()].map((x) => x.token)).toEqual(['@shop-api/checkout', '@blog/checkout', '@unique']);
    const shared = tokenTable(
      [F('a', 's'), F('b', 's', { project_id: 'p3' })],
      [
        { id: 'p1', name: 'x y' },
        { id: 'p3', name: 'X-Y' },
      ],
    );
    expect(shared.get('a').token).toBe('@p1/s');
    expect(shared.get('b').token).toBe('@p3/s');
    const empty = tokenTable([F('a', 's'), F('b', 's', { project_id: 'p2' })], [{ id: 'p1', name: '日本' }, P[1]]);
    expect(empty.get('a').token).toBe('@p1/s');
    expect(empty.get('b').token).toBe('@blog/s');
  });
  it('an archived feature does not make a slug shared', () => {
    const t = tokenTable([F('a', 's'), F('b', 's', { project_id: 'p2', archived: true })], P);
    expect(t.get('a').token).toBe('@s');
    expect(t.has('b')).toBe(false);
  });
});

describe('candidates', () => {
  const fs = [
    F('a', 'alpha', { run: run('done'), created_at_ms: 3 }),
    F('b', 'beta', { run: run('executing'), created_at_ms: 1 }),
    F('c', 'gamma', { created_at_ms: 2 }),
    F('d', 'delta', { run: run('paused'), created_at_ms: 4 }),
    F('e', 'gone', { archived: true, run: run('executing') }),
  ];
  it('current first (any run not done / failed), then the others; empty query by recency; archived out', () => {
    const c = candidates(fs, P, '');
    expect(c.current.map((r) => r.id)).toEqual(['d', 'b']); // paused counts; newest first
    expect(c.other.map((r) => r.id)).toEqual(['a', 'c']); // a finished run is "other"; newest first
    expect(c.rows.map((r) => r.id)).toEqual(['d', 'b', 'a', 'c']);
  });
  it('failed runs are not current', () => {
    expect(candidates([F('x', 'x', { run: run('failed') })], P, '').current).toEqual([]);
  });
  it('fuzzy on slug, title and <key>/<slug>, best score first within a group', () => {
    const g = [F('a', 'checkout-flow'), F('b', 'billing', { title: 'Check out later' }), F('c', 'zzz')];
    expect(candidates(g, P, 'check').rows.map((r) => r.id)).toEqual(['a', 'b']);
    expect(candidates(g, P, 'shop-api/zz').rows.map((r) => r.id)).toEqual(['c']);
    expect(candidates(g, P, 'xyz').rows).toEqual([]);
  });
  it('same score: project name A→Z', () => {
    const g = [F('a', 'same', { project_id: 'p1' }), F('b', 'same', { project_id: 'p2' })];
    expect(candidates(g, P, 'same').rows.map((r) => r.project)).toEqual(['blog', 'Shop API']);
  });
  it('rows carry the token and the emphasis runs', () => {
    const [r] = candidates([F('a', 'add-mention')], P, 'ment').rows;
    expect(r.token).toBe('@add-mention');
    expect(r.slug).toEqual([
      { t: 'add-', on: false },
      { t: 'ment', on: true },
      { t: 'ion', on: false },
    ]);
    const [t] = candidates([F('a', 'zzz', { title: 'Fix the cart' })], P, 'cart').rows;
    expect(t.slug).toEqual([{ t: 'zzz', on: false }]); // the match came from the title
    expect(t.title.filter((s) => s.on).map((s) => s.t)).toEqual(['cart']);
  });
  it('filters 200 features fast', () => {
    const many = Array.from({ length: 200 }, (_, i) => F(`f${i}`, `feature-number-${i}`));
    const t0 = performance.now();
    candidates(many, P, 'fn5');
    expect(performance.now() - t0).toBeLessThan(50);
  });
});

describe('matchPositions, emphasis, insertToken, countText', () => {
  it('walks the query in order', () => {
    expect(matchPositions('amo', 'add-mention')).toEqual([0, 4, 9]);
    expect(matchPositions('zz', 'abc')).toBeNull();
    expect(emphasis('abc', [])).toEqual([{ t: 'abc', on: false }]);
  });
  it('insert: one trailing space, none doubled, caret after', () => {
    expect(insertToken('hi @ad', 3, 6, '@add')).toEqual({ text: 'hi @add ', caret: 8 });
    expect(insertToken('hi @ad there', 3, 6, '@add')).toEqual({ text: 'hi @add there', caret: 8 });
  });
  it('insert: a caret inside a token replaces the whole token', () => {
    expect(insertToken('hi @adxyz there', 3, 6, '@add')).toEqual({ text: 'hi @add there', caret: 8 });
  });
  it('counts', () => {
    expect([countText(3, true), countText(1, true), countText(0, true), countText(0, false)]).toEqual([
      '3 features',
      '1 feature',
      'No feature matches',
      'No features yet',
    ]);
  });
});

describe('mentionsToSend', () => {
  const fs = [
    F('a', 'alpha'),
    F('b', 'beta'),
    F('c', 'dup'),
    F('d', 'dup', { project_id: 'p2' }),
    F('z', 'old', { archived: true }),
  ];
  const ids = (text, picks = []) => mentionsToSend(text, picks, fs, P).map((m) => m.id);
  it('a pick whose token is still in the text is kept, a deleted one dropped', () => {
    const picks = [{ token: '@alpha', kind: 'feature', id: 'a' }];
    expect(ids('what about @alpha now', picks)).toEqual(['a']);
    expect(ids('what about now', picks)).toEqual([]);
    expect(ids('what about @alphabet', picks)).toEqual([]); // a whole token only
  });
  it('a hand-typed exact unique slug is sent; ambiguous or unknown is not', () => {
    expect(ids('@beta and @alpha.')).toEqual(['b', 'a']);
    expect(ids('@dup')).toEqual([]);
    expect(ids('@nope @old')).toEqual([]); // unknown, archived
    expect(ids('@alp')).toEqual([]);
  });
  it('a hand-typed @key/slug (or @project id/slug) resolves exactly', () => {
    expect(ids('@shop-api/dup')).toEqual(['c']);
    expect(ids('@blog/dup @p2/dup')).toEqual(['d']);
  });
  it('a picked shared-slug token is kept by id', () => {
    expect(ids('x @blog/dup', [{ token: '@blog/dup', kind: 'feature', id: 'd' }])).toEqual(['d']);
  });
  it('dedupes and caps at 10', () => {
    expect(ids('@alpha @alpha')).toEqual(['a']);
    const many = Array.from({ length: 14 }, (_, i) => F(`m${i}`, `m${i}`));
    const text = many.map((f) => `@${f.slug}`).join(' ');
    expect(mentionsToSend(text, [], many, P)).toHaveLength(10);
  });
  it('snapshots carry slug, title, project id and name', () => {
    expect(mentionsToSend('@alpha', [], fs, P)[0]).toEqual({
      kind: 'feature',
      id: 'a',
      slug: 'alpha',
      title: 'alpha',
      project_id: 'p1',
      project: 'Shop API',
    });
  });
});

describe('mentionRuns', () => {
  const m = { kind: 'feature', id: 'a', slug: 'add-mention-in-overwatch', project_id: 'p1', project: 'Shop API' };
  it('cuts the text at whole tokens, both forms, plain around', () => {
    const runs = mentionRuns('hi @add-mention-in-overwatch. and @shop-api/add-mention-in-overwatch!', [m]);
    expect(runs.map((r) => [r.t, !!r.m])).toEqual([
      ['hi ', false],
      ['@add-mention-in-overwatch', true],
      ['. and ', false],
      ['@shop-api/add-mention-in-overwatch', true],
      ['!', false],
    ]);
  });
  it('@add never links inside a longer token; tokens without a mention stay plain', () => {
    const add = { ...m, id: 'x', slug: 'add' };
    expect(mentionRuns('@add-mention-in-overwatch', [add]).every((r) => !r.m)).toBe(true);
    expect(mentionRuns('@other', [m])).toEqual([{ t: '@other' }]);
    expect(mentionRuns('', [m])).toEqual([]);
  });
  it('keeps newlines and spacing exactly', () => {
    const runs = mentionRuns('a\n  @add-mention-in-overwatch\nb', [m]);
    expect(runs.map((r) => r.t).join('')).toBe('a\n  @add-mention-in-overwatch\nb');
  });
});

describe('review fixes', () => {
  it('a match through <key>/<slug> emphasises the slug part and the project caption', () => {
    const [r] = candidates([F('a', 'checkout')], [{ id: 'p1', name: 'shop-api' }], 'shop-api/ch').rows;
    expect(r.slug.filter((x) => x.on).map((x) => x.t)).toEqual(['ch']);
    expect(
      r.proj
        .filter((x) => x.on)
        .map((x) => x.t)
        .join(''),
    ).toBe('shop-api');
  });
  it('empty query: recent run activity first', () => {
    const fs = [
      F('a', 'a', { created_at_ms: 5 }),
      F('b', 'b', { created_at_ms: 1, run: { id: 'r', status: 'done', updated_at_ms: 9 } }),
    ];
    expect(candidates(fs, P, '').rows.map((r) => r.id)).toEqual(['b', 'a']);
  });
});
