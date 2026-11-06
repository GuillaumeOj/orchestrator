# P4.4 design notes: the chat with the Overwatch

UX pass for P4.4 (The Overwatch), 2026-11-03, by `ux` for `ui` (then `front`). Sources: [spec](../../.orchestrator/features/p-4-4-the-overwatch/spec.md) §3.3, §3.7–3.9, [API contract](../../.orchestrator/features/p-4-4-the-overwatch/api-contract.md) §3, [UX brief](../../.orchestrator/features/p-4-4-the-overwatch/ux-brief.md), [decisions](../../.orchestrator/features/p-4-4-the-overwatch/decisions.md), and the earlier notes: [P4.2](p4-2-design-notes.md) §7–8 (**P4.2 §n**), [P4.3](p4-3-design-notes.md) §1, §2, §10, §12 (**P4.3 §n**), [P4.1](p4-design-notes.md) §3–5 (**P4.1 §n**). Their copy and rules stand unless a section here says "supersedes".

Copy rules as before: sentence case, no exclamation marks, name the agent and the project, local 24 h times, **every number says what it is**. The Overwatch is "the Overwatch" in running text and "Overwatch" as a label. It never says "I" in app chrome; in its own chat text it does.

Point numbers follow ux-brief §2 (1–13). Widths: the column is 360 px (card 336), 340 at a 1280 shell (card 316) (P4.3 §9). Check every board at both.

---

## 0. Findings for lead

| # | Finding | Proposal |
|---|---|---|
| F1 | **`deciding` has no exit when the Overwatch will not decide.** Spec §3.9 shows "deciding" for 60 s while the Overwatch is not `off` / `down`. Two gaps: (a) `paused` (Pause all, limit) holds the Overwatch, so a "deciding" card would count down for nothing; (b) when the Overwatch only annotates (it is unsure) or its turn ends without deciding, the user still waits out the 60 s, and the notification fires 60 s late for a call it has already passed to the user. | Core sets `deciding_until_ms` only when status is `asleep`, `waking` or `working`; absent for `off`, `down`, `paused` (card is "Waiting for you" at once, no flash). Core ends deciding **at once** (`waits_on: "user"`) when `annotate` is called on that decision, when `decide_permission` returns `not_allowed`, or when the turn that carried the event ends. The UI only reads `deciding_until_ms` / `waits_on`. **After a hand-over `decide_permission` must return `too_late`** (else the card the user is reading collapses under them). The 300 s hold should restart at hand-over, and `permission_secs` must stay below `hold_secs`. |
| F2 | **Status copy needs reasons and a session flag.** `why` is free text. `paused` and `down` need short, stable reasons; and the status click (spec §3.3 "opens its session") has nothing to open before the first event of the day (D2 lazy start). | Core sends `why` from this set (≤ 40 chars): paused → `Pause all` · `limit reached`; down → `could not start` · `stopped unexpectedly` · `login expired` · `limit reached` · `too slow, 2 turns over 10 min`. Add `overwatch.has_session: bool` (a session exists today, running or suspended). `agent` stays always present. |
| F3 | **A user-only permission does not say why it is asking.** The user sees a card for `curl evil.example` or a Linear write and cannot tell "policy says ask" from "the app never lets the Overwatch decide this". The annotation is the Overwatch's opinion, not the reason. | Add optional `asks_because` on `permission` decisions: `{ kind: "core" \| "policy" \| "default" \| "overwatch_unsure", detail }` shown as one muted line (§4.5). `core` detail comes from `policy::hard` (e.g. "network host outside the allow-list"), `policy` detail is the rule id. |
| F4 | **The core-denied line has no tail slot.** P4.3 §2.2 gives `by: core` the tail "· by the app". Spec §3.2 wants "denied back-1: … · core: outside the worktree". `by_detail` is the missing piece. | `line.by = "core"` with `by_detail` → tail `· core: <by_detail>` (≤ 40 chars); without `by_detail` the P4.3 tail `· by the app` stays. `by = "policy"` → `· policy: <id>` (replaces `rule: <id>`, spec). Migrated rules show `policy: r12` as generated; ui shows them as is. |
| F5 | **Two verbs are missing for the rule proposal** and one state. Spec lists `added`; "Not now" has no line. A line for the declined proposal keeps the history ("Not now" history is a spec reason for the own entry kind), and the corrected-answer state is not "undone". | `line.verb` gains `not now`; `line` gains `corrected_at_ms?` beside `undone_at_ms` (answered lines: "corrected 14:05"; others "undone 14:03"). |
| F6 | **The rule proposal needs to know it is the second ask, its count and threshold** (the threshold is a setting, 2–10, so "3" in copy must come from the core: add `count` and `threshold`). D6 says two declines end it. The user is never told that the second "Not now" is the last. | `rule_proposal` decisions carry `declined_before: 0 \| 1`. When 1, the button stays **Not now** and the card adds the line "You said not now on Tue 14:20. It won't be asked again if you decline." (§7). Needs the date of the first decline too: `declined_at_ms?`. |
| F7 | **Edit rule leaves the card in limbo.** Spec §3.7: Edit rule opens the file in nvim with the rule appended, commented out. Nothing closes or changes the card. | The card stays open with the marker "Opened in nvim · 15:22. Uncomment the rule to enable it." ; core closes it as `added` (muted marker "Added by hand · 15:24") when the file then contains an active rule with the same `match`, `effect` and scope. Otherwise the user presses **Not now** or **Add rule**. |
| F8 | **Annotation length against the card.** P4.2 §8.1 says "at most two lines"; the contract allows 280 chars (about 5 lines at 316 px). | UI clamps to 3 lines with "Show more" (expands in place); role prompt asks the Overwatch for ≤ 140 chars; the 280 cap stays. |
| F9 | **Source links in free text beyond docs.** J12 frame 3 (P4.2 §7) cites "Governor log 11:02–11:40 ↗" and "attention records". The contract's link kinds are doc paths and `chat:<id>`. Nothing opens a Governor log. | Link schemes in `post` text and `sources`: a doc path (opens in the feature pane), `chat:<id>`, and two new: `agent:<id>` (opens the session over the board), `run:<id>` (opens the run in the inspector). The Governor's log is cited as plain text ("Governor log, 11:02–11:40"). J12 frame 3 redrawn that way (§12). |
| F10 | **Pending messages have no way out.** A user entry with `delivery: pending` while `off` may sit for days and then reach a session that started the next morning. | `chat_discard { entry }` for `delivery: pending` entries only (removes the entry; the line "Removed" is not kept). Pending entries older than the current day are shown with their day ("Mon 2 Nov") and delivered with the next briefing marked "sent earlier". Small; lead may defer to P4.4.x, then the UI shows no Remove. |
| F11 | **Take over in the Overwatch's pane.** Spec §3.3 keeps it as an escape hatch. Typing there is a pane intervention by FR-1311 if counted like any agent, which would hurt the exit criterion for the very act of fixing the Overwatch. | Decision for lead. Default in these notes: watch works as for any session; `⏎` take over works; the strip reads "Typing in the Overwatch's pane. Ask in the chat instead when you can."; typed input there **is** recorded as `pane_intervention` (honest count). If lead prefers not to count it, only the strip copy changes. |
| F12 | **`u` has three meanings now.** P4.3 §10.2: `u` undoes the last decision within 5 s. P4.4 adds `u` on a focused line (spec §3.8) and the rule proposal's 5 s undo. | Precedence in §6.6: a focused line with Undo → that line; else a focused card in its 5 s window; else the last decision in its window. P4.3 §10.2's `u` row is superseded by §11. |
| F13 | **`deciding` cards and the 5-card ceiling / ⌘⇧I.** P4.3 §4 folds past 5 open cards and ⌘⇧I focuses the oldest open one. A deciding card is open for ≤ 60 s and blocks nobody. | Deciding cards are **always shown** (never folded), **do not count** toward the 5 or "N more open", and ⌘⇧I skips them unless nothing else is open. `j` / `k` include them (they are actionable). |
| F14 | **Past day: no input.** A past day is read-only (P4.3 §6.2). The brief and spec are silent on the input there. | The input is replaced by P4.3's "Read-only · Back to today t" footer on a past day; on a search it stays, and typing there is a message, not a search (search has its own field, `/`). |
| F15 | **Policy files that do not exist.** `policy_files` lists scopes; a project with no file yet has nothing to open, and D4 says Open in editor is the only editor. | `policy_files` returns every registered project and the global scope even when the file is missing (`exists: false`, `rules: 0`). **Open in editor** on a missing file creates it with `version = 1` and a one-line comment, then opens it. |
| F16 | **Planner / report: is the Overwatch inside the plan?** Spec: usage of `overwatch` allocation, `by_role` key `overwatch`, `CostClass::Critical` (the reserve). Not said whether it counts in "week against plan". | Notes assume it **counts in the week's usage** (it is real spend) and is **not planned**: the row has no planned share and says it draws on the reserve (§10). If it should be excluded from the % in the morning line, say so before ui draws. |
| F17 | **J12 callouts that are no longer true.** P4.2 §7 frame 1 says "The core starts the day's session with its briefing" and status "awake"; neither exists (D2: lazy start; statuses are `off asleep waking working paused down`). Frame 3 ends on "awake". | Redrawn in §12: frame 1 status `asleep`, callout "The morning line is the app's; the Overwatch wakes on the first event"; frame 3 ends `working · 1 event` then `asleep` after the idle grace. |
| F18 | **User message length.** Contract has none for `chat_send`; the turn event text is capped (4 k chars total). | `chat_send` rejects > 2 000 chars (`bad_request`); the input shows a counter from 1 800: "1 950 / 2 000" and disables send above. Longer text: "Too long for a message. Put it in a file and ask the Overwatch to read it." (hint only). |

---

## 1. Header with the Overwatch (point 1)

```
┌ Overwatch   asleep            Today ▾  🔍 ┐  header 40 px
│ strip (only when down, or reconnecting)    │  28 px
│ filter row                                  │  32 px (P4.3 §5)
```

- **Title**: `Overwatch` (the one string, P4.3 §1). **Status** sits right after it, 8 px gap, flex, truncates with …; then `Today ▾` and 🔍 (unchanged, right). The status is text + a 6 px dot (shape differs by state so colour is never the only signal), `text.secondary`, 12 px.
- **Status copy** (short form in the header; long form is the tooltip and the accessible name):

| Status | Header | Dot | Tooltip / accessible name |
|---|---|---|---|
| `off` | `off` | hollow, muted | "Overwatch is off. Turn it on in Settings." |
| `asleep` | `asleep` | hollow | "Overwatch is asleep. It wakes on the next event." |
| `waking` | `waking…` | half, accent | "Overwatch is waking · 2 events waiting" |
| `working` | `working · 2 events` | filled, accent | "Overwatch is working on 2 events" (1: `working · 1 event`) |
| `paused` | `paused` | two bars, warning | "Overwatch is paused: Pause all" / "…paused: limit reached" (from `why`) |
| `down` | `down` | filled, warning | "Overwatch is down: login expired. Cards still work." |

  Numbers say what they are: "2 events", never a bare 2. The word is always present.
- **Strip** (28 px, under the header, same family as P4.3 "Reconnecting"): only when `down`:
  `Overwatch is down · login expired · cards still work`  and the button **Restart** (right). A second strip never stacks with "Reconnecting to the core": the reconnecting strip wins; Restart is disabled (dim) while the core is unreachable. Restart pressed: button reads "Restarting…" until status leaves `down`; if it fails the strip keeps the reason and the button returns ("Could not restart: <why>" replaces the reason once). `paused` and `off` need no strip (they are the user's own choices); `off` adds the hint under the input (§2).
- **Click on the status** (or `Enter` on it when focused): opens the Overwatch's session over the board, as a row's watch (P4.2 §6): a tab after Mission control's, watch mode, strip "Watching the Overwatch · ⏎ take over · Esc back". Enabled when `has_session` (F2); with no session today, the status is not a control (no hover, tooltip as above). While `off` it is not a control either. See F11 for take over.
- **At 340 px**: title (~70 px) + status (up to ~120 px) + day button (~76) + search (28) + paddings fit. Worst status `working · 2 events` is 112 px. Below that the status truncates to `working · 2`; the tooltip keeps the full text. `Today ▾` never truncates. The filter row and strips are separate rows so nothing in row 1 needs Restart.
- **Day / search / filter keep their P4.3 behavior**; the search field opens as its own row under the header (P4.3 §6.4), so the status stays visible.
- **Accessibility**: the status is a `role="status"` region only for changes the user did not cause (down, back), announced politely once; the header title `h2` "Overwatch".

---

## 2. Input (point 2)

```
├────────────────────────────────────────────┤
│ Ask the Overwatch…                          │  min 40 px, grows to 5 lines (120 px)
│                                  ⏎ send     │  hint row 20 px, muted
└────────────────────────────────────────────┘
```

- **Placeholder**: `Ask the Overwatch…` (always). **Label** (accessible): "Message the Overwatch". Hint row under the field: left, the state hint (below); right, `⏎ send · ⇧⏎ new line` (key-hint container) while focused or non-empty, else `⌘L`.
- **Keys**: `⏎` sends (not while IME composing); `⇧⏎` newline; `Esc` leaves the field to the thread and **keeps the draft**; a second `Esc` is P4.3's (focus returns to the centre pane). Sending clears and keeps focus. Empty or whitespace-only: `⏎` does nothing, no beep. Over 2 000 chars: send disabled, counter in warning (F18).
- **Hint row by status** (left side; one line, muted; replaces `⌘L` hint when it applies):

| Status | Hint | Entry marker after sending (under the user's message) |
|---|---|---|
| `working` | none | none (the thread shows "Overwatch is working · 12 s" below, §2.2) |
| `asleep` | none | `waking…` until the status leaves `waking`, then it clears |
| `waking` | none | `waking…` |
| `paused` | `Paused · it answers when it resumes` | `Sent when the Overwatch resumes` (core: `delivery: pending` also while paused; open point for back) |
| `down` | `Down · messages are kept until it is back` | `Sent when the Overwatch is back` |
| `off` | `Off · turn it on in Settings` (link) | `Sent when the Overwatch is turned on` |

  The input is never disabled by a status (principle: nothing waits on the Overwatch; here it is the user's message that waits, and says so). A delivered entry shows no marker. A pending entry ends with `Remove` (F10) as a text button at the right of the marker.
- **Draft**: kept while the app is open, across focus changes and past-day visits; one draft; not saved on quit.
- **Past day** (F14): no input; the P4.3 footer "Read-only · Back to today t" stands.

### 2.1 The thread: entries

- **User entry**: right-aligned block, max 85 % of the column, `accent wash` fill (the P4.3 tint token), radius as cards, text left-aligned, label row above: `You · 14:05`. Plain text with line breaks; links in it are not auto-linked.
- **Overwatch entry**: left-aligned, full width, no fill, a 2 px left rule in `border.default`; label row `Overwatch · 14:05` (secondary) and, when it answers a user entry that is not the previous item, the P4.3 reference line above the label: `↳ re: why is checkout slow today?` (cut at 48 chars, a link to the user entry, `reply_to`).
- **Markdown subset**: bold, italic, inline code (mono), links, bullet and numbered lists, fenced code (mono, scrolls horizontally with a fade, collapsed past 8 lines: "Show 12 more lines"), block quotes. Headings render as bold paragraphs. Tables render as a mono block that scrolls horizontally. No images, no HTML (escaped). External `http(s)` links open in the browser with a trailing ↗.
- **Source links** (contract §3.2): a doc path shows as its file name plus section: `api-contract.md §3 ↗` (`#3` → `§3`, `#slug` → `#slug`) and opens the rendered doc in the feature pane; `chat:<id>` shows as `your answer 11:02 ↗` / `earlier card 11:02 ↗` and jumps to that entry with the P4.3 §8.1 highlight; `agent:<id>` as `back-1 ↗`, `run:<id>` as `run checkout ↗` (F9). A link that no longer resolves is plain text with a tooltip "No longer available".
- **Long answers**: over 12 lines (about 600 chars) the entry clamps to 12 lines with `Show more` / `Show less`; the clamp never cuts a list item or a code block mid-line; `⏎` on the entry toggles it.
- **Order and quietness**: entries sit at their posting time. A new Overwatch text does not scroll the thread if the user is not at the bottom; instead a pill at the bottom edge `New from the Overwatch ↓` (click or `⏎` scrolls). If at the bottom, it stays at the bottom. It does not notify and does not touch the badge; screen readers get the throttled live region: "The Overwatch replied" (not the text).
- **Accessibility**: each entry is an `article` with `aria-label` "You, 14:05" / "Overwatch, 14:05".

### 2.2 In flight and failures
- After the user sends and the entry is delivered, a muted row at the foot of the thread: `Overwatch is working on it · 12 s` (elapsed, updates every 5 s, not every second; not announced). It is removed by the reply or when status leaves `working` and `waking`.
- **Turn ended with no reply** (delivered, status back to `asleep` or `working` for other events, no entry with `reply_to` for it): the muted line under the user's entry becomes `No reply. Ask again, or look at its session.` with the second part a link when `has_session`. Nothing is retried by the UI.
- **Send fails** (`chat_send` rejects): entry stays at the foot, error colour, `Not sent: <why> · Retry`; the draft is gone, the entry carries the text.

### 2.3 ⌘L and `/` (supersedes P4.3 §10.1 and §6.4 for the key)
- **⌘L** focuses the input from anywhere, including a terminal pane (chords never reach the PTY); closes the overlay and an open search if any. When the input is already focused: nothing.
- **`/`** focuses the search field (opens it); also the 🔍 button. In the input, `/` types a slash. Palette: "Ask the Overwatch" (⌘L), "Search decisions" (/).

---

## 3. Deciding (point 3)

A `permission` decision with `deciding_until_ms` (effect `overwatch`, status `asleep`/`waking`/`working`, F1).

```
PERMISSION                                  just now
Bash · pnpm add zod
back-1 · Backend · shop-api › checkout · T-4
┌ tool call ─────────────────────────────────┐
│ pnpm add zod                                │
└─────────────────────────────────────────────┘
[ Allow once a ] [ Deny r ] [ Deny with message ⏎ ] …
◔ Overwatch is deciding · 42 s left
```

- **Marker** (foot, replaces "Waiting for you · n min"): `Overwatch is deciding · 42 s left`, with a 12 px ring that drains over the 60 s. Remaining seconds update once a second visually (`aria-hidden`); the accessible name is static: "Overwatch is deciding, up to 1 minute". No sound, no notification, **not in the badge** (core: `waits_on: overwatch`, P4.1 count unchanged).
- **Actions stay live**, all of them, with their keys; the card is a normal card in every way except the marker and F13 (never folded, not counted toward 5, skipped by ⌘⇧I when something waits on the user).
- **Hand-over**: at 0 (or at once, F1), the marker changes to `Waiting for you · just now` in the same slot (no reflow), the badge counts it, the level-1 notification fires from that moment, the live region announces "Waiting for you: Bash pnpm add zod". If the Overwatch added an annotation, it shows from the same moment (or earlier, annotation first appears even while deciding; the marker does not change).
- **The Overwatch decides first**: the card collapses to the line at once (P4.3 §2.1: settled by someone else = a line, never a lingering card): `▸ allowed back-1: pnpm add zod · Overwatch: routine install, no rule`. The user, if they were reading it, loses the card; so while the pointer is over a deciding card or focus is inside it, the card does not collapse until they leave (P4.3 §2.1 pause rule), the marker reading `Allowed by the Overwatch · 14:02`.
- **The user acts first**: normal path ("Allowing once · Undo u" → "Sent to back-1"). The Overwatch's late call returns `stale`; no Overwatch line is written; the user's line carries no mention of it.
- **`off` / `down` / `paused` at the time**: no deciding marker ever shows; the card is "Waiting for you" from its creation. If the status changes to `down` while deciding, the marker flips to "Waiting for you" at once (F1) with no flash.
- **Reduced motion**: the ring is replaced by the text alone.

---

## 4. Annotation (point 4)

### 4.1 Anatomy
One block above the actions, `surface.subtle`, 12 px padding, left rule in accent, never a button:
```
Overwatch · recommends deny
Not in the allow list; nothing in the docs says registry.internal is used.
per allow-list.md §2 ↗ · your answer 11:02 ↗
```
- Row 1: label `Overwatch` and, when `recommend` is set, ` · recommends <action label>` (the card's own label for that action id: "recommends deny", "recommends approve", "recommends always allow").
- Row 2: the `note`, plain text (it is model text: no markdown, no links), clamped to 3 lines with **Show more** (F8).
- Row 3: sources, up to 3 as links (§2.1 forms), `+ 2 more` expands; none → row omitted.
- **Recommended action**: that button gets an accent outline and a small `recommended` tag inside the label ("Deny r · recommended" wraps to a second row on 316). **Never pre-selected**: focus, default `⏎` and the primary key stay the card's own (`a` still means the primary action even when the Overwatch recommends deny). Only one action is highlighted. When `recommend` names an action the card does not have (stale id), nothing is highlighted and the note still shows.
- An annotation replaced by a later call replaces the block in place (no animation, `aria-live` polite "The Overwatch updated its note").

### 4.2 Per card kind (where it can appear; the Overwatch may annotate only open decisions)
| Card | Note forms (examples, ≤ 140 chars, the role prompt gets these) | Highlighted action |
|---|---|---|
| Permission (user-only) | "Host not in the allow list; I'd deny." · "Writes to Linear; routine for this task, I'd allow once." | Deny · Allow once |
| Permission (after the 60 s, unsure) | "No rule covers this and I can't tell if it is safe." | none |
| Spec gate | "Scope matches the brief; no open questions left." · "The spec drops the retry on 409 that the brief asks for." | Approve spec · Send back |
| PR gate | "7 files, tests added, matches the spec." · "Touches billing/, which the spec does not mention." | Approve · Reject |
| Restart gate | "back-1 was idle, nothing to lose; I'd approve." | Approve |
| Governor ask (Run now) | "Takes a fourth live slot for ≈ +2.1 pp; checkout is the plan's priority." | Run now · Keep queued |
| Budget | "Reserve is 10 %; applying costs nothing you planned." | Apply · Skip |
| Question it escalates | "Product call: the docs say nothing on guest checkout." | none (questions have no recommended answer button; the note may name an option in words) |
| Startup / Error / Wait / Trust / Notion | not annotated (the Overwatch acts on these instead) | — |

### 4.3 "I'd allow it for this project" notes
For a recommended **Always allow**: "Routine for this project: the same call was allowed 2 times today. I'd allow it for shop-api." The highlighted button is **Always allow** (its label is unchanged). Always allow still writes the policy file (spec §3.2.4) and the card's effect marker reads "Allowed · rule added to shop-api policy". Notes never propose a rule themselves; that is the proposal card (§7).

### 4.4 Annotation on lines
Expanded lines keep the annotation, read-only, muted (P4.2 §8.4), without the highlight.

### 4.5 Why the user is asked (F3)
Permission cards that reach the user because of the core, a policy rule or an unsure Overwatch show one muted line under the subject line:
- core: `Only you can allow this: network host outside the allow-list.`
- policy: `Only you can allow this: policy installs-ask.` (rule id)
- default/off: nothing (it is the normal ask).
- unsure: `The Overwatch did not decide this.`
Copy ≤ 70 chars, plain, no jargon ("hard-deny", "user-only" never shown).

---

## 5. Overwatch lines (point 5)

Same line pattern as P4.3 §2.2 (`▸ <verb> <agent>: <what> · <tail>        time`). Tail by `by`:

| `by` | Tail |
|---|---|
| overwatch | `· Overwatch: <by_detail>` (reason, ≤ 60) |
| policy | `· policy: <id>` (replaces `rule: <id>`; old entries keep `rule:`) |
| core | `· core: <by_detail>` or `· by the app` (F4) |

**Truncation** (one 28 px line at 340): the `what` shrinks first down to 16 characters, then the tail truncates with …; the full text is the tooltip and the expanded view. Sources never appear in the collapsed tail except for `answered` (below).

### 5.1 Every verb (final copy)

| Verb | Collapsed line | Expanded adds (below the read-only card or text) |
|---|---|---|
| allowed (Overwatch) | `▸ allowed back-1: pnpm add zod · Overwatch: routine install, no rule` | marker `Allowed by the Overwatch · 14:02`; reason in full; annotation if any |
| allowed (policy) | `▸ allowed back-1: cargo add serde_json · policy: installs` | marker `Allowed by policy rule installs · 14:02`; `Rule in shop-api/.orchestrator/policy.toml` |
| denied (Overwatch) | `▸ denied back-1: rm -rf build · Overwatch: outside the task's files` | marker `Denied by the Overwatch · 14:02`; the denial message sent to the agent |
| denied (policy) | `▸ denied back-1: curl … · policy: no-network` | marker `Denied by policy rule no-network` |
| denied (core) | `▸ denied back-1: git push origin main · core: git push is never allowed` | marker `Denied by the app · 14:02`; line `This is always denied. No rule or answer can allow it.` |
| answered (Overwatch) | `▸ answered front-1: API pagination · from api-contract.md §3` | the question, the answer, **sources** as links; marker `Answered by the Overwatch · 14:02` |
| restarted (Overwatch) | `▸ restarted ui-1: first session never started · Overwatch: fresh start` | marker `Restarted fresh by the Overwatch · 14:02`; what it told the agent |
| resumed (Overwatch) | `▸ resumed back-1: parked on an expired hold · Overwatch: answer was given` | marker `Resumed by the Overwatch · 14:02 · how: resumed / queued / restarted fresh` |
| nudged (Overwatch) | `▸ nudged back-1: "rebase on the integration branch" · Overwatch: idle 12 min` | the full nudge text as sent: `ORCH: from the Overwatch: …` |
| reprioritized (Overwatch) | `▸ reprioritized back-2: Normal → High in the queue · Overwatch: unblocks checkout` | marker `Tier changed by the Overwatch · 14:02 · was Normal` |
| added (rule, you) | `▸ added rule: allow pnpm add * in shop-api` | the rule block (mono), file, the three decisions it came from |
| not now (rule, you) | `▸ not now: allow pnpm add * in shop-api · asking again after {threshold} more` | the rule block; second time: `won't ask again` |

- **Answered** is the one verb whose tail is the source, not the reason (the reason is the answer; the source is the proof): `· from <first source>`, and ` +1` when more (`· from api-contract.md §3 +1`). Several sources expand in the line's body.
- **Colours**: `denied` verb in warning text as in P4.3; the `core:` tail is plain secondary. Nothing else is coloured; the word carries it.
- **`what`** is the core's cut text (≤ 48): command, question title, agent state, queue move. Verbs never name an action the Overwatch did not take.
- **No grouping in P4.4** (brief 13): 10 consecutive lines are 280 px; each is separately undoable and searchable, a group would hide that. Revisit when a real day shows 30+ (the digest P4.3 §7 already folds Done outcomes, not lines).
- Lines never notify, never count in the badge, never take focus; a line inserted while the user reads is quiet (P4.3 §2.1).
- Search (P4.3 §6.4) matches verb, `by_detail` and sources (P4.3 F9 is now needed, not optional: ui promises "Search decisions, lines and the Overwatch's replies").
- `aria-label`: "Allowed back-1: pnpm add zod, by the Overwatch, routine install, no rule, 14:02. Undo available."

---

## 6. Undo on lines (point 6)

### 6.1 Table (contract labels kept; text below them is new)

| Line | Button (expanded) | `u` on the collapsed line | Explanation under the button | Done state |
|---|---|---|---|---|
| allowed by policy | **Revoke rule** | revokes | `Removes rule installs from shop-api policy. The call already ran.` | `▸ … · policy: installs · undone 14:03`, marker `Rule installs revoked · 14:03` |
| allowed by Overwatch | **Tell the agent** | tells | `Tells back-1 not to repeat this call and denies the next identical one for 1 hour. The call already ran.` | marker `Undone 14:03 · back-1 was told not to repeat it` |
| denied by Overwatch | **Allow instead** | allows | `Lets back-1 retry this exact call once and tells it so.` | marker `Undone 14:03 · back-1 was told it may retry` |
| answered by Overwatch | **Correct the answer** | opens the field | `Send back-1 what it should know instead.` | marker `Corrected 14:05 · back-1 was told` |
| reprioritized | **Restore tier** | restores | `Puts back-2 back at Normal.` | marker `Restored to Normal · 14:03` |
| rule added (from a proposal) | **Remove rule** | removes | `Removes the rule from shop-api policy.` | marker `Rule removed · 14:03` |
| nudged · resumed · restarted | none | — | (§6.3) | — |
| denied by core | none | — | `Always denied by the app. Nothing can change it.` | — |

### 6.2 Collapsed and expanded
- **Collapsed**: when undo is available, the right edge shows `Undo u` before the time (`Undo u   14:02`); it pushes the `what` shorter, not the time. Click on `Undo u` acts like `u`. Where the label needs input ("Correct the answer") it reads `Correct u`.
- **Expanded**: the button with its explanation; the button is secondary style. After the click: the button reads `Telling back-1…` (label by kind: `Revoking…`, `Allowing…`, `Restoring…`, `Removing…`), disabled; on success the done state of the table; on a core error the button returns with `Could not undo: <why>` in error colour.
- **Undone**: the line reads `… · undone 14:03` (muted, whole line secondary; `corrected 14:05` for answers, F5). It keeps the expanded body and the original reason. No second undo. `u` does nothing.

### 6.3 Not available
- While the line still could be undone but the condition is gone (`undo_gone`), `Undo` disappears from the collapsed line and the expanded body says `Can't be undone: back-1 already moved on` (muted). Other reasons use the same prefix: `the rule is no longer in the file`, `the task was restarted`, `it was restored already`.
- Lines with no undo ever (nudged, resumed, restarted, core denial): expanded body ends `Can't be undone: it already ran.` (core denial: `Always denied by the app. Nothing can change it.`). It is a statement, not a button; muted.

### 6.4 "Correct the answer" field
Expanded under the answer, a text area (2 rows, grows to 6), placeholder `What should front-1 know instead?`, buttons **Send correction ⌘⏎** and **Cancel Esc**. `⏎` is a newline here (the field is a reply area like P4.1's, where ⌘⏎ sends). Sent: marker `Corrected 14:05 · front-1 was told`, the correction text shown quoted. Empty: Send disabled. The agent receives the P4.4 string (spec §3.8). The field survives collapse/expand of the line (draft kept until sent or cancelled).

### 6.5 Timing
No fixed window except the condition (spec §3.8 "available while"). The only timed undo is the rule proposal's card (5 s, §7), after which the line's **Remove rule** takes over while the rule is in the file.

### 6.6 `u` on a focused line
`u` with the focus on a line (or inside it) that has Undo undoes that line, no expand, with a toast after the core confirms (P4.1 §4 rule): `Rule installs revoked.` · `Told back-1 not to repeat it.` · `back-1 may retry.` · `Restored back-2 to Normal.` · `Rule removed.` For "answered", `u` expands the line and focuses the correction field. If the focus is on a card or elsewhere: P4.3's rules (F12, §11).

---

## 7. Rule proposal (point 7)

### 7.1 Card
```
RULE PROPOSAL                                  15:20
Allow `pnpm add *` in shop-api?
You allowed this {count} times as front. Add a rule so it stops asking.
  Mon 14:02 · front-1 · allowed ↗
  Mon 14:40 · front-1 · allowed ↗
  Mon 15:10 · front-1 · allowed ↗
┌ rule ──────────────────────────────────────┐
│ allow  Bash(pnpm add:*)                     │
│ scope  shop-api · role front                │
│ file   shop-api/.orchestrator/policy.toml   │
└─────────────────────────────────────────────┘
The Overwatch: same package manager, never touched outside node_modules.   (only when it proposed with a `why`)
[ Add rule a ] [ Not now r ] [ Edit rule ⏎ ]
Waiting for you · just now
```
- **Title**: `<Effect> \`<match shown readable>\` in <project>?` — "Allow `pnpm add *` in shop-api?" (the mono rule block keeps the exact FR-708 grammar `Bash(pnpm add:*)`). Global scope: "in every project". With a role: "…in shop-api for front".
- **Text**: "You allowed this {count} times as front. Add a rule so it stops asking." Deny rules: "You denied this 3 times…  Add a rule so it is denied without asking." Core-posted (Overwatch off/down/no proposal): same text, no "The Overwatch:" line.
- **Links**: the three earlier decisions, `<day> <time> · <agent> · allowed|denied ↗`; click jumps to the entry (or reads "(earlier day)" plain text when beyond 30 days).
- **Level 4**: never notifies, not in the badge (spec); sits in the thread at its time; `j/k` stop on it; ⌘⇧I skips it unless nothing else is open.
- **Second ask** (F6): the text line is added: `Not now on Tue 14:20. It won't be asked again if you decline.`

### 7.2 States
| State | Marker / body | Actions |
|---|---|---|
| Open | `Waiting for you · 3 min` | Add rule a · Not now r · Edit rule ⏎ |
| Adding (core call) | `Adding rule…` | disabled |
| Added | `Added to shop-api policy · 15:21 · Undo u`, muted, 5 s | Undo u (removes the rule; the card returns to open) |
| Added, after 5 s | collapses at once to `▸ added rule: allow pnpm add * in shop-api` with **Remove rule** in its expanded body (§6) | |
| Not now (1st) | `Not now. Asking again after {threshold} more.`, muted; collapses at once to the `not now` line | none |
| Not now (2nd) | `Not now. It won't be asked again for pnpm add.`, muted; collapses to the line | none |
| Edit opened (F7) | `Opened in nvim · 15:22. Uncomment the rule to enable it.` | Add rule a · Not now r · Edit rule ⏎ stay |
| Added by hand (F7) | `Added by hand · 15:24`, muted; collapses | none |
| Failed | `Failed · policy.toml could not be written: <why>` (error) | actions again · Dismiss |
| Stale | `Stale · a rule for this already exists`, muted | Dismiss |

Never more than one proposal open per kind of call (spec). A line from a second ask reads the same; its expanded body says `Second time asked`.

---

## 8. Overwatch down, off, paused (point 8)

| Status | Header | Strip | Input hint | Cards | Permission with effect `overwatch` |
|---|---|---|---|---|---|
| `off` | `off` | none | `Off · turn it on in Settings` | no annotation ever; normal | "Waiting for you" from the start; no deciding marker |
| `paused` | `paused` (warning) | none | `Paused · it answers when it resumes` | existing annotations stay; no new; no deciding | straight to "Waiting for you" |
| `down` | `down` (warning) | `Overwatch is down · <reason> · cards still work` + **Restart** | `Down · messages are kept until it is back` | all cards live, annotations kept | straight to "Waiting for you"; a permission that was deciding flips at once (F1) |

- **What the user sees on the card**: nothing about the Overwatch. A permission whose default effect is `overwatch` and arrives while down looks like any user permission; its muted line (§4.5) is omitted (the cause is the status, which is already in the header). The status is the only place the state is said.
- **Back to normal** (`down` → `asleep`/`waking`): the strip disappears silently; pending user messages show `waking…` then clear; one toast: `Overwatch is back.` (only after a `down`, never after `paused`/`off`).
- **Restart** (strip): fresh session, new briefing (spec). Copy of the toast on success is the one above; no confirm dialog (nothing is lost; the day's session was already unusable).
- **Turning it off in Settings with a session working**: status goes `off` at once; open deciding cards flip to "Waiting for you" (F1).

---

## 9. Settings (point 9, page 04.7)

Two sections, in this order, after Integrations: **Overwatch** then **Permissions**. Same row pattern and save behavior as the existing sections (ui copies the Integrations rows: label left, control right, one muted help line under the label; changes apply on blur / ⏎ with the same inline confirmation the other rows use).

### 9.1 Overwatch
| Row | Control | Default | Help (muted, one line) | Range / validation |
|---|---|---|---|---|
| **Overwatch** | switch | on | "An agent that settles routine prompts, answers questions from the docs and keeps agents moving. You see a line for everything it does." | — |
| **Model** | select: Haiku · Sonnet · Opus (newest of each; the model list as elsewhere) | Sonnet | "The model for its session. Takes effect at the next session." | — |
| **Batch window** | number + `s` | 30 s | "How long it waits to collect non-urgent events before a turn." | 5–300 s |
| **Permission time** | number + `s` | 60 s | "How long a permission can wait for it before it comes to you." | 10–300 s, below the hold length (strictly, api-contract §5); else "Must be below the hold length (300 s). Still using 60 s." |
| **Idle grace** | number + `s` | 120 s | "How long it stays awake after its last event before it sleeps." | 30–1800 s |
| **Rule threshold** | number + `times` | 3 | "Proposes a rule after you give the same answer this many times." | 2–10 |

- Switch off with a session working: no dialog; below the row, one muted line: `Off. Open permissions come to you. Its session ends now.` Switch on: `On. It wakes on the next event.`
- A `down`/`paused` status shows in the section header as the same text as the chat header status (`Overwatch · down: login expired`), with Restart.
- Invalid value: the field's border in error and the message under it; the old value stays applied.

### 9.2 Permissions
- **Policy files** (list from `policy_files`): one row per scope, `Global` first then each project by name:
  `Global · ~/…/Orchestrator/policy.toml · 4 rules                  [Open in editor]`
  `shop-api · .orchestrator/policy.toml · 12 rules                  [Open in editor]`
  A missing file: `shop-api · no policy file yet · created with your first rule   [Create and open]` (F15). Path shown shortened in the middle; full path in the tooltip; **Open in editor** opens nvim (FR-1101).
  - **Parse error**: the row's rules count is replaced by `Parse error at line 14: expected "=". Using the last good version.` (error), and the file's button stays. The same raises the `alert` decision of spec §3.2.2 (its copy: `policy.toml has an error at line 14` · subject `shop-api`; body as above; action **Open in editor**).
  - **Warnings** (below the row, one line each, warning text): `Rule installs: allow Bash(git push:*) has no effect: git push is always denied by the app.` Format: `Rule <id>: <message from the core>`. At most 3, then `Show 2 more`.
  - Empty state (no projects): `No projects yet. Policy files appear when you add one.`
- **Default effects** (two selects, same list): `Team roles` · `Solo sessions`. Options: **The Overwatch decides** (`overwatch`) · **Ask me** (`user`). Help: "When no rule matches a permission. The Overwatch's decisions are lines you can undo; Ask me makes a card." Solo default `Ask me`. With the Overwatch off, both selects show a muted note: "The Overwatch is off: every unmatched permission asks you."
- **Hold length** (existing): label **Hold a permission for** · number + `s` · help "How long a permission waits for you before the agent parks." Moves here from wherever it sits now (ui: keep the control, add the heading).
- No rule editor, no rule list (D4); the section ends with a muted line: "Rules are added from cards and removed from their lines. The files are plain text."

---

## 10. Week planner and report (point 10, page 04.6)

- **Row**: **Overwatch**, after the team roles and Solo sessions, separated by a hairline; icon: the Overwatch glyph used for its lines (mdi `eye-outline`, or the P4.1 set equivalent, ui picks from the icon table). No planned share (F16): the planned column shows `—` with tooltip "Not planned: it draws on the reserve."
- **Planner**: `Overwatch · 1.4 pp so far this week · from the reserve`. Numbers say what they are: "pp of the week", never a bare %.
- **Weekly report** table: `Overwatch   —   1.4 pp   from the reserve`; included in the "by role" table and the total; in the model windows it appears under its model like any agent. A week with none: row `Overwatch · 0 pp` is omitted when the Overwatch was off all week; shown with `0 pp` otherwise.
- Not a role card on the team canvas (spec: fixed in code); the canvas shows nothing new.

---

## 11. Keys (point 11, page 05)

Supersedes P4.3 §10.1 (⌘L), §10.2 (`u`, `/`, `Esc`) and the conflict table rows noted.

### 11.1 Global
| Key | Action |
|---|---|
| ⌘L | Focus the Overwatch input (closes the overlay and an open search). Replaces "focus search". |
| ⌘⇧I | As P4.3, with F13 (skips deciding cards and rule proposals unless nothing else is open). |

### 11.2 In the chat (not in a text field)
| Key | Action | On |
|---|---|---|
| `/` | Focus search (was ⌘L / `/`) | any |
| `u` | Undo, in this order (F12): the focused line's Undo; else the focused card in its 5 s window; else the last decision within 5 s | line · card · any |
| `⏎` | Adds: toggles `Show more` on a long Overwatch entry; opens the status's session when the status has focus; jumps to a reference | entry · status |
| `Tab` | Order: header (title, status if enabled, day, search) → thread items → input → digest | — |
| `a r o x e c f [ ] t j k` | Unchanged | — |

### 11.3 In the input
| Key | Action |
|---|---|
| `⏎` | Send (not while composing) |
| `⇧⏎` | New line |
| `Esc` | Leave to the thread, keep the draft; again: centre pane (P4.3) |
| `⌘⇧I` | Leave to the oldest open card, keep the draft |
| ⌘L | No-op |
| Every other single key | Types. The card keys (`a r u o x e c f j k t / [ ]`) **never** fire while the input has focus; the hint row says `⏎ send · ⇧⏎ new line · Esc leave`. |
| In "Correct the answer" and the other reply fields | ⌘⏎ sends, `Esc` cancels (as P4.1) |

### 11.4 Conflict check
- ⌘L: was search; no other binding. The palette entry "Search decisions" now shows `/`; "Ask the Overwatch" shows ⌘L. Anyone who learned ⌘L = search from P4.3 gets the input; the placeholder and hint teach the swap, no toast.
- `/` is unbound in Mission control (P4.3 §10.3), so no collision; `u` is unbound there.
- The status control joins the Tab order; `⏎`/`Space` activate it. `Esc` from the opened session returns to the status.
- Palette adds: "Ask the Overwatch" (⌘L), "Restart the Overwatch" (only when `down`), "Open the Overwatch's session" (when `has_session`), "Turn the Overwatch on/off" (opens its Settings row; no direct toggle from the palette).

---

## 12. J12 (point 12, page 06.1)

Five frames as P4.2 §7, with the built column and the real strings. Fixture: shop-api › checkout, back-1 / front-1 / ui-1, Overwatch on. Footer on each frame: `Interventions this day: 0`.

| # | Frame | Chat column (top to bottom) | Callouts | Keys |
|---|---|---|---|---|
| 1 | **Morning line** · 08:30 | status `asleep`; morning line "Good morning. 1 run in progress, 1 decision waiting, week 38 % against 35 % planned."; one open card (spec gate); input "Ask the Overwatch…" | ① "The morning line is the app's: no model, no cost" ② "The Overwatch is asleep: it wakes on the first event" ③ "Mission control shows the same numbers" | none; `⌘⇧I` for the card |
| 2 | **The day, settled** · 11:10 | status `asleep`; lines: `▸ allowed back-1: cargo add serde_json · policy: installs`, `▸ answered front-1: API pagination · from api-contract.md §3`, `▸ restarted ui-1: first session never started · Overwatch: fresh start`; two cards: **Spec gate · checkout** (annotation "Scope matches the brief; no open questions left." recommend Approve) and **Permission · WebFetch registry.internal** (muted "Only you can allow this: network host outside the allow-list."; annotation "Host not in the allow list; I'd deny." recommend Deny) | ① "Routine prompts settle themselves: a line each, expand to see why, undo while it still means something" ② "Cards that need you carry its recommendation, never a pre-selected answer" ③ "Lines never notify" | `⌘⇧I` · `a` approve spec · `j` · `r` deny · `u` on a line to undo |
| 3 | **Ask why** · 14:05 | `You · 14:05` "why is checkout slow today?"; muted `Overwatch is working on it · 12 s`; then `Overwatch · 14:06`: "checkout waited **52 min** today: 38 min on the live cap (2/2) behind infra/deps, 14 min on back-1's permission. From the Governor log, 11:02–11:40, and `run:checkout` ↗." with links `run checkout ↗` and `back-1 ↗`; status `working · 1 event` | ① "Ask in words: ⌘L, ⏎" ② "It answers from the live state, with links to what it read" ③ "Status: asleep → waking → working → asleep after the idle grace (2 min)" ④ "Cards still work if it is down" | `⌘L` · type · `⏎` · `Esc` |
| 4 | **Third time, a rule** · 15:20 | Rule proposal card §7.1; second state: `Added to shop-api policy · 15:21 · Undo u`; then the next matching call as `▸ allowed front-1: pnpm add zod · policy: r41` | ① "Proposed after the third identical answer" ② "One key writes it to the project's policy file, comments kept" ③ "The next match is a line, no card" ④ "Undo removes the rule while it is in the file" | `⌘⇧I` · `a` · `u` |
| 5 | **Watch, don't open** · 16:00 | as P4.2 §7 frame 5 (Mission control, watch strip, take-over strip); the chat column shows the status `working · 1 event` clickable | ① as before ② "The status opens the Overwatch's own session the same way" | `j l ⏎` · `Esc` · `⏎ ⏎` |

Callouts use the key-hint container for keys. The P4.2 §7 frames that said "awake" are replaced (F17).

---

## 13. States matrix (point 13) and what ui draws

Page 04.3. Header is always "Overwatch · <status> · Today ▾ · 🔍"; the input is always at the foot (unless noted).

| # | Case | Status | Thread | Input |
|---|---|---|---|---|
| 1 | **Asleep, no events today** | `asleep` | morning line, then "Nothing needs you." / "Cards appear here when an agent needs you." (P4.3 §12 row 1) | placeholder, hint `⌘L` |
| 2 | **Working on 2 events** | `working · 2 events` | morning line, one deciding card, one line just written | placeholder |
| 3 | **Waking** (first event of the day) | `waking…` | a card arriving, deciding marker | placeholder |
| 4 | **Down with open cards** | `down` + strip with Restart | 3 open cards (no deciding marker), annotations kept on one | hint `Down · messages are kept until it is back` |
| 5 | **Down, restarting** | `down` + strip, button `Restarting…` | as 4 | as 4 |
| 6 | **Back after down** | `waking…`, toast `Overwatch is back.` | | |
| 7 | **Off** | `off` | cards, no annotation, no deciding | hint `Off · turn it on in Settings` |
| 8 | **Paused** (Pause all) | `paused` | cards, no deciding | hint `Paused · it answers when it resumes` |
| 9 | **A user message pending** | `down` | `You · 14:05` + `Sent when the Overwatch is back` + `Remove` | as 4 |
| 10 | **Message sent while asleep** | `waking…` | `You · 14:05` + `waking…` | |
| 11 | **Waiting for the reply** | `working · 1 event` | the entry + `Overwatch is working on it · 12 s` | |
| 12 | **Long Overwatch answer** | `working` | clamped at 12 lines, `Show more`, one code block collapsed, source links at the foot | |
| 13 | **Answer with `reply_to` not adjacent** | | `↳ re: why is checkout slow today?` above the reply, two cards between | |
| 14 | **No reply** | `asleep` | `No reply. Ask again, or look at its session.` | |
| 15 | **Many lines in a row** | `asleep` | 12 consecutive Overwatch lines of every verb of §5.1, none grouped, two with `Undo u` | |
| 16 | **Deciding** | `working` | permission card, marker `Overwatch is deciding · 42 s left` | |
| 17 | **Deciding → handed over** | `asleep` | same card, `Waiting for you · just now`, annotation shown | |
| 18 | **Deciding → settled by the Overwatch** | | the line `▸ allowed … · Overwatch: …` replacing the card | |
| 19 | **Annotation variants** | | Permission (deny recommended), Spec gate (approve recommended), PR gate, Governor ask, a question it escalates (no highlight), a 280-char note clamped with `Show more`, sources `+ 2 more` | |
| 20 | **Why-asked line** (F3) | | a permission with the three variants of §4.5 | |
| 21 | **Lines per verb** | | each row of §5.1 collapsed; three expanded (answered with sources, denied by core, nudged) | |
| 22 | **Undo states** | | allowed by policy: collapsed with `Undo u`; expanded with button; in progress; done `undone 14:03`; gone `Can't be undone: back-1 already moved on`; failed `Could not undo: …`; answered: correction field open, sent `corrected 14:05`; denied by core; nudged | |
| 23 | **Rule proposal** | | every row of §7.2 | |
| 24 | **Past day** | `asleep` | P4.3 §12 row 10 with two Overwatch lines and one Overwatch text; footer "Read-only · Back to today t", no input | none |
| 25 | **Search with Overwatch hits** | | results including an Overwatch reply and a line matched by its source | input unchanged |
| 26 | **Status popover/tooltips** | | tooltips of §1 for `paused`, `down` | |
| 27 | **Reconnecting** (P4.3 row 18) with `down` | | only the reconnecting strip; Restart disabled | |
| 28 | **1280 shell** | `working · 2` truncated | one card with a recommended action wrapping to two button rows | |
| 29 | **Settings 04.7**: Overwatch section (on, off, invalid value, down); Permissions section (files with 2 projects, parse error, warnings, missing file, defaults, hold) | | | |
| 30 | **04.6**: planner row, report row | | | |
| 31 | **05**: keys of §11 | | | |
| 32 | **06.1**: J12 frames of §12 | | | |

Sections for ui to draw, in order of dependency: **§1 header** → **§2 input and entries** → **§3 deciding** → **§4 annotation** → **§5 lines** → **§6 undo** → **§7 proposal** → **§8 down/off/paused** → **§9 settings** → **§10 planner/report** → **§11 keys** → **§12 J12** → **§13 matrix**. The P4.3 footer board is marked "Replaced by the input in P4.4". Tokens: none new expected (the user bubble uses the accent wash; the status dots reuse text/accent/warning tokens).

---

## 14. Copy that changes in the code (for front/back)

- `ChatColumn.svelte`: `TITLE` "Decisions" → "Overwatch"; remove the footer string "Cards work here. The Overwatch joins in a later release."; the ⌘L search hint becomes `/`.
- Rust (back): agent-facing strings per spec §3.5/§3.8; notification titles unchanged (deciding never notifies).
- The spec's "Overwatch recommends:" form (P4.2 §8.1) becomes the label row of §4.1; update the P4.2 text reference only in the design file.
