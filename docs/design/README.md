# Design

The UI source of truth is the Penpot file **Orchestrator** on design.penpot.app (file id `65d5ee1e-9774-8045-8008-d16813b441c7`). See [ADR-008](../adr/008-ui-design-source-of-truth-penpot.md) and [SPEC §9](../SPEC.md).

## Pages

| Page | Content | Status |
|---|---|---|
| 00 Cover | Purpose, page index, P1 gate, changelog | Done |
| 01 Foundations | Tokens: color, type, spacing, radius, sizing, elevation, rules | In review |
| 02 Components | Components of SPEC §9.4, states as variants | Not started |
| 03 Shell & layouts | Title bar, sidebar + rail, panes, inspector, status bar; 1280 / 1512 / 1728 | Not started |
| 04 Screens | Pane types of SPEC §9.3 + global banners | Not started |
| 05 Keyboard & copy | Shortcut map (§9.7) with conflicts; notification and banner copy | Not started |
| 06 Flows | Prototypes of J2, J5, J8 with key sequences | Not started |
| 07 Explorations | Drafts, not binding | — |

**P1 gate:** before P1 UI work starts, 01, 03 and the P1 panes in 04 (Overview, Agent pane, Week planner, Inbox) are marked ready for dev on `00 Cover`.

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
