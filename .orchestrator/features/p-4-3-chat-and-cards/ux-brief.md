# P4.3 — UX brief

> Companion of [spec.md](spec.md). The design file is FigCraft [`docs/design/Orchestrator.figcraft`](../../../docs/design/Orchestrator.figcraft) (Git LFS; [README](../../../docs/design/README.md), ADR-012). Start from P4.2 Part B: page **04.3 Chat column**, page **06.1 J12**, page **03** P4.2 row (1512 with chat, 1728, 1280), and [p4-2-design-notes.md](../../../docs/design/p4-2-design-notes.md) §4 and §8, plus the P4.1 copy in [p4-design-notes.md](../../../docs/design/p4-design-notes.md) §3–5, 7. Exports of Part B: `.orchestrator/features/p-4-2-mission-control/design/p42b-*.png`.
> New notes go in `docs/design/p4-3-design-notes.md`. Exports go under `.orchestrator/features/p-4-3-chat-and-cards/design/` (one PNG per board, prefix `p43-`).

## 1. Principles applied (SPEC §9.1)
- **Nothing requires a pane** (9): every Inbox action is on its card, in a 360 px column (340 at 1280).
- **One source of state** (10): the card shows the core's lifecycle; nothing is claimed before the core confirms (P4.1 rule).
- **Every number says what it is** (11): "3 more open", "Today · 5 outcomes", "waiting 4 min".
- **Keyboard-first** (1): every card action keeps its FR-707 key, shown on the card.

## 2. ux — notes and copy (`p4-3-design-notes.md`), first

Settle each point, then hand off to ui (one note per point, with the copy):
1. **The column without the Overwatch** (spec D1): what the header shows where the Overwatch status will go in P4.4, and the footer in place of the chat input (what ⌘L focuses; proposal: a muted line "Cards work here. The Overwatch joins in a later release." and ⌘L focuses the thread's search). No dead input field.
2. **Collapse rules** (spec §3.3): 10 s after an effect the user caused; at once for rule / pane / expiry / core and Dismiss; open, parked, failed, stale stay cards. The line copy for every `line.verb` × `line.by` (api-contract §1), including "· in the pane", "· expired", "· rule: installs".
3. **Threading**: confirm the chronological thread with a follow-up reference ("↳ after: spec approved 14:02", jumps) vs regrouping under the run; recommendation: chronological (D4).
4. **Many open**: the fold past 5 open cards; which ones fold (the older or the newer?) given ⌘⇧I focuses the oldest.
5. **Filter** in the header: All / Blocking / Errors / Budget, applies to cards only; how a filter that hides open cards says so.
6. **Older days and search**: how to reach yesterday and a past day, the day separator, a past day read-only (lines only, expandable, no actions), search field and result list, back to today (key).
7. **Digest** at the end of the thread: "Today · 5 outcomes · Clear c"; empty and "all caught up" copy.
8. **Notification-opened card**: the highlight; the toast when that decision already closed ("Already allowed in the pane · 14:02"); the copy of the notification itself (single and batched).
9. **Shell**: 1280 / 1512 / 1728 with the chat built; Inspector overlay in the chat slot below 1728 (P4.2 notes §4 rules 1–5), the banner "1 card needs you · ⌘⇧I" over the overlay; zoomed pane (does the chat stay?); shell dock under the centre only.
10. **Keys** (page 05): remove the Inbox map; chat map (⌘⇧I, ⌘L, `j/k` skip lines, `a r ⏎ o x u e c`, Tab to lines, `⏎` expands, `Esc` back to the centre); conflicts with Mission control's `j/k`, `r`, `d`, `o` when focus moves between the two.
11. **"Inbox" in copy**: the title badge label, the palette entries, Mission control's Needs-you line and `d` hint, the waiting-on chip tooltip, the Decision Inspector button, and every other string that says "Inbox" (grep `ui/src`): the replacement for each.
12. **D15 leftovers** for ui: each of the five cards (restart gate, run now, PR gate, budget, Notion) gets its own body; no block text clipped at the card edge.
13. **States matrix** for the column: no decisions today (morning line only), nothing at all yet (first start), 1 open, 20 open, a past day, search with / without results, core reconnecting.

## 3. ui — FigCraft, after ux hands off
- Page **04.3**: fix the D15 leftovers; add the P4.3 states from §2 (footer without the Overwatch, header with filter / days / search, collapsed lines per verb, fold of many open cards, digest group, day separator, past day, search results, notification-opened highlight, every state of §2.13). Keep the P4.4-only parts (annotation, deciding, rule proposal, Overwatch status) on the page, marked "P4.4".
- Page **03**: the shell with the chat built at 1280 / 1512 / 1728, Inspector overlay and docked, zoomed pane.
- Page **05**: chat keys, the Inbox map removed, conflicts.
- Page **06**: update the J5 storyboard so it plays in the chat (cards instead of the Inbox pane), 4–6 frames.
- `00 Cover` and `docs/design/README.md`: P4.3 rows **In review**; any new token in `docs/design/tokens.json` (and the CSS through the tokens pipeline).
- Export every new or changed board to `design/` (`p43-*.png`), then hand off to front with the list of boards and the notes' sections. Review front's chat column on request.
