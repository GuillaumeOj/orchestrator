# P4.5 — UX brief

> Companion of [spec.md](spec.md). The design file is the FigCraft file **in Google Drive** (`~/Library/CloudStorage/GoogleDrive-…/My Drive/Orchestrator.figcraft`, the project file attached to this project; ADR-012, [README](../../../docs/design/README.md)). Do **not** commit `docs/design/Orchestrator.figcraft`. Start from page **04.2** (Mission control, P4.2 lanes, Queue row with Run now and its cost), **04.4** (Launch on feature dialog), **04.3** (cards, `rule_proposal` as the closest card), **04.7** (Project settings: Linear, Allocation, Files), and [p4-2-design-notes.md](../../../docs/design/p4-2-design-notes.md) §1.1 lanes, §1.5 queue row, §1.11 empty states, §1.12 Run now, §3 keys.
> New notes go in `docs/design/p4-5-design-notes.md`. Exports go under `.orchestrator/features/p-4-5-intake/design/` (one PNG per board, prefix `p45-`).

## 1. Principles applied (SPEC §9.1)
- **Starting stays yours**: every start goes through a dialog the user submits; nothing in the lane is one click to spend the week.
- **Every number says what it is**: "≈ 4–7 pp week · team", "updated 12 min ago", "3 pp left this week".
- **Ready is not attention**: the lane never notifies and never counts in "needs you"; it must read calmer than Needs you and Queued (palette distinct from the Board's "Ready" column, FR-502).
- **Keyboard-first**: the lane works with `j/k`, ⏎ and one key per action.

## 2. ux — notes and copy (`p4-5-design-notes.md`), first
Settle each point (copy, states, keys), then hand off to ui. Report findings for lead (contract gaps) in a "Findings for lead" table, as in P4.4.
1. **Lane** *Ready to start*: position (after Queued, before Done today), header with count and per-source status ("refreshing…", "updated 12 min ago", error with **Fix**), Refresh, "n hidden · Show", "+ n more in <source>". Grouping when several projects are shown (by project, then source order?).
2. **Row** per source (GitHub `#123`, Linear `ENG-42`, repo doc path): glyph, ref, title, project, ≤ 2 labels, the estimate for the suggested mode (and the other mode in details), the over-room warning, the suggested-mode actions **Start team run** / **Start solo**, Hide. Density at 1280.
3. **Row details** (`space`): excerpt / body, source link, both estimates with basis ("median of 7 runs in checkout · size M"; "rough: no history yet"), labels, milestone / priority.
4. **Lane states**: project with no source (one line, several projects), source with zero items, source error per code (api-contract §2.1), refreshing, many items (cap 50, "+ n more"), hidden items shown, first launch (cache empty, refreshing).
5. **Prefilled dialogs**: launch (04.4) and new session with "From #123 · GitHub" (or "From the Overwatch's brief"), the brief / message prefilled and editable, slug derived; the `stale` error ("Already started: <run>").
6. **Origin mark** on run and solo rows ("from #123") and in row details / Inspector.
7. **Project settings › Intake** (04.7): None / GitHub / Linear / Repo docs; fields and options per source; Preview (count, 5 titles, error); Save; links to Settings › Integrations; Light copy of the hints (why `orch` issues are excluded, what front matter does). Global: refresh interval and default estimates (where they sit).
8. **Plan with the Overwatch**: the entry points (lane line, palette), the chat draft "Plan a feature for <project>:", the conversation, the **brief card** (rendered brief, project · mode · team, estimate, Launch / Edit / Dismiss), revised / launched / dismissed lines, Overwatch off / down.
9. **Report / planner**: the "Started from intake n of m" line.
10. **Keys** (page 05): on a Ready row (proposal: ⏎ start suggested mode, `t` team run, `s`… check against `s` = suspend on agent rows, `o` open source, `⌫` / `x` hide, `space` details); the brief card's ⏎ / `e` / `r`; the palette entry. Conflict check as p4-2 notes §3.2.
11. **States matrix** for the lane and the brief card.

## 3. ui — FigCraft, after ux hands off
- Page **04.2**: boards "P4.5 · Ready to start · …" (lane in Mission control at 1512 with the fixture, row per source, details, lane states, 1280 density, origin marks on run / solo rows).
- Page **04.4**: launch dialog and new-session dialog prefilled from an item and from a brief.
- Page **04.3**: brief card and its lifecycle; the planning thread.
- Page **04.7**: Project settings › Intake (each source, preview, errors) and the global settings block.
- Page **04.6**: the report line.
- Page **05**: keys of §2.10. `00 Cover` and `docs/design/README.md`: P4.5 rows **In review**; components on **02** if new ones (Ready row, source glyphs); tokens only if needed (`docs/design/tokens.json` through the pipeline).
- Export every new or changed board to `design/` (`p45-*.png`), commit the exports and the README, then hand off to front with the list of boards and the notes' sections. Review front's work on request.
