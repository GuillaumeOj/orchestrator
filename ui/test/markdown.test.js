import { fireEvent, getQueriesForElement, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('../src/lib/app.svelte.js', () => ({ app: { themeRev: 0 }, openUrl: vi.fn(), toast: vi.fn() }));
vi.mock('../src/lib/ipc.js', () => ({
  invoke: vi.fn(),
  errText: (e) => (typeof e === 'string' ? e : (e?.message ?? String(e))),
}));
vi.mock('mermaid', () => ({ default: { initialize: vi.fn(), render: vi.fn() } }));

const { openUrl, toast } = await import('../src/lib/app.svelte.js');
const { invoke } = await import('../src/lib/ipc.js');
const { default: mermaid } = await import('mermaid');
const { default: Markdown } = await import('../src/components/Markdown.svelte');
const { frontPairs, imageTarget, linkKind, loadHighlight, parse, prepare, slug } = await import(
  '../src/lib/markdown.js'
);
const { HL_MAX_LINES, highlight } = await import('../src/lib/highlight.js');
const { sanitizeSvg } = await import('../src/lib/mermaid.js');

afterEach(() => vi.clearAllMocks());

/** The tree, loosely typed: tests reach into nodes of known shapes. */
const tree = (md) => /** @type {any} */ (parse(md));

/** Node types in document order (depth first). */
function types(n, out = []) {
  out.push(n.type);
  for (const c of n.children ?? []) types(c, out);
  return out;
}

const FIXTURE = `---
title: Checkout v2
owner: lead
---

Setext title
============

Para with *em*, **strong**, \`code\`, ~~gone~~, a [link](https://x.example "T"), an
autolink www.example.com, &amp; an escape \\* and a hard\\
break.

    indented code

\`\`\`rust
fn main() {}
\`\`\`

> quote with a list:
> - one
>   1. nested
> - two

5. five
6. six

- [ ] todo
- [x] done

---

![alt](design/a.png) [ref][r] note[^1]

[r]: https://ref.example
[^1]: The note.

| a | b |
|:--|--:|
| x \\| y | 2 |
`;

describe('markdown parser', () => {
  it('reads every CommonMark + GFM construct into the tree', () => {
    const t = types(tree(FIXTURE));
    for (const ty of [
      'yaml',
      'heading',
      'paragraph',
      'emphasis',
      'strong',
      'inlineCode',
      'delete',
      'link',
      'break',
      'code',
      'blockquote',
      'list',
      'listItem',
      'thematicBreak',
      'image',
      'linkReference',
      'definition',
      'footnoteReference',
      'footnoteDefinition',
      'table',
      'tableRow',
      'tableCell',
    ])
      expect(t, ty).toContain(ty);
    const root = tree(FIXTURE);
    expect(root.children[0].type).toBe('yaml');
    expect(root.children[1]).toMatchObject({ type: 'heading', depth: 1 });
    const codes = root.children.filter((n) => n.type === 'code');
    expect(codes.map((c) => [c.lang, c.value])).toEqual([
      [null, 'indented code'],
      ['rust', 'fn main() {}'],
    ]);
    const lists = root.children.filter((n) => n.type === 'list');
    expect(lists[0]).toMatchObject({ ordered: true, start: 5 });
    expect(lists[1].children.map((li) => li.checked)).toEqual([false, true]);
    const quote = root.children.find((n) => n.type === 'blockquote');
    expect(quote.children[1].children[0].children[1]).toMatchObject({ type: 'list', ordered: true });
  });

  it('keeps an escaped pipe inside a table cell and reads the alignment', () => {
    const table = tree(FIXTURE).children.find((n) => n.type === 'table');
    expect(table.align).toEqual(['left', 'right']);
    expect(table.children[1].children[0].children[0].value).toBe('x | y');
  });

  it('slugs headings like GitHub, unique within the doc', () => {
    expect(slug('2. Code & colors')).toBe('2-code--colors');
    const { root } = /** @type {any} */ (prepare('# Scope\n\n## Scope\n\n## `api` · v2'));
    expect(root.children.map((h) => h.data.slug)).toEqual(['scope', 'scope-1', 'api--v2']);
    const taken = /** @type {any} */ (prepare('# Foo\n\n# Foo\n\n# Foo 1'));
    expect(taken.root.children.map((h) => h.data.slug)).toEqual(['foo', 'foo-1', 'foo-1-1']);
  });

  it('numbers footnotes by first reference', () => {
    const { footnotes } = prepare('b[^b] a[^a] b again[^b]\n\n[^a]: A\n[^b]: B\n[^c]: unused');
    expect(footnotes.map((f) => [f.num, f.node.identifier])).toEqual([
      [1, 'b'],
      [2, 'a'],
    ]);
  });

  it('classifies link targets', () => {
    expect(linkKind('https://a.example')).toEqual({ kind: 'external', url: 'https://a.example' });
    expect(linkKind('mailto:me@x.example').kind).toBe('external');
    expect(linkKind('#2-code')).toEqual({ kind: 'anchor', anchor: '2-code' });
    expect(linkKind('javascript:alert(1)').kind).toBe('text');
    expect(linkKind('JavaScript:alert(1)', true).kind).toBe('text');
    expect(linkKind('file:///etc/passwd').kind).toBe('text');
    expect(linkKind('./ux-brief.md#2-code', true)).toEqual({ kind: 'doc', name: 'ux-brief.md', anchor: '2-code' });
    expect(linkKind('api-contract.md', true)).toEqual({ kind: 'doc', name: 'api-contract.md', anchor: null });
    expect(linkKind('api-contract.md').kind).toBe('repo');
    expect(linkKind('../../../docs/SPEC.md', true)).toEqual({ kind: 'repo', path: '../../../docs/SPEC.md' });
    // A browser drops tabs and leading controls from an href: never a `javascript:` in disguise.
    expect(linkKind('java\tscript:alert(1);x.md', true).kind).toBe('text');
    expect(linkKind('\u0001javascript:alert(1);x.md', true).kind).toBe('text');
  });

  it('reads flat front-matter as pairs', () => {
    expect(frontPairs('title: A\nowner: lead')).toEqual([
      ['title', 'A'],
      ['owner', 'lead'],
    ]);
    expect(frontPairs('list:\n  - a')).toBeNull();
  });

  it('highlights known languages only, and not very long blocks', () => {
    const runs = highlight('fn main() { let x = "s"; }', 'rust');
    expect(runs.map((r) => r.v).join('')).toBe('fn main() { let x = "s"; }');
    expect(runs.find((r) => r.v === 'fn').t).toBe('keyword');
    expect(runs.find((r) => r.v === 'main').t).toBe('function');
    expect(runs.find((r) => r.v === '"s"').t).toBe('string');
    expect(highlight('[a]\nb = 1', 'toml')).not.toBeNull();
    expect(highlight('let x = 1', null)).toBeNull();
    expect(highlight('let x = 1', 'nope')).toBeNull();
    expect(highlight('x\n'.repeat(HL_MAX_LINES + 1), 'js')).toBeNull();
  });
});

// fix-md-rendering-quoted-tables §4: a table right after a quote, rows without `>`.
describe('lazy table after a quote', () => {
  // p-4-1-one-now/ux-brief.md §2, shape kept: a quote, then the table without a blank line.
  const BRIEF = `> **Superseded by [§2.1](#21-the-pattern)**: \`waiting_on\` is one line now. The table below is the first draft.
| waiting_on.kind | Pattern | Examples |
|---|---|---|
| user | \`<what>: <subject>\` | \`review: T-12\` |
| gate | \`<gate>: <subject>\` | \`spec gate: checkout\` |
| review | \`review: <branch>\` | [example](https://x.example) |
| merge | \`merge: <branch>\` | \`merge: front-1\` |
| task | \`task: <id>\` | \`task: T-3\` |
| message | \`<node>: <topic>\` | \`lead: scope\` |
| ci | \`ci: <check>\` | \`ci: lint\` |
| none | — | — |`;

  /** The types of a node's children. */
  const kids = (n) => n.children.map((c) => c.type);

  it('renders the brief as a quote, then a 3-column table with 8 body rows', () => {
    const root = tree(BRIEF);
    expect(kids(root)).toEqual(['blockquote', 'table']);
    expect(types(root.children[0])).not.toContain('table');
    const [head, ...body] = root.children[1].children;
    expect(head.children).toHaveLength(3);
    expect(body).toHaveLength(8);
    expect(body.every((r) => r.children.length === 3)).toBe(true);
    expect(body[0].children[1].children[0]).toMatchObject({ type: 'inlineCode', value: '<what>: <subject>' });
    expect(body[2].children[2].children[0]).toMatchObject({ type: 'link', url: 'https://x.example' });
  });

  it('keeps a table with `>` on every row inside the quote', () => {
    const root = tree('> text\n> | a | b |\n> |---|---|\n> | 1 | 2 |');
    expect(kids(root)).toEqual(['blockquote']);
    expect(kids(root.children[0])).toEqual(['paragraph', 'table']);
  });

  it('leaves plain lazy text in the quote paragraph', () => {
    const root = tree('> text\nmore text');
    expect(kids(root)).toEqual(['blockquote']);
    expect(root.children[0].children[0].children[0].value).toBe('text\nmore text');
  });

  it('leaves a `|` line without a delimiter row as lazy text', () => {
    const root = tree('> text\n| not | a table');
    expect(kids(root)).toEqual(['blockquote']);
    expect(types(root)).not.toContain('table');
    // A delimiter row with another cell count isn't a table either.
    expect(types(tree('> text\n| a | b |\n|---|'))).not.toContain('table');
  });

  it('leaves fenced code verbatim, quoted or not', () => {
    const body = '> x\n| a | b |\n|---|---|';
    for (const fence of ['```', '~~~~']) {
      const root = tree(`${fence}\n${body}\n${fence}`);
      expect(root.children).toHaveLength(1);
      expect(root.children[0]).toMatchObject({ type: 'code', value: body });
    }
    const quoted = tree('> ```\n> > x\n> | a | b |\n> |---|---|\n> ```');
    expect(quoted.children[0].children[0]).toMatchObject({ type: 'code', value: '> x\n| a | b |\n|---|---|' });
    // A quoted fence line inside an unquoted fence is content, not its close.
    const shown = '> ```\n> x\n| a | b |\n|---|---|';
    expect(tree(`\`\`\`md\n${shown}\n\`\`\``).children[0]).toMatchObject({ type: 'code', value: shown });
    // A quoted fence ends with its quote: the next fence is a new block.
    const after = tree(`> \`\`\`\n> code\n\n\`\`\`\n${body}\n\`\`\``);
    expect(after.children[1]).toMatchObject({ type: 'code', value: body });
    // A fence in a list item.
    const listed = tree('- ```\n  > x\n  | a | b |\n  |---|---|\n  ```');
    expect(listed.children[0].children[0].children[0]).toMatchObject({ type: 'code', value: body });
    // ``` with a backtick after it is inline code, not a fence.
    expect(tree(`\`\`\`inline\`\`\` is code.\n\n\`\`\`\n${body}\n\`\`\``).children.at(-1)).toMatchObject({
      type: 'code',
      value: body,
    });
  });

  it('still fixes a lazy table after a quoted fence closed by its quote', () => {
    expect(kids(tree('> ```\n> code\n\ntext\n\n> quote\n| a | b |\n|---|---|'))).toEqual([
      'blockquote',
      'paragraph',
      'blockquote',
      'table',
    ]);
  });

  it('leaves a `>` line inside an HTML block alone', () => {
    expect(kids(tree('<div>\n> x\n| a | b |\n|---|---|\n</div>'))).toEqual(['html']);
  });

  it('leaves a lazy row indented as code alone, and counts `\\\\|` as a cell edge', () => {
    const lazy = [
      '> q\n    | a | b |\n|---|---|\n| 1 | 2 |',
      '> q\n\t| a | b |\n|---|---|\n| 1 | 2 |',
      '> q\n \t| a | b |\n|---|---|',
      // A delimiter row indented 4+ columns can't make a table: the rows stay lazy.
      '> q\n| a | b |\n        ---|---',
      '> q\n| a | b |\n\t---|---',
    ];
    for (const md of lazy) expect(kids(tree(md)), md).toEqual(['blockquote']);
    expect(kids(tree('> q\n| a \\\\| b |\n|---|---|'))).toEqual(['blockquote', 'table']);
  });

  it('ends nested quotes and alerts before the table; the alert stays an alert', () => {
    const nested = tree('>> text\n| a | b |\n|---|---|\n| 1 | 2 |');
    expect(kids(nested)).toEqual(['blockquote', 'table']);
    expect(nested.children[0].children[0].type).toBe('blockquote');

    const root = /** @type {any} */ (prepare('> [!NOTE]\n> Read this.\n| a | b |\n|---|---|\n| 1 | 2 |').root);
    expect(kids(root)).toEqual(['blockquote', 'table']);
    expect(root.children[0].data).toMatchObject({ alert: 'note' });
    expect(root.children[0].children[0].children[0].value).toBe('Read this.');
  });
});

describe('Markdown renderer', () => {
  it('renders a table with header cells and column alignment', () => {
    const { container } = render(Markdown, { text: '| a | b | c |\n|:--|:-:|--:|\n| x \\| y | `z` | 3 |' });
    const ths = /** @type {HTMLElement[]} */ ([...container.querySelectorAll('table thead th')]);
    expect(ths.map((th) => [th.textContent, th.style.textAlign])).toEqual([
      ['a', 'left'],
      ['b', 'center'],
      ['c', 'right'],
    ]);
    const tds = [...container.querySelectorAll('tbody td')];
    expect(tds[0].textContent).toBe('x | y');
    expect(tds[1].querySelector('code').textContent).toBe('z');
  });

  it('colors code with hl- spans and copies the raw text', async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    const src = 'fn main() {\n    println!("hi");\n}';
    const { container, getByText } = render(Markdown, { text: `\`\`\`rust\n${src}\n\`\`\`` });
    expect(container.querySelector('pre code').textContent).toBe(src);
    // lib/highlight.js loads on the first block with a language.
    await vi.waitFor(() => expect(container.querySelector('.hl-keyword')?.textContent).toBe('fn'));
    expect(container.querySelector('pre code').textContent).toBe(src);
    expect(container.querySelector('.hl-string').textContent).toBe('"hi"');
    expect(getByText('rust')).toBeTruthy();
    await fireEvent.click(getByText('Copy'));
    expect(writeText).toHaveBeenCalledWith(src);
    await vi.waitFor(() => expect(toast).toHaveBeenCalledWith('Copied'));
  });

  it('renders unknown languages as plain code', async () => {
    const { container } = render(Markdown, { text: '```nope\nx\n```\n\n```\ny\n```' });
    // The highlighter has loaded (the block with a language asked for it): still no colors.
    await loadHighlight();
    await tick();
    expect(container.querySelectorAll('pre code')).toHaveLength(2);
    expect(container.querySelector('[class^="hl-"]')).toBeNull();
  });

  it('renders nested lists, task boxes, quotes, footnotes and front-matter', () => {
    const { container } = render(Markdown, { text: FIXTURE });
    expect(container.querySelector('dl.front dt').textContent).toBe('title');
    expect(container.querySelector('h1').textContent).toBe('Setext title');
    expect(container.querySelector('blockquote ul ol li').textContent).toBe('nested');
    expect(container.querySelector('ol[start="5"]')).not.toBeNull();
    const boxes = /** @type {HTMLInputElement[]} */ ([...container.querySelectorAll('li.task input[type=checkbox]')]);
    expect(boxes.map((b) => [b.checked, b.disabled])).toEqual([
      [false, true],
      [true, true],
    ]);
    expect(container.querySelector('del').textContent).toBe('gone');
    const ref = container.querySelector('sup.fnref a');
    expect(ref.textContent).toBe('1');
    const note = container.querySelector('.footnotes li');
    expect(note.id).toBe(ref.getAttribute('href').slice(1));
    expect(note.textContent).toContain('The note.');
    expect(container.querySelector('.footnotes a.back').getAttribute('href')).toBe(`#${ref.id}`);
    // The ref link resolves through its definition.
    expect([...container.querySelectorAll('a.ext')].map((a) => a.getAttribute('href'))).toContain(
      'https://ref.example',
    );
  });

  it('opens external links through openUrl and keeps other schemes as text', async () => {
    const { container, getByText } = render(Markdown, {
      text: '[site](https://x.example) [mail](mailto:a@x.example) [bad](javascript:alert(1)) [file](file:///etc/hosts)',
    });
    await fireEvent.click(getByText('site'));
    expect(openUrl).toHaveBeenCalledWith('https://x.example');
    expect(getByText('site').closest('a').textContent).toBe('site↗');
    expect(getByText('mail').closest('a')).not.toBeNull();
    for (const t of ['bad', 'file']) {
      expect(getByText(t).closest('a')).toBeNull();
      expect(getByText(t).hasAttribute('href')).toBe(false);
    }
    expect(container.querySelector('[href^="javascript"], [href^="file"]')).toBeNull();
  });

  it('switches docs through ondoc, else shows the link as a repo path', async () => {
    const ondoc = vi.fn();
    const { container } = render(Markdown, {
      text: '[contract](api-contract.md) [ux](./ux-brief.md#2-code) [repo](../../docs/SPEC.md)',
      ondoc,
    });
    const getByText = (t) => getQueriesForElement(container).getByText(t);
    await fireEvent.click(getByText('contract'));
    expect(ondoc).toHaveBeenCalledWith('api-contract.md', null);
    await fireEvent.click(getByText('ux'));
    expect(ondoc).toHaveBeenCalledWith('ux-brief.md', '2-code');
    const repo = getByText('repo');
    expect(repo.closest('a')).toBeNull();
    expect(repo.getAttribute('title')).toBe('../../docs/SPEC.md');

    const plain = render(Markdown, { text: '[contract](api-contract.md)' });
    expect(plain.container.querySelector('a')).toBeNull();
    expect(plain.container.querySelector('.repo').textContent).toBe('contract');
  });

  it('scrolls to a heading of the same doc, with ids unique per instance', async () => {
    const a = render(Markdown, { text: '# Top\n\n[go](#top)' });
    const b = render(Markdown, { text: '# Top' });
    const ha = a.container.querySelector('h1');
    expect(ha.id).toMatch(/top$/);
    expect(ha.id).not.toBe(b.container.querySelector('h1').id);
    ha.scrollIntoView = vi.fn();
    await fireEvent.click(a.getByText('go'));
    expect(ha.scrollIntoView).toHaveBeenCalled();
  });

  it('without a feature: remote chip, not-available line, never a request', async () => {
    const { container, getByText } = render(Markdown, {
      text: '![shot](https://x.example/p.png) ![home](design/home.png) ![x](data:text/html,hi)',
    });
    expect(container.querySelector('img')).toBeNull();
    await fireEvent.click(getByText('🖼 shot ↗'));
    expect(openUrl).toHaveBeenCalledWith('https://x.example/p.png');
    expect(container.textContent).toContain('image: home (not available here)');
    expect(container.textContent).not.toContain('data:');
    expect(invoke).not.toHaveBeenCalled();
  });

  it('loads a relative image through feature_asset and opens it large', async () => {
    vi.mocked(invoke).mockResolvedValue({ mime: 'image/png', data_b64: 'AAAA', bytes: 3 });
    const { container, getByLabelText } = render(Markdown, {
      text: '![Home](./design/home%20page.png#frag) ![remote](https://x.example/r.png)',
      feature: 'f-1',
    });
    expect(container.querySelector('.holder').textContent).toBe('Home');
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith('feature_asset', { feature: 'f-1', path: 'design/home page.png' });
    await vi.waitFor(() => expect(container.querySelector('img.pic')).not.toBeNull());
    const img = container.querySelector('img.pic');
    expect(img.getAttribute('src')).toBe('data:image/png;base64,AAAA');
    expect(img.getAttribute('alt')).toBe('Home');
    await fireEvent.click(getByLabelText('Open Home large'));
    expect(container.querySelector('button.overlay img')).not.toBeNull();
    await fireEvent.keyDown(window, { key: 'Escape' });
    expect(container.querySelector('.overlay')).toBeNull();
  });

  it('shows the error line when feature_asset refuses', async () => {
    vi.mocked(invoke).mockRejectedValue('outside the feature folder');
    const { container } = render(Markdown, { text: '![x](../../secret.png)', feature: 'f-1' });
    await vi.waitFor(() => expect(container.textContent).toContain('image: x (outside the feature folder)'));
    expect(container.querySelector('img')).toBeNull();
  });

  it('reads image targets', () => {
    expect(imageTarget('./design/a%20b.png?v=2')).toEqual({ kind: 'asset', path: 'design/a b.png' });
    expect(imageTarget('https://x.example/a.png').kind).toBe('remote');
    expect(imageTarget('data:image/png;base64,AAAA').kind).toBe('data');
    expect(imageTarget('data:image/svg+xml;base64,AAAA').kind).toBe('dropped');
    expect(imageTarget('file:///etc/a.png').kind).toBe('other');
  });

  it('lets a badge inside a link open the link only', async () => {
    const { container, getByText } = render(Markdown, {
      text: '[![CI](https://x.example/b.svg)](https://x.example/ci)',
    });
    expect(container.querySelector('a button')).toBeNull();
    await fireEvent.click(getByText('🖼 CI'));
    expect(openUrl).toHaveBeenCalledTimes(1);
    expect(openUrl).toHaveBeenCalledWith('https://x.example/ci');
  });

  it('pads short table rows and keeps footnote paragraphs apart', () => {
    const { container } = render(Markdown, {
      text: '| a | b | c |\n|---|---|---|\n| 1 |\n\nx[^1]\n\n[^1]: First.\n\n    Second.',
    });
    expect(container.querySelectorAll('tbody td').length).toBe(3);
    expect([...container.querySelectorAll('.footnotes li p')].map((p) => p.textContent)).toEqual(['First.', 'Second.']);
  });

  it('shows raw HTML as muted text, except <br>', () => {
    const { container } = render(Markdown, { text: 'a<br>b <span>c</span>\n\n<div>\nblock\n</div>' });
    expect(container.querySelector('p br')).not.toBeNull();
    expect(container.querySelector('span:not([class])')).toBeNull();
    expect([...container.querySelectorAll('.raw')].map((e) => e.textContent)).toEqual([
      '<span>',
      '</span>',
      '<div>\nblock\n</div>',
    ]);
  });

  it('renders GitHub alerts as tinted quotes', () => {
    const { container } = render(Markdown, { text: '> [!WARNING]\n> Mind the gap.' });
    const q = container.querySelector('blockquote.alert.warning');
    expect(q.querySelector('.alert-title').textContent).toBe('Warning');
    expect(q.textContent).not.toContain('[!WARNING]');
    expect(q.textContent).toContain('Mind the gap.');
    const lower = render(Markdown, { text: '> [!note]\n> Lower case.' }).container;
    expect(lower.querySelector('blockquote.alert.note .alert-title').textContent).toBe('Note');
  });

  it('never renders HTML from the text', () => {
    const { container } = render(Markdown, {
      text: [
        '# <img src=x onerror=alert(1)>',
        '',
        'text <script>alert(1)</script> **b**',
        '',
        '| a | b |',
        '|---|---|',
        '| <img src=x onerror=alert(1)> | <script>alert(2)</script> |',
        '',
        '[t](https://x.example "<img src=x onerror=alert(1)>") ![<img src=x onerror=alert(1)>](https://x.example/i.png)',
        '',
        'note[^n]',
        '',
        '[^n]: <img src=x onerror=alert(1)> <iframe src="javascript:alert(1)"></iframe>',
      ].join('\n'),
    });
    expect(container.querySelector('img, script, iframe')).toBeNull();
    expect(container.querySelector('[onerror], [onclick], [src]')).toBeNull();
    expect(container.textContent).toContain('<script>alert(1)</script>');
    expect(container.querySelector('td').textContent).toBe('<img src=x onerror=alert(1)>');
    expect(container.querySelector('a.ext').getAttribute('title')).toBe('<img src=x onerror=alert(1)>');
    expect(container.querySelector('.chip').textContent).toContain('<img src=x onerror=alert(1)>');
    expect(container.querySelector('.footnotes').textContent).toContain('<img src=x onerror=alert(1)>');
    expect(container.querySelector('strong').textContent).toBe('b');
  });
});

describe('Mermaid diagrams', () => {
  const ok = (svg) => vi.mocked(mermaid.render).mockResolvedValue(/** @type {any} */ ({ svg }));

  it('loads mermaid on a diagram only and inserts its SVG', async () => {
    ok('<svg id="m" viewBox="0 0 10 10"><g class="node"><rect width="5" height="5"></rect><text>A</text></g></svg>');
    const { container } = render(Markdown, { text: '```mermaid\ngraph TD; A-->B\n```' });
    await vi.waitFor(() => expect(container.querySelector('.diagram svg')).not.toBeNull());
    expect(mermaid.render).toHaveBeenCalledWith(expect.any(String), 'graph TD; A-->B', expect.any(HTMLElement));
    expect(mermaid.initialize).toHaveBeenCalledWith(
      expect.objectContaining({
        securityLevel: 'strict',
        theme: 'base',
        // `%%{init}%%` directives can't bring their own CSS or theme.
        secure: expect.arrayContaining(['themeCSS', 'themeVariables', 'theme']),
      }),
    );
    // Mermaid's scratch node is gone.
    expect(document.body.querySelector('div[hidden]')).toBeNull();
    expect(container.querySelector('.diagram text').textContent).toBe('A');
    expect(container.querySelector('pre')).toBeNull();
  });

  it('strips script, handlers, HTML and outside links from the SVG', async () => {
    ok(
      '<svg onload="alert(1)"><script>alert(2)</script><foreignObject><div onclick="alert(3)">x</div></foreignObject>' +
        '<a href="javascript:alert(4)"><text>t</text></a><image href="https://evil.example/p.png"></image>' +
        '<rect onerror="alert(5)" style="fill:url(https://evil.example/t)"></rect><path marker-end="url(#arrow)"></path>' +
        '<style>@import url(https://evil.example/x.css); .a{fill:red}</style></svg>',
    );
    const { container } = render(Markdown, { text: '```mermaid\ngraph TD; A["<script>alert(1)</script>"]\n```' });
    await vi.waitFor(() => expect(container.querySelector('.diagram svg')).not.toBeNull());
    const d = container.querySelector('.diagram');
    expect(d.querySelector('script, foreignObject, div, a, image')).toBeNull();
    for (const el of d.querySelectorAll('*'))
      for (const at of el.attributes) {
        expect(at.name.startsWith('on'), at.name).toBe(false);
        expect(at.value).not.toMatch(/javascript:|evil\.example/);
      }
    expect(d.querySelector('path').getAttribute('marker-end')).toBe('url(#arrow)');
    expect(d.innerHTML).not.toContain('evil.example');
  });

  it('shows the source and the error for an invalid diagram', async () => {
    vi.mocked(mermaid.render).mockRejectedValue(new Error('Parse error on line 1:\nexpecting X'));
    const { container } = render(Markdown, { text: '```mermaid\ngraph ???\n```' });
    await vi.waitFor(() =>
      expect(container.querySelector('.err')?.textContent).toBe('Mermaid: Parse error on line 1:'),
    );
    expect(container.querySelector('pre code').textContent).toBe('graph ???');
    expect(container.querySelector('svg')).toBeNull();
  });

  it('sanitizes on its own too', () => {
    const out = sanitizeSvg(
      '<svg><g onclick="x()"><use href="#a"></use><use href="https://e.example/s.svg#a"></use></g></svg>',
    );
    expect(out).not.toContain('onclick');
    expect(out).not.toContain('e.example');
  });

  it('keeps quoted fragment refs, drops image-set and CSS-escaped urls', () => {
    const out = sanitizeSvg(
      '<svg><path marker-end=\'url("#arrow")\'></path>' +
        '<rect style=\'background:image-set("https://e.example/p.png" 1x)\'></rect>' +
        '<style>svg{background:\\75 rl(https://e.example/t.png)} .a{fill:red}</style></svg>',
    );
    expect(out).toContain('marker-end="url(&quot;#arrow&quot;)"');
    expect(out).not.toContain('e.example');
  });
});
