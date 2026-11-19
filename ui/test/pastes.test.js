// Pasted long texts (allow-long-text-in-overwatch spec §3.2, §3.3, §5, api-contract §0): the pure
// logic of lib/pastes.js. Nothing here depends on the date, the hour or the time zone.

import { describe, expect, it } from 'vitest';
import {
  atInPlaceholder,
  chipName,
  chipParts,
  countChars,
  countLines,
  diffEdit,
  isLong,
  keep,
  nextN,
  normalize,
  panelHead,
  pasteError,
  pasteRuns,
  pastesContext,
  pastesOf,
  pastesToSend,
  placeholder,
  placeholdersIn,
  referenced,
  spaced,
  wholeAt,
  wholeIn,
} from '../src/lib/pastes.js';

const paste = (n, chars = 5000, lines = 120, text = 'x') => ({ n, text, chars, lines });

describe('counting (api-contract §0)', () => {
  it('normalises CRLF and a lone CR to LF', () => {
    expect(normalize('a\r\nb\rc\nd')).toBe('a\nb\nc\nd');
    expect(normalize('\r\n\r\n')).toBe('\n\n');
  });
  it('counts code points, not UTF-16 units', () => {
    expect(countChars('abc')).toBe(3);
    expect(countChars('😀😀')).toBe(2);
    expect(countChars('')).toBe(0);
  });
  it('counts lines like Rust str::lines()', () => {
    expect(countLines('')).toBe(0);
    expect(countLines('a')).toBe(1);
    expect(countLines('a\n')).toBe(1);
    expect(countLines('a\nb')).toBe(2);
    expect(countLines('a\n\n')).toBe(2);
    expect(countLines('\n')).toBe(1);
  });
  it('groups thousands with a plain space', () => {
    expect(spaced(5000)).toBe('5 000');
    expect(spaced(200000)).toBe('200 000');
    expect(spaced(999)).toBe('999');
    expect(spaced(5000)).not.toMatch(/ | /);
  });
});

describe('the threshold: more than 800 characters or more than 10 lines', () => {
  it('800 characters is inline, 801 is a paste', () => {
    expect(isLong('x'.repeat(800))).toBe(false);
    expect(isLong('x'.repeat(801))).toBe(true);
  });
  it('counts code points: 800 emoji are not long, 801 are', () => {
    expect(isLong('😀'.repeat(800))).toBe(false);
    expect(isLong('😀'.repeat(801))).toBe(true);
  });
  it('10 lines is inline, 11 is a paste', () => {
    expect(isLong('a\n'.repeat(10))).toBe(false);
    expect(isLong(`${'a\n'.repeat(10)}a`)).toBe(true);
    expect(isLong('a\n'.repeat(11))).toBe(true);
  });
  it('a short text is not long', () => {
    expect(isLong('')).toBe(false);
    expect(isLong('hello')).toBe(false);
  });
});

describe('the placeholder', () => {
  it('is [Pasted text #n · chars chars]', () => {
    expect(placeholder(1, 5000)).toBe('[Pasted text #1 · 5 000 chars]');
    expect(placeholder(10, 200000)).toBe('[Pasted text #10 · 200 000 chars]');
    expect(placeholder(2, 801)).toBe('[Pasted text #2 · 801 chars]');
  });
  it('the longest form is 33 characters', () => {
    expect(placeholder(10, 200000).length).toBe(33);
  });
  it('is found in a text, with its position', () => {
    const t = `see ${placeholder(1, 5000)} and ${placeholder(12, 900)}!`;
    const found = placeholdersIn(t);
    expect(found.map((f) => [f.n, f.token])).toEqual([
      [1, '[Pasted text #1 · 5 000 chars]'],
      [12, '[Pasted text #12 · 900 chars]'],
    ]);
    expect(t.slice(found[0].start, found[0].end)).toBe(found[0].token);
  });
  it('is not found when it is not whole (a letter in the number, #0, #100)', () => {
    expect(placeholdersIn('[Pasted text #0 · 5 000 chars]')).toEqual([]);
    expect(placeholdersIn('[Pasted text #100 · 5 000 chars]')).toEqual([]);
    expect(placeholdersIn('[Pasted text #1 · 5 000 char]')).toEqual([]);
    expect(placeholdersIn('[Pasted text #1 · 5 000 chars')).toEqual([]);
  });
});

describe('numbering', () => {
  it('starts at 1, then one above every kept paste, referenced or not', () => {
    expect(nextN([])).toBe(1);
    expect(nextN([paste(1)])).toBe(2);
    expect(nextN([paste(1), paste(3)])).toBe(4);
  });
  it('is never reused while a removed paste is kept for undo', () => {
    // #1 removed from the draft (⌘A ⌫) but still kept: the next is #2.
    const kept = [paste(1)];
    expect(referenced('', kept).size).toBe(0);
    expect(nextN(kept)).toBe(2);
  });
});

describe('whole and unchanged placeholders', () => {
  const kept = [paste(1, 5000), paste(2, 1200)];
  it('wholeIn keeps the ones equal to their paste token', () => {
    const t = `${placeholder(1, 5000)} ${placeholder(2, 1200)}`;
    expect(wholeIn(t, kept).map((x) => x.n)).toEqual([1, 2]);
  });
  it('an edited placeholder (other count) is not whole', () => {
    expect(wholeIn('[Pasted text #1 · 4 999 chars]', kept)).toEqual([]);
  });
  it('a placeholder of an unknown paste is not whole', () => {
    expect(wholeIn(placeholder(7, 5000), kept)).toEqual([]);
  });
  it('referenced is the set of numbers still whole', () => {
    expect([...referenced(`a ${placeholder(2, 1200)} b ${placeholder(2, 1200)}`, kept)]).toEqual([2]);
  });
});

describe('what a send carries', () => {
  const kept = [paste(1, 5000, 3, 'AAA'), paste(2, 1200, 2, 'BBB'), paste(3, 900, 1, 'CCC')];
  it('only the pastes still referenced, by number', () => {
    const t = `x ${placeholder(3, 900)} ${placeholder(1, 5000)}`;
    expect(pastesToSend(t, kept).map((p) => p.n)).toEqual([1, 3]);
    expect(pastesContext(pastesToSend(t, kept))).toEqual([
      { n: 1, text: 'AAA' },
      { n: 3, text: 'CCC' },
    ]);
  });
  it('a removed or edited placeholder drops its paste', () => {
    expect(pastesToSend('no placeholders', kept)).toEqual([]);
    expect(pastesToSend('[Pasted text #1 · 5 00 chars]', kept)).toEqual([]);
  });
  it('the same token twice is one paste', () => {
    const t = `${placeholder(2, 1200)} then ${placeholder(2, 1200)}`;
    expect(pastesToSend(t, kept).map((p) => p.n)).toEqual([2]);
  });
  it('a message made only of a placeholder carries its paste', () => {
    expect(pastesToSend(placeholder(1, 5000), kept).map((p) => p.n)).toEqual([1]);
  });
  it('never more than 10 (an undo can bring an 11th back)', () => {
    const many = Array.from({ length: 11 }, (_, i) => paste(i + 1, 900));
    const t = many.map((p) => placeholder(p.n, p.chars)).join(' ');
    expect(pastesToSend(t, many).map((p) => p.n)).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
  });
});

describe('limits and their errors', () => {
  it('a paste over 200 000 characters is refused with its real count', () => {
    expect(pasteError('', 312450, [])).toBe('Not pasted: 312 450 characters, at most 200 000 in a pasted text.');
    expect(pasteError('', 200000, [])).toBe('');
    expect(pasteError('', 200001, [])).toBe('Not pasted: 200 001 characters, at most 200 000 in a pasted text.');
  });
  it('an 11th placeholder in the draft is refused', () => {
    const ten = Array.from({ length: 10 }, (_, i) => paste(i + 1, 900));
    const draft = ten.map((p) => placeholder(p.n, p.chars)).join(' ');
    expect(pasteError(draft, 900, ten)).toBe('Not pasted: at most 10 pasted texts in one message.');
  });
  it('a removed placeholder frees its slot (the limit counts the draft, not the pastes made)', () => {
    const ten = Array.from({ length: 10 }, (_, i) => paste(i + 1, 900));
    const draft = ten
      .slice(1)
      .map((p) => placeholder(p.n, p.chars))
      .join(' ');
    expect(pasteError(draft, 900, ten)).toBe('');
  });
  it('past #99 shows the 11th-paste error', () => {
    expect(pasteError('', 900, [paste(99, 900)])).toBe('Not pasted: at most 10 pasted texts in one message.');
  });
  it('the too-big error comes before the limit one', () => {
    const ten = Array.from({ length: 10 }, (_, i) => paste(i + 1, 900));
    const draft = ten.map((p) => placeholder(p.n, p.chars)).join(' ');
    expect(pasteError(draft, 250000, ten)).toMatch(/^Not pasted: 250 000 characters/);
  });
});

describe('kept pastes', () => {
  it('keeps the referenced ones and the 10 most recent others', () => {
    const kept = Array.from({ length: 14 }, (_, i) => paste(i + 1, 900));
    const draft = placeholder(1, 900); // only #1 is still in the draft
    const next = keep(draft, kept, paste(15, 900));
    const ns = next.map((p) => p.n);
    expect(ns).toContain(1);
    expect(ns).toContain(15);
    // the 10 most recent loose ones: 5..14
    for (let n = 5; n <= 14; n++) expect(ns).toContain(n);
    for (const n of [2, 3, 4]) expect(ns).not.toContain(n);
  });
});

describe('Backspace and Delete next to a placeholder', () => {
  const kept = [paste(1, 5000)];
  const ph = placeholder(1, 5000);
  const t = `ab${ph}cd`;
  it('Backspace with the caret just after it finds it', () => {
    expect(wholeAt(t, 2 + ph.length, 'back', kept)).toMatchObject({ start: 2, end: 2 + ph.length, n: 1 });
  });
  it('Delete with the caret just before it finds it', () => {
    expect(wholeAt(t, 2, 'forward', kept)).toMatchObject({ start: 2, n: 1 });
  });
  it('elsewhere nothing (a native deletion)', () => {
    expect(wholeAt(t, 1, 'forward', kept)).toBeNull();
    expect(wholeAt(t, 2 + ph.length, 'forward', kept)).toBeNull();
    expect(wholeAt(t, 2, 'back', kept)).toBeNull();
    expect(wholeAt(t, 5, 'back', kept)).toBeNull();
  });
  it('an edited placeholder is plain text: nothing whole to remove', () => {
    const edited = `ab${ph.replace('5 000', '5 00')}cd`;
    expect(wholeAt(edited, 2 + ph.length - 1, 'back', kept)).toBeNull();
  });
});

describe('an @ inside a placeholder', () => {
  const kept = [paste(1, 5000)];
  const ph = placeholder(1, 5000);
  it('is inside when removing it leaves a whole placeholder around it', () => {
    const at = 8;
    const typed = `${ph.slice(0, at)}@${ph.slice(at)}`;
    expect(atInPlaceholder(typed, at, at + 1, kept)).toBe(true);
  });
  it('is inside with a query typed after it', () => {
    const typed = `${ph.slice(0, 8)}@xy${ph.slice(8)}`;
    expect(atInPlaceholder(typed, 8, 11, kept)).toBe(true);
  });
  it('is not inside right before or right after the placeholder', () => {
    expect(atInPlaceholder(`@${ph}`, 0, 1, kept)).toBe(false);
    expect(atInPlaceholder(`${ph}@`, ph.length, ph.length + 1, kept)).toBe(false);
  });
  it('is not inside text that is not a placeholder', () => {
    expect(atInPlaceholder('see @refunds', 4, 12, kept)).toBe(false);
  });
});

describe('chips in a sent text', () => {
  const byN = new Map([[1, { n: 1, chars: 5000, lines: 120 }]]);
  it('cuts a plain run at the placeholders of known pastes', () => {
    const runs = pasteRuns([{ t: `before ${placeholder(1, 5000)} after` }], byN);
    expect(runs.map((r) => r.t)).toEqual(['before ', placeholder(1, 5000), ' after']);
    expect(runs[1].p).toBe(byN.get(1));
    expect(runs[0].p).toBeUndefined();
  });
  it('leaves a placeholder with no paste as plain text', () => {
    const runs = pasteRuns([{ t: `x ${placeholder(2, 900)} y` }], byN);
    expect(runs).toEqual([{ t: `x ${placeholder(2, 900)} y` }]);
  });
  it('the same placeholder twice gives two chips for the one paste', () => {
    const runs = pasteRuns([{ t: `${placeholder(1, 5000)}${placeholder(1, 5000)}` }], byN);
    expect(runs.filter((r) => r.p).length).toBe(2);
  });
  it('keeps the mention runs as they are', () => {
    const m = { t: '@refunds', m: { id: 'f2' } };
    expect(pasteRuns([m, { t: placeholder(1, 5000) }], byN)).toEqual([m, { t: placeholder(1, 5000), p: byN.get(1) }]);
  });
  it('a message of only a placeholder is one chip', () => {
    expect(pasteRuns([{ t: placeholder(1, 5000) }], byN).map((r) => !!r.p)).toEqual([true]);
  });
  it('no pastes: the runs are returned unchanged', () => {
    const runs = [{ t: placeholder(1, 5000) }];
    expect(pasteRuns(runs, new Map())).toBe(runs);
  });
});

describe('labels', () => {
  it('the chip parts and name', () => {
    expect(chipParts({ n: 1, chars: 5000, lines: 120 })).toEqual({ n: '#1', chars: '5 000 chars', lines: '120 lines' });
    expect(chipParts({ n: 2, chars: 900, lines: 1 }).lines).toBe('1 line');
    expect(chipName({ n: 1, chars: 5000, lines: 120 })).toBe('Pasted text 1, 5 000 characters, 120 lines');
    expect(chipName({ n: 3, chars: 1, lines: 1 })).toBe('Pasted text 3, 1 character, 1 line');
  });
  it('the panel header always has the full numbers', () => {
    expect(panelHead({ n: 1, chars: 5000, lines: 120 })).toBe('#1 · 5 000 chars · 120 lines');
    expect(panelHead({ n: 2, chars: 900, lines: 1 })).toBe('#2 · 900 chars · 1 line');
  });
});

describe('the pastes of an entry', () => {
  it('a sent entry: meta.pastes', () => {
    const e = { meta: { pastes: [{ n: 1, chars: 5000, lines: 120, path: '/x' }] } };
    expect(pastesOf(e).get(1)).toMatchObject({ n: 1, chars: 5000, lines: 120 });
  });
  it('an unsent entry keeps the text in memory', () => {
    const e = { meta: { pastes: [paste(1, 5000, 120, 'abc')] } };
    expect(pastesOf(e).get(1).text).toBe('abc');
  });
  it('a waking entry: context.pastes, counted like the core (normalised, code points, lines)', () => {
    const e = { context: { pastes: [{ n: 2, text: 'a\r\nb😀' }] } };
    expect(pastesOf(e).get(2)).toEqual({ n: 2, chars: 4, lines: 2, text: 'a\nb😀' });
  });
  it('none', () => {
    expect(pastesOf({}).size).toBe(0);
    expect(pastesOf(null).size).toBe(0);
  });
});

describe('the smallest edit between two drafts', () => {
  it('an insertion', () => {
    expect(diffEdit('ab', 'aXb')).toEqual({ from: 1, to: 1, s: 'X' });
  });
  it('a replacement', () => {
    expect(diffEdit('see @re ok', 'see @refunds  ok')).toEqual({ from: 7, to: 7, s: 'funds ' });
  });
  it('a deletion', () => {
    expect(diffEdit('abc', 'ac')).toEqual({ from: 1, to: 2, s: '' });
  });
  it('no change', () => {
    expect(diffEdit('abc', 'abc')).toEqual({ from: 3, to: 3, s: '' });
  });
});
