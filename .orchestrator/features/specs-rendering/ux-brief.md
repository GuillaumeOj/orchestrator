# Specs Rendering · UX brief

> Companion of [spec.md](spec.md). No designer on this team and no Penpot pass: the rules below use the existing tokens (`ui/src/styles/tokens.css`, generated from `docs/design/tokens.json`) and GitHub's Markdown look as the reference. Every color is a token, dark and light.

## 1. Text and blocks
- Body text `--text-body`, `--color-text-secondary`; headings `--color-text-primary` (h1 `--text-heading` with a subtle bottom border, h2 `--text-title` with a subtle bottom border, h3–h6 `--text-body-strong`). Vertical rhythm `--space-sm` between blocks, `--space-md` before h1/h2.
- Links `--color-accent-default`, underline on hover; external links end with a small `↗`; non-clickable repo links: same color at `--color-text-muted`, dotted underline, path in tooltip.
- `del` muted with line-through. Footnotes: small superscript refs, notes under a subtle rule at the end, `↩` back link.
- Raw HTML shown as text: `--text-mono-sm`, `--color-text-muted`.

## 2. Code
- Blocks: `--color-surface-chrome`, `--radius-sm`, `--text-mono-sm`, **no wrap** (horizontal scroll, as code is read by columns), language label top-right in `--color-text-muted`, Copy button appears on hover/focus next to it.
- Syntax colors: add to `docs/design/tokens.json` (then `bun run tokens`) a `syntax` group, dark and light: `keyword`, `string`, `number`, `comment` (muted, italic), `function`, `type`, `variable`, `constant`, `attr`, `tag`, `meta`, `addition` / `deletion` (diff, on a tinted background), `punctuation` (= text secondary). Pick them from the terminal theme's ANSI palette already used by the agent panes, so code looks the same in a doc and in a terminal. Map highlight.js classes to these tokens in one place.
- Inline code: `--text-mono-sm`, `--color-text-primary` on `--color-surface-raised`, small horizontal padding, `--radius-sm`.

## 3. Tables
- Full-width up to content, horizontal scroll inside a wrapper when wider than the pane (the pane never scrolls sideways).
- Header row `--text-body-strong` on `--color-surface-raised`; cells padded `--space-xs` `--space-sm`; `--border-width-default` `--color-border-subtle` between rows and columns; alternate rows lightly tinted (`--color-surface-chrome`). Column alignment from the Markdown (`:--`, `:-:`, `--:`). Cells wrap text; code in cells doesn't wrap.

## 4. Lists, quotes, rules
- Nested lists indent `--space-xl` per level with bullet styles disc → circle → square. Task items: a real (disabled) checkbox look, not `☐` glyphs; done items keep normal color.
- Quotes: left bar `--border-width-focus` `--color-border-strong`, text `--color-text-muted`, may hold any block. GitHub alerts (`> [!NOTE]`, `[!WARNING]`…) are nice to have: tinted bar with status colors if cheap, else plain quotes.

## 5. Diagrams and images
- Mermaid: centered, on `--color-surface-pane`, max width 100 %, horizontal scroll if larger. Mermaid `base` theme with `themeVariables` from tokens (primary = `--color-surface-raised`, lines = `--color-border-strong`, text = `--color-text-primary`, font = app body font). Re-render on theme switch.
- Images: max width 100 %, `--radius-sm`, subtle border; loading placeholder = a box of `--color-surface-chrome` with the alt text muted; click → large preview (same overlay as the Design section). Remote image chip: `--color-surface-raised` pill, `🖼 alt ↗`.

## 6. Keyboard and selection
- Text stays selectable (as today). Links and the Copy buttons are focusable with Tab; Enter activates. No new pane keys.
