# Fix MD Rendering for Quoted Tables — spec

## 1. Problem
`p-4-1-one-now/ux-brief.md` §2 has a quote followed directly by a table, with no blank line between them:

```md
> **Superseded by …**: … The table below is the first draft.
| waiting_on.kind | Pattern | Examples |
|---|---|---|
| user | `<what>: <subject>` | … |
```

The Feature pane and the Inbox spec gate show the table as raw text inside the quote (`| waiting_on.kind | Pattern | … |---|---|---| …`).

## 2. Cause
The GFM renderer (`ui/src/lib/markdown.js`, from `orch/specs-rendering/integration`: micromark + `mdast-util-gfm`) follows CommonMark. A line without `>` right after a quoted paragraph is a **lazy continuation line**: it joins the quote's paragraph. micromark's table construct refuses to start on a lazy line, so every table row is added to the quote as text. Reproduced with the same package versions:

| Input | Tree today |
|---|---|
| quote, then table rows **without** `>` (the brief) | `blockquote > paragraph` holding all the rows as text ✗ |
| quote, then table rows **with** `>` on every line | `blockquote > [paragraph, table]` ✓ |
| quote, blank line, table | `blockquote`, then `table` ✓ |

Tables written correctly inside a quote already render. Only the lazy case breaks.

## 3. Decision
**A table whose rows lack `>` and that directly follows a quote ends the quote. The table renders after the quote, as if a blank line separated them.** That is what the author meant (the quote says "the table below"), and a table can never be a lazy line anyway, so no valid table is lost.

Rule, applied in `parse()` before micromark runs, as a line pre-pass:
- A line is a *lazy table start* when all of these hold:
  - the previous line is a quote line (starts with `>`, after up to 3 spaces);
  - this line does not start with `>`;
  - this line looks like a table row (contains `|`);
  - the next line is a GFM delimiter row (`|? :?-+:? (| :?-+:?)* |?`) with as many cells as this line.
- A blank line is inserted before each lazy table start. Nothing else changes.
- Lines inside fenced code blocks (``` or ~~~, quoted or not) are never touched.

Out of scope: other lazy constructs (a lazy list or heading already ends a quote in CommonMark), and editing `p-4-1-one-now/ux-brief.md` (the renderer must cope with docs agents write).

## 4. Acceptance
1. The brief's §2 renders as a quote and then a 3-column table with 8 body rows. The rows, the inline code and the links render as they do in any other table.
2. `> text\n> | a | b |\n> |---|---|\n> | 1 | 2 |` still renders a table **inside** the quote.
3. `> text\nmore text` still renders one quoted paragraph "text more text" (normal lazy continuation unchanged).
4. `> text\n| not | a table` with no delimiter row: unchanged, the line stays lazy text.
5. A fenced code block containing `> x` then `| a | b |` / `|---|---|` stays verbatim.
6. Nested quotes (`>> text` then a lazy table) and alerts (`> [!NOTE]` then a lazy table) render the table after the quote; the alert stays an alert.
7. Unit tests in `ui/test/markdown.test.js` cover 1–6 (tree level), plus one component test showing a `<table>` after the `<blockquote>`. `bun run test`, `bun run check` and `bun run lint` pass in `ui/`.

## 5. Base branch
The GFM renderer this fix changes is only on `orch/specs-rendering/integration`; `main` still has the old line parser, which has no tables at all. The fix is built on top of `specs-rendering`, so it lands after that branch or together with it.

## 6. Work
One frontend task (`front`): the pre-pass in `ui/src/lib/markdown.js` and its tests. `back` reviews. There is no backend, API or design work.
