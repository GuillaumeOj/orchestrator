# P4.3 — Chat & cards · Feature spec

> Feature `p-4-3-chat-and-cards` · Integration branch `orch/p-4-3-chat-and-cards/integration` · Source: [docs/SPEC.md](../../../docs/SPEC.md) §11 P4.3 (v0.12), FR-1304, FR-1305, FR-707 (from P4.3), J5, §9.2 "From P4.2", §9.3, §9.6 "From P4.3", §9.7 "From P4" · Why: [brief 2026-10-28](../../../docs/brief/2026-10-28-overwatch.md), [ADR-011](../../../docs/adr/011-overwatch-and-attention-model.md)
> Companions: [api-contract.md](api-contract.md) (core ↔ UI) · [ux-brief.md](ux-brief.md) (design pass, copy, states) · [decisions.md](decisions.md)
> Builds on: P4.1 Decisions, context blocks, lifecycle and outcomes ([p-4-1-one-now](../p-4-1-one-now/spec.md)); P4.2 Mission control and the Part B card designs ([p-4-2-mission-control](../p-4-2-mission-control/spec.md), D1, D15; [p4-2-design-notes.md](../../../docs/design/p4-2-design-notes.md) §4, §8).

## 1. Goal and exit criterion

SPEC §11: *"Design pass, then the chat column (FR-1305), cards for every decision kind with their context and lifecycle (FR-1304); the Inbox pane is removed; notification clicks open the card."*

**Exit criterion: every Inbox interaction is available from a card; no decision needs the pane.** Checked by (a) a UI test that, for every decision kind of the P4.1 fixture, finds its card in the chat column with its context blocks and every action the Inbox offered (keyed and keyless), and drives each FR-707 key (`j/k a r ⏎ o x u e`, `c` for the digest) on the cards; (b) `InboxPane.svelte` is gone and no code path opens an `inbox` pane; (c) the demo in §6.

## 2. What the code does today (HEAD 28493ec)

| Item | Today |
|---|---|
| Decisions | The core pushes every Decision (kind, subject, context blocks, actions, lifecycle with `version`, `thread`, `thread_prev`, `waits_on`) in the `now` event; closed ones stay in `now_snapshot` for 10 min (`decision::RECENT_MS`). `inbox_act` acts on them; the UI keeps a 5 s undo window (`app.pending`, `UNDO_MS`). |
| Inbox pane | `InboxPane.svelte` (460 lines) + `lib/inbox.js`: list by level with filters All / Blocking / Errors / Budget, detail with `ContextBlocks`, `LifecycleMarker`, actions; keys `j/k a r ⏎ o x u e c`; the level-4 digest (`OutcomeRow`, Clear). Opened by ⌘⇧I, the title bar badge "Inbox n", the palette, Mission control's `d` and Needs-you line, the waiting-on chip, the Decision Inspector ("Open in Inbox"); `app.inboxFocus` selects a decision. |
| Chat log | None. No `chat` table, no morning line, no record of decisions settled by a rule, in the pane or by expiry once they leave `now` after 10 min. |
| Notifications | `orch-app/src/notify.rs`: levels 1–2 notify (batched), woken by the `now` push. A click only brings the app forward; nothing focuses the decision. |
| Shell | Right column = Inspector, docked ≥ 1512, overlay below. Designed (P4.2 notes §4, In review): chat column 360 (340 at 1280); Inspector overlays the chat's slot below 1728, docked 320 at ≥ 1728. |
| Design | FigCraft page 04.3 (Part B): card anatomy, a card per kind, lifecycle states, collapsed lines, rule proposal, chat states; 06.1 J12. **In review**, with known leftovers (D15: five cards reuse the "Checkout v2" excerpt as their body; some block text clipped at the card edge). |
| Overwatch | Not built (P4.4). Nothing produces `overwatch` decisions, annotations, rule proposals or chat text. |

## 3. Scope

### 3.1 Design pass (M) — ux, then ui
SPEC §11 starts P4.3 with a design pass. Part B is the base; this pass turns it into what P4.3 builds (ux-brief §2–3): fix the D15 leftovers, draw the chat column **without the Overwatch** (decision D1), the collapse rules, threading, the filter, older days and search, the shell with the column built (1280 / 1512 / 1728, zoomed pane, dock), the notification-opened card, the keyboard map (page 05) with the Inbox map removed, and the copy that still says "Inbox". `00 Cover` In review → the user sets Ready for dev. Exports under `design/`. Front starts when ui hands off.

### 3.2 Chat log in the core — FR-1305 (M) — back, api-contract §1–3
- A store table of **chat entries**, one thread per **working day** (the local calendar day, as the digest's `today()`). Authors `core` | `overwatch` | `user`; P4.3 writes only `core` entries, the schema and commands take all three so P4.4 adds no migration.
- **Decision entries**: every decision the core produces (stored or synthesized) gets one entry when it is first listed open, with its `thread` (run, else agent). When it closes, the entry gets its **line** (verb, agent, what, by, final state, time; P4.2 notes §8.4) and a **card snapshot** (kind, gate, level, subject, title, summary, context, actions, final lifecycle) so it still expands after it leaves `now` and on older days. A synthesized id that comes back later in the day is a new entry.
- **Permissions answered by a store rule** (`decided_by = rule:<id>`), in the pane (`pane`) or by expiry (`expired`) are entries too: posted settled, as a line, never as an open card, never notified (J5 · 2, §9.6). A rule-answered permission that never was a decision is posted from the permission broker's audit write (FR-709).
- **Morning line**: on the first `now` computation of a working day (app start or local midnight while running), the core posts one `morning` entry: "Good morning. 1 run in progress, 1 decision waiting, week 38 % against 35 % planned." (parts at zero left out, P4.2 notes §8.6).
- Pushed in the `now` event (`chat`: new and changed entries) and in `now_snapshot` (today's entries). Commands `chat_day`, `chat_days`, `chat_search` for older days (api-contract §3). `orchd chat [--day D]` prints a day for debugging.
- Retention: none purged in P4.3 (entries are small); the weekly report does not read them.

### 3.3 The chat column and cards — FR-1304 (M) — front
- A fixed **chat column** on the right of the shell (not a pane type, not in the tiling tree): header (title, filter, older days / search), the day's thread, the footer (D1). Widths and the Inspector rule from P4.2 notes §4: 360 px (340 at 1280); **below 1728 the Inspector is an overlay over the chat's slot** (⌘I toggles; ⌘⇧I and ⌘L close it), docked 320 at ≥ 1728. SPEC §9.2 is updated to say so.
- **Thread**: today's entries in time order (P4.2 notes §8.3 "newest at the bottom"). An open card for a run that already has an earlier decision today shows the follow-up reference ("↳ after: spec approved 14:02") that jumps to it (exact form: ux, D4).
- **Card** = the Inbox detail laid in the column (P4.2 notes §8): kind label + age · title · subject line · context blocks (P4.1 `ContextBlocks`, options with every question) · actions (keyed and keyless, the Inbox's set) · lifecycle marker, and reply text area in place. Rendered from the live decision in `app.now.decisions` while it is listed; from the entry's snapshot after.
- **Lifecycle** as it moves (P4.2 notes §8.3): open, parked, sent (undo window, `u`), effect (P4.1 §4 text), failed (actions again + Dismiss), stale (muted, Dismiss), dismissed. A toast never claims an action before the core confirms it (P4.1 rule, kept). **Collapse**: a decision settled by the user collapses to its line 10 s after its effect (or at once on Dismiss); one settled by rule / pane / expiry / core is a line from the start; open, parked, failed and stale stay cards.
- **Collapsed line** (P4.2 notes §8.4): one line, `▸ <verb> <agent>: <what> · <by>   14:02`; expands (`⏎` or click) to the read-only card with its marker; the 5 s undo shows on the line while it runs. Undo of a rule's effect is P4.4 (FR-1310).
- **Outcome digest** (level 4): today's outcomes (P4.1 `OutcomeRow`) as one group at the end of the thread, "Today · 5 outcomes · Clear `c`", `o` opens; Clear keeps them in Done today (P4.2 §3.7).
- **Morning line**: first entry of the day, one line, no card.
- **Filter**: All / Blocking / Errors / Budget (the Inbox's filters) on cards; lines and the digest are not filtered; the badge never follows the filter.
- **Many open**: past 5 open cards, the older ones fold into headers ("3 more open · j k") that expand on focus (P4.2 notes §8.3).
- **Older days**: the header offers the previous days (`chat_days`) and a search (`chat_search`); a past day is read-only (cards as lines, expandable; no actions). Today stays one key away (exact controls: ux).
- **Keyboard** (§9.7 "From P4", FR-707): ⌘⇧I focuses the chat on the oldest open card (and closes the Inspector overlay); ⌘L focuses the chat's search until the Overwatch's input exists (D1); in the chat `j/k` move between open cards skipping collapsed lines, `a r ⏎ o x u e` act on the focused card as in the Inbox (batch `x` included), `c` clears the digest, `Esc` returns focus to the centre pane. Collapsed lines are reached with Tab or the mouse and expand with `⏎`.
- **Badge**: the title bar badge counts open level 1–3 decisions waiting on the user (`counts.decisions`, unchanged) and focuses the chat on click; its label is ux's (no longer "Inbox").

### 3.4 The Inbox pane is removed (S) — front
`InboxPane.svelte` is deleted; `lib/inbox.js` keeps what the cards reuse (renamed only if ux renames the concept in copy). Every entry point is rewired to the card: ⌘⇧I, title badge, palette ("Show decisions"; "Clear today's digest" kept), Mission control `d` and its Needs-you line, the waiting-on chip, the Decision Inspector ("Show card"), `app.inboxFocus` → `app.chatFocus {id}`. A saved layout's `inbox` pane is dropped at restore (its slot closes; Mission control stays home). Copy that says "Inbox" in other panes (settings, planner, board, report, MCP) follows ux's list. The core keeps the `inbox_*` command names (no API rename).

### 3.5 Notification clicks open the card (S) — back, then front
A level 1–2 notification carries the decision id (the batched one: the oldest of the batch). Clicking it brings the app forward and focuses that card in the chat (or, if it closed meanwhile, the next open card, with a toast saying what happened to it). Back first checks what the notification plugin reports on macOS (task `plan_first`): a real click callback if one is available; otherwise the fallback is that **the app being activated within 60 s of a notification** focuses its card. The core emits the Tauri event `focus_card {id}`; front handles it (api-contract §4).

## 4. Out of scope (P4.3)
- **The Overwatch** (P4.4): its session and status, the chat input and user messages, its text, annotations, "deciding" state, rule proposals, learned rules, undo of rule decisions (FR-1306–1310). The contract reserves their fields (api-contract §1); nothing renders them in P4.3 (D1).
- Intake (P4.5). Team-level actions from a card. Chat retention or export.
- Renaming the core's `inbox_*` commands or the `inbox_items` table.

## 5. Non-functional
- **One source of state** (§9.1 · 10): the column renders `app.now` (decisions, chat entries, outcomes, counts); it calls `chat_day` / `chat_search` only for past days, on demand.
- **Performance**: a day of 300 entries and 20 open cards renders in < 16 ms per `now` event (fixture test); past days and search results load in < 200 ms for 90 days of entries (core test).
- **Accessibility**: the thread is a `feed` of `article`s with labels; the focused card has the focus ring; lines say their state in text; new level 1–2 cards are announced through a throttled `aria-live` region; colour is never the only signal.
- **Tests**: `make ci` green. Core: an entry per decision (stored and synthesized; a returning synthesized id is a new entry), the line and snapshot written at close for each `decided_by`, rule / pane / expired permissions posted settled, the morning line once per day (start and midnight, zero parts left out), `chat_day` / `chat_days` / `chat_search`, the `now` push carries new and changed entries, the notification's decision id and `focus_card`. UI: the exit-criterion test (§1), collapse timing (10 s after effect, at once for rule / pane / expiry), parked / failed / stale stay cards, the snapshot renders after the decision leaves `now`, filter does not touch lines or the badge, many-open folding, `j/k` skip lines, ⌘⇧I on the oldest open card and closing the overlay, the Inspector overlay / docked rule at 1280 / 1512 / 1728, a saved `inbox` pane dropped at restore, `focus_card` on an open and on a closed decision, a past day read-only. Tests hold at any date, hour and time zone.

## 6. Acceptance (demo at the PR gate)
1. With the P4.1 fixture's kinds open (permission, question with two questions, spec gate, PR gate, startup, Governor ask, budget, error): each is a card in the chat with its blocks and the Inbox's actions; the title badge counts them.
2. ⌘⇧I focuses the oldest open card; `j` moves to the next; `a` on a permission: "Allowing once · Undo u", then "Allowed · back-1 resumed", and 10 s later the card is a line; `⏎` on the line expands it.
3. `x` on two permissions, `a`: both allowed; `u` undoes the last one within 5 s.
4. A permission answered by an "always allow" rule appears as a line only, with no notification and no badge change; one answered in the pane reads "· in the pane".
5. Quit with a decision closed 15 min ago: after restart its line is still in today's thread and expands from the snapshot.
6. Open the previous day from the header: its morning line and lines, read-only; search "serde" finds yesterday's line.
7. A level 1 decision raises a notification; clicking it (or activating the app within 60 s, per §3.5) focuses that card.
8. At 1512, ⌘I opens the Inspector over the chat; ⌘⇧I closes it and focuses the oldest card; at 1728 both are docked.
9. No `Inbox` pane anywhere (palette, ⌘⇧I, layouts); FigCraft pages 04.3, 03, 05 In review → Ready for dev, exports under `design/`.
