# Update Feature Overview Header · Feature spec

> Feature `update-feature-overview-header` · Integration branch `orch/update-feature-overview-header/integration` · UI only (no core, no API change). Design first in the Drive `Orchestrator.figcraft`, then code. Companion: [ux-brief.md](ux-brief.md).

## 1. Problem

The Feature pane (the feature's overview) header packs everything on one row: slug · run phase pill · team name + version · integration branch · `started …` · (Publishing…) · actions. On a normal pane width the row is crowded and the meta text competes with the identity and the actions.

## 2. Today (HEAD f841c019)

`ui/src/panes/FeaturePane.svelte:271-287`, `<header class="fh">` (flex row, `gap: --space-sm`):

1. `.slug` (heading font)
2. `.pill` run phase (`runStateLabel`, coloured, tooltip `since …`)
3. `.muted` span: `<team> v<version> · <branch icon> orch/<slug>/integration` (or `—`) ` · started <ago>`
4. `.grow` spacer
5. `◷ Publishing N docs…` (only while publishing)
6. `.head-acts`: Pause/Resume, Stop (run not finished) · Delete feature… · `⋯` Menu

## 3. Scope

The header becomes **two rows**:

**Row 1 — identity and actions**
- Left: slug, then the run phase pill (same label, colour, tooltip as today).
- Right: the actions exactly as today (same buttons, order, tooltips, keys, `⋯` menu), right-aligned.
- The `◷ Publishing N docs…` indicator stays on row 1, just left of the actions (it is a transient status of an action, not meta).

**Row 2 — meta**
- The other information, in today's order and wording: `<team> v<version>` · `<branch icon> orch/<slug>/integration` (or `—`) · `started <ago>`.
- Same separators (` · `), muted, caption-sized text; aligned with the slug's left edge.
- Pieces that are absent today stay absent (no team → no team part; no run → no `started`); only the pieces present are joined by ` · ` (no leading separator). The branch piece (or `—`) is always there, so row 2 is always rendered.

Width pressure: on row 1 the slug ellipsizes, then the Publishing text (its `◷` stays); the pill and the actions never shrink or wrap. Row 2 stays on one line and ellipsizes at its end. Details in the UX brief.

Out of scope: other pane headers (Project, Agent…), the Notion lines below the header, the menu content, the data shown.

## 4. Acceptance

1. In the running app, the Feature pane header shows slug + phase pill on the left and the actions on the right of the first row, and team · branch · started on a second row below.
2. Actions keep their order, tooltips, aria-labels and keys (`headers.test.js` R9 pattern still passes unchanged in intent).
3. With no run (no phase, no `started`) row 1 shows slug + actions, row 2 shows team · branch.
4. At a narrow pane width, the actions stay on row 1, the slug ellipsizes, row 2 ellipsizes; nothing overflows the pane.
5. Figcraft: the Feature screen's header in the Drive `Orchestrator.figcraft` shows the two rows; exports are committed under `.orchestrator/features/update-feature-overview-header/design/`.
6. Component tests cover the two rows (row 1 holds `.slug`, `.pill`, `.head-acts`; row 2 holds the meta text) and the no-run case; `bun run test`, `bun run check`, `bun run lint` pass in `ui/`.

## 5. Tasks

| Task | Owner | Depends on | Notes |
|---|---|---|---|
| Figcraft: two-row Feature pane header | ui | — | Edit the Drive `Orchestrator.figcraft` (never commit a repo copy); export PNGs to the feature's `design/` folder |
| Code: two-row header + tests | front | ui task | `FeaturePane.svelte` header markup/CSS, `ui/test/headers.test.js` (+ `surfaces.test.js` selector if needed); asks `ui` for a visual review before reporting done |

`ux`: no separate task; the layout rules are settled in [ux-brief.md](ux-brief.md) (the user's brief fixes the content of each row). `back`: no work (no data change).
