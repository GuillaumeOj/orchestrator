# P4.3 design notes: the chat column and its cards

UX pass for P4.3 (Chat & cards), 2026-11-02, by `ux` for `ui` (then `front`). Sources: [spec](../../.orchestrator/features/p-4-3-chat-and-cards/spec.md), [UX brief](../../.orchestrator/features/p-4-3-chat-and-cards/ux-brief.md), [decisions](../../.orchestrator/features/p-4-3-chat-and-cards/decisions.md), [API contract](../../.orchestrator/features/p-4-3-chat-and-cards/api-contract.md), and the Part B base: [P4.2 notes](p4-2-design-notes.md) §3, §4, §8 (called **P4.2 §n**) and [P4.1 notes](p4-design-notes.md) §3–5, §7 (**P4.1 §n**). Their copy rules and strings stand and are not repeated.

Copy rules as before: sentence case, no exclamation marks, name the agent and the project, local 24 h times, and **every number says what it is**. Words: a **card** is a decision laid out in the column; the badge, palette and notifications say **decisions**; a line is a settled card collapsed. "Inbox" disappears from the app (§11). The column's header title is **Decisions** until P4.4, which swaps it for the Overwatch (§1).

Point numbers below follow ux-brief §2 (1–13).

---

## 0. Findings for lead

| # | Finding | Proposal |
|---|---|---|
| F1 | **A decision still open at midnight has no home.** `ChatEntry.day` is the day the decision was first listed; the first entry of a new day starts a new thread (contract §2.1). A card opened Sunday 17:40 and still open Monday is rendered from `now.decisions`, but its entry belongs to Sunday: Monday's thread would not show it, and its `line` (added when it closes, Monday) would land in Sunday's thread. | **UI rule, no contract change**: today's thread shows every open decision of `now.decisions` as a card, whatever its entry's `day`, under a separator "Still open from Sun 1 Nov" right after today's morning line (the morning line stays first). When it closes, its line is shown in today's thread (at `line.at_ms`) and Sunday's thread shows that entry as "left open" (§6). **If back prefers a data fix instead**: post a new entry for today (new id, same `decision`) at rollover for each still-open decision and leave the old entry without a line. Either way `chat_day` / `chat_search` must return entries with no `line` (open at the end of their day); say which in the contract. |
| F2 | **Many open: fold the newer, not the older (revises P4.2 §8.3).** P4.2 folds the older ones, but ⌘⇧I focuses the *oldest* open card, `j` walks oldest to newest, and the oldest waited longest. Folding the older would hide the first thing every key reaches. | Show the **5 oldest** open cards; fold the rest into one header "N more open" at their place in the thread (§4). Resolving a card pulls the next one in. No contract impact; P4.2 §8.3 sentence is superseded. |
| F3 | **"Inbox" is also in SPEC text and in agent-facing strings.** SPEC §9.2 title bar (`[Inbox 3]`, line 548), FR-210 ("The core watches the Inbox"), FR-705 and the v0.7/v0.8/v0.11 notes say Inbox; Rust strings sent to agents say "the Inbox" (list in §11.3). | Lead updates SPEC §9.2 and FR-210 ("Decisions 3", "watches the open decisions"); history notes stay. Back replaces the agent-facing strings in §11.3 (no API rename; `inbox_*` commands stay). |
| F4 | **"All caught up" is false when cards are open** (P4.1 §5 digest copy: "All caught up · 5 done today"). | The cleared digest says "5 done today, all seen" (§7). "Nothing needs you." is a separate line, shown only when no card is open. Supersedes P4.1 §5's cleared copy. |
| F5 | **The morning line has no all-zero form.** Parts at zero are left out (P4.2 §8.6); with nothing running, nothing waiting and no plan the sentence would be "Good morning." alone. | Back writes: "Good morning. Nothing is running and nothing is waiting for you." when all parts are zero; the week part is left out when there is no plan. Other forms as in P4.2 §8.6, with counts that say what they are: "1 run in progress", "3 decisions waiting", "week 38 % against 35 % planned". |
| F6 | **No way to hide the chat column in P4.3.** It is a fixed column (D5), also over a zoomed pane (§9). At 1280 a terminal gets 884 px. | Keep fixed for P4.3 (oversight must not be one key from hidden). If the first week shows a need, a "Hide decisions" command is a P4.4 candidate. No action now. |
| F8 | **Auto-cleared synthesized decisions never close** (`error-`, `wait-`, `trust-`, `ask-` vanish from `now` when their condition clears). Their entry gets no `line`. | Back closes the entry when a synthesized id leaves `now` with `line.verb: "dismissed"`, `by: "core"`, `state: "stale"`; the UI then shows `▸ dismissed back-1: … · by the app`. |
| F9 | **Search misses what the line shows** (verb, rule label) and live open decisions. | Back also matches `line.verb`, `line.by_detail` and open decisions' title/summary (contract §3). Until then ui must not promise it. |
| F10 | **Expiry and parked.** A permission hold that expires *parks* (stays a card, FR-705). | `expired` lines are for decisions with a default (ask, budget: `kept queued`/`applied` · expired) and for a parked permission dismissed by the core; a hold expiring itself is the parked card. Spec §3.2 "expiry" read this way. |
| F7 | **Notification copy changes in Rust** (`notify.rs`: "N items need you", title = the decision's title). | New titles and bodies in §8.3, for back. |

---

## 1. The column without the Overwatch (point 1, D1)

```
┌ Decisions                          Today ▾  🔍 ┐  header 40 px
│ All 5 · Blocking 2 · Errors 2 · Budget 1       │  filter row 32 px (§5)
├────────────────────────────────────────────────┤
│  thread (feed): morning line, lines, cards,    │
│  fold header, digest                            │
├────────────────────────────────────────────────┤
│ Cards work here. The Overwatch joins in a      │  footer 36 px, muted
│ later release.                       ⌘L search │
└────────────────────────────────────────────────┘
```

**Header** (one row, 40 px): title **Decisions**; right side, in order: the **day button** `Today ▾` (opens older days, §6) and the **search button** 🔍 (⌘L, `/`). The slot between the title and the day button is empty in P4.3: **P4.4 puts the Overwatch status there** (P4.2 §8.6: asleep / waking / working / down) and renames the title to "Overwatch". Nothing is drawn in the slot now (no placeholder, no disabled control). Keep the title in a variable so P4.4 changes one string.

**Footer** (no input field, no disabled field): a muted two-line note, 36 px:
- Text: **"Cards work here. The Overwatch joins in a later release."**
- Right, aligned to the first line: the key hint **⌘L search**.
- The footer is not focusable. It is the only text that names the Overwatch in P4.3.
- On a past day (§6) the same footer shows "Read-only · Back to today t" instead, and on a search the footer is unchanged.
- ⌘L focuses the search field in the header (it opens it if closed). The field's placeholder says what it searches: "Search decisions and lines".

**Header badge on the title bar** counts open level 1–3 decisions waiting on the user (unchanged), label "Decisions" (§11).

---

## 2. Collapse rules and line copy (point 2)

### 2.1 When a card becomes a line

| Case | Becomes a line | Notes |
|---|---|---|
| Settled by the user (`by = you`), effect reached | **10 s after the effect** | Timer starts at `effect`, not at the click. Paused while the pointer is over the card or focus is inside it with text typed in the reply area (a reply being written never collapses under the cursor). Collapse never moves focus off the thread: if the card had focus, focus goes to the next open card, else to the line itself. |
| Dismiss (any card) | **at once** | The "Dismissed by you · 14:05" marker is not shown first; the line says it. |
| Settled by a rule, in the pane, by expiry (F10) or by the core | **at once, and never as a card** | Posted as a line from the start (D3). It does not notify, does not change the badge, and does not take focus. |
| open · parked · failed · stale | **stays a card** | Failed has its actions again plus Dismiss; stale has Dismiss. |
| sent (5 s undo window) | stays a card | The card shows "Allowing once · Undo u"; undo returns it to open. |
| PR gate in effect | collapses after 10 s like the rest | The expanded line keeps **Open PR o**. |
| Reduced motion | no height animation | The line replaces the card in one frame; otherwise a 120 ms fade of the card's height. |

A line posted already settled (rule, pane, expiry) is inserted at its `at_ms` in the thread, quietly: no scroll, no highlight; if the user is at the bottom, the thread stays at the bottom.

### 2.2 The line

```
▸ allowed back-1: cargo add serde_json · rule: installs                      14:02
```
28 px, no card chrome, left to right: `▸` (`▾` when expanded) · verb · agent `:` · what (truncates with …, the full text is the tooltip) · ` · ` tail · time (right, local 24 h from `line.at_ms`). With no agent (budget, battery, MCP): `▸ applied: Reserve for your own work used up` (the verb, a colon, the what). The verb and agent are secondary text, the what is primary. `denied`, `rejected`, `failed`, `interrupted`, `expired` use the warning text colour on the verb; the word is always there, colour is never the only signal.

**Verb text** is `line.verb`, except `run now` is shown as **started** (`▸ started back-1: Run back-1 now? ≈ +2.1 pp of the week`). Others as written.

**Tail** from `line.by`:

| `by` | Tail | Example |
|---|---|---|
| you | none (you know) | `▸ approved lead-1: Spec gate · checkout` |
| rule | `· rule: <by_detail>` | `▸ allowed back-1: cargo add serde_json · rule: installs` |
| pane | `· in the pane` | `▸ allowed back-1: Bash · pnpm add zod · in the pane` |
| expired | `· expired` | `▸ kept queued back-1: Run back-1 now? · expired` |
| core | `· by the app` | `▸ dismissed front-1: front-1 did not start · by the app` |
| overwatch (P4.4) | `· Overwatch: <by_detail>` | `▸ denied back-1: rm -rf build · Overwatch: not in the allow list` |

When `by_detail` is missing for a rule, the tail is `· rule`. The verb `expired` has no second "expired" tail: `▸ expired back-1: Bash · pnpm add zod`.

### 2.3 Every verb × who (final copy; `what` is the core's cut text)

| `verb` | by you | by rule | in the pane | expired | by the app |
|---|---|---|---|---|---|
| allowed | `▸ allowed back-1: cargo add serde_json` | `… · rule: installs` | `… · in the pane` | (parks, no line) | — |
| denied | `▸ denied back-1: rm -rf build` | `… · rule: no-rm` | `… · in the pane` | `… · expired` | — |
| answered | `▸ answered front-1: Which auth provider?` | — | `… · in the pane` | — | — |
| approved | `▸ approved lead-1: Spec gate · checkout` | — | — | — | — |
| sent back | `▸ sent back lead-1: Spec gate · checkout` | — | — | — | — |
| rejected | `▸ rejected lead-1: PR gate · checkout` | — | — | — | — |
| restarted | `▸ restarted front-1: front-1 did not start` | — | — | — | `… · by the app` |
| applied | `▸ applied: Reserve for your own work used up` | — | — | `… · expired` | — |
| skipped | `▸ skipped: Reserve for your own work used up` | — | — | — | — |
| kept queued | `▸ kept queued back-1: Run back-1 now? ≈ +2.1 pp` | — | — | `… · expired` | — |
| started (run now) | `▸ started back-1: Run back-1 now? ≈ +2.1 pp` | — | — | `… · expired` | — |
| dismissed | `▸ dismissed front-1: front-1 did not start` | — | — | — | `… · by the app` |
| expired | — | — | — | `▸ expired back-1: Bash · pnpm add zod` | — |
| interrupted | — | — | `▸ interrupted back-1: Bash · pnpm add zod · in the pane` | — | `… · by the app` |
| failed | `▸ failed lead-1: PR gate · checkout` | — | — | — | `… · by the app` |

(— = the combination does not occur; if it does, render the generic pattern.) A `failed` line appears only for a final failure shown on a past day or after a restart; while it is live a failed decision stays a card.

### 2.4 Line states
- **Undo window** (5 s, user's own decision, before it reaches the effect): the card is still a card (§2.1). A line never carries Undo in P4.3: undo of a rule's effect is P4.4 (FR-1310).
- **Undone**: the card returns to open; no line.
- **Order**: entries sit at their posting time (`at`); a card that collapses keeps its place and shows `line.at_ms` on the right. "↳ after" picks the earlier decision by `line.at_ms`.
- **Dismiss has no undo** (it collapses at once); `u` applies only to a decision in its 5 s window, where the card is still shown. This supersedes spec §3.3's "undo shows on the line" for P4.3.
- **Expanded** (`⏎`, click, `▾`): the read-only card from the snapshot (or the live decision), with its marker ("Allowed by rule installs · 14:02", "Allowed in the pane · 14:02", P4.1 §3.4 `<by>` words), no action buttons except the ones that stay meaningful: **Open PR o** (PR gate), **Open pane o** where the pane still exists, **Open in Notion ↗**. `⏎` or click again collapses. At most one line is expanded per click; expanding does not move other items.
- **Accessibility**: `article`, `aria-label` = "Allowed back-1: cargo add serde_json, by rule installs, 14:02". `aria-expanded` goes on the line's toggle `button` inside the article (the `article` role does not support it).

---

## 3. Threading (point 3, D4: confirmed, one refinement)

**Confirmed: chronological.** Reasons: a thread that reorders under the eyes breaks `j/k` and the "newest at the bottom" scan; indenting under the earlier answer costs 16 px of a 340 px column and makes two kinds of card; the reference gives the same information without moving anything.

**Reference line**, first row inside an open, parked, failed or stale card, above the kind label, 20 px, muted:
```
↳ after: approved Spec gate · checkout · 14:02
```
- Pattern: `↳ after: <verb> <what> · <time>` of the **most recent earlier decision of the same thread** (`thread`: the run, else the agent) **that closed today**. One line, truncates with …
- It is a link: click or `⏎` (it is in the Tab order, not in `j/k`) scrolls to that entry, expands it, and gives it focus with the highlight of §8.1. To return, `j` / `k` continue from there; the card you came from is the next open card.
- If that decision closed on an earlier day, the reference is plain text with the day: `↳ after: approved Spec gate · checkout · Sun 1 Nov 17:40`, no link.
- Not shown on lines, not shown when the thread has no earlier decision, never nested more than one level (only the previous decision is named).
- `aria-label`: "Follows: approved Spec gate · checkout, 14:02. Jump to it".

---

## 4. Many open cards (point 4)

- **Up to 5 open cards** show as cards. With 6 or more, the **5 oldest** show and the others fold (F2).
- The fold is **one header row at the place of the 6th card in the thread** (lines between folded cards that closed meanwhile stay where their time puts them):
```
▸ 3 more open · 1 blocking · j                                              
```
  - Pattern: `<n> more open` then ` · <m> blocking` when m ≥ 1 (level 1), then the key. Expanded: `▾ Fold 3 cards · ⏎`.
  - `⏎` or click shows all folded cards. They stay shown until open cards drop to 5 or fewer, or `⏎` on `▾ Fold 3 cards`. `j` from the 5th card lands on the header (it is a stop); `j` again from an expanded header goes to the 6th card.
- When a card is resolved and the count drops, the next folded card slides in at the bottom of the 5 and keeps the same 5-card ceiling.
- **A level 1 card that arrives folded**: the header goes to the warning style and reads `▸ 4 more open · 1 blocking · j`; the new-card announcement (aria-live, throttled) still fires. The badge and notification are unaffected by the fold.
- Folding applies after the filter (§5): "N more open" counts what the filter shows.
- ⌘⇧I always focuses the oldest open card (never folded).

---

## 5. Header filter (point 5, D6)

- Row under the title, 32 px: chips **All n · Blocking n · Errors n · Budget n**, counts are **open cards** (levels 1, 2, 3; same tests as `FILTERS` in `lib/inbox.js`). Zero-count chips other than All are hidden. The whole row is hidden when fewer than 2 cards are open and the filter is All (an active filter keeps it visible) and on a past day.
- Applies to **cards only**: open, parked, failed, stale cards. Lines, the morning line, the fold header's count (after filtering), the digest, separators are never filtered. The title bar badge ignores the filter.
- **A filter that hides open cards says so**, in a strip under the chips (replaces the filter row's bottom edge, 28 px, warning-neutral):
  `Showing Blocking · 2 of 5 open cards. 3 hidden · Show all`
  Pattern: `Showing <filter> · <shown> of <open> open cards. <hidden> hidden · Show all`. "Show all" and `f` cycling back to All clear it. The strip is absent when nothing is hidden (filter All, or the filter matches every open card).
- Keys: `f` cycles All → Blocking → Errors → Budget → All (skipping hidden chips). Palette: "Filter decisions: All / Blocking / Errors / Budget".
- A filter never survives ⌘⇧I, `focus_card` or a jump to a card it hides: the target forces the filter back to All. The filter resets to All at a new day and when the app restarts; it is not saved with the layout.
- Empty after filtering (filter set, no card matches): "No Budget cards open. 5 open in other filters · Show all".

---

## 6. Older days and search (point 6)

### 6.1 Reaching a day
- The header's **day button** shows `Today ▾`. It opens a popover (up to 30 days from `chat_days`, newest first, today excluded), 320 px wide:
```
Yesterday · Sun 1 Nov · 12 decisions
Fri 30 Oct · 9 decisions
Thu 29 Oct · 4 decisions
Show earlier days
```
  - Row = `[Yesterday · ]<weekday day month> · <n> decisions` (`decisions` of `chat_days`; "1 decision"; a day with only a morning line: "no decisions"). The "Yesterday · " prefix only for the previous calendar day. "Show earlier days" loads the next 30 (`before`). No days: "No earlier days yet." Loading: "Loading days…". Error: "Could not load days: <why> · Retry".
- Keys: `[` previous day, `]` next day (towards today), `t` back to today, all while the chat has focus. In the popover: `j/k` or arrows, `⏎` opens, `Esc` closes.
- Palette: "Older days…", "Back to today" (only on a past day).

### 6.2 A past day (read-only)
- Header: title unchanged, the day button reads `Sun 1 Nov ▾`; a strip under it (28 px): **"Sunday 1 Nov · read-only · Back to today t"**. The filter row is hidden. Footer: "Read-only · Back to today t" (as §1).
- Content: that day's morning line, then every entry **as a line** (cards that were still open at the end of the day: `▸ left open back-1: Bash · pnpm add zod`, muted, no tail; their expanded card shows the snapshot or the live decision's last state, with the marker "Open until the day ended" — see F1). Lines expand (`⏎`) to the read-only card with its marker; **no actions at all, no Undo, no reply field**, `o` still opens a PR, a report or a task when the subject exists.
- At the end: the day's outcomes (`chat_day.outcomes`) as the digest group, **"Sun 1 Nov · 4 outcomes"** with no Clear; rows keep `o`. No outcomes: "No outcomes that day."
- Loading: skeleton of three lines and "Loading Sun 1 Nov…". Error: "Could not load Sun 1 Nov: <why> · Retry". An empty day cannot be listed (days with at least one entry); a day from a stale list returning nothing: "Nothing was recorded on Sun 1 Nov."
- ⌘⇧I from a past day returns to today and focuses the oldest open card; `focus_card` does the same.

### 6.3 Day separator
A centred small-caps label with a hairline on each side, 24 px, muted: `SUN 1 NOV`. Used in **search results** (between days) and for the **"Still open from Sun 1 Nov"** group at the top of today's thread (F1; left-aligned, no hairline on the left). Today's own thread has no separator (the header says Today).

### 6.4 Search
- ⌘L, `/` or the 🔍 button turns the header's title row into a field (full width, 40 px): placeholder **"Search decisions and lines"**, a clear ✕ when typed, `Esc` to close. Matches what `chat_search` matches (line text, agent, card title and summary, morning text); case-insensitive; today included.
- Under 2 characters nothing is searched; below the field: "Type 2 or more characters." Debounce 200 ms. Loading: "Searching…". 
- **Results** replace the thread (the filter row is hidden), newest first, grouped by day separators. Each result is two rows: row 1 the line as in §2.2 (open cards: kind label + title with a "waiting" mark: `● Permission · Bash · pnpm add zod`), row 2 muted: `Sun 1 Nov · 14:02`. The match is not highlighted (substring on fields not all shown). Header of the list: **"4 results for “serde”"** ("1 result for…"; at the limit: "First 50 results for “serde”. Narrow the search.").
- `↓` from the field enters the results; `j/k` move once inside; `⏎` on a result opens it: today's entry → close the search, scroll the thread to it and expand it with the §8.1 highlight; an earlier day → open that day (§6.2) with the entry scrolled to and expanded. Search is kept when going back (`Esc` from a past day opened via search returns to the results; second `Esc` closes the search).
- **No results**: "Nothing found for “serde” in decisions and lines." Error: "Could not search: <why> · Retry".
- `Esc` order: clear the text (first press, if any) → close the search and return to the thread → (next press) focus returns to the centre.

---

## 7. Digest (point 7)

At the end of the thread, above the footer, after all cards, lines and the fold header. Rows are P4.1 `OutcomeRow` unchanged.

| State | Group header | Body |
|---|---|---|
| Outcomes today | **"Today · 5 outcomes · Clear c"** | up to 5 rows, then "Show 3 more outcomes"; `o` on a focused row opens its subject |
| One outcome | "Today · 1 outcome · Clear c" | |
| None yet | "Today · no outcomes yet" (no Clear) | "Nothing done yet today." |
| Cleared (all seen) | "Today · 5 done" (no Clear) | "5 done today, all seen." (F4; the P4.1 "All caught up" is dropped) |
| Cleared, new ones arrive | "Today · 2 outcomes · Clear c" | only the new rows; the header says "· 5 done today" at the right in muted when the count differs |

- **Thread-wide empty line** (F4), shown once **above the digest** when no card is open: **"Nothing needs you."** (muted, with the time of the last settled card when there is one: "Nothing needs you · last settled 14:02").
- `c` clears from anywhere in the chat (when outcomes are listed); the palette entry "Clear today's digest" stays. Clear keeps them in Done today (P4.2 §3.7). Clearing never touches cards or lines.
- Digest is part of today only; a past day shows its own outcomes (§6.2).

---

## 8. Notification-opened card (point 8)

### 8.1 The highlight
When `focus_card {id}` arrives (or a notification click, or the app activated within 60 s of one, D7):
1. The window comes forward; the Inspector overlay closes (it never auto-opens, P4.2 §4 rule 1).
2. The column leaves a past day or a search (back to today), the filter resets to All, a fold containing the card expands.
3. The card scrolls into view (centred when it does not fit at its current place), takes keyboard focus and shows the **standard focus ring** plus a **tint pulse** of the card background (accent at 12 %), 1.5 s, fading out. Reduced motion: the tint stays 1.5 s without fading.
4. Screen readers get the throttled live message "back-1 asks: Bash · pnpm add zod", the card is focused.
Nothing is decided by arriving; no action fires.

### 8.2 The decision already closed
`focus_card` for a decision that is no longer open focuses **the next open card** (the oldest) and shows a toast (6 s, info) that says what happened to it from its `line`:

`Already <verb> <by> · <time>` — verbs from §2.3 in the past form they already have; by: you → "by you", rule → "by rule installs", pane → "in the pane", expired → "(expired)", core → "by the app".

| Closed how | Toast |
|---|---|
| allowed in the pane | **"Already allowed in the pane · 14:02"** |
| allowed by rule | "Already allowed by rule installs · 14:02" |
| denied by you | "Already denied by you · 14:02" |
| approved by you | "Already approved by you · 14:02" |
| expired | "Already expired · 14:02" |
| dismissed by the app | "Already dismissed by the app · 14:02" |
| failed | "Already failed · 14:02" |

When another card is open the toast continues "Showing the next card." (one toast: "Already allowed in the pane · 14:02. Showing the next card."). When none is open, the column scrolls to that line and expands it, and the toast has no second sentence. A decision the column does not know at all: "That decision is no longer listed." and the oldest open card is focused.

### 8.3 The notification itself (for back: `notify.rs`)
Levels 1–2 only, as today; rules and panes never notify (D3).

| Case | Title | Body |
|---|---|---|
| One decision, agent known | **"back-1 needs you"** | the card title, then the summary's first line, ≤ 120 chars: `Bash · pnpm add zod` ⏎ `Add zod` |
| One decision, no agent (budget, battery, MCP) | **"Orchestrator needs you"** | title, then summary's first line |
| Two or more, one agent | **"5 decisions need you · back-1"** | up to 3 card titles one per line, then "+2 more" (no "+n more" line when 3 or fewer) |
| Two or more, several agents | **"5 decisions need you · 2 agents"** | up to 3 lines `back-1: <title>`, then "+2 more" (none when 3 or fewer) |
| Click | brings the app forward and focuses the **oldest** decision of the notification (§8.1) | |

The old "N items need you" is dropped (principle 11: items → decisions).

---

## 9. Shell with the column built (point 9)

Widths (P4.2 §4 stand; sidebar 240 or the 56 px rail below 1512; centre min 640 where possible).

| Viewport | Sidebar | Centre | Chat | Inspector |
|---|---|---|---|---|
| 1280 | rail 56 | 884 | **340** | overlay in the chat's slot (340) |
| 1512 | 240 | 912 | 360 | overlay in the chat's slot (360) |
| 1728 | 240 | 808 | 360 | **docked 320** (240+808+360+320 = 1728: the Inspector is added to the centre's 808, it does not take from it) |

- The chat spans from under the title bar to above the status bar, full height. The **shell dock** (⌘J) sits under the **centre only**; the chat column is never covered by it.
- **Inspector overlay** (below 1728), ⌘I toggles, `Esc` or ⌘I closes (P4.2 §4 rules 1–5):
  - It covers the chat visually, same width, same top and bottom.
  - **Banner** at the top of the overlay when cards are open, 32 px: **"1 card needs you · ⌘⇧I"** ("3 cards need you · ⌘⇧I"; "3 cards need you, 1 blocking · ⌘⇧I" when at least one is level 1). It is a button (click = ⌘⇧I). A level 1–2 card posted while the overlay is open shows the banner (if hidden) and does not close the overlay. No banner when nothing is open.
  - ⌘⇧I and ⌘L close the overlay and focus the chat. `focus_card` too.
  - The title bar badge stays visible and counts.
  - Crossing 1728 keeps the user's open/closed state.
- **Docked at 1728**: the chat and the Inspector sit side by side; no banner (the chat is visible).
- **Zoomed pane** (⌘; z): zoom enlarges a pane within the **centre**; **the chat stays** (it is not in the tiling tree). The Inspector's overlay still works over it. Reason: oversight must not be one pane toggle away from hidden (F6). The 1280 zoomed terminal is therefore 884 × content height.
- Minimum widths: card content 316 px (340 column − 2×12 padding); blocks wrap, long tokens break (`overflow-wrap: anywhere`), mono blocks scroll horizontally inside the block, buttons wrap to rows with each label + key together (no clipped button).
- Page 03 frames: P4.2 row at 1280 (rail + centre + 340 chat), 1512 (overlay closed, overlay open with banner), 1728 (docked), zoomed pane at 1280, dock open at 1512. Draw the chat with the P4.3 states of §12 (a card open, a line, the fold off).

---

## 10. Chat key map and conflicts (point 10, page 05)

**Remove** the Inbox map from page 05 and the "Inbox" column of the conflict table. **Add** this map. Keys fire only with the chat focused (not in a text field, no ⌘/⌥/⌃) except the global chords.

### 10.1 Global (chords)
| Key | Action |
|---|---|
| ⌘⇧I | Focus the chat on the **oldest open card** (today, filter All, overlay closed, fold expanded when needed). No open card: focus the digest header. Replaces "open Inbox". |
| ⌘L | Focus the chat's search (opens it; closes the overlay). Until the Overwatch input exists (D1); P4.4 gives ⌘L to the input and moves search to `/` only. |

### 10.2 In the chat
| Key | Action | On |
|---|---|---|
| `j` `k` / ↓ ↑ | Next / previous **open card**; skips lines; the fold header and the digest header are stops | any |
| `Tab` / `⇧Tab` | Move through every focusable thing: header controls, reference links, lines, fold header, digest rows | any |
| `⏎` | The card's default action (Answer · Reply · Send back with notes — as in the Inbox); expands/collapses a line; expands a fold; follows a "↳ after" reference; opens a search result | card · line · header |
| `a` | Primary action (Allow once · Approve · Apply · Run now · Restart fresh) | open/parked card |
| `r` | Secondary (Deny · Send back · Reject · Skip · Keep queued · Dismiss for a question) | open card |
| `o` | Open pane / PR / outcome / Notion page | card · outcome row |
| `x` | Mark the card for the batch (Inbox batch behaviour), then `a` / `r` apply to all marked | open card |
| `u` | Undo the last decision within 5 s | any |
| `e` | Open the card's file in nvim (spec gate, docs) | card with a file |
| `c` | Clear the digest | any, when outcomes are listed |
| `f` | Cycle the level filter (§5) | any |
| `[` `]` | Previous / next day (§6) | any |
| `t` | Back to today | past day |
| `/` | Focus search (same as ⌘L) | any |
| `Esc` | Closes a popover or the search (order in §6.4); in a reply field leaves it and keeps card focus; otherwise **returns focus to the centre pane that had it** (Mission control if none) | — |

Other keys do nothing (no beep). In the reply area and the search field every key types; `Esc` leaves.

### 10.3 Conflict check
| Against | Result |
|---|---|
| **Global chords** | ⌘⇧I and ⌘L are unclaimed elsewhere (`keyboard.js` CHORDS has no ⌘L). Both work from a terminal pane (⌘ chords never reach the PTY). The command id `inbox` becomes `decisions`. |
| **Leader keys** (⌘; then s v h j k l z t ] [ e w p o g) | None: the leader consumes the next key. `[`/`]` and `t` here are bare keys with the chat focused; ⌘; ] / [ are the dock. `⌘; w` (next waiting agent) is unrelated to `j/k`. |
| **Mission control map** (j k h l ⏎ space i p s r ⇧K d n f o ⌫) | **Shared letters, same screen now**: both panes are visible together, so the focus ring decides. `j/k` same meaning in both. **`r` = Resume in Mission control, Reject/Deny/Skip in the chat** (F2 of P4.2, now on one screen): mitigations — the focused region's header takes an accent underline, the card carries its own `r` label on the button, and Mission control's hint row shows `r resume`. `⏎` replies in the chat, opens a session in Mission control. `o` opens "the thing" in both. `f` filters in both (levels there, projects here). `d` is the bridge: Mission control `d` focuses that decision's card in the chat. `i p s h l n space ⇧K ⌫` are unbound in the chat and do nothing. `x u e a c t [ ] /` are unbound in Mission control. |
| **Text fields** (reply, search) | They swallow single keys; `Esc` leaves. |
| **Terminal panes** | Chat keys never fire in a terminal; the chat is a separate focus target. After `Esc` focus returns to the pane (terminal) that had it. |
| **Palette / dialogs** | A dialog takes ⌘⏎ / ⏎ / Esc first. |
| **Tab order** | sidebar → centre panes → chat (header → thread → digest). |

Result: **no binding collision, one semantic collision (`r`) now visible on one screen**, handled by the focus ring and the hint rows. Palette entries: "Show decisions", "Search decisions", "Older days…", "Back to today", "Filter decisions: …", "Clear today's digest".

---

## 11. Copy that says "Inbox" (point 11)

### 11.1 `ui/src` (each file:line, new copy)

| File:line | Today | New |
|---|---|---|
| `components/TitleBar.svelte:22` | `Inbox <count>` + `⌘⇧I` | **`Decisions <count>`** + `⌘⇧I`. Tooltip/aria-label: "3 decisions need you · ⌘⇧I"; zero: "No decisions waiting · ⌘⇧I". Click = ⌘⇧I (focus the chat on the oldest open card; at 0 focus the column). Class `.inbox` → `.decisions` (code). |
| `components/TitleBar.svelte:20` | `openSingleton('inbox')` | focus the chat (code) |
| `components/PaneFrame.svelte:10,26,46` | imports `InboxPane`, kind map `inbox: 'inbox'`, `spec.type === 'inbox'` branch | remove all three with the pane (code) |
| `lib/app.svelte.js:350` | `openSingleton('inbox')` (code) | focus the chat (`chatFocus`) |
| `lib/app.svelte.js:181` | pane title `inbox: 'Inbox'` | remove the entry (no `inbox` pane; a saved one is dropped at restore) |
| `lib/app.svelte.js:1045` | palette "Open Inbox" | **"Show decisions"** (id `decisions`, hint ⌘⇧I). New: "Search decisions" (⌘L), "Older days…", "Back to today", "Filter decisions: All / Blocking / Errors / Budget" |
| `lib/app.svelte.js:1047` | "Clear today's digest" | unchanged |
| `lib/app.svelte.js:1395` | failed-toast action `label: 'Open'` with `openInbox` | **"Show card"** (→ `chatFocus`) |
| `lib/keyboard.js:15` | `'cmd+shift+i': 'inbox'` | `'decisions'`; add `'cmd+l': 'decisions.search'` |
| `lib/mission.js:516` | hint `['⏎', 'open in Inbox']` | **`['⏎', 'show card']`** |
| `panes/MissionPane.svelte:276` | link "Open in Inbox" + `d` | **"Show card"** + `d` |
| `panes/MissionPane.svelte:180,208` | `openInbox(...)` | `focusCard(...)` (code; `d` hint in the row help: "d show card") |
| `components/WaitingChip.svelte:24` | click → `openInbox(ref)`; the chip's tooltip is the wait line | tooltip: `<wait line> · click to show its card`; link `aria-label`: "Show the card for <reason>". Behaviour: `focusCard(ref)` |
| `components/inspector/DecisionInspector.svelte:24` | "Open in Inbox `d`" | **"Show card `d`"** |
| `components/inspector/DecisionInspector.svelte:26` | "Decide in the Inbox (the card after P4.3): it owns the undo window." | **"Decide on its card: it owns the undo window."** |
| `components/inspector/RunInspector.svelte:33` | `' · Open in Inbox d'` | **`' · Show card d'`** |
| `panes/AgentPane.svelte:146` | "… · also in the Inbox" | **"… · also on its card · ⌘⇧I"** |
| `panes/AgentPane.svelte:160` | "answer it in the Inbox" | **"answer it on its card (⌘⇧I)"** (in the pane: unchanged "answer it in the pane below: the app never does") |
| `panes/ProjectSettingsPane.svelte:261` | "…you get an Inbox note." | **"…you get a note in today's digest."** (it is an outcome, P4.1 F4) |
| `panes/InboxPane.svelte:219,256,257` (deleted) | aria "Inbox"; "Inbox clear."; "Agents that need you show up here first; the digest below keeps what got done today." | feed `aria-label` "Decisions, today"; "Nothing needs you."; "Cards appear here when an agent needs you; the digest at the end keeps what got done today." |
| `panes/InboxPane.svelte:267` (deleted) | "All caught up · 5 done today" / "Nothing done yet today." | "5 done today, all seen." / "Nothing done yet today." (F4) |
| Comments only (reword with the code): `McpSettings.svelte:66`, `PermissionCall.svelte:4`, `AgentPane.svelte:6,36,40`, `ReportPane.svelte:83`, `BoardPane.svelte:43`, `lib/now.svelte.js:192`, `lib/status.js:31`, `lib/app.svelte.js:38,340,347,355,397,1302`, `lib/inbox.js:1` | "Inbox …" | "decision cards" / "the chat column" |

### 11.2 Rust UI strings (front/back)
| File:line | Today | New |
|---|---|---|
| `crates/orch-app/src/menu.rs:34` and `:107` | menu item "Inbox" (⌘⇧I) and the accelerator map `("CmdOrCtrl+Shift+I", "inbox")` | **"Show Decisions"** (id `decisions` in both places, matching `keyboard.js`) |
| `crates/orch-app/src/notify.rs:55-56` | "N items need you" / "N agents need you" | §8.3 |

### 11.3 Agent-facing strings (back; no behaviour change; the agent may repeat them to the user)
| File:line | Today | New |
|---|---|---|
| `crates/agent-runtime/src/inbox.rs:500` | "ORCH: the user denied this in the Inbox." | "ORCH: the user denied this." |
| `crates/orch-app/src/core.rs:818` | "…the user answered from the Inbox while a prompt was open in your pane: …" | "…the user answered in the app while a prompt was open in your pane: …" |
| `crates/agent-runtime/src/questions.rs:117` | "ORCH: forwarded to the user through the Inbox; …" | "ORCH: forwarded to the user in the app; …" |
| `crates/agent-runtime/src/team.rs:1694` | "The spec gate is in the user's Inbox. End your turn now; …" | "The spec gate is waiting for the user. End your turn now; …" |
| `crates/agent-runtime/src/inbox.rs:1031`, `pr.rs:513` | "Requested: Inbox item {id} asks the user to …" | "Requested: decision {id} asks the user to …" |
| `crates/agent-runtime/src/reconcile.rs:299,301,367,371,375` | "…parked in the Inbox: …", "back in the Inbox as a restart gate", "Inbox item {item} …" | "…parked as a decision: …", "back as a restart gate", "Decision {item} …" |
| `crates/governor/src/governor.rs:1428` | "…Inbox item for the user" | "…decision for the user" |
| `crates/orch-mcp/src/lib.rs:121` | "…fyi (shown in the Inbox, no notification)." | "…fyi (shown in the decisions column, no notification)." |
| `inbox.rs:723,729,774,976` (errors), `orch-app/src/core.rs:1123` | "no Inbox item {id}" / "Inbox item {id} is already …" / "Inbox item {id} is no longer failed or stale" | "no decision {id}" / "Decision {id} is already …" / "Decision {id} is no longer failed or stale" |

SPEC text (§9.2 line 548, FR-210, FR-705 and the notes) is lead's (F3). Command and table names stay `inbox_*`.

---

## 12. States matrix: the column (point 13) and what ui draws

Every row is a frame on page 04.3 (or a labelled variant). "Footer" is always §1.

| # | Case | Header / filter | Thread | Digest |
|---|---|---|---|---|
| 1 | **Morning, no decisions today** | Decisions · Today ▾ · 🔍; no filter row | the morning line, then **"Nothing needs you."** and, below, "Cards appear here when an agent needs you." | "Today · no outcomes yet" · "Nothing done yet today." |
| 2 | **First start, nothing at all** (no projects, no runs) | as 1 | "Good morning. Nothing is running and nothing is waiting for you." then "Nothing needs you. Add a project and start a session; its cards appear here." | "Nothing done yet today." |
| 3 | 1 open | no filter row | morning line, 1 card (full), nothing folded | per outcomes |
| 4 | 5 open, mixed kinds | filter row All 5 · Blocking 2 · Errors 2 · Budget 1 | 5 cards | |
| 5 | **20 open** | filter row All 20 | 5 oldest cards, then `▸ 15 more open · 3 blocking · j`; badge counts levels 1–3 | |
| 6 | Filter hides open cards | chip Blocking active + strip "Showing Blocking · 2 of 5 open cards. 3 hidden · Show all" | only the 2 level 1 cards, lines unfiltered | |
| 7 | A card collapsing | | the card at 10 s: a line replaces it; the same row with a rule line ("· rule: installs") and a pane line ("· in the pane") never as a card | |
| 8 | Failed / stale cards | | failed: card with error marker, actions again, Dismiss; stale: muted card, Dismiss | |
| 9 | Carried over from yesterday (F1) | | separator "Still open from Sun 1 Nov" + the card | |
| 10 | **A past day** | `Sun 1 Nov ▾`, strip "Sunday 1 Nov · read-only · Back to today t"; no filter row | morning line + lines, one expanded read-only | "Sun 1 Nov · 4 outcomes" |
| 11 | Past day loading / error | | skeleton + "Loading Sun 1 Nov…" / "Could not load Sun 1 Nov: <why> · Retry" | |
| 12 | Days popover | open on the day button | (thread dimmed) | |
| 13 | **Search with results** | field with “serde”, "4 results for “serde”" | results with day separators | none |
| 14 | **Search, no results** | field “zzz” | "Nothing found for “zzz” in decisions and lines." | none |
| 15 | Search loading / error / too short | | "Searching…" / "Could not search: <why> · Retry" / "Type 2 or more characters." | none |
| 16 | **Notification-opened card** | | the card with the focus ring and the tint pulse (draw the pulse peak) | |
| 17 | Notification, decision already closed | | the next open card focused + the toast "Already allowed in the pane · 14:02. Showing the next card." | |
| 18 | **Core reconnecting** | a strip under the header, 28 px, warning: **"Reconnecting to the core · showing 14:02 · decisions paused"** | cards stay as last seen, muted, actions disabled (buttons dim, keys do nothing), countdowns frozen | digest as last seen |
| 19 | Reconnected | strip disappears silently; no toast | live state resumes | |
| 20 | Inspector overlay open (< 1728) | the overlay with the banner "1 card needs you · ⌘⇧I" | (hidden behind) | |
| 21 | P4.4 only, marked "P4.4" on the page | Overwatch status slot, annotation, deciding, rule proposal, input | not built in P4.3 | |

---

## 13. D15 leftovers for ui (point 12)

On page 04.3, give each of these five cards its own subject line, blocks and actions, taken from the P4.1 fixture and P4.1 §3. None may reuse the "Checkout v2" excerpt, and no block text may be clipped at the card edge.

| Card | Title · subject line | Blocks (copy) | Actions |
|---|---|---|---|
| **Restart gate** | "Still waiting after a restart · back-1" · "back-1 · Backend · shop-api › checkout · T-4" | markdown: "back-1 was waiting for your approval of the push plan when Orchestrator restarted. The gate is still open." | Approve `a` · Reject `r` · Open `o` |
| **Run now** (Governor ask) | "Run back-1 now? ≈ +2.1 pp of the week" · "back-1 · Backend · shop-api › checkout · T-4" | markdown: "Queued: live cap 3/3. Running it now takes a fourth live slot."; cost: "Cost · ≈ +2.1 pp of the week · +6 % of the 5h window · done ≈14:40"; countdown: "Keeps queued in 18 min" | Run now `a` · Keep queued `r` · Open `o` |
| **PR gate** | "PR gate · checkout" · "lead-1 · Lead · shop-api › checkout" | markdown: "orch/checkout/integration → main"; diff_summary: "7 files changed · +142 −9 lines", file rows with path · bar · "+40 −3" (5 rows, then "Show 2 more files"), "Show patch" | Approve: push + open PR `a` · Reject `r` · Reply `⏎` |
| **Budget** | "Reserve for your own work used up" · (no agent, project only: none) | markdown: "Agents are squeezed harder until Mon 09:00."; cost: "Cost · +3.4 pp of the week reserved"; countdown: "Applies in 18 min" | Apply `a` · Skip `r` |
| **Notion** | "Comment on checkout · spec.md" · "shop-api › checkout" | markdown with source header "From Notion · checkout spec ↗": "Sam: Can we drop the retry on 409? It hides the conflict." | Reply · Dismiss `r` · Open in Notion ↗ |

Clipping rules for all cards (also the other kinds): card width 336 (316 at 1280: column minus 2×12 column padding, §9); card padding 12; blocks wrap and break long tokens; tool-call and pty blocks keep mono and scroll horizontally inside their frame with a fade at the edge, collapsed past 6 lines ("Show 14 more lines"); action buttons wrap to a second row, each label and its `Kbd` stay together; a marker line wraps to two lines, never truncates. Check every kind at 340.

Also on 04.3 (the P4.3 additions, brief §3): header with day button and search, filter row and strip, collapsed lines per verb (§2.3), the reference line (§3), the fold header (§4), the digest states (§7), day separators (§6.3), past day and search results (§6), the notification highlight (§8.1), every row of §12. Page 05: §10. Page 03: §9. Page 06: J5 storyboard in the chat, 4–6 frames: (1) a permission card arrives with a notification; (2) `a`, "Allowing once · Undo u"; (3) "Allowed · back-1 resumed"; (4) 10 s later it is a line; (5) a rule-answered permission arrives as a line only, with no notification; (6) `⏎` expands a line. `00 Cover` and the README: P4.3 rows **In review**; no new token is expected (accent tint 12 % reuses `--color-accent`; if the pulse needs a token, add `--color-accent-wash` to `tokens.json`).
