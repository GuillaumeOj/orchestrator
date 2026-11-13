# Implement Improve Intake · UX brief (build)

> Companion of [spec.md](spec.md). The design is **done** (improve-intake, approved by the user): this brief says how front uses it and how ui / ux check the result. No new design work unless lead asks.

## 1. Sources, in order of authority
1. [docs/design/improve-intake-design-notes.md](../../../docs/design/improve-intake-design-notes.md): copy, states, keys, rules (decisions D1–D4 of [improve-intake/decisions.md](../improve-intake/decisions.md) amend it).
2. Exports [improve-intake/design/ii-*.png](../improve-intake/design/) (1512 and 1280): the look.
3. The FigCraft file in Google Drive (project file, pages 02, 04.2, 04.3, 04.4, 04.7, 04.8, 05) for exact sizes, tokens and component anatomy (`mcp__figcraft__open_file` then `inspect_document` / `get_node`). Read only: nobody edits the design in this feature.

Tokens come from `docs/design/tokens.json` / the existing CSS variables; no new token (the design added none).

## 2. Boards per front task
| Task | Notes § | Exports |
|---|---|---|
| F1 Tracker settings | 10 | `ii-tracker-*` |
| F2 Feature pane, Board removal | 1.3, 9 | `ii-feature-*`, `ii-board-removed-*` |
| F3 Work browser | 2–5, 11, 12.1 | `ii-work-*`, `ii-components-tree`, `ii-keys` |
| F4 Mission control, start dialogs | 1.2, 5.2, 8, 12.2 | `ii-mc-*`, `ii-dialogs-prefilled` |
| F5 Programs | 6, 7, 12.3–12.4 | `ii-plan-*`, `ii-program-*`, `ii-components-program-row` |

## 3. Reviews
- **ui** reviews each front task's screens against the exports (front asks; ui answers with a list of differences, each "must fix" or "acceptable"). Then U1: a final pass over all boards and `docs/design/README.md` rows → Implemented.
- **ux** (X1) checks copy, keys (notes §12, conflict table §12.7) and states S1–S19 in the built UI and its tests, and reports findings to lead. ux does not change code.
