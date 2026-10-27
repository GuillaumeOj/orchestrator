# Design

The UI source of truth is the Penpot file **Orchestrator** on design.penpot.app (file id `65d5ee1e-9774-8045-8008-d16813b441c7`). See [ADR-008](../adr/008-ui-design-source-of-truth-penpot.md) and [SPEC §9](../SPEC.md).

## Pages

| Page | Content | Status |
|---|---|---|
| 00 Cover | Purpose, page index, P1 gate, changelog | Done |
| 01 Foundations | Tokens: color, type, spacing, radius, sizing, elevation, rules | Ready for dev |
| 02 Components | P1 subset of SPEC §9.4 (14 component sets), states as variants. **P2**: Canvas node, Edge (4 types), Task card, Kanban column header, Feature card, Rules panel, Preset card, Minimap, Connect menu / target | P1 Ready for dev · **P2 Ready for dev** |
| 03 Shell & layouts | Shell at 1512 (docked inspector), 1280 (icon rail + overlay inspector), zoomed pane; 1728 not drawn | Ready for dev |
| 04 Screens | P1 panes: Overview, Agent pane (+ states), Week planner, Inbox. **P2** (row at y = 2000): Team canvas, Feature, Board, Launch on feature dialog | P1 Ready for dev · **P2 Ready for dev** |
| 05 Keyboard & copy | Global, leader and pane-local shortcut maps (**P2**: Team canvas, Board, Feature rows); P0 conflicts; banner, notification and toast copy; copy rules | P1 Ready for dev · **P2 Ready for dev** |
| 06 Flows | Storyboards (annotated frames, no clickable prototype): J2 solo session (5 frames), J5 Inbox triage (6), J8 pacing the week (4). **P2**: J3 team on a feature, 5 notes over the 04 frames (y = 4500) | P1 Ready for dev · **P2 Ready for dev** |
| 07 Explorations | Drafts, not binding | — |

**P1 gate:** before P1 UI work starts, 01, 03 and the P1 panes in 04 (Overview, Agent pane, Week planner, Inbox) are marked ready for dev on `00 Cover`. Reviewed by Guillaume on 2026-10-26: all pages are **Ready for dev** and the P1 design gate is passed. Open questions and spec gaps from the design pass: [p1-design-notes.md](p1-design-notes.md).

**P2 gate:** before P2 UI work starts (Team canvas, Feature, Board, Launch dialog, P2 keys), the P2 additions on 02, 04, 05 and 06 are marked Ready for dev on `00 Cover`. Status 2026-10-27: **approved as designed by Guillaume; the P2 gate is passed**. The spec and PR gate Inbox items were already designed in P1 (04 Inbox, J2 · 4), so P2 adds no gate screen.

## Screenshots

Exports of the Penpot boards (Dark theme unless noted), refreshed by hand.

| Page | Image |
|---|---|
| 00 Cover | [00-cover.png](img/00-cover.png) |
| 01 Foundations | [01-foundations.png](img/01-foundations.png) |
| 05 Keyboard & copy | [05-keyboard-copy.png](img/05-keyboard-copy.png) |
| 02 Components | [02-components.png](img/02-components.png) |
| 03 Shell · 1512 | [03-shell-1512.png](img/03-shell-1512.png) |
| 03 Shell · 1280 | [03-shell-1280.png](img/03-shell-1280.png) |
| 03 Shell · zoomed pane | [03-shell-zoomed.png](img/03-shell-zoomed.png) |
| 03 Icon rail hover · 1280 | [03-rail-hover.png](img/03-rail-hover.png) |
| 04 Overview | [04-overview.png](img/04-overview.png) |
| 04 Agent pane | [04-agent-pane.png](img/04-agent-pane.png) · states: [04-agent-pane-states.png](img/04-agent-pane-states.png) |
| 04 Week planner | [04-week-planner.png](img/04-week-planner.png) |
| 04 Inbox | [04-inbox.png](img/04-inbox.png) · Light: [04-inbox-light.png](img/04-inbox-light.png) |
| 04 Team canvas (P2) | [04-team-canvas.png](img/04-team-canvas.png) |
| 04 Feature (P2) | [04-feature.png](img/04-feature.png) |
| 04 Board (P2) | [04-board.png](img/04-board.png) |
| 04 Launch on feature (P2) | [04-launch-feature.png](img/04-launch-feature.png) |
| 06 J5 Inbox triage | [1 gate](img/06-j5-1-gate.png) → [2 approved](img/06-j5-2-approved.png) → [3 allowed](img/06-j5-3-allowed.png) → [4 reply](img/06-j5-4-reply.png) → [5 budget ask](img/06-j5-5-budget.png) → [6 clear](img/06-j5-6-clear.png) |
| 06 J2 Solo session | [1 new session](img/06-j2-1-new-session.png) → [2 running](img/06-j2-2-running.png) → [3 palette](img/06-j2-3-palette.png) → [4 PR gate](img/06-j2-4-pr-gate.png) → [5 PR opened](img/06-j2-5-pr-opened.png) |
| 06 J8 Pacing the week | [1 plan](img/06-j8-1-plan.png) → [2 run now](img/06-j8-2-run-now.png) → [3 catch-up](img/06-j8-3-catch-up.png) → [4 report](img/06-j8-4-report.png) |

All screens use one scenario: Wednesday 21 Oct 11:00, Governor in Red (+9 pp), live cap 1; shop-api › checkout-v2 team (lead at gate, back-1 working, front-1 waiting, ui-1 suspended), solo sessions in shop-api and blog, infra team queued.

## Tokens

[`tokens.json`](tokens.json) is a snapshot of the Penpot tokens in W3C design-token form (`$type` / `$value`, references as `{token.name}`).
- **Sets**:
  - `core`: palettes, font families, sizes and weights, typography composites, spacing, radius, border widths, opacity, sizing, breakpoints.
  - `theme/dark`: semantic colors and shadow, from Tokyo Night Storm (the user's kitty theme).
  - `theme/light`: the same names, from Tokyo Night Day.
- **Themes**: group `mode` with `Dark` = core + theme/dark (default) and `Light` = core + theme/light.
- **Naming**: semantic colors are `color.surface.*`, `color.text.*`, `color.border.*`, `color.accent.*`, `color.focus`, `color.status.*` (SPEC §9.5), `color.pacing.*` (ADR-007) and `color.role.1…8`. Code uses semantic tokens only, never `palette.*`.
- **Fixed across kitty-derived themes**: status, pacing and role colors. Only surface, text and accent may be derived from the user's kitty palette (SPEC §9.1).
- **Fonts**: one face for everything, **FiraCode Nerd Font Mono** (`font.family.ui` = `font.family.mono` = `font.family.icon`, since 2026-10-26; was IBM Plex Sans / Mono). In agent and shell panes the app replaces `font.family.mono` with the font kitty actually renders (SPEC FR-1002).
- **Icon font**: `font.family.icon` is the same face (uploaded to the Penpot team; bundled with the app). Typography `text.icon` (status glyphs) and `text.key` (key hints, which need ⌘ ⇧ ⏎). Status icons are Material Design glyphs:

  | State | Glyph | Codepoint |
  |---|---|---|
  | Working | play | U+F040A |
  | Waiting for you | account-alert | U+F0005 |
  | At gate | shield-check | U+F0565 |
  | Queued | timer-sand | U+F051F |
  | Paused | pause | U+F03E4 |
  | Blocked by limit | cancel | U+F073A |
  | Idle | circle-outline | U+F0766 |
  | Suspended | sleep | U+F04B2 |
  | Error | close-circle | U+F0159 |
  | Done | check | U+F012C |

Pipeline (to build in P1): Penpot token export → this JSON → Style Dictionary → CSS variables, with a CI check that fails when the code and the export drift. Until that exists, refresh `tokens.json` by hand after token changes in Penpot.

**Contrast**: `text.primary` is ≥ 7:1 and `text.secondary` / `text.muted` are ≥ 4.5:1 on `surface.pane`, `surface.chrome` and `surface.raised` in both themes. Status and pacing colors (icons, glyphs, pill outlines) are ≥ 3:1 on `surface.raised` and `surface.selected` in both themes (WCAG 1.4.11); the label always carries the meaning too. The `*-aa` palette entries adjust Tokyo Night values that fell short: storm `muted-aa`, `dark5-aa`, `red1-aa`, `suspended-aa`; day `fg-aa`, `fg-dark-aa`, `muted-aa` and the status/pacing `*-aa` entries (cyan, orange, magenta, yellow, red, red1, green, blue1, teal, dark5, suspended). `text.disabled` is exempt.
