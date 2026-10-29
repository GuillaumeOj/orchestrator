# Specs Rendering · Feature spec

> Feature `specs-rendering` · Integration branch `orch/specs-rendering/integration`
> Companions: [api-contract.md](api-contract.md) (core ↔ UI) · [ux-brief.md](ux-brief.md) (how each element looks)

## 1. Goal

The user's brief: *"when a md file is rendered in the feature window, inbox, etc., it is not fully rendered. The most obvious bug is tables rendered as raw strings. We need to see a fully visual file: meanings, tables, syntax colors for code, graphics, etc."*

**Done when** a feature doc like this one (or the `p-4-0-stop-the-bleeding` docs, which are full of tables, links and code) renders in the Feature pane and in the Inbox spec gate the way GitHub renders it: tables as tables, code with syntax colors, links you can follow, Mermaid diagrams as diagrams, images from the feature's `design/` folder as images. No agent-written text can inject markup or run script.

## 2. What the code does today (HEAD 44599b6)

| Item | Today |
|---|---|
| Parser | `ui/src/lib/markdown.js`: a hand-written line reader (~100 lines). Knows `#` headings, paragraphs, flat `-`/`1.` lists and task boxes, `>` quotes (one paragraph), fenced code, `---`. Inline: `` `code` ``, `**b**`, `*i*`, `[text](url)` → text only. |
| Missing | **Tables** (shown as `| a | b |` paragraphs), nested lists, list items over several lines, strikethrough, autolinks, links (target dropped), images, footnotes, setext headings, indented code, hard breaks, HTML entities, escapes (`\|`, `\*`), code language (fence info ignored, no colors), Mermaid. |
| Renderer | `ui/src/components/Markdown.svelte`: renders the blocks as Svelte elements, never `{@html}`. Tested in `ui/test/markdown.test.js` (incl. "never renders HTML from the text"). The webview has **no CSP** (`tauri.conf.json` `"csp": null`), so this "elements only" rule is the only guard. |
| Where it is used | Feature pane › Docs (`FeaturePane.svelte:221`), Inbox › Spec gate docs (`InboxPane.svelte:233`), Inbox › PR gate body (`:239`). |
| Agent prose shown raw | Inbox › team question (`ask_user`, `payload.question`, `<pre>` at `:248`) and Inbox › other gates (`payload.detail`, `<pre>` at `:219`). Leads write these in Markdown (tables of options, lists, code). |
| Images | Docs can't show any image. Design exports live under the feature folder (`.orchestrator/features/<slug>/design/`), the only image path today is `artifact_preview` (design artifacts, by artifact id). |

## 3. Scope

### 3.1 A CommonMark + GFM parser (front)
- Replace the hand-written reader with a spec-compliant parser that returns a **syntax tree, not HTML**: `mdast-util-from-markdown` + `mdast-util-gfm` / `micromark-extension-gfm` (the remark parser without the HTML stage). `lib/markdown.js` keeps the role of "text → tree" and the tree is rendered by Svelte components.
- Covered: everything in CommonMark (headings ATX + setext, paragraphs, hard/soft breaks, emphasis, strong, code spans, fenced + indented code, block quotes with any content, nested ordered/unordered lists with `start`, thematic breaks, links, images, autolinks, entities, escapes) and GFM (tables with column alignment, task list items, strikethrough, autolink literals, footnotes).
- **Raw HTML** in the Markdown is never parsed as HTML: an `html` node renders as its literal text (muted mono), except `<br>` / `<br/>` which renders a line break.
- Front-matter (`---\nkey: v\n---` at the top) is not shown as a heading + rule: it renders as a small key/value block or is skipped (front picks; mdast `frontmatter` extension).

### 3.2 Rendering as elements, never `{@html}` of agent text (front)
- A recursive Svelte renderer walks the tree and emits elements (`h1-h6`, `p`, `ul/ol/li`, `table/thead/tbody/tr/th/td`, `blockquote`, `pre/code`, `del`, `a`, `img`, `sup`, `hr`…). Text is always text. The test "never renders HTML from the text" keeps passing and grows (§5).
- **The one exception is Mermaid's SVG** (§3.4): it is produced by Mermaid from the diagram source with `securityLevel: 'strict'`, then passed through **DOMPurify** (SVG profile, no `<foreignObject>` HTML, no `on*`, no `javascript:`/external `href`) before it is inserted. No other `{@html}`.
- Headings get an `id` (GitHub-style slug, made unique within the doc and prefixed per instance so two rendered docs on screen don't clash).

### 3.3 Code with syntax colors (front)
- Fenced code with a language (` ```rust `, ` ```ts `, …) is highlighted with **lowlight** (highlight.js grammars returning a tree, rendered as `<span class="hl-…">` elements: no HTML string). Grammars: lowlight's `common` set plus `toml`, `svelte`/`xml` already in it; unknown or absent language → plain, no guessing (`highlightAuto` is off: wrong guesses look worse than none).
- Colors come from new `--color-syntax-*` tokens (dark and light), see ux-brief §2.
- Each code block shows its language label and a **Copy** button (copies the raw text, toast "Copied").
- Very long blocks (> 2 000 lines or 100 KB) render plain, to keep the pane responsive.
- Inline code spans are not highlighted.

### 3.4 Diagrams: Mermaid (front)
- ` ```mermaid ` fences render as diagrams with the `mermaid` package, **imported lazily** (only when a doc has a diagram, so the app's start bundle doesn't grow).
- Theme follows the app (dark / light, re-render on theme change), font = the app's body font, colors from tokens (ux-brief §3).
- An invalid diagram shows its source as a code block with one error line under it (`Mermaid: <message>`), never a blank or Mermaid's own "bomb" error SVG.
- A diagram wider than the pane scrolls horizontally inside its block.
- Math (`$…$`, ` ```math `) is **out of scope** (decision D3).

### 3.5 Images (back + front)
- `![alt](design/home.png)` with a **relative path** loads the file from the feature's docs folder through a new command `feature_asset` (api-contract §1): png, jpg/jpeg, gif, webp, svg; plain files only (no symlink), confined to `.orchestrator/features/<slug>/` of the integration worktree, same 5 MB cap as `artifact_preview` (`MAX_PREVIEW_BYTES`). SVG is shown through `<img src="data:…">`, where script can't run.
- While loading: a placeholder box with the alt text. On error: a muted line `image: <alt> (<error>)`.
- Click on an image opens it large (reuse the Design section's preview overlay if it fits; else a simple overlay with Esc to close).
- **Remote images** (`http(s)://…`) are **not fetched** (no tracking pixels, no network from a doc): they render as a link chip `🖼 <alt> ↗` that opens in the browser (decision D2). `data:` URLs other than raster images are dropped.
- Images only load where a doc has a feature behind it (Feature pane, spec gate). In the PR body / question / other gates, a relative image renders as the error line `image: <alt> (not available here)`.

### 3.6 Links (front)
- `http(s)://` and `mailto:` → open in the browser through `openUrl` (existing `open_url` command). Shown with a trailing `↗`. Other schemes (`javascript:`, `file:`, …) render as plain text.
- `#anchor` → scrolls to the heading in the same rendered doc.
- A relative link to another doc of the feature (`api-contract.md`, `./ux-brief.md#2-…`) → switches the doc tab in the Feature pane / spec gate (and scrolls to the anchor). The Markdown component exposes an `ondoc(name, anchor)` callback; views without tabs don't pass it and the link renders as text.
- Any other relative link (`../../../docs/SPEC.md`) renders as link-styled text with the path in its tooltip, not clickable (decision D4).
- Footnote refs jump to the note and back.

### 3.7 Where it renders (front)
| Surface | Text | Images / doc links |
|---|---|---|
| Feature pane › Docs | `feature_doc` | yes (feature id) |
| Inbox › Spec gate docs | `gate_detail.docs[].text` | yes (`gate_detail.feature_id`, api-contract §2) |
| Inbox › PR gate body | `gate_detail.body` | no |
| Inbox › team question (`ask_user`) | `payload.question` (was `<pre>`) | no |
| Inbox › other gates | `payload.detail` (was `<pre>`) | no |

The Inbox list rows keep their one/two-line plain previews (no Markdown there). The permission, startup and pane-tail views keep `<pre>`: they show tool input and terminal output, not prose.

### 3.8 Images command and gate field (back)
- `feature_asset(feature, path)` → `{ mime, data_b64, bytes }` (api-contract §1), in `agent-runtime` next to `read_feature_doc`, with tests for: `..`, absolute path, a symlink pointing out of the folder, a disallowed extension, a missing file, too large, a file in `design/` read OK.
- `gate_detail` for `spec_approval` adds `feature_id` (api-contract §2) so the spec gate can ask for images.

## 4. Out of scope
- Editing docs in the app (still "Open in nvim").
- Math / KaTeX (D3), PlantUML, Graphviz, embedded video/HTML.
- Rendering Markdown in agent terminals, the Board or task titles.
- A full-text search inside docs.

## 5. Tests
- **Parser** (`ui/test/markdown.test.js`, extended): a fixture doc covering each construct of §3.1 → the expected node types; a GFM table with alignment and an escaped `\|` in a cell; nested list in a quote; task list; footnote; setext heading; HTML stays text; `javascript:` link has no `href`.
- **Renderer**: tables render `<table>` with `th` alignment; ` ```rust ` has `hl-` spans and the Copy button copies the raw text; a `mermaid` fence calls the (mocked) Mermaid module and inserts sanitized SVG — a diagram source carrying `<script>`/`onerror` produces no script and no handler in the DOM; an invalid diagram shows the source + error line; a relative image calls `feature_asset` with the right args; a remote image never makes a request and renders the chip; an `ondoc` link calls the callback.
- **Security**: the existing "never renders HTML from the text" test, plus `<img src=x onerror=…>` inside a table cell, a link title, an image alt and a footnote.
- **Back**: `feature_asset` path confinement and caps (§3.8); `gate_detail` returns `feature_id`.
- **Real check**: the `p-4-0-stop-the-bleeding` docs and this feature's docs render with tables, colored code and working tab links in the Feature pane and the spec gate (screenshot in the front report).
- `bun run check`, `bun run lint`, `bun run test`, `bun run build` (tokens check) and `cargo test -p agent-runtime -p orch-app` pass.

## 6. Decisions
- **D1 — tree, not HTML.** We render a syntax tree as elements (mdast + lowlight trees) rather than `marked`/`markdown-it` HTML + a sanitizer, because the webview has no CSP and the "no `{@html}` of agent text" rule is the guard the code already relies on. Mermaid SVG is the single, sanitized exception.
- **D2 — no remote fetches.** Remote images are links, not loaded: docs are written by agents, and an image URL is a way to send data out.
- **D3 — no math.** Specs here don't use it; KaTeX adds ~300 KB and fonts. Easy to add later on the same renderer.
- **D4 — repo links not clickable.** Opening repo files needs a "which worktree / which editor" decision that belongs with the editor work, not here.
- **D5 — dependencies.** New UI deps, pinned exact like the others: `mdast-util-from-markdown`, `mdast-util-gfm`, `micromark-extension-gfm`, `mdast-util-frontmatter` + `micromark-extension-frontmatter` (if front renders front-matter), `lowlight`, `mermaid`, `dompurify`. Nothing new on the Rust side (base64 is already there).
