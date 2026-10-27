import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import Markdown from '../src/components/Markdown.svelte';
import { inline, parse } from '../src/lib/markdown.js';

describe('markdown', () => {
  it('reads headings, paragraphs, lists, tasks, code and quotes', () => {
    const b = parse(
      '# Checkout v2\n\nReplace the\nsingle POST.\n\n## Scope\n- one `api`\n- **two**\n\n1. first\n2. second\n\n- [ ] todo\n- [x] done\n\n```\nlet x = 1;\n```\n> note\n---',
    );
    expect(b.map((x) => x.type)).toEqual(['h', 'p', 'h', 'ul', 'ol', 'ul', 'code', 'quote', 'hr']);
    expect(b[0]).toEqual({ type: 'h', level: 1, spans: [{ t: 'text', s: 'Checkout v2' }] });
    expect(b[1].spans[0].s).toBe('Replace the single POST.');
    expect(b[3].items[0].spans).toEqual([
      { t: 'text', s: 'one ' },
      { t: 'code', s: 'api' },
    ]);
    expect(b[5].items.map((i) => i.checked)).toEqual([false, true]);
    expect(b[6].text).toBe('let x = 1;');
  });

  it('keeps link text and drops the target', () => {
    expect(inline('see [the contract](https://x.example/a)')).toEqual([
      { t: 'text', s: 'see ' },
      { t: 'text', s: 'the contract' },
    ]);
  });

  it('never renders HTML from the text', () => {
    const { container } = render(Markdown, {
      text: '# <img src=x onerror=alert(1)>\n\n<script>alert(1)</script> **b**',
    });
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('script')).toBeNull();
    expect(container.textContent).toContain('<script>alert(1)</script>');
    expect(container.querySelector('b').textContent).toBe('b');
  });
});
