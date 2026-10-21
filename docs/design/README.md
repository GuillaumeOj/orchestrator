# Design

The UI source of truth is the Penpot file **Orchestrator** on design.penpot.app (file id `65d5ee1e-9774-8045-8008-d16813b441c7`). See [ADR-008](../adr/008-ui-design-source-of-truth-penpot.md) and [SPEC §9](../SPEC.md).

## Pages

| Page | Content | Status |
|---|---|---|
| 00 Cover | Purpose, page index, P1 gate, changelog | Done |
| 01 Foundations | Tokens: color, type, spacing, radius, sizing, elevation, rules | In review |
| 02 Components | P1 subset of SPEC §9.4 (14 component sets), states as variants | In review (P1 subset) |
| 03 Shell & layouts | Shell at 1512 (docked inspector), 1280 (icon rail + overlay inspector), zoomed pane; 1728 not drawn | In review |
| 04 Screens | P1 panes: Overview, Agent pane (+ states), Week planner, Inbox | In review (P1 panes) |
| 05 Keyboard & copy | Shortcut map (§9.7) with conflicts; notification and banner copy | Not started |
| 06 Flows | Prototypes J2 solo session (5 frames), J5 Inbox triage (6), J8 pacing the week (4), key sequences annotated | In review |
| 07 Explorations | Drafts, not binding | — |

**P1 gate:** before P1 UI work starts, 01, 03 and the P1 panes in 04 (Overview, Agent pane, Week planner, Inbox) are marked ready for dev on `00 Cover`. As of 2026-10-20 they are all **In review**; Guillaume decides Ready for dev. Open questions and spec gaps from the design pass: [p1-design-notes.md](p1-design-notes.md).

## Screenshots

Exports of the Penpot boards (Dark theme unless noted), refreshed by hand:

| Page | Image |
|---|---|
| 02 Components | [02-components.png](img/02-components.png) |
| 03 Shell · 1512 | [03-shell-1512.png](img/03-shell-1512.png) |
| 03 Shell · 1280 | [03-shell-1280.png](img/03-shell-1280.png) |
| 03 Shell · zoomed pane | [03-shell-zoomed.png](img/03-shell-zoomed.png) |
| 03 Icon rail hover · 1280 | [03-rail-hover.png](img/03-rail-hover.png) |
| 04 Overview | [04-overview.png](img/04-overview.png) |
| 04 Agent pane | [04-agent-pane.png](img/04-agent-pane.png) · states: [04-agent-pane-states.png](img/04-agent-pane-states.png) |
| 04 Week planner | [04-week-planner.png](img/04-week-planner.png) |
| 04 Inbox | [04-inbox.png](img/04-inbox.png) · Light: [04-inbox-light.png](img/04-inbox-light.png) |
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
- **Mono font**: `font.family.mono` is IBM Plex Mono in the designs. In the app it is replaced by the font kitty actually renders (SPEC FR-1002).

Pipeline (to build in P1): Penpot token export → this JSON → Style Dictionary → CSS variables, with a CI check that fails when the code and the export drift. Until that exists, refresh `tokens.json` by hand after token changes in Penpot.

**Contrast**: `text.primary` is ≥ 7:1 and `text.secondary` / `text.muted` are ≥ 4.5:1 on `surface.pane` and `surface.chrome` in both themes. Four `*-aa` palette entries adjust Tokyo Night values that fell short: storm `muted-aa` and day `fg-aa`, `fg-dark-aa`, `muted-aa`. `text.disabled` is exempt.
